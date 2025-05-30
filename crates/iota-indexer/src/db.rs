// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use anyhow::anyhow;
use diesel::{
    PgConnection,
    connection::BoxableConnection,
    query_dsl::RunQueryDsl,
    r2d2::{ConnectionManager, Pool, PooledConnection, R2D2Connection},
};

use crate::errors::IndexerError;

pub type ConnectionPool = Pool<ConnectionManager<PgConnection>>;
pub type PoolConnection = PooledConnection<ConnectionManager<PgConnection>>;

#[derive(Debug, Clone, Copy)]
pub struct ConnectionPoolConfig {
    pub pool_size: u32,
    pub connection_timeout: Duration,
    pub statement_timeout: Duration,
}

impl ConnectionPoolConfig {
    const DEFAULT_POOL_SIZE: u32 = 100;
    const DEFAULT_CONNECTION_TIMEOUT: u64 = 3600;
    const DEFAULT_STATEMENT_TIMEOUT: u64 = 3600;

    fn connection_config(&self) -> ConnectionConfig {
        ConnectionConfig {
            statement_timeout: self.statement_timeout,
            read_only: false,
        }
    }

    pub fn set_pool_size(&mut self, size: u32) {
        self.pool_size = size;
    }

    pub fn set_connection_timeout(&mut self, timeout: Duration) {
        self.connection_timeout = timeout;
    }

    pub fn set_statement_timeout(&mut self, timeout: Duration) {
        self.statement_timeout = timeout;
    }
}

