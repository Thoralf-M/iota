// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{
    any::Any,
    convert::Infallible,
    net::{SocketAddr, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};

use async_graphql::{
    EmptySubscription, Schema, SchemaBuilder,
    extensions::{ApolloTracing, ExtensionFactory, Tracing},
};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    Extension, Router,
    body::Body,
    extract::{ConnectInfo, FromRef, Query as AxumQuery, State},
    http::{HeaderMap, StatusCode},
    middleware::{self},
    response::IntoResponse,
    routing::{MethodRouter, Route, get, post},
};
use chrono::Utc;
use http::{HeaderValue, Method, Request};
use iota_graphql_rpc_headers::LIMITS_HEADER;
use iota_metrics::spawn_monitored_task;
use iota_network_stack::callback::{CallbackLayer, MakeCallbackHandler, ResponseHandler};
use iota_package_resolver::{PackageStoreWithLruCache, Resolver};
use iota_sdk::IotaClientBuilder;
use tokio::{join, net::TcpListener, sync::OnceCell};
use tokio_util::sync::CancellationToken;
use tower::{Layer, Service};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    config::{
        ConnectionConfig, MAX_CONCURRENT_REQUESTS, RPC_TIMEOUT_ERR_SLEEP_RETRY_PERIOD,
        ServerConfig, ServiceConfig, Version,
    },
    context_data::db_data_provider::PgManager,
    data::{
        DataLoader, Db,
        package_resolver::{DbPackageStore, PackageResolver},
    },
    error::Error,
    extensions::{
        directive_checker::DirectiveChecker,
        feature_gate::FeatureGate,
        logger::Logger,
        query_limits_checker::{QueryLimitsChecker, ShowUsage},
        timeout::Timeout,
    },
    metrics::Metrics,
    mutation::Mutation,
    server::{
        compatibility_check::check_all_tables,
        exchange_rates_task::TriggerExchangeRatesTask,
        system_package_task::SystemPackageTask,
        version::{check_version_middleware, set_version_middleware},
        watermark_task::{Watermark, WatermarkLock, WatermarkTask},
    },
    types::{
        datatype::IMoveDatatype,
        move_object::IMoveObject,
        object::IObject,
        owner::IOwner,
        query::{IotaGraphQLSchema, Query},
    },
};

/// The default allowed maximum lag between the current timestamp and the
/// checkpoint timestamp.
const DEFAULT_MAX_CHECKPOINT_LAG: Duration = Duration::from_secs(300);

pub(crate) struct Server {
    router: Router,
    address: SocketAddr,
    watermark_task: WatermarkTask,
    system_package_task: SystemPackageTask,
    trigger_exchange_rates_task: TriggerExchangeRatesTask,
    state: AppState,
    db_reader: Db,
}

