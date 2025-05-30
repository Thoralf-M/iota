// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{path::PathBuf, time::Duration};

use clap::Parser;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[arg(long, help = "Number of seconds to surf, default to 30")]
    pub run_duration: Option<u64>,

    #[arg(long, help = "Number of seconds per epoch, default to 15")]
    pub epoch_duration: Option<u64>,

    #[arg(long, help = "List of package paths to surf")]
    packages: Vec<PathBuf>,
}

const DEFAULT_RUN_DURATION: u64 = 30;
const DEFAULT_EPOCH_DURATION: u64 = 15;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.packages.is_empty() {
        eprintln!("At least one package is required");
        return;
    }

    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_log_level("off,iota_surfer=info")
        .with_env()
        .init();

    let results = iota_surfer::run(
        Duration::from_secs(args.run_duration.unwrap_or(DEFAULT_RUN_DURATION)),
        Duration::from_secs(args.run_duration.unwrap_or(DEFAULT_EPOCH_DURATION)),
        args.packages,
    )
    .await;
    results.print_stats();
    info!("Finished surfing");
}
