// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use iota_data_ingestion::{ArchivalConfig, ArchivalReducer, RelayWorker};
use iota_data_ingestion_core::{
    DataIngestionMetrics, IndexerExecutor, ReaderOptions, ShimProgressStore, WorkerPool,
};
use prometheus::Registry;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    remote_store_url: String,
    archive_url: String,
    archive_remote_store_options: Vec<(String, String)>,
    #[serde(default = "default_commit_file_size")]
    commit_file_size: usize,
    #[serde(default = "default_commit_duration_seconds")]
    commit_duration_seconds: u64,
}

fn default_commit_file_size() -> usize {
    268435456
}

fn default_commit_duration_seconds() -> u64 {
    600
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 2, "configuration yaml file is required");
    let config: Config = serde_yaml::from_str(&std::fs::read_to_string(&args[1])?)?;

    let archival_config = ArchivalConfig {
        remote_url: config.archive_url,
        remote_store_options: config.archive_remote_store_options,
        commit_file_size: config.commit_file_size,
        commit_duration_seconds: config.commit_duration_seconds,
    };

    let reducer = ArchivalReducer::new(archival_config).await?;
    let progress_store = ShimProgressStore(reducer.get_watermark().await?);
    let mut executor = IndexerExecutor::new(
        progress_store,
        1,
        DataIngestionMetrics::new(&Registry::new()),
        CancellationToken::new(),
    );
    let worker_pool = WorkerPool::new_with_reducer(
        RelayWorker,
        "archival".to_string(),
        1,
        Default::default(),
        reducer,
    );
    executor.register(worker_pool).await?;
    executor
        .run(
            tempfile::tempdir()?.into_path(),
            Some(config.remote_store_url),
            vec![],
            ReaderOptions::default(),
        )
        .await?;
    Ok(())
}