impl Server {
    /// Start the GraphQL service and any background tasks it is dependent on.
    /// When a cancellation signal is received, the method waits for all
    /// tasks to complete before returning.
    pub async fn run(mut self) -> Result<(), Error> {
        get_or_init_server_start_time().await;

        // Compatibility check
        info!("Starting compatibility check");
        check_all_tables(&self.db_reader).await?;
        info!("Compatibility check passed");

        // A handle that spawns a background task to periodically update the
        // `Watermark`, which consists of the checkpoint upper bound and current
        // epoch.
        let watermark_task = {
            info!("Starting watermark update task");
            spawn_monitored_task!(async move {
                self.watermark_task.run().await;
            })
        };

        // A handle that spawns a background task to evict system packages on epoch
        // changes.
        let system_package_task = {
            info!("Starting system package task");
            spawn_monitored_task!(async move {
                self.system_package_task.run().await;
            })
        };

        let trigger_exchange_rates_task = {
            info!("Starting trigger exchange rates task");
            spawn_monitored_task!(async move {
                self.trigger_exchange_rates_task.run().await;
            })
        };

        let server_task = {
            info!("Starting graphql service");
            let cancellation_token = self.state.cancellation_token.clone();
            let address = self.address;
            let router = self.router;
            spawn_monitored_task!(async move {
                axum::serve(
                    TcpListener::bind(address)
                        .await
                        .map_err(|e| Error::Internal(format!("listener bind failed: {}", e)))?,
                    router.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(async move {
                    cancellation_token.cancelled().await;
                    info!("Shutdown signal received, terminating graphql service");
                })
                .await
                .map_err(|e| Error::Internal(format!("Server run failed: {}", e)))
            })
        };

        // Wait for all tasks to complete. This ensures that the service doesn't fully
        // shut down until all tasks and the server have completed their
        // shutdown processes.
        let _ = join!(
            watermark_task,
            system_package_task,
            trigger_exchange_rates_task,
            server_task
        );

        Ok(())
    }
}

pub(crate) struct ServerBuilder {
    state: AppState,
    schema: SchemaBuilder<Query, Mutation, EmptySubscription>,
    router: Option<Router>,
    db_reader: Option<Db>,
    resolver: Option<PackageResolver>,
}

#[derive(Clone)]
pub(crate) struct AppState {
    connection: ConnectionConfig,
    service: ServiceConfig,
    metrics: Metrics,
    cancellation_token: CancellationToken,
    pub version: Version,
}

impl AppState {
    pub(crate) fn new(
        connection: ConnectionConfig,
        service: ServiceConfig,
        metrics: Metrics,
        cancellation_token: CancellationToken,
        version: Version,
    ) -> Self {
        Self {
            connection,
            service,
            metrics,
            cancellation_token,
            version,
        }
    }
}

impl FromRef<AppState> for ConnectionConfig {
    fn from_ref(app_state: &AppState) -> ConnectionConfig {
        app_state.connection.clone()
    }
}

impl FromRef<AppState> for Metrics {
    fn from_ref(app_state: &AppState) -> Metrics {
        app_state.metrics.clone()
    }
}

impl ServerBuilder {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            schema: schema_builder(),
            router: None,
            db_reader: None,
            resolver: None,
        }
    }

    pub fn address(&self) -> String {
        format!(
            "{}:{}",
            self.state.connection.host, self.state.connection.port
        )
    }

    pub fn context_data(mut self, context_data: impl Any + Send + Sync) -> Self {
        self.schema = self.schema.data(context_data);
        self
    }

    pub fn extension(mut self, extension: impl ExtensionFactory) -> Self {
        self.schema = self.schema.extension(extension);
        self
    }

    fn build_schema(self) -> Schema<Query, Mutation, EmptySubscription> {
        self.schema.finish()
    }

    /// Prepares the components of the server to be run. Finalizes the graphql
    /// schema, and expects the `Db` and `Router` to have been initialized.
    fn build_components(
        self,
    ) -> (
        String,
        Schema<Query, Mutation, EmptySubscription>,
        Db,
        PackageResolver,
        Router,
    ) {
        let address = self.address();
        let ServerBuilder {
            state: _,
            schema,
            db_reader,
            resolver,
            router,
        } = self;
        (
            address,
            schema.finish(),
            db_reader.expect("DB reader not initialized"),
            resolver.expect("Package resolver not initialized"),
            router.expect("Router not initialized"),
        )
    }

    fn init_router(&mut self) {
        if self.router.is_none() {
            let router: Router = Router::new()
                .route("/", post(graphql_handler))
                .route("/:version", post(graphql_handler))
                .route("/graphql", post(graphql_handler))
                .route("/graphql/:version", post(graphql_handler))
                .route("/health", get(health_check))
                .route("/graphql/health", get(health_check))
                .route("/graphql/:version/health", get(health_check))
                .with_state(self.state.clone())
                .route_layer(CallbackLayer::new(MetricsMakeCallbackHandler {
                    metrics: self.state.metrics.clone(),
                }));
            self.router = Some(router);
        }
    }

    pub fn route(mut self, path: &str, method_handler: MethodRouter) -> Self {
        self.init_router();
        self.router = self.router.map(|router| router.route(path, method_handler));
        self
    }

    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + 'static,
        L::Service: Service<Request<Body>> + Clone + Send + 'static,
        <L::Service as Service<Request<Body>>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request<Body>>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request<Body>>>::Future: Send + 'static,
    {
        self.init_router();
        self.router = self.router.map(|router| router.layer(layer));
        self
    }

    fn cors() -> Result<CorsLayer, Error> {
        let acl = match std::env::var("ACCESS_CONTROL_ALLOW_ORIGIN") {
            Ok(value) => {
                let allow_hosts = value
                    .split(',')
                    .map(HeaderValue::from_str)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| {
                        Error::Internal(
                            "Cannot resolve access control origin env variable".to_string(),
                        )
                    })?;
                AllowOrigin::list(allow_hosts)
            }
            _ => AllowOrigin::any(),
        };
        info!("Access control allow origin set to: {acl:?}");

        let cors = CorsLayer::new()
            // Allow `POST` when accessing the resource
            .allow_methods([Method::POST])
            // Allow requests from any origin
            .allow_origin(acl)
            .allow_headers([hyper::header::CONTENT_TYPE, LIMITS_HEADER.clone()]);
        Ok(cors)
    }

    /// Consumes the `ServerBuilder` to create a `Server` that can be run.
    pub fn build(self) -> Result<Server, Error> {
        let state = self.state.clone();
        let (address, schema, db_reader, resolver, router) = self.build_components();

        // Initialize the watermark background task struct.
        let watermark_task = WatermarkTask::new(
            db_reader.clone(),
            state.metrics.clone(),
            std::time::Duration::from_millis(state.service.background_tasks.watermark_update_ms),
            state.cancellation_token.clone(),
        );

        let system_package_task = SystemPackageTask::new(
            resolver,
            watermark_task.epoch_receiver(),
            state.cancellation_token.clone(),
        );

        let trigger_exchange_rates_task = TriggerExchangeRatesTask::new(
            db_reader.clone(),
            watermark_task.epoch_receiver(),
            state.cancellation_token.clone(),
        );

        let router = router
            .route_layer(middleware::from_fn_with_state(
                state.version,
                set_version_middleware,
            ))
            .route_layer(middleware::from_fn_with_state(
                state.version,
                check_version_middleware,
            ))
            .layer(axum::extract::Extension(schema))
            .layer(axum::extract::Extension(watermark_task.lock()))
            .layer(Self::cors()?);

        Ok(Server {
            router,
            address: address
                .parse()
                .map_err(|_| Error::Internal(format!("Failed to parse address {}", address)))?,
            watermark_task,
            system_package_task,
            trigger_exchange_rates_task,
            state,
            db_reader,
        })
    }

    /// Instantiate a `ServerBuilder` from a `ServerConfig`, typically called
    /// when building the graphql service for production usage.
    pub async fn from_config(
        config: &ServerConfig,
        version: &Version,
        cancellation_token: CancellationToken,
    ) -> Result<Self, Error> {
        // PROMETHEUS
        let prom_addr: SocketAddr = format!(
            "{}:{}",
            config.connection.prom_url, config.connection.prom_port
        )
        .parse()
        .map_err(|_| {
            Error::Internal(format!(
                "Failed to parse url {}, port {} into socket address",
                config.connection.prom_url, config.connection.prom_port
            ))
        })?;

        let registry_service = iota_metrics::start_prometheus_server(prom_addr);
        info!("Starting Prometheus HTTP endpoint at {}", prom_addr);
        let registry = registry_service.default_registry();
        registry
            .register(iota_metrics::uptime_metric(
                "graphql",
                version.full,
                "unknown",
            ))
            .unwrap();

        // METRICS
        let metrics = Metrics::new(&registry);
        let state = AppState::new(
            config.connection.clone(),
            config.service.clone(),
            metrics.clone(),
            cancellation_token,
            *version,
        );
        let mut builder = ServerBuilder::new(state);

        let iota_names_config = config.service.iota_names.clone();
        let zklogin_config = config.service.zklogin.clone();
        let reader = PgManager::reader_with_config(
            config.connection.db_url.clone(),
            config.connection.db_pool_size,
            // Bound each statement in a request with the overall request timeout, to bound DB
            // utilisation (in the worst case we will use 2x the request timeout time in DB wall
            // time).
            config.service.limits.request_timeout_ms.into(),
        )
        .map_err(|e| Error::Internal(format!("Failed to create pg connection pool: {}", e)))?;

        // DB
        let db = Db::new(
            reader.clone(),
            config.service.limits.clone(),
            metrics.clone(),
        );
        let loader = DataLoader::new(db.clone());
        let pg_conn_pool = PgManager::new(reader.clone());
        let package_store = DbPackageStore::new(loader.clone());
        let resolver = Arc::new(Resolver::new_with_limits(
            PackageStoreWithLruCache::new(package_store),
            config.service.limits.package_resolver_limits(),
        ));

        builder.db_reader = Some(db.clone());
        builder.resolver = Some(resolver.clone());

        // SDK for talking to fullnode. Used for executing transactions only
        // TODO: fail fast if no url, once we enable mutations fully
        let iota_sdk_client = if let Some(url) = &config.tx_exec_full_node.node_rpc_url {
            Some(
                IotaClientBuilder::default()
                    .request_timeout(RPC_TIMEOUT_ERR_SLEEP_RETRY_PERIOD)
                    .max_concurrent_requests(MAX_CONCURRENT_REQUESTS)
                    .build(url)
                    .await
                    .map_err(|e| {
                        Error::Internal(format!(
                            "Failed to connect to fullnode {e}. Is the node server running?"
                        ))
                    })?,
            )
        } else {
            warn!(
                "No fullnode url found in config. `dryRunTransactionBlock` and `executeTransactionBlock` will not work"
            );
            None
        };

        builder = builder
            .context_data(config.service.clone())
            .context_data(loader)
            .context_data(db)
            .context_data(pg_conn_pool)
            .context_data(resolver)
            .context_data(iota_sdk_client)
            .context_data(iota_names_config)
            .context_data(zklogin_config)
            .context_data(metrics.clone())
            .context_data(config.clone());

        if config.internal_features.feature_gate {
            builder = builder.extension(FeatureGate);
        }

        if config.internal_features.logger {
            builder = builder.extension(Logger::default());
        }

        if config.internal_features.query_limits_checker {
            builder = builder.extension(QueryLimitsChecker);
        }

        if config.internal_features.directive_checker {
            builder = builder.extension(DirectiveChecker);
        }

        if config.internal_features.query_timeout {
            builder = builder.extension(Timeout);
        }

        if config.internal_features.tracing {
            builder = builder.extension(Tracing);
        }

        if config.internal_features.apollo_tracing {
            builder = builder.extension(ApolloTracing);
        }

        // TODO: uncomment once impl
        // if config.internal_features.open_telemetry { }

        Ok(builder)
    }
}

