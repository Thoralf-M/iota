// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! A mock implementation of IOTA JSON-RPC client.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use iota_json_rpc_types::{EventFilter, EventPage, IotaEvent, IotaTransactionBlockResponse};
use iota_types::{
    Identifier,
    base_types::{ObjectID, ObjectRef},
    bridge::{BridgeCommitteeSummary, BridgeSummary, MoveTypeParsedTokenTransferMessage},
    digests::TransactionDigest,
    event::EventID,
    gas_coin::GasCoin,
    object::Owner,
    transaction::{ObjectArg, Transaction},
};

use crate::{
    error::{BridgeError, BridgeResult},
    iota_client::IotaClientInner,
    test_utils::DUMMY_MUTABLE_BRIDGE_OBJECT_ARG,
    types::{BridgeAction, BridgeActionStatus, IsBridgePaused},
};

/// Mock client used in test environments.
#[expect(clippy::type_complexity)]
#[derive(Clone, Debug)]
pub struct IotaMockClient {
    // the top two fields do not change during tests so we don't need them to be Arc<Mutex>>
    chain_identifier: String,
    latest_checkpoint_sequence_number: u64,
    events: Arc<Mutex<HashMap<(ObjectID, Identifier, Option<EventID>), EventPage>>>,
    past_event_query_params: Arc<Mutex<VecDeque<(ObjectID, Identifier, Option<EventID>)>>>,
    events_by_tx_digest:
        Arc<Mutex<HashMap<TransactionDigest, Result<Vec<IotaEvent>, iota_sdk::error::Error>>>>,
    transaction_responses:
        Arc<Mutex<HashMap<TransactionDigest, BridgeResult<IotaTransactionBlockResponse>>>>,
    wildcard_transaction_response: Arc<Mutex<Option<BridgeResult<IotaTransactionBlockResponse>>>>,
    get_object_info: Arc<Mutex<HashMap<ObjectID, (GasCoin, ObjectRef, Owner)>>>,
    onchain_status: Arc<Mutex<HashMap<(u8, u64), BridgeActionStatus>>>,
    bridge_committee_summary: Arc<Mutex<Option<BridgeCommitteeSummary>>>,
    is_paused: Arc<Mutex<Option<IsBridgePaused>>>,
    requested_transactions_tx: tokio::sync::broadcast::Sender<TransactionDigest>,
}

impl IotaMockClient {
    pub fn default() -> Self {
        Self {
            chain_identifier: "".to_string(),
            latest_checkpoint_sequence_number: 0,
            events: Default::default(),
            past_event_query_params: Default::default(),
            events_by_tx_digest: Default::default(),
            transaction_responses: Default::default(),
            wildcard_transaction_response: Default::default(),
            get_object_info: Default::default(),
            onchain_status: Default::default(),
            bridge_committee_summary: Default::default(),
            is_paused: Default::default(),
            requested_transactions_tx: tokio::sync::broadcast::channel(10000).0,
        }
    }

    pub fn add_event_response(
        &self,
        package: ObjectID,
        module: Identifier,
        cursor: EventID,
        events: EventPage,
    ) {
        self.events
            .lock()
            .unwrap()
            .insert((package, module, Some(cursor)), events);
    }

    pub fn add_events_by_tx_digest(&self, tx_digest: TransactionDigest, events: Vec<IotaEvent>) {
        self.events_by_tx_digest
            .lock()
            .unwrap()
            .insert(tx_digest, Ok(events));
    }

    pub fn add_events_by_tx_digest_error(&self, tx_digest: TransactionDigest) {
        self.events_by_tx_digest
            .lock()
            .unwrap()
            .insert(tx_digest, Err(iota_sdk::error::Error::Data("".to_string())));
    }

    pub fn add_transaction_response(
        &self,
        tx_digest: TransactionDigest,
        response: BridgeResult<IotaTransactionBlockResponse>,
    ) {
        self.transaction_responses
            .lock()
            .unwrap()
            .insert(tx_digest, response);
    }

    pub fn set_action_onchain_status(&self, action: &BridgeAction, status: BridgeActionStatus) {
        self.onchain_status
            .lock()
            .unwrap()
            .insert((action.chain_id() as u8, action.seq_number()), status);
    }

    pub fn set_bridge_committee(&self, committee: BridgeCommitteeSummary) {
        self.bridge_committee_summary
            .lock()
            .unwrap()
            .replace(committee);
    }

    pub fn set_is_bridge_paused(&self, value: IsBridgePaused) {
        self.is_paused.lock().unwrap().replace(value);
    }

    pub fn set_wildcard_transaction_response(
        &self,
        response: BridgeResult<IotaTransactionBlockResponse>,
    ) {
        *self.wildcard_transaction_response.lock().unwrap() = Some(response);
    }

