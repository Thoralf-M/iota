// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{path::Path, sync::Arc};

use anyhow::Result;
use fastcrypto::encoding::{Base64, Encoding};
use iota_data_ingestion_core::Worker;
use iota_json_rpc_types::type_and_fields_from_move_event_data;
use iota_package_resolver::Resolver;
use iota_rest_api::CheckpointData;
use iota_types::{
    SYSTEM_PACKAGE_ADDRESSES, digests::TransactionDigest, effects::TransactionEvents, event::Event,
};
use move_core_types::annotated_value::MoveValue;
use tokio::sync::Mutex;

use crate::{
    FileType,
    handlers::AnalyticsHandler,
    package_store::{LocalDBPackageStore, PackageCache},
    tables::EventEntry,
};

pub struct EventHandler {
    state: Mutex<State>,
}

struct State {
    events: Vec<EventEntry>,
    package_store: LocalDBPackageStore,
    resolver: Resolver<PackageCache>,
}

#[async_trait::async_trait]
impl Worker for EventHandler {
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
            for object in checkpoint_transaction.output_objects.iter() {
                state.package_store.update(object)?;
            }
            if let Some(events) = &checkpoint_transaction.events {
                self.process_events(
                    checkpoint_summary.epoch,
                    checkpoint_summary.sequence_number,
                    checkpoint_transaction.transaction.digest(),
                    checkpoint_summary.timestamp_ms,
                    events,
                    &mut state,
                )
                .await?;
            }
            if checkpoint_summary.end_of_epoch_data.is_some() {
                state
                    .resolver
                    .package_store()
                    .evict(SYSTEM_PACKAGE_ADDRESSES.iter().copied());
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl AnalyticsHandler<EventEntry> for EventHandler {
    async fn read(&self) -> Result<Vec<EventEntry>> {
        let mut state = self.state.lock().await;
        let cloned = state.events.clone();
        state.events.clear();
        Ok(cloned)
    }

    fn file_type(&self) -> Result<FileType> {
        Ok(FileType::Event)
    }

    fn name(&self) -> &str {
        "event"
    }
}

impl EventHandler {
    pub fn new(store_path: &Path, rest_uri: &str) -> Self {
        let package_store = LocalDBPackageStore::new(&store_path.join("event"), rest_uri);
        let state = State {
            events: vec![],
            package_store: package_store.clone(),
            resolver: Resolver::new(PackageCache::new(package_store)),
        };
        Self {
            state: Mutex::new(state),
        }
    }
    async fn process_events(
        &self,
        epoch: u64,
        checkpoint: u64,
        digest: &TransactionDigest,
        timestamp_ms: u64,
        events: &TransactionEvents,
        state: &mut State,
    ) -> Result<()> {
        for (idx, event) in events.data.iter().enumerate() {
            let Event {
                package_id,
                transaction_module,
                sender,
                type_,
                contents,
            } = event;
            let layout = state
                .resolver
                .type_layout(move_core_types::language_storage::TypeTag::Struct(
                    Box::new(type_.clone()),
                ))
                .await?;
            let move_value = MoveValue::simple_deserialize(contents, &layout)?;
            let (_, event_json) = type_and_fields_from_move_event_data(move_value)?;
            let entry = EventEntry {
                transaction_digest: digest.base58_encode(),
                event_index: idx as u64,
                checkpoint,
                epoch,
                timestamp_ms,
                sender: sender.to_string(),
                package: package_id.to_string(),
                module: transaction_module.to_string(),
                event_type: type_.to_string(),
                bcs: Base64::encode(contents.clone()),
                event_json: event_json.to_string(),
            };

            state.events.push(entry);
        }
        Ok(())
    }
}