fn schema_builder() -> SchemaBuilder<Query, Mutation, EmptySubscription> {
    async_graphql::Schema::build(Query, Mutation, EmptySubscription)
        .register_output_type::<IMoveObject>()
        .register_output_type::<IObject>()
        .register_output_type::<IOwner>()
        .register_output_type::<IMoveDatatype>()
}

/// Return the string representation of the schema used by this server.
pub fn export_schema() -> String {
    schema_builder().finish().sdl()
}

/// Entry point for graphql requests. Each request is stamped with a unique ID,
/// a `ShowUsage` flag if set in the request headers, and the watermark as set
/// by the background task.
async fn graphql_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    schema: Extension<IotaGraphQLSchema>,
    Extension(watermark_lock): Extension<WatermarkLock>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> (axum::http::Extensions, GraphQLResponse) {
    let mut req = req.into_inner();
    req.data.insert(Uuid::new_v4());
    if headers.contains_key(ShowUsage::name()) {
        req.data.insert(ShowUsage)
    }
    // Capture the IP address of the client
    // Note: if a load balancer is used it must be configured to forward the client
    // IP address
    req.data.insert(addr);

    req.data.insert(Watermark::new(watermark_lock).await);

    let result = schema.execute(req).await;

    // If there are errors, insert them as an extension so that the Metrics callback
    // handler can pull it out later.
    let mut extensions = axum::http::Extensions::new();
    if result.is_err() {
        extensions.insert(GraphqlErrors(std::sync::Arc::new(result.errors.clone())));
    };
    (extensions, result.into())
}