    pub fn add_gas_object_info(&self, gas_coin: GasCoin, object_ref: ObjectRef, owner: Owner) {
        self.get_object_info
            .lock()
            .unwrap()
            .insert(object_ref.0, (gas_coin, object_ref, owner));
    }

    pub fn subscribe_to_requested_transactions(
        &self,
    ) -> tokio::sync::broadcast::Receiver<TransactionDigest> {
        self.requested_transactions_tx.subscribe()
    }
}

#[async_trait]
impl IotaClientInner for IotaMockClient {
    type Error = iota_sdk::error::Error;

    // Unwraps in this function: We assume the responses are pre-populated
    // by the test before calling into this function.
    async fn query_events(
        &self,
        query: EventFilter,
        cursor: Option<EventID>,
    ) -> Result<EventPage, Self::Error> {
        let events = self.events.lock().unwrap();
        match query {
            EventFilter::MoveEventModule { package, module } => {
                self.past_event_query_params.lock().unwrap().push_back((
                    package,
                    module.clone(),
                    cursor,
                ));
                Ok(events
                    .get(&(package, module.clone(), cursor))
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!(
                            "No preset events found for package: {:?}, module: {:?}, cursor: {:?}",
                            package, module, cursor
                        )
                    }))
            }
            _ => unimplemented!(),
        }
    }

    async fn get_events_by_tx_digest(
        &self,
        tx_digest: TransactionDigest,
    ) -> Result<Vec<IotaEvent>, Self::Error> {
        let events = self.events_by_tx_digest.lock().unwrap();

        match events
            .get(&tx_digest)
            .unwrap_or_else(|| panic!("No preset events found for tx_digest: {:?}", tx_digest))
        {
            Ok(events) => Ok(events.clone()),
            // iota_sdk::error::Error is not Clone
            Err(_) => Err(iota_sdk::error::Error::Data("".to_string())),
        }
    }

    async fn get_chain_identifier(&self) -> Result<String, Self::Error> {
        Ok(self.chain_identifier.clone())
    }

    async fn get_latest_checkpoint_sequence_number(&self) -> Result<u64, Self::Error> {
        Ok(self.latest_checkpoint_sequence_number)
    }

    async fn get_mutable_bridge_object_arg(&self) -> Result<ObjectArg, Self::Error> {
        Ok(DUMMY_MUTABLE_BRIDGE_OBJECT_ARG)
    }

    async fn get_reference_gas_price(&self) -> Result<u64, Self::Error> {
        Ok(1000)
    }

    async fn get_bridge_summary(&self) -> Result<BridgeSummary, Self::Error> {
        Ok(BridgeSummary {
            bridge_version: 0,
            message_version: 0,
            chain_id: 0,
            sequence_nums: vec![],
            bridge_records_id: ObjectID::random(),
            is_frozen: self.is_paused.lock().unwrap().unwrap_or_default(),
            limiter: Default::default(),
            committee: self
                .bridge_committee_summary
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default(),
            treasury: Default::default(),
        })
    }

    async fn get_token_transfer_action_onchain_status(
        &self,
        _bridge_object_arg: ObjectArg,
        source_chain_id: u8,
        seq_number: u64,
    ) -> Result<BridgeActionStatus, BridgeError> {
        Ok(self
            .onchain_status
            .lock()
            .unwrap()
            .get(&(source_chain_id, seq_number))
            .cloned()
            .unwrap_or(BridgeActionStatus::Pending))
    }

    async fn get_token_transfer_action_onchain_signatures(
        &self,
        _bridge_object_arg: ObjectArg,
        _source_chain_id: u8,
        _seq_number: u64,
    ) -> Result<Option<Vec<Vec<u8>>>, BridgeError> {
        unimplemented!()
    }

    async fn get_parsed_token_transfer_message(
        &self,
        _bridge_object_arg: ObjectArg,
        _source_chain_id: u8,
        _seq_number: u64,
    ) -> Result<Option<MoveTypeParsedTokenTransferMessage>, BridgeError> {
        unimplemented!()
    }

    async fn execute_transaction_block_with_effects(
        &self,
        tx: Transaction,
    ) -> Result<IotaTransactionBlockResponse, BridgeError> {
        self.requested_transactions_tx.send(*tx.digest()).unwrap();
        match self.transaction_responses.lock().unwrap().get(tx.digest()) {
            Some(response) => response.clone(),
            None => self
                .wildcard_transaction_response
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| panic!("No preset transaction response found for tx: {:?}", tx)),
        }
    }

    async fn get_gas_data_panic_if_not_gas(
        &self,
        gas_object_id: ObjectID,
    ) -> (GasCoin, ObjectRef, Owner) {
        self.get_object_info
            .lock()
            .unwrap()
            .get(&gas_object_id)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "No preset gas object info found for gas_object_id: {:?}",
                    gas_object_id
                )
            })
    }
}