impl Default for ConnectionPoolConfig {
    fn default() -> Self {
        let db_pool_size = std::env::var("DB_POOL_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(Self::DEFAULT_POOL_SIZE);
        let conn_timeout_secs = std::env::var("DB_CONNECTION_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(Self::DEFAULT_CONNECTION_TIMEOUT);
        let statement_timeout_secs = std::env::var("DB_STATEMENT_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(Self::DEFAULT_STATEMENT_TIMEOUT);

        Self {
            pool_size: db_pool_size,
            connection_timeout: Duration::from_secs(conn_timeout_secs),
            statement_timeout: Duration::from_secs(statement_timeout_secs),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConnectionConfig {
    pub statement_timeout: Duration,
    pub read_only: bool,
}

impl<T: R2D2Connection + 'static> diesel::r2d2::CustomizeConnection<T, diesel::r2d2::Error>
    for ConnectionConfig
{
    fn on_acquire(&self, _conn: &mut T) -> std::result::Result<(), diesel::r2d2::Error> {
        _conn
            .as_any_mut()
            .downcast_mut::<diesel::PgConnection>()
            .map_or_else(
                || {
                    Err(diesel::r2d2::Error::QueryError(
                        diesel::result::Error::DeserializationError(
                            "Failed to downcast connection to PgConnection"
                                .to_string()
                                .into(),
                        ),
                    ))
                },
                |pg_conn| {
                    diesel::sql_query(format!(
                        "SET statement_timeout = {}",
                        self.statement_timeout.as_millis(),
                    ))
                    .execute(pg_conn)
                    .map_err(diesel::r2d2::Error::QueryError)?;

                    if self.read_only {
                        diesel::sql_query("SET default_transaction_read_only = 't'")
                            .execute(pg_conn)
                            .map_err(diesel::r2d2::Error::QueryError)?;
                    }
                    Ok(())
                },
            )?;
        Ok(())
    }
}

pub fn new_connection_pool(
    db_url: &str,
    pool_size: Option<u32>,
) -> Result<ConnectionPool, IndexerError> {
    let pool_config = ConnectionPoolConfig::default();
    new_connection_pool_with_config(db_url, pool_size, pool_config)
}

pub fn new_connection_pool_with_config(
    db_url: &str,
    pool_size: Option<u32>,
    pool_config: ConnectionPoolConfig,
) -> Result<ConnectionPool, IndexerError> {
    let manager = ConnectionManager::<PgConnection>::new(db_url);

    let pool_size = pool_size.unwrap_or(pool_config.pool_size);
    Pool::builder()
        .max_size(pool_size)
        .connection_timeout(pool_config.connection_timeout)
        .connection_customizer(Box::new(pool_config.connection_config()))
        .build(manager)
        .map_err(|e| {
            IndexerError::PgConnectionPoolInit(format!(
                "Failed to initialize connection pool for {db_url} with error: {e:?}"
            ))
        })
}

pub fn get_pool_connection(pool: &ConnectionPool) -> Result<PoolConnection, IndexerError> {
    pool.get().map_err(|e| {
        IndexerError::PgPoolConnection(format!(
            "Failed to get connection from PG connection pool with error: {:?}",
            e
        ))
    })
}

pub fn reset_database(conn: &mut PoolConnection) -> Result<(), anyhow::Error> {
    {
        conn.as_any_mut()
            .downcast_mut::<PoolConnection>()
            .map_or_else(
                || Err(anyhow!("Failed to downcast connection to PgConnection")),
                |pg_conn| {
                    setup_postgres::reset_database(pg_conn)?;
                    Ok(())
                },
            )?;
    }
    Ok(())
}

pub mod setup_postgres {
    use anyhow::anyhow;
    use diesel::{RunQueryDsl, migration::MigrationSource};
    use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
    use prometheus::Registry;
    use secrecy::ExposeSecret;
    use tracing::{error, info};

    use crate::{
        IndexerConfig,
        db::{PoolConnection, get_pool_connection, new_connection_pool},
        errors::IndexerError,
        indexer::Indexer,
        metrics::IndexerMetrics,
        store::{PgIndexerAnalyticalStore, PgIndexerStore},
    };

    const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/pg");

    pub fn reset_database(conn: &mut PoolConnection) -> Result<(), anyhow::Error> {
        info!("Resetting PG database ...");

        let drop_all_tables = "
        DO $$ DECLARE
            r RECORD;
        BEGIN
        FOR r IN (SELECT tablename FROM pg_tables WHERE schemaname = 'public')
            LOOP
                EXECUTE 'DROP TABLE IF EXISTS ' || quote_ident(r.tablename) || ' CASCADE';
            END LOOP;
        END $$;";
        diesel::sql_query(drop_all_tables).execute(conn)?;
        info!("Dropped all tables.");

        let drop_all_procedures = "
        DO $$ DECLARE
            r RECORD;
        BEGIN
            FOR r IN (SELECT proname, oidvectortypes(proargtypes) as argtypes
                      FROM pg_proc INNER JOIN pg_namespace ns ON (pg_proc.pronamespace = ns.oid)
                      WHERE ns.nspname = 'public' AND prokind = 'p')
            LOOP
                EXECUTE 'DROP PROCEDURE IF EXISTS ' || quote_ident(r.proname) || '(' || r.argtypes || ') CASCADE';
            END LOOP;
        END $$;";
        diesel::sql_query(drop_all_procedures).execute(conn)?;
        info!("Dropped all procedures.");

        let drop_all_functions = "
        DO $$ DECLARE
            r RECORD;
        BEGIN
            FOR r IN (SELECT proname, oidvectortypes(proargtypes) as argtypes
                      FROM pg_proc INNER JOIN pg_namespace ON (pg_proc.pronamespace = pg_namespace.oid)
                      WHERE pg_namespace.nspname = 'public' AND prokind = 'f')
            LOOP
                EXECUTE 'DROP FUNCTION IF EXISTS ' || quote_ident(r.proname) || '(' || r.argtypes || ') CASCADE';
            END LOOP;
        END $$;";
        diesel::sql_query(drop_all_functions).execute(conn)?;
        info!("Dropped all functions.");

        diesel::sql_query(
            "
        CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
            version VARCHAR(50) PRIMARY KEY,
            run_on TIMESTAMP NOT NULL DEFAULT NOW()
        )",
        )
        .execute(conn)?;
        info!("Created __diesel_schema_migrations table.");

        conn.run_migrations(&MIGRATIONS.migrations().unwrap())
            .map_err(|e| anyhow!("Failed to run migrations {e}"))?;
        info!("Reset database complete.");
        Ok(())
    }

    pub async fn setup(
        indexer_config: IndexerConfig,
        registry: Registry,
    ) -> Result<(), IndexerError> {
        let db_url_secret = indexer_config.get_db_url().map_err(|e| {
            IndexerError::PgPoolConnection(format!(
                "Failed parsing database url with error {:?}",
                e
            ))
        })?;
        let db_url = db_url_secret.expose_secret();
        let blocking_cp = new_connection_pool(db_url, None).map_err(|e| {
            error!(
                "Failed creating Postgres connection pool with error {:?}",
                e
            );
            e
        })?;
        info!("Postgres database connection pool is created at {}", db_url);
        let mut conn = get_pool_connection(&blocking_cp).map_err(|e| {
            error!(
                "Failed getting Postgres connection from connection pool with error {:?}",
                e
            );
            e
        })?;
        if indexer_config.reset_db {
            reset_database(&mut conn).map_err(|e| {
                let db_err_msg = format!(
                    "Failed resetting database with url: {:?} and error: {:?}",
                    db_url, e
                );
                error!("{}", db_err_msg);
                IndexerError::PostgresReset(db_err_msg)
            })?;
            info!("Reset Postgres database complete.");
        } else {
            conn.run_pending_migrations(MIGRATIONS)
                .map_err(|e| anyhow!("Failed to run pending migrations {e}"))?;
            info!("Database migrations are up to date.");
        }
        let indexer_metrics = IndexerMetrics::new(&registry);
        iota_metrics::init_metrics(&registry);

        let report_cp = blocking_cp.clone();
        let report_metrics = indexer_metrics.clone();
        tokio::spawn(async move {
            loop {
                let cp_state = report_cp.state();
                info!(
                    "DB connection pool size: {}, with idle conn: {}.",
                    cp_state.connections, cp_state.idle_connections
                );
                report_metrics
                    .db_conn_pool_size
                    .set(cp_state.connections as i64);
                report_metrics
                    .idle_db_conn
                    .set(cp_state.idle_connections as i64);
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            }
        });
        if indexer_config.fullnode_sync_worker {
            let store = PgIndexerStore::new(blocking_cp, indexer_metrics.clone());
            return Indexer::start_writer(&indexer_config, store, indexer_metrics).await;
        } else if indexer_config.rpc_server_worker {
            return Indexer::start_reader(&indexer_config, &registry, db_url.to_string()).await;
        } else if indexer_config.analytical_worker {
            let store = PgIndexerAnalyticalStore::new(blocking_cp);
            return Indexer::start_analytical_worker(store, indexer_metrics.clone()).await;
        }
        Ok(())
    }
}