#[derive(Clone)]
struct MetricsMakeCallbackHandler {
    metrics: Metrics,
}

impl MakeCallbackHandler for MetricsMakeCallbackHandler {
    type Handler = MetricsCallbackHandler;

    fn make_handler(&self, _request: &http::request::Parts) -> Self::Handler {
        let start = Instant::now();
        let metrics = self.metrics.clone();

        metrics.request_metrics.inflight_requests.inc();
        metrics.inc_num_queries();

        MetricsCallbackHandler { metrics, start }
    }
}

struct MetricsCallbackHandler {
    metrics: Metrics,
    start: Instant,
}

impl ResponseHandler for MetricsCallbackHandler {
    fn on_response(self, response: &http::response::Parts) {
        if let Some(errors) = response.extensions.get::<GraphqlErrors>() {
            self.metrics.inc_errors(&errors.0);
        }
    }

    fn on_error<E>(self, _error: &E) {
        // Do nothing if the whole service errored
        //
        // in Axum this isn't possible since all services are required to have
        // an error type of Infallible
    }
}

impl Drop for MetricsCallbackHandler {
    fn drop(&mut self) {
        self.metrics.query_latency(self.start.elapsed());
        self.metrics.request_metrics.inflight_requests.dec();
    }
}

#[derive(Debug, Clone)]
struct GraphqlErrors(std::sync::Arc<Vec<async_graphql::ServerError>>);

