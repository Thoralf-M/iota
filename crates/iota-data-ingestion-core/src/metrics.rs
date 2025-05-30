// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use prometheus::{IntGaugeVec, Registry, register_int_gauge_vec_with_registry};

#[derive(Clone)]
pub struct DataIngestionMetrics {
    pub data_ingestion_checkpoint: IntGaugeVec,
}

impl DataIngestionMetrics {
    pub fn new(registry: &Registry) -> Self {
        Self {
            data_ingestion_checkpoint: register_int_gauge_vec_with_registry!(
                "data_ingestion_checkpoint",
                "Number of uploaded checkpoints.",
                &["task"],
                registry,
            )
            .unwrap(),
        }
    }
}
