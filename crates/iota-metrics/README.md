# IOTA Metrics

Metrics are exposed on the `/metrics` endpoint of a running IOTA node.
They are then scraped by prometheus and displayed in the Grafana UI.

To only see metrics exposed on the endpoint `/metrics`, you need to:

- launch a local swarm via `RUST_LOG="off,iota_swarm=info,iota_node=info" cargo run --bin iota -- start --force-regenesis`
- search the beginning of the logs for `Prometheus` (the full log line should look like `Started Prometheus HTTP endpoint. To query metrics use curl -s http://127.0.0.1:51426/metrics` )
- run the curl command you found, containing the right port to reach the prometheus endpoint

To launch the full grafana UI, you need to:

- launch a private network via docker (See ../../dev-tools/iota-private-network/README.md)
- then launch the grafana UI via docker (See ../../dev-tools/grafana-local/README.md)
  You can then find the grafana UI at http://localhost:3000

## Implementation

The `iota-metrics` crate defines a `Metrics` struct with various [`IntGaugeVec`] metrics to monitor running tasks,
pending futures, channel sizes, and scope activities. A gauge is a type of metric that represents a single numerical value
that can go up or down.

```rust
#[derive(Debug)]
pub struct Metrics {
    pub tasks: IntGaugeVec,
    pub futures: IntGaugeVec,
    pub channels: IntGaugeVec,
    pub scope_iterations: IntGaugeVec,
    pub scope_duration_ns: IntGaugeVec,
    pub scope_entrance: IntGaugeVec,
}
```

These gauges are initialized as `IntGaugeVec` metrics in the `Metrics::new` function and added a given `prometheus::Registry`.

```rust
impl Metrics {
    fn new(registry: &Registry) -> Self {
        Self {
            tasks: register_int_gauge_vec_with_registry!(
                "monitored_tasks",
                "Number of running tasks per callsite.",
                &["callsite"],
                registry,
            )
            .unwrap(),
            futures: register_int_gauge_vec_with_registry!(
                "monitored_futures",
                "Number of pending futures per callsite.",
                &["callsite"],
                registry,
            )
            .unwrap(),
            channels: register_int_gauge_vec_with_registry!(
                "monitored_channels",
                "Size of channels.",
                &["name"],
                registry,
            )
            .unwrap(),
            // More metrics initialized here..
        }
    }
}
```

These metrics can only be initialized and accessed using `OnceCell`, ensuring they are only initialized once and are thread-safe.
Typically, the `init_metrics` function is called at the beginning of a main function or in a test setup.

```rust
static METRICS: OnceCell<Metrics> = OnceCell::new();

pub fn init_metrics(registry: &Registry) {
    let _ = METRICS
        .set(Metrics::new(registry))
        // this happens many times during tests
        .tap_err(|_| warn!("init_metrics registry overwritten"));
}

pub fn get_metrics() -> Option<&'static Metrics> {
    METRICS.get()
}
```

To monitor futures and tasks in a consistent and simple way, the crate defines multiple macros like `monitored_future!` and `spawn_monitored_task!`, which wrap a given future or task while updating metrics.
These macros call the global `get_metrics` function to retrieve the `Metrics` struct and update the relevant gauges.

To monitor code scopes, the `monitored_scope` function can create a named scope that keeps track of:

- The total iterations where the scope is called in the `monitored_scope_iterations` metric.
- The total duration of the scope in the `monitored_scope_duration_ns` metric. The total duration of the scope is updated when the scope is dropped, as shown below:

```rust
impl Drop for MonitoredScopeGuard {
    fn drop(&mut self) {
        self.metrics
            .scope_duration_ns
            .with_label_values(&[self.name])
            .add(self.timer.elapsed().as_nanos() as i64);
        self.metrics
            .scope_entrance
            .with_label_values(&[self.name])
            .dec();
    }
}
```

Monitored scopes are used in multiple parts of the node. For example, the `consensus_handler` module uses monitored scopes to track the duration and number of iterations of
the `handle_consensus_output` function:

```rust
    async fn handle_consensus_output(&mut self, consensus_output: ConsensusOutput) {
    let _scope = monitored_scope("HandleConsensusOutput");
    self.handle_consensus_output_internal(consensus_output)
        .await;
}
```

## `RegistryService`

To manage Prometheus registries with their metrics more easily, the crate provides a `RegistryService` struct with a default registry and a collection of additional registries identified by unique UUIDs:

```rust
/// A service to manage the prometheus registries. This service allow us to
/// create a new Registry on demand and keep it accessible for
/// processing/polling.
#[derive(Clone)]
pub struct RegistryService {
    // Holds a Registry that is supposed to be used
    default_registry: Registry,
    registries_by_id: Arc<DashMap<Uuid, Registry>>,
}
```

This `RegistryService` allows for the creation, addition, and removal of Prometheus registries and also provides a function to gather all metrics from all registries.
It works as follows:

```rust
        // Create a default registry
        let default_registry = Registry::new_custom(Some("default".to_string()), None).unwrap();

        // Create a registry service with the default registry
        let registry_service = RegistryService::new(default_registry.clone());
        let default_counter = IntCounter::new("counter", "counter_desc").unwrap();
        default_counter.inc();
        default_registry
            .register(Box::new(default_counter))
            .unwrap();

        // Create another registry and add a metric to it
        let registry_1 = Registry::new_custom(Some("consensus".to_string()), None).unwrap();
        registry_1
            .register(Box::new(
                IntCounter::new("counter_1", "counter_1_desc").unwrap(),
            ))
            .unwrap();

        // Add the new registry to the registry service
        let registry_1_id = registry_service.add(registry_1);

        // Gather all metrics from all registries
        let mut metrics = registry_service.gather_all();
        metrics.sort_by(|m1, m2| Ord::cmp(m1.get_name(), m2.get_name()));

        // There should be two metrics
        assert_eq!(metrics.len(), 2);

        // Check the first metric
        let metric_default = metrics.remove(0);
        assert_eq!(metric_default.get_name(), "default_counter");
        assert_eq!(metric_default.get_help(), "counter_desc");

        // Check the second metric
        let metric_2 = metrics.remove(0);
        assert_eq!(metric_2.get_name(), "consensus_counter_1");
        assert_eq!(metric_2.get_help(), "counter_1_desc");
```

## Exposing the Prometheus Metrics

In order to expose the Prometheus metrics, the `RegistryService` gets exposed by the `start_prometheus_server` function, which starts an `axum` HTTP server and serves the metrics by the `/metrics` endpoint from the registries.

```rust
pub fn start_prometheus_server(addr: SocketAddr) -> RegistryService {
    let registry = Registry::new();

    let registry_service = RegistryService::new(registry);

    if cfg!(msim) {
        // prometheus uses difficult-to-support features such as
        // TcpSocket::from_raw_fd(), so we can't yet run it in the simulator.
        warn!("not starting prometheus server in simulator");
        return registry_service;
    }

    let app = Router::new()
        .route(METRICS_ROUTE, get(metrics))
        .layer(Extension(registry_service.clone()));

    tokio::spawn(async move {
        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await
            .unwrap();
    });

    registry_service
}
```

Typically, the Prometheus server is started in the main function of the node as follows:

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_env()
        .init();

    ...

    let registry_service = iota_metrics::start_prometheus_server(
        format!(
            "{}:{}",
            config.client_metric_host, config.client_metric_port
        )
        .parse()
        .unwrap(),
    );
    let registry: Registry = registry_service.default_registry();
    iota_metrics::init_metrics(&registry);

    ...
}
```

Additionally, instead of exposing the metrics via HTTP only, the metrics can also be pushed, for example, to a given `push_url`.
The `iota-node` crate, for example, starts a push task `start_metrics_push_task` that pushes all metrics regularly to a given endpoint defined in the `NodeConfig`.
This push task assigns the current timestamp to each metric, encodes the metrics into the Protobuf format, adds compression and
pushes the compressed metrics data via an HTTP POST.

[`IntGaugeVec`]: https://docs.rs/prometheus/latest/prometheus/type.IntGaugeVec.html

### Measuring Latencies With Prometheus

Latency or any distribution can be formed with the Histogram Prometheus metric type.

A histogram samples observations (usually things like request duration or response sizes) and counts them in configurable buckets.
When custom buckets are not provided, histogram metric will be defined for default set of percentiles: `50, 95 and 99`.
E.g. for `checkpoint_exec_latency_us` defined as Histogram we can see in metrics:

```shell
# TYPE checkpoint_exec_latency_us_count counter
checkpoint_exec_latency_us_count 48294

# TYPE checkpoint_exec_latency_us_sum counter
checkpoint_exec_latency_us_sum 146438652

# TYPE checkpoint_exec_latency_us gauge
checkpoint_exec_latency_us{pct="50"} 13351
checkpoint_exec_latency_us{pct="95"} 29339
checkpoint_exec_latency_us{pct="99"} 35587
```

If we want to customize the histogram buckets, to create more sophisticated plots,
we can provide an additional argument to register macro:

```rust
const LATENCY_SEC_BUCKETS: &[f64] = &[0005, 0.001, 0.005, 0.01];

let request_latency = register_histogram_vec_with_registry!("transaction_manager_transaction_queue_age_s", "Description", LATENCY_SEC_BUCKETS.to_vec(), registry).unwrap()
```

Then metrics will be calculated for the provided buckets:

```shell
# TYPE transaction_manager_transaction_queue_age_s histogram
transaction_manager_transaction_queue_age_s_bucket{le="0.0005"} 2344
transaction_manager_transaction_queue_age_s_bucket{le="0.001"} 65467
transaction_manager_transaction_queue_age_s_bucket{le="0.005"} 158996
transaction_manager_transaction_queue_age_s_bucket{le="0.01"} 159274
...
transaction_manager_transaction_queue_age_s_bucket{le="+Inf"} 23441
```