/// Connect via a TCPStream to the DB to check if it is alive
async fn db_health_check(State(connection): State<ConnectionConfig>) -> StatusCode {
    let Ok(url) = reqwest::Url::parse(connection.db_url.as_str()) else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };

    let Some(host) = url.host_str() else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };

    let tcp_url = if let Some(port) = url.port() {
        format!("{host}:{port}")
    } else {
        host.to_string()
    };

    if TcpStream::connect(tcp_url).is_err() {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    }
}

#[derive(serde::Deserialize)]
struct HealthParam {
    max_checkpoint_lag_ms: Option<u64>,
}

/// Endpoint for querying the health of the service.
/// It returns 500 for any internal error, including not connecting to the DB,
/// and 504 if the checkpoint timestamp is too far behind the current timestamp
/// as per the max checkpoint timestamp lag query parameter, or the default
/// value if not provided.
async fn health_check(
    State(connection): State<ConnectionConfig>,
    Extension(watermark_lock): Extension<WatermarkLock>,
    AxumQuery(query_params): AxumQuery<HealthParam>,
) -> StatusCode {
    let db_health_check = db_health_check(axum::extract::State(connection)).await;
    if db_health_check != StatusCode::OK {
        return db_health_check;
    }

    let max_checkpoint_lag_ms = query_params
        .max_checkpoint_lag_ms
        .map(Duration::from_millis)
        .unwrap_or_else(|| DEFAULT_MAX_CHECKPOINT_LAG);

    let checkpoint_timestamp =
        Duration::from_millis(watermark_lock.read().await.checkpoint_timestamp_ms);

    let now_millis = Utc::now().timestamp_millis();

    // Check for negative timestamp or conversion failure
    let now: Duration = match u64::try_from(now_millis) {
        Ok(val) => Duration::from_millis(val),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    if (now - checkpoint_timestamp) > max_checkpoint_lag_ms {
        return StatusCode::GATEWAY_TIMEOUT;
    }

    db_health_check
}

// One server per proc, so this is okay
async fn get_or_init_server_start_time() -> &'static Instant {
    static ONCE: OnceCell<Instant> = OnceCell::const_new();
    ONCE.get_or_init(|| async move { Instant::now() }).await
}

