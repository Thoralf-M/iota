// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

// DO NOT MODIFY, Generated by ./scripts/execution-layer

use std::{path::PathBuf, sync::Arc};

pub use executor::Executor;
use iota_protocol_config::ProtocolConfig;
use iota_types::{error::IotaResult, metrics::BytecodeVerifierMetrics};
pub use verifier::Verifier;

pub mod executor;
pub mod verifier;

mod latest;

#[cfg(test)]
mod tests;

pub fn executor(
    protocol_config: &ProtocolConfig,
    silent: bool,
    enable_profiler: Option<PathBuf>,
) -> IotaResult<Arc<dyn Executor + Send + Sync>> {
    let version = protocol_config.execution_version_as_option().unwrap_or(1);
    Ok(match version {
        1 => Arc::new(latest::Executor::new(
            protocol_config,
            silent,
            enable_profiler,
        )?),

        v => panic!("Unsupported execution version {v}"),
    })
}

pub fn verifier<'m>(
    protocol_config: &ProtocolConfig,
    signing_limits: Option<(usize, usize)>,
    metrics: &'m Arc<BytecodeVerifierMetrics>,
) -> Box<dyn Verifier + 'm> {
    let version = protocol_config.execution_version_as_option().unwrap_or(1);
    let config = protocol_config.verifier_config(signing_limits);
    match version {
        1 => Box::new(latest::Verifier::new(config, metrics)),
        v => panic!("Unsupported execution version {v}"),
    }
}
