// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use anyhow::Result;
use iota_data_ingestion_core::Worker;
use iota_rest_api::{CheckpointData, CheckpointTransaction};
use iota_types::{
    base_types::ObjectID, effects::TransactionEffects, transaction::TransactionDataAPI,
};
use tokio::sync::Mutex;

use crate::{
    FileType,
    handlers::{AnalyticsHandler, InputObjectTracker, ObjectStatusTracker},
    tables::TransactionObjectEntry,
};

pub struct TransactionObjectsHandler {
    state: Mutex<State>,
}

struct State {
    transaction_objects: Vec<TransactionObjectEntry>,
}

#[async_trait::async_trait]
impl Worker for TransactionObjectsHandler {
    type Message = ();
    type Error = anyhow::Error;

    async fn process_checkpoint(
        &self,
        checkpoint_data: Arc<CheckpointData>,
    ) -> Result<Self::Message, Self::Error> {
        let CheckpointData {
            checkpoint_summary,
            transactions: checkpoint_transactions,
            ..
        } = checkpoint_data.as_ref();
        let mut state = self.state.lock().await;
        for checkpoint_transaction in checkpoint_transactions {
            self.process_transaction(
                checkpoint_summary.epoch,
                checkpoint_summary.sequence_number,
                checkpoint_summary.timestamp_ms,
                checkpoint_transaction,
                &checkpoint_transaction.effects,
                &mut state,
            );
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl AnalyticsHandler<TransactionObjectEntry> for TransactionObjectsHandler {
    async fn read(&self) -> Result<Vec<TransactionObjectEntry>> {
        let mut state = self.state.lock().await;
        let cloned = state.transaction_objects.clone();
        state.transaction_objects.clear();
        Ok(cloned)
    }

    fn file_type(&self) -> Result<FileType> {
        Ok(FileType::TransactionObjects)
    }

    fn name(&self) -> &str {
        "transaction_objects"
    }
}

impl TransactionObjectsHandler {
    pub fn new() -> Self {
        TransactionObjectsHandler {
            state: Mutex::new(State {
                transaction_objects: vec![],
            }),
        }
    }
    fn process_transaction(
        &self,
        epoch: u64,
        checkpoint: u64,
        timestamp_ms: u64,
        checkpoint_transaction: &CheckpointTransaction,
        effects: &TransactionEffects,
        state: &mut State,
    ) {
        let transaction = &checkpoint_transaction.transaction;
        let transaction_digest = transaction.digest().base58_encode();
        let txn_data = transaction.transaction_data();
        let input_object_tracker = InputObjectTracker::new(txn_data);
        let object_status_tracker = ObjectStatusTracker::new(effects);
        // input
        txn_data
            .input_objects()
            .expect("Input objects must be valid")
            .iter()
            .map(|object| (object.object_id(), object.version().map(|v| v.value())))
            .for_each(|(object_id, version)| {
                self.process_transaction_object(
                    epoch,
                    checkpoint,
                    timestamp_ms,
                    transaction_digest.clone(),
                    &object_id,
                    version,
                    &input_object_tracker,
                    &object_status_tracker,
                    state,
                )
            });
        // output
        checkpoint_transaction
            .output_objects
            .iter()
            .map(|object| (object.id(), Some(object.version().value())))
            .for_each(|(object_id, version)| {
                self.process_transaction_object(
                    epoch,
                    checkpoint,
                    timestamp_ms,
                    transaction_digest.clone(),
                    &object_id,
                    version,
                    &input_object_tracker,
                    &object_status_tracker,
                    state,
                )
            });
    }
    // Transaction object data.
    // Builds a view of the object in input and output of a transaction.
    fn process_transaction_object(
        &self,
        epoch: u64,
        checkpoint: u64,
        timestamp_ms: u64,
        transaction_digest: String,
        object_id: &ObjectID,
        version: Option<u64>,
        input_object_tracker: &InputObjectTracker,
        object_status_tracker: &ObjectStatusTracker,
        state: &mut State,
    ) {
        let entry = TransactionObjectEntry {
            object_id: object_id.to_string(),
            version,
            transaction_digest,
            checkpoint,
            epoch,
            timestamp_ms,
            input_kind: input_object_tracker.get_input_object_kind(object_id),
            object_status: object_status_tracker.get_object_status(object_id),
        };
        state.transaction_objects.push(entry);
    }
}