pub mod tests {
    use std::{sync::Arc, time::Duration};

    use async_graphql::{
        Response,
        extensions::{Extension, ExtensionContext, NextExecute},
    };
    use iota_sdk::{IotaClient, wallet_context::WalletContext};
    use iota_types::transaction::TransactionData;
    use uuid::Uuid;

    use super::*;
    use crate::{
        config::{ConnectionConfig, Limits, ServiceConfig, Version},
        context_data::db_data_provider::PgManager,
        extensions::{query_limits_checker::QueryLimitsChecker, timeout::Timeout},
    };

    /// Prepares a schema for tests dealing with extensions. Returns a
    /// `ServerBuilder` that can be further extended with `context_data` and
    /// `extension` for testing.
    fn prep_schema(
        connection_config: Option<ConnectionConfig>,
        service_config: Option<ServiceConfig>,
    ) -> ServerBuilder {
        let connection_config = connection_config.unwrap_or_default();
        let service_config = service_config.unwrap_or_default();

        let reader = PgManager::reader_with_config(
            connection_config.db_url.clone(),
            connection_config.db_pool_size,
            service_config.limits.request_timeout_ms.into(),
        )
        .expect("Failed to create pg connection pool");

        let version = Version::for_testing();
        let metrics = metrics();
        let db = Db::new(
            reader.clone(),
            service_config.limits.clone(),
            metrics.clone(),
        );
        let pg_conn_pool = PgManager::new(reader);
        let cancellation_token = CancellationToken::new();
        let watermark = Watermark {
            checkpoint: 1,
            checkpoint_timestamp_ms: 1,
            epoch: 0,
        };
        let state = AppState::new(
            connection_config.clone(),
            service_config.clone(),
            metrics.clone(),
            cancellation_token.clone(),
            version,
        );
        ServerBuilder::new(state)
            .context_data(db)
            .context_data(pg_conn_pool)
            .context_data(service_config)
            .context_data(query_id())
            .context_data(ip_address())
            .context_data(watermark)
            .context_data(metrics)
    }

    fn metrics() -> Metrics {
        let binding_address: SocketAddr = "0.0.0.0:9185".parse().unwrap();
        let registry = iota_metrics::start_prometheus_server(binding_address).default_registry();
        Metrics::new(&registry)
    }

    fn ip_address() -> SocketAddr {
        let binding_address: SocketAddr = "0.0.0.0:51515".parse().unwrap();
        binding_address
    }

    fn query_id() -> Uuid {
        Uuid::new_v4()
    }

    pub async fn test_timeout_impl(wallet: &WalletContext) {
        struct TimedExecuteExt {
            pub min_req_delay: Duration,
        }

        impl ExtensionFactory for TimedExecuteExt {
            fn create(&self) -> Arc<dyn Extension> {
                Arc::new(TimedExecuteExt {
                    min_req_delay: self.min_req_delay,
                })
            }
        }

        #[async_trait::async_trait]
        impl Extension for TimedExecuteExt {
            async fn execute(
                &self,
                ctx: &ExtensionContext<'_>,
                operation_name: Option<&str>,
                next: NextExecute<'_>,
            ) -> Response {
                tokio::time::sleep(self.min_req_delay).await;
                next.run(ctx, operation_name).await
            }
        }

        async fn test_timeout(
            delay: Duration,
            timeout: Duration,
            query: &str,
            iota_client: &IotaClient,
        ) -> Response {
            let mut cfg = ServiceConfig::default();
            cfg.limits.request_timeout_ms = timeout.as_millis() as u32;
            cfg.limits.mutation_timeout_ms = timeout.as_millis() as u32;

            let schema = prep_schema(None, Some(cfg))
                .context_data(Some(iota_client.clone()))
                .extension(Timeout)
                .extension(TimedExecuteExt {
                    min_req_delay: delay,
                })
                .build_schema();

            schema.execute(query).await
        }

        let query = "{ chainIdentifier }";
        let timeout = Duration::from_millis(1000);
        let delay = Duration::from_millis(100);
        let iota_client = wallet.get_client().await.unwrap();

        test_timeout(delay, timeout, query, &iota_client)
            .await
            .into_result()
            .expect("Should complete successfully");

        // Should timeout
        let errs: Vec<_> = test_timeout(delay, delay, query, &iota_client)
            .await
            .into_result()
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect();
        let exp = format!("Query request timed out. Limit: {}s", delay.as_secs_f32());
        assert_eq!(errs, vec![exp]);

        // Should timeout for mutation
        // Create a transaction and sign it, and use the tx_bytes + signatures for the
        // GraphQL executeTransactionBlock mutation call.
        let addresses = wallet.get_addresses();
        let gas = wallet
            .get_one_gas_object_owned_by_address(addresses[0])
            .await
            .unwrap();
        let tx_data = TransactionData::new_transfer_iota(
            addresses[1],
            addresses[0],
            Some(1000),
            gas.unwrap(),
            1_000_000,
            wallet.get_reference_gas_price().await.unwrap(),
        );

        let tx = wallet.sign_transaction(&tx_data);
        let (tx_bytes, signatures) = tx.to_tx_bytes_and_signatures();

        let signature_base64 = &signatures[0];
        let query = format!(
            r#"
            mutation {{
              executeTransactionBlock(txBytes: "{}", signatures: "{}") {{
                effects {{
                  status
                }}
              }}
            }}"#,
            tx_bytes.encoded(),
            signature_base64.encoded()
        );
        let errs: Vec<_> = test_timeout(delay, delay, &query, &iota_client)
            .await
            .into_result()
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect();
        let exp = format!(
            "Mutation request timed out. Limit: {}s",
            delay.as_secs_f32()
        );
        assert_eq!(errs, vec![exp]);
    }

    pub async fn test_query_depth_limit_impl() {
        async fn exec_query_depth_limit(depth: u32, query: &str) -> Response {
            let service_config = ServiceConfig {
                limits: Limits {
                    max_query_depth: depth,
                    ..Default::default()
                },
                ..Default::default()
            };

            let schema = prep_schema(None, Some(service_config))
                .extension(QueryLimitsChecker)
                .build_schema();
            schema.execute(query).await
        }

        exec_query_depth_limit(1, "{ chainIdentifier }")
            .await
            .into_result()
            .expect("Should complete successfully");

        exec_query_depth_limit(
            5,
            "{ chainIdentifier protocolConfig { configs { value key }} }",
        )
        .await
        .into_result()
        .expect("Should complete successfully");

        // Should fail
        let errs: Vec<_> = exec_query_depth_limit(0, "{ chainIdentifier }")
            .await
            .into_result()
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect();

        assert_eq!(errs, vec!["Query nesting is over 0".to_string()]);
        let errs: Vec<_> = exec_query_depth_limit(
            2,
            "{ chainIdentifier protocolConfig { configs { value key }} }",
        )
        .await
        .into_result()
        .unwrap_err()
        .into_iter()
        .map(|e| e.message)
        .collect();
        assert_eq!(errs, vec!["Query nesting is over 2".to_string()]);
    }

    pub async fn test_query_node_limit_impl() {
        async fn exec_query_node_limit(nodes: u32, query: &str) -> Response {
            let service_config = ServiceConfig {
                limits: Limits {
                    max_query_nodes: nodes,
                    ..Default::default()
                },
                ..Default::default()
            };

            let schema = prep_schema(None, Some(service_config))
                .extension(QueryLimitsChecker)
                .build_schema();
            schema.execute(query).await
        }

        exec_query_node_limit(1, "{ chainIdentifier }")
            .await
            .into_result()
            .expect("Should complete successfully");

        exec_query_node_limit(
            5,
            "{ chainIdentifier protocolConfig { configs { value key }} }",
        )
        .await
        .into_result()
        .expect("Should complete successfully");

        // Should fail
        let err: Vec<_> = exec_query_node_limit(0, "{ chainIdentifier }")
            .await
            .into_result()
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect();
        assert_eq!(err, vec!["Query has over 0 nodes".to_string()]);

        let err: Vec<_> = exec_query_node_limit(
            4,
            "{ chainIdentifier protocolConfig { configs { value key }} }",
        )
        .await
        .into_result()
        .unwrap_err()
        .into_iter()
        .map(|e| e.message)
        .collect();
        assert_eq!(err, vec!["Query has over 4 nodes".to_string()]);
    }

    pub async fn test_query_default_page_limit_impl(connection_config: ConnectionConfig) {
        let service_config = ServiceConfig {
            limits: Limits {
                default_page_size: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let schema = prep_schema(Some(connection_config), Some(service_config)).build_schema();

        let resp = schema
            .execute("{ checkpoints { nodes { sequenceNumber } } }")
            .await;
        let data = resp.data.clone().into_json().unwrap();
        let checkpoints = data
            .get("checkpoints")
            .unwrap()
            .get("nodes")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            checkpoints.len(),
            1,
            "Checkpoints should have exactly one element"
        );

        let resp = schema
            .execute("{ checkpoints(first: 2) { nodes { sequenceNumber } } }")
            .await;
        let data = resp.data.clone().into_json().unwrap();
        let checkpoints = data
            .get("checkpoints")
            .unwrap()
            .get("nodes")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            checkpoints.len(),
            2,
            "Checkpoints should return two elements"
        );
    }

    pub async fn test_query_max_page_limit_impl() {
        let schema = prep_schema(None, None).build_schema();

        schema
            .execute("{ objects(first: 1) { nodes { version } } }")
            .await
            .into_result()
            .expect("Should complete successfully");

        // Should fail
        let err: Vec<_> = schema
            .execute("{ objects(first: 51) { nodes { version } } }")
            .await
            .into_result()
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect();
        assert_eq!(
            err,
            vec!["Connection's page size of 51 exceeds max of 50".to_string()]
        );
    }

    pub async fn test_query_complexity_metrics_impl() {
        let server_builder = prep_schema(None, None);
        let metrics = server_builder.state.metrics.clone();
        let schema = server_builder
            .extension(QueryLimitsChecker) // QueryLimitsChecker is where we actually set the metrics
            .build_schema();

        schema
            .execute("{ chainIdentifier }")
            .await
            .into_result()
            .expect("Should complete successfully");

        let req_metrics = metrics.request_metrics;
        assert_eq!(req_metrics.input_nodes.get_sample_count(), 1);
        assert_eq!(req_metrics.output_nodes.get_sample_count(), 1);
        assert_eq!(req_metrics.query_depth.get_sample_count(), 1);
        assert_eq!(req_metrics.input_nodes.get_sample_sum(), 1.);
        assert_eq!(req_metrics.output_nodes.get_sample_sum(), 1.);
        assert_eq!(req_metrics.query_depth.get_sample_sum(), 1.);

        schema
            .execute("{ chainIdentifier protocolConfig { configs { value key }} }")
            .await
            .into_result()
            .expect("Should complete successfully");

        assert_eq!(req_metrics.input_nodes.get_sample_count(), 2);
        assert_eq!(req_metrics.output_nodes.get_sample_count(), 2);
        assert_eq!(req_metrics.query_depth.get_sample_count(), 2);
        assert_eq!(req_metrics.input_nodes.get_sample_sum(), 2. + 4.);
        assert_eq!(req_metrics.output_nodes.get_sample_sum(), 2. + 4.);
        assert_eq!(req_metrics.query_depth.get_sample_sum(), 1. + 3.);
    }

    pub async fn test_health_check_impl() {
        let server_builder = prep_schema(None, None);
        let url = format!(
            "http://{}:{}/health",
            server_builder.state.connection.host, server_builder.state.connection.port
        );
        server_builder.build_schema();

        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let url_with_param = format!("{}?max_checkpoint_lag_ms=1", url);
        let resp = reqwest::get(&url_with_param).await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    }
}
