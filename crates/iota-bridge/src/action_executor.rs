// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! BridgeActionExecutor receives BridgeActions (from BridgeOrchestrator),
//! collects bridge authority signatures and submit signatures on chain.

use std::{collections::HashMap, sync::Arc};

use arc_swap::ArcSwap;
use iota_json_rpc_types::{
    IotaExecutionStatus, IotaTransactionBlockEffectsAPI, IotaTransactionBlockResponse,
};
use iota_metrics::spawn_logged_monitored_task;
use iota_types::{
    TypeTag,
    base_types::{IotaAddress, ObjectID, ObjectRef},
    crypto::{IotaKeyPair, Signature},
    digests::TransactionDigest,
    gas_coin::GasCoin,
    object::Owner,
    transaction::{ObjectArg, Transaction},
};
use shared_crypto::intent::{Intent, IntentMessage};
use tokio::{sync::Semaphore, time::Duration};
use tracing::{Instrument, error, info, instrument, warn};

use crate::{
    client::bridge_authority_aggregator::BridgeAuthorityAggregator,
    error::BridgeError,
    events::{
        TokenTransferAlreadyApproved, TokenTransferAlreadyClaimed, TokenTransferApproved,
        TokenTransferClaimed,
    },
    iota_client::{IotaClient, IotaClientInner},
    iota_transaction_builder::build_iota_transaction,
    metrics::BridgeMetrics,
    retry_with_max_elapsed_time,
    storage::BridgeOrchestratorTables,
    types::{BridgeAction, BridgeActionStatus, IsBridgePaused, VerifiedCertifiedBridgeAction},
};

pub const CHANNEL_SIZE: usize = 1000;
pub const SIGNING_CONCURRENCY: usize = 10;

// delay schedule: at most 16 times including the initial attempt
// 0.1s, 0.2s, 0.4s, 0.8s, 1.6s, 3.2s, 6.4s, 12.8s, 25.6s, 51.2s, 102.4s,
// 204.8s, 409.6s, 819.2s, 1638.4s
pub const MAX_SIGNING_ATTEMPTS: u64 = 16;
pub const MAX_EXECUTION_ATTEMPTS: u64 = 16;

async fn delay(attempt_times: u64) {
    let delay_ms = 100 * (2 ^ attempt_times);
    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
}

#[derive(Debug)]
pub struct BridgeActionExecutionWrapper(pub BridgeAction, pub u64);

#[derive(Debug)]
pub struct CertifiedBridgeActionExecutionWrapper(pub VerifiedCertifiedBridgeAction, pub u64);

pub trait BridgeActionExecutorTrait {
    fn run(
        self,
    ) -> (
        Vec<tokio::task::JoinHandle<()>>,
        iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
    );
}

pub struct BridgeActionExecutor<C> {
    iota_client: Arc<IotaClient<C>>,
    bridge_auth_agg: Arc<ArcSwap<BridgeAuthorityAggregator>>,
    key: IotaKeyPair,
    iota_address: IotaAddress,
    gas_object_id: ObjectID,
    store: Arc<BridgeOrchestratorTables>,
    bridge_object_arg: ObjectArg,
    iota_token_type_tags: Arc<ArcSwap<HashMap<u8, TypeTag>>>,
    bridge_pause_rx: tokio::sync::watch::Receiver<IsBridgePaused>,
    metrics: Arc<BridgeMetrics>,
}

impl<C> BridgeActionExecutorTrait for BridgeActionExecutor<C>
where
    C: IotaClientInner + 'static,
{
    fn run(
        self,
    ) -> (
        Vec<tokio::task::JoinHandle<()>>,
        iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
    ) {
        let (tasks, sender, _) = self.run_inner();
        (tasks, sender)
    }
}

impl<C> BridgeActionExecutor<C>
where
    C: IotaClientInner + 'static,
{
    pub async fn new(
        iota_client: Arc<IotaClient<C>>,
        bridge_auth_agg: Arc<ArcSwap<BridgeAuthorityAggregator>>,
        store: Arc<BridgeOrchestratorTables>,
        key: IotaKeyPair,
        iota_address: IotaAddress,
        gas_object_id: ObjectID,
        iota_token_type_tags: Arc<ArcSwap<HashMap<u8, TypeTag>>>,
        bridge_pause_rx: tokio::sync::watch::Receiver<IsBridgePaused>,
        metrics: Arc<BridgeMetrics>,
    ) -> Self {
        let bridge_object_arg = iota_client
            .get_mutable_bridge_object_arg_must_succeed()
            .await;
        Self {
            iota_client,
            bridge_auth_agg,
            store,
            key,
            gas_object_id,
            iota_address,
            bridge_object_arg,
            iota_token_type_tags,
            bridge_pause_rx,
            metrics,
        }
    }

    fn run_inner(
        self,
    ) -> (
        Vec<tokio::task::JoinHandle<()>>,
        iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
        iota_metrics::metered_channel::Sender<CertifiedBridgeActionExecutionWrapper>,
    ) {
        let key = self.key;

        let (sender, receiver) = iota_metrics::metered_channel::channel(
            CHANNEL_SIZE,
            &iota_metrics::get_metrics()
                .unwrap()
                .channel_inflight
                .with_label_values(&["executor_signing_queue"]),
        );

        let (execution_tx, execution_rx) = iota_metrics::metered_channel::channel(
            CHANNEL_SIZE,
            &iota_metrics::get_metrics()
                .unwrap()
                .channel_inflight
                .with_label_values(&["executor_execution_queue"]),
        );
        let execution_tx_clone = execution_tx.clone();
        let sender_clone = sender.clone();
        let store_clone = self.store.clone();
        let client_clone = self.iota_client.clone();
        let mut tasks = vec![];
        let metrics = self.metrics.clone();
        tasks.push(spawn_logged_monitored_task!(
            Self::run_signature_aggregation_loop(
                client_clone,
                self.bridge_auth_agg,
                store_clone,
                sender_clone,
                receiver,
                execution_tx_clone,
                metrics,
            )
        ));

        let metrics = self.metrics.clone();
        let execution_tx_clone = execution_tx.clone();
        tasks.push(spawn_logged_monitored_task!(
            Self::run_onchain_execution_loop(
                self.iota_client.clone(),
                key,
                self.iota_address,
                self.gas_object_id,
                self.store.clone(),
                execution_tx_clone,
                execution_rx,
                self.bridge_object_arg,
                self.iota_token_type_tags,
                self.bridge_pause_rx,
                metrics,
            )
        ));
        (tasks, sender, execution_tx)
    }

    async fn run_signature_aggregation_loop(
        iota_client: Arc<IotaClient<C>>,
        auth_agg: Arc<ArcSwap<BridgeAuthorityAggregator>>,
        store: Arc<BridgeOrchestratorTables>,
        signing_queue_sender: iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
        mut signing_queue_receiver: iota_metrics::metered_channel::Receiver<
            BridgeActionExecutionWrapper,
        >,
        execution_queue_sender: iota_metrics::metered_channel::Sender<
            CertifiedBridgeActionExecutionWrapper,
        >,
        metrics: Arc<BridgeMetrics>,
    ) {
        info!("Starting run_signature_aggregation_loop");
        let semaphore = Arc::new(Semaphore::new(SIGNING_CONCURRENCY));
        while let Some(action) = signing_queue_receiver.recv().await {
            Self::handle_signing_task(
                &semaphore,
                &auth_agg,
                &signing_queue_sender,
                &execution_queue_sender,
                &iota_client,
                &store,
                action,
                &metrics,
            )
            .await;
        }
    }

    async fn should_proceed_signing(iota_client: &Arc<IotaClient<C>>) -> bool {
        let Ok(Ok(is_paused)) =
            retry_with_max_elapsed_time!(iota_client.is_bridge_paused(), Duration::from_secs(600))
        else {
            error!("Failed to get bridge status after retry");
            return false;
        };
        !is_paused
    }

    #[instrument(level = "error", skip_all, fields(action_key=?action.0.key(), attempt_times=?action.1))]
    async fn handle_signing_task(
        semaphore: &Arc<Semaphore>,
        auth_agg: &Arc<ArcSwap<BridgeAuthorityAggregator>>,
        signing_queue_sender: &iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
        execution_queue_sender: &iota_metrics::metered_channel::Sender<
            CertifiedBridgeActionExecutionWrapper,
        >,
        iota_client: &Arc<IotaClient<C>>,
        store: &Arc<BridgeOrchestratorTables>,
        action: BridgeActionExecutionWrapper,
        metrics: &Arc<BridgeMetrics>,
    ) {
        metrics.action_executor_signing_queue_received_actions.inc();
        let action_key = action.0.key();
        info!("Received action for signing: {:?}", action.0);

        // TODO: this is a temporary fix to avoid signing when the bridge is paused.
        // but the way is implemented is not ideal:
        // 1. it should check the direction
        // 2. should use a better mechanism to check the bridge status instead of
        //    polling for each action
        let should_proceed = Self::should_proceed_signing(iota_client).await;
        if !should_proceed {
            metrics.action_executor_signing_queue_skipped_actions.inc();
            warn!("skipping signing task: {:?}", action_key);
            return;
        }

        let auth_agg_clone = auth_agg.clone();
        let signing_queue_sender_clone = signing_queue_sender.clone();
        let execution_queue_sender_clone = execution_queue_sender.clone();
        let iota_client_clone = iota_client.clone();
        let store_clone = store.clone();
        let metrics_clone = metrics.clone();
        let semaphore_clone = semaphore.clone();
        spawn_logged_monitored_task!(
            Self::request_signatures(
                semaphore_clone,
                iota_client_clone,
                auth_agg_clone,
                action,
                store_clone,
                signing_queue_sender_clone,
                execution_queue_sender_clone,
                metrics_clone,
            )
            .instrument(tracing::debug_span!("request_signatures", action_key=?action_key)),
            "request_signatures"
        );
    }

    // Checks if the action is already processed on chain.
    // If yes, skip this action and remove it from the pending log.
    // Returns true if the action is already processed.
    async fn handle_already_processed_token_transfer_action_maybe(
        iota_client: &Arc<IotaClient<C>>,
        action: &BridgeAction,
        store: &Arc<BridgeOrchestratorTables>,
        metrics: &Arc<BridgeMetrics>,
    ) -> bool {
        let status = iota_client
            .get_token_transfer_action_onchain_status_until_success(
                action.chain_id() as u8,
                action.seq_number(),
            )
            .await;
        match status {
            BridgeActionStatus::Approved | BridgeActionStatus::Claimed => {
                info!(
                    "Action already approved or claimed, removing action from pending logs: {:?}",
                    action
                );
                metrics.action_executor_already_processed_actions.inc();
                store
                    .remove_pending_actions(&[action.digest()])
                    .unwrap_or_else(|e| {
                        panic!("Write to DB should not fail: {:?}", e);
                    });
                true
            }
            // Although theoretically a legit IotaToEthBridgeAction should not have
            // status `NotFound`
            BridgeActionStatus::Pending | BridgeActionStatus::NotFound => false,
        }
    }

    // TODO: introduce a way to properly stagger the handling
    // for various validators.
    async fn request_signatures(
        semaphore: Arc<Semaphore>,
        iota_client: Arc<IotaClient<C>>,
        auth_agg: Arc<ArcSwap<BridgeAuthorityAggregator>>,
        action: BridgeActionExecutionWrapper,
        store: Arc<BridgeOrchestratorTables>,
        signing_queue_sender: iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
        execution_queue_sender: iota_metrics::metered_channel::Sender<
            CertifiedBridgeActionExecutionWrapper,
        >,
        metrics: Arc<BridgeMetrics>,
    ) {
        let _permit = semaphore
            .acquire()
            .await
            .expect("semaphore should not be closed");
        info!("requesting signatures");
        let BridgeActionExecutionWrapper(action, attempt_times) = action;

        // Only token transfer action should reach here
        match &action {
            BridgeAction::IotaToEthBridgeAction(_) | BridgeAction::EthToIotaBridgeAction(_) => (),
            _ => unreachable!("Non token transfer action should not reach here"),
        };

        // If the action is already processed, skip it.
        if Self::handle_already_processed_token_transfer_action_maybe(
            &iota_client,
            &action,
            &store,
            &metrics,
        )
        .await
        {
            return;
        }
        match auth_agg
            .load()
            .request_committee_signatures(action.clone())
            .await
        {
            Ok(certificate) => {
                info!("Sending certificate to execution");
                execution_queue_sender
                    .send(CertifiedBridgeActionExecutionWrapper(certificate, 0))
                    .await
                    .unwrap_or_else(|e| {
                        panic!("Sending to execution queue should not fail: {:?}", e);
                    });
            }
            Err(e) => {
                warn!("Failed to collect sigs for bridge action: {:?}", e);
                metrics.err_signature_aggregation.inc();

                // TODO: spawn a task for this
                if attempt_times >= MAX_SIGNING_ATTEMPTS {
                    error!(
                        "Manual intervention is required. Failed to collect sigs for bridge action after {MAX_SIGNING_ATTEMPTS} attempts: {:?}",
                        e
                    );
                    return;
                }
                delay(attempt_times).await;
                signing_queue_sender
                    .send(BridgeActionExecutionWrapper(action, attempt_times + 1))
                    .await
                    .unwrap_or_else(|e| {
                        panic!("Sending to signing queue should not fail: {:?}", e);
                    });
            }
        }
    }

    // Before calling this function, `key` and `iota_address` need to be
    // verified to match.
    async fn run_onchain_execution_loop(
        iota_client: Arc<IotaClient<C>>,
        iota_key: IotaKeyPair,
        iota_address: IotaAddress,
        gas_object_id: ObjectID,
        store: Arc<BridgeOrchestratorTables>,
        execution_queue_sender: iota_metrics::metered_channel::Sender<
            CertifiedBridgeActionExecutionWrapper,
        >,
        mut execution_queue_receiver: iota_metrics::metered_channel::Receiver<
            CertifiedBridgeActionExecutionWrapper,
        >,
        bridge_object_arg: ObjectArg,
        iota_token_type_tags: Arc<ArcSwap<HashMap<u8, TypeTag>>>,
        bridge_pause_rx: tokio::sync::watch::Receiver<IsBridgePaused>,
        metrics: Arc<BridgeMetrics>,
    ) {
        info!("Starting run_onchain_execution_loop");
        while let Some(certificate_wrapper) = execution_queue_receiver.recv().await {
            // When bridge is paused, skip execution.
            // Skipped actions will be picked up upon node restarting
            // if bridge is unpaused.
            if *bridge_pause_rx.borrow() {
                warn!("Bridge is paused, skipping execution");
                metrics
                    .action_executor_execution_queue_skipped_actions_due_to_pausing
                    .inc();
                continue;
            }
            Self::handle_execution_task(
                certificate_wrapper,
                &iota_client,
                &iota_key,
                &iota_address,
                gas_object_id,
                &store,
                &execution_queue_sender,
                &bridge_object_arg,
                &iota_token_type_tags,
                &metrics,
            )
            .await;
        }
        panic!("Execution queue closed unexpectedly");
    }

    #[instrument(level = "error", skip_all, fields(action_key=?certificate_wrapper.0.data().key(), attempt_times=?certificate_wrapper.1))]
    async fn handle_execution_task(
        certificate_wrapper: CertifiedBridgeActionExecutionWrapper,
        iota_client: &Arc<IotaClient<C>>,
        iota_key: &IotaKeyPair,
        iota_address: &IotaAddress,
        gas_object_id: ObjectID,
        store: &Arc<BridgeOrchestratorTables>,
        execution_queue_sender: &iota_metrics::metered_channel::Sender<
            CertifiedBridgeActionExecutionWrapper,
        >,
        bridge_object_arg: &ObjectArg,
        iota_token_type_tags: &ArcSwap<HashMap<u8, TypeTag>>,
        metrics: &Arc<BridgeMetrics>,
    ) {
        metrics
            .action_executor_execution_queue_received_actions
            .inc();
        let CertifiedBridgeActionExecutionWrapper(certificate, attempt_times) = certificate_wrapper;
        let action = certificate.data();
        let action_key = action.key();

        info!("Received certified action for execution: {:?}", action);

        // TODO check gas coin balance here. If gas balance too low, do not proceed.
        let (gas_coin, gas_object_ref) =
            Self::get_gas_data_assert_ownership(*iota_address, gas_object_id, iota_client).await;
        metrics.gas_coin_balance.set(gas_coin.value() as i64);

        let ceriticate_clone = certificate.clone();

        // Check once: if the action is already processed, skip it.
        if Self::handle_already_processed_token_transfer_action_maybe(
            iota_client,
            action,
            store,
            metrics,
        )
        .await
        {
            info!("Action already processed, skipping");
            return;
        }

        info!("Building IOTA transaction");
        let rgp = iota_client.get_reference_gas_price_until_success().await;
        let tx_data = match build_iota_transaction(
            *iota_address,
            &gas_object_ref,
            ceriticate_clone,
            *bridge_object_arg,
            iota_token_type_tags.load().as_ref(),
            rgp,
        ) {
            Ok(tx_data) => tx_data,
            Err(err) => {
                metrics.err_build_iota_transaction.inc();
                error!(
                    "Manual intervention is required. Failed to build transaction for action {:?}: {:?}",
                    action, err
                );
                // This should not happen, but in case it does, we do not want to
                // panic, instead we log here for manual intervention.
                return;
            }
        };
        let sig = Signature::new_secure(
            &IntentMessage::new(Intent::iota_transaction(), &tx_data),
            iota_key,
        );
        let signed_tx = Transaction::from_data(tx_data, vec![sig]);
        let tx_digest = *signed_tx.digest();

        // Check twice: If the action is already processed, skip it.
        if Self::handle_already_processed_token_transfer_action_maybe(
            iota_client,
            action,
            store,
            metrics,
        )
        .await
        {
            info!("Action already processed, skipping");
            return;
        }

        info!(?tx_digest, ?gas_object_ref, "Sending transaction to IOTA");
        match iota_client
            .execute_transaction_block_with_effects(signed_tx)
            .await
        {
            Ok(resp) => {
                Self::handle_execution_effects(tx_digest, resp, store, action, metrics).await
            }

            // If the transaction did not go through, retry up to a certain times.
            Err(err) => {
                error!(
                    ?action_key,
                    ?tx_digest,
                    "IOTA transaction failed at signing: {err:?}"
                );
                metrics.err_iota_transaction_submission.inc();
                let metrics_clone = metrics.clone();
                // Do this in a separate task so we won't deadlock here
                let sender_clone = execution_queue_sender.clone();
                spawn_logged_monitored_task!(async move {
                    // If it fails for too many times, log and ask for manual intervention.
                    metrics_clone
                        .err_iota_transaction_submission_too_many_failures
                        .inc();
                    if attempt_times >= MAX_EXECUTION_ATTEMPTS {
                        error!("Manual intervention is required. Failed to collect execute transaction for bridge action after {MAX_EXECUTION_ATTEMPTS} attempts: {:?}", err);
                        return;
                    }
                    delay(attempt_times).await;
                    sender_clone
                        .send(CertifiedBridgeActionExecutionWrapper(
                            certificate,
                            attempt_times + 1,
                        ))
                        .await
                        .unwrap_or_else(|e| {
                            panic!("Sending to execution queue should not fail: {:?}", e);
                        });
                    info!("Re-enqueued certificate for execution");
                }.instrument(tracing::debug_span!("reenqueue_execution_task", action_key=?action_key)));
            }
        }
    }

    // TODO: do we need a mechanism to periodically read pending actions from DB?
    async fn handle_execution_effects(
        tx_digest: TransactionDigest,
        response: IotaTransactionBlockResponse,
        store: &Arc<BridgeOrchestratorTables>,
        action: &BridgeAction,
        metrics: &Arc<BridgeMetrics>,
    ) {
        let effects = response
            .effects
            .clone()
            .expect("We requested effects but got None.");
        let status = effects.status();
        match status {
            IotaExecutionStatus::Success => {
                let events = response.events.expect("We requested events but got None.");
                // If the transaction is successful, there must be either
                // TokenTransferAlreadyClaimed or TokenTransferClaimed event.
                assert!(
                    events.data.iter().any(|e| e.type_
                        == *TokenTransferAlreadyClaimed.get().unwrap()
                        || e.type_ == *TokenTransferClaimed.get().unwrap()
                        || e.type_ == *TokenTransferApproved.get().unwrap()
                        || e.type_ == *TokenTransferAlreadyApproved.get().unwrap()),
                    "Expected TokenTransferAlreadyClaimed, TokenTransferClaimed, TokenTransferApproved or TokenTransferAlreadyApproved event but got: {:?}",
                    events,
                );
                info!(?tx_digest, "IOTA transaction executed successfully");
                store
                    .remove_pending_actions(&[action.digest()])
                    .unwrap_or_else(|e| {
                        panic!("Write to DB should not fail: {:?}", e);
                    })
            }
            IotaExecutionStatus::Failure { error } => {
                // In practice the transaction could fail because of running out of gas, but
                // really should not be due to other reasons.
                // This means manual intervention is needed. So we do not push them back to
                // the execution queue because retries are mostly likely going to fail anyway.
                // After human examination, the node should be restarted and fetch them from
                // WAL.

                metrics.err_iota_transaction_execution.inc();
                error!(
                    ?tx_digest,
                    "Manual intervention is needed. IOTA transaction executed and failed with error: {error:?}"
                );
            }
        }
    }

    /// Panics if the gas object is not owned by the address.
    async fn get_gas_data_assert_ownership(
        iota_address: IotaAddress,
        gas_object_id: ObjectID,
        iota_client: &IotaClient<C>,
    ) -> (GasCoin, ObjectRef) {
        let (gas_coin, gas_obj_ref, owner) = iota_client
            .get_gas_data_panic_if_not_gas(gas_object_id)
            .await;

        // TODO: when we add multiple gas support in the future we could discard
        // transferred gas object instead.
        assert_eq!(
            owner,
            Owner::AddressOwner(iota_address),
            "Gas object {:?} is no longer owned by address {}",
            gas_object_id,
            iota_address
        );
        (gas_coin, gas_obj_ref)
    }
}

pub async fn submit_to_executor(
    tx: &iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
    action: BridgeAction,
) -> Result<(), BridgeError> {
    tx.send(BridgeActionExecutionWrapper(action, 0))
        .await
        .map_err(|e| BridgeError::Generic(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        str::FromStr,
    };

    use fastcrypto::traits::KeyPair;
    use iota_json_rpc_types::{
        IotaEvent, IotaTransactionBlockEffects, IotaTransactionBlockEvents,
        IotaTransactionBlockResponse,
    };
    use iota_types::{
        TypeTag, base_types::random_object_ref, crypto::get_key_pair, gas_coin::GasCoin,
        transaction::TransactionData,
    };
    use prometheus::Registry;

    use super::*;
    use crate::{
        crypto::{
            BridgeAuthorityKeyPair, BridgeAuthorityPublicKeyBytes,
            BridgeAuthorityRecoverableSignature,
        },
        events::init_all_struct_tags,
        iota_mock_client::IotaMockClient,
        server::mock_handler::BridgeRequestMockHandler,
        test_utils::{
            DUMMY_MUTABLE_BRIDGE_OBJECT_ARG, get_test_authorities_and_run_mock_bridge_server,
            get_test_eth_to_iota_bridge_action, get_test_iota_to_eth_bridge_action,
            sign_action_with_key,
        },
        types::{
            BRIDGE_PAUSED, BridgeCommittee, BridgeCommitteeValiditySignInfo, CertifiedBridgeAction,
        },
    };

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_onchain_execution_loop() {
        let SetupData {
            signing_tx,
            iota_client_mock,
            mut tx_subscription,
            store,
            secrets,
            dummy_iota_key,
            mock0,
            mock1,
            mock2,
            mock3,
            gas_object_ref,
            iota_address,
            iota_token_type_tags,
            ..
        } = setup().await;
        let (action_certificate, _, _) = get_bridge_authority_approved_action(
            vec![&mock0, &mock1, &mock2, &mock3],
            vec![&secrets[0], &secrets[1], &secrets[2], &secrets[3]],
            None,
            true,
        );
        let action = action_certificate.data().clone();
        let id_token_map = (*iota_token_type_tags.load().clone()).clone();
        let tx_data = build_iota_transaction(
            iota_address,
            &gas_object_ref,
            action_certificate,
            DUMMY_MUTABLE_BRIDGE_OBJECT_ARG,
            &id_token_map,
            1000,
        )
        .unwrap();

        let tx_digest = get_tx_digest(tx_data, &dummy_iota_key);

        let gas_coin = GasCoin::new_for_testing(1_000_000_000_000); // dummy gas coin
        iota_client_mock.add_gas_object_info(
            gas_coin.clone(),
            gas_object_ref,
            Owner::AddressOwner(iota_address),
        );

        // Mock the transaction to be successfully executed
        let mut event = IotaEvent::random_for_testing();
        event.type_ = TokenTransferClaimed.get().unwrap().clone();
        let events = vec![event];
        mock_transaction_response(
            &iota_client_mock,
            tx_digest,
            IotaExecutionStatus::Success,
            Some(events),
            true,
        );

        store.insert_pending_actions(&[action.clone()]).unwrap();
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Kick it
        submit_to_executor(&signing_tx, action.clone())
            .await
            .unwrap();

        // Expect to see the transaction to be requested and successfully executed hence
        // removed from WAL
        tx_subscription.recv().await.unwrap();
        assert!(store.get_all_pending_actions().is_empty());

        /////////////////////////////////////////////////////////////////////////////////////////////////
        ////////////////////////////////////// Test execution failure
        ////////////////////////////////////// ///////////////////////////////////
        ////////////////////////////////////// /////////////////////////////////////////
        ////////////////////////////////////// //////////////////

        let (action_certificate, _, _) = get_bridge_authority_approved_action(
            vec![&mock0, &mock1, &mock2, &mock3],
            vec![&secrets[0], &secrets[1], &secrets[2], &secrets[3]],
            None,
            true,
        );

        let action = action_certificate.data().clone();

        let tx_data = build_iota_transaction(
            iota_address,
            &gas_object_ref,
            action_certificate,
            DUMMY_MUTABLE_BRIDGE_OBJECT_ARG,
            &id_token_map,
            1000,
        )
        .unwrap();
        let tx_digest = get_tx_digest(tx_data, &dummy_iota_key);

        // Mock the transaction to fail
        mock_transaction_response(
            &iota_client_mock,
            tx_digest,
            IotaExecutionStatus::Failure {
                error: "failure is mother of success".to_string(),
            },
            None,
            true,
        );

        store.insert_pending_actions(&[action.clone()]).unwrap();
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Kick it
        submit_to_executor(&signing_tx, action.clone())
            .await
            .unwrap();

        // Expect to see the transaction to be requested and but failed
        tx_subscription.recv().await.unwrap();
        // The action is not removed from WAL because the transaction failed
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        /////////////////////////////////////////////////////////////////////////////////////////////////
        //////////////////////////// Test transaction failed at signing stage
        //////////////////////////// /////////////////////////// ///////////////
        //////////////////////////// ///////////////////////////////////////////////////
        //////////////////////////// ///

        let (action_certificate, _, _) = get_bridge_authority_approved_action(
            vec![&mock0, &mock1, &mock2, &mock3],
            vec![&secrets[0], &secrets[1], &secrets[2], &secrets[3]],
            None,
            true,
        );

        let action = action_certificate.data().clone();

        let tx_data = build_iota_transaction(
            iota_address,
            &gas_object_ref,
            action_certificate,
            DUMMY_MUTABLE_BRIDGE_OBJECT_ARG,
            &id_token_map,
            1000,
        )
        .unwrap();
        let tx_digest = get_tx_digest(tx_data, &dummy_iota_key);
        mock_transaction_error(
            &iota_client_mock,
            tx_digest,
            BridgeError::Generic("some random error".to_string()),
            true,
        );

        store.insert_pending_actions(&[action.clone()]).unwrap();
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Kick it
        submit_to_executor(&signing_tx, action.clone())
            .await
            .unwrap();

        // Failure will trigger retry, we wait for 2 requests before checking WAL log
        let tx_digest = tx_subscription.recv().await.unwrap();
        assert_eq!(tx_subscription.recv().await.unwrap(), tx_digest);

        // The retry is still going on, action still in WAL
        assert!(
            store
                .get_all_pending_actions()
                .contains_key(&action.digest())
        );

        // Now let it succeed
        let mut event = IotaEvent::random_for_testing();
        event.type_ = TokenTransferClaimed.get().unwrap().clone();
        let events = vec![event];
        mock_transaction_response(
            &iota_client_mock,
            tx_digest,
            IotaExecutionStatus::Success,
            Some(events),
            true,
        );

        // Give it 1 second to retry and succeed
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        // The action is successful and should be removed from WAL now
        assert!(
            !store
                .get_all_pending_actions()
                .contains_key(&action.digest())
        );
    }

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_signature_aggregation_loop() {
        let SetupData {
            signing_tx,
            iota_client_mock,
            mut tx_subscription,
            store,
            secrets,
            dummy_iota_key,
            mock0,
            mock1,
            mock2,
            mock3,
            gas_object_ref,
            iota_address,
            iota_token_type_tags,
            ..
        } = setup().await;
        let id_token_map = (*iota_token_type_tags.load().clone()).clone();
        let (action_certificate, iota_tx_digest, iota_tx_event_index) =
            get_bridge_authority_approved_action(
                vec![&mock0, &mock1, &mock2, &mock3],
                vec![&secrets[0], &secrets[1], &secrets[2], &secrets[3]],
                None,
                true,
            );
        let action = action_certificate.data().clone();
        mock_bridge_authority_signing_errors(
            vec![&mock0, &mock1, &mock2],
            iota_tx_digest,
            iota_tx_event_index,
        );
        let mut sigs = mock_bridge_authority_sigs(
            vec![&mock3],
            &action,
            vec![&secrets[3]],
            iota_tx_digest,
            iota_tx_event_index,
        );

        let gas_coin = GasCoin::new_for_testing(1_000_000_000_000); // dummy gas coin
        iota_client_mock.add_gas_object_info(
            gas_coin,
            gas_object_ref,
            Owner::AddressOwner(iota_address),
        );
        store.insert_pending_actions(&[action.clone()]).unwrap();
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Kick it
        submit_to_executor(&signing_tx, action.clone())
            .await
            .unwrap();

        // Wait until the transaction is retried at least once (instead of deing
        // dropped)
        loop {
            let requested_times =
                mock0.get_iota_token_events_requested(iota_tx_digest, iota_tx_event_index);
            if requested_times >= 2 {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
        // Nothing is sent to execute yet
        assert_eq!(
            tx_subscription.try_recv().unwrap_err(),
            tokio::sync::broadcast::error::TryRecvError::Empty
        );
        // Still in WAL
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Let authorities sign the action too. Now we are above the threshold
        let sig_from_2 = mock_bridge_authority_sigs(
            vec![&mock2],
            &action,
            vec![&secrets[2]],
            iota_tx_digest,
            iota_tx_event_index,
        );
        sigs.extend(sig_from_2);
        let certified_action = CertifiedBridgeAction::new_from_data_and_sig(
            action.clone(),
            BridgeCommitteeValiditySignInfo { signatures: sigs },
        );
        let action_certificate = VerifiedCertifiedBridgeAction::new_from_verified(certified_action);
        let tx_data = build_iota_transaction(
            iota_address,
            &gas_object_ref,
            action_certificate,
            DUMMY_MUTABLE_BRIDGE_OBJECT_ARG,
            &id_token_map,
            1000,
        )
        .unwrap();
        let tx_digest = get_tx_digest(tx_data, &dummy_iota_key);

        let mut event = IotaEvent::random_for_testing();
        event.type_ = TokenTransferClaimed.get().unwrap().clone();
        let events = vec![event];
        mock_transaction_response(
            &iota_client_mock,
            tx_digest,
            IotaExecutionStatus::Success,
            Some(events),
            true,
        );

        // Expect to see the transaction to be requested and succeed
        assert_eq!(tx_subscription.recv().await.unwrap(), tx_digest);
        // The action is removed from WAL
        assert!(
            !store
                .get_all_pending_actions()
                .contains_key(&action.digest())
        );
    }

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_skip_request_signature_if_already_processed_on_chain() {
        let SetupData {
            signing_tx,
            iota_client_mock,
            mut tx_subscription,
            store,
            mock0,
            mock1,
            mock2,
            mock3,
            ..
        } = setup().await;

        let iota_tx_digest = TransactionDigest::random();
        let iota_tx_event_index = 0;
        let action = get_test_iota_to_eth_bridge_action(
            Some(iota_tx_digest),
            Some(iota_tx_event_index),
            None,
            None,
            None,
            None,
            None,
        );
        mock_bridge_authority_signing_errors(
            vec![&mock0, &mock1, &mock2, &mock3],
            iota_tx_digest,
            iota_tx_event_index,
        );
        store.insert_pending_actions(&[action.clone()]).unwrap();
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Kick it
        submit_to_executor(&signing_tx, action.clone())
            .await
            .unwrap();
        let action_digest = action.digest();

        // Wait for 1 second. It should still in the process of retrying requesting sigs
        // because we mock errors above.
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        tx_subscription.try_recv().unwrap_err();
        // And the action is still in WAL
        assert!(store.get_all_pending_actions().contains_key(&action_digest));

        iota_client_mock.set_action_onchain_status(&action, BridgeActionStatus::Approved);

        // The next retry will see the action is already processed on chain and remove
        // it from WAL
        let now = std::time::Instant::now();
        while store.get_all_pending_actions().contains_key(&action_digest) {
            if now.elapsed().as_secs() > 10 {
                panic!("Timeout waiting for action to be removed from WAL");
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        tx_subscription.try_recv().unwrap_err();
    }

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_skip_tx_submission_if_already_processed_on_chain() {
        let SetupData {
            execution_tx,
            iota_client_mock,
            mut tx_subscription,
            store,
            secrets,
            dummy_iota_key,
            mock0,
            mock1,
            mock2,
            mock3,
            gas_object_ref,
            iota_address,
            iota_token_type_tags,
            ..
        } = setup().await;
        let id_token_map = (*iota_token_type_tags.load().clone()).clone();
        let (action_certificate, _, _) = get_bridge_authority_approved_action(
            vec![&mock0, &mock1, &mock2, &mock3],
            vec![&secrets[0], &secrets[1], &secrets[2], &secrets[3]],
            None,
            true,
        );

        let action = action_certificate.data().clone();
        let arg = DUMMY_MUTABLE_BRIDGE_OBJECT_ARG;
        let tx_data = build_iota_transaction(
            iota_address,
            &gas_object_ref,
            action_certificate.clone(),
            arg,
            &id_token_map,
            1000,
        )
        .unwrap();
        let tx_digest = get_tx_digest(tx_data, &dummy_iota_key);
        mock_transaction_error(
            &iota_client_mock,
            tx_digest,
            BridgeError::Generic("some random error".to_string()),
            true,
        );

        let gas_coin = GasCoin::new_for_testing(1_000_000_000_000); // dummy gas coin
        iota_client_mock.add_gas_object_info(
            gas_coin.clone(),
            gas_object_ref,
            Owner::AddressOwner(iota_address),
        );

        iota_client_mock.set_action_onchain_status(&action, BridgeActionStatus::Pending);

        store.insert_pending_actions(&[action.clone()]).unwrap();
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Kick it (send to the execution queue, skipping the signing queue)
        execution_tx
            .send(CertifiedBridgeActionExecutionWrapper(action_certificate, 0))
            .await
            .unwrap();

        // Some requests come in and will fail.
        tx_subscription.recv().await.unwrap();

        // Set the action to be already approved on chain
        iota_client_mock.set_action_onchain_status(&action, BridgeActionStatus::Approved);

        // The next retry will see the action is already processed on chain and remove
        // it from WAL
        let now = std::time::Instant::now();
        let action_digest = action.digest();
        while store.get_all_pending_actions().contains_key(&action_digest) {
            if now.elapsed().as_secs() > 10 {
                panic!("Timeout waiting for action to be removed from WAL");
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_skip_tx_submission_if_bridge_is_paused() {
        let SetupData {
            execution_tx,
            iota_client_mock,
            mut tx_subscription,
            store,
            secrets,
            dummy_iota_key,
            mock0,
            mock1,
            mock2,
            mock3,
            gas_object_ref,
            iota_address,
            iota_token_type_tags,
            bridge_pause_tx,
            ..
        } = setup().await;
        let id_token_map: HashMap<u8, TypeTag> = (*iota_token_type_tags.load().clone()).clone();
        let (action_certificate, _, _) = get_bridge_authority_approved_action(
            vec![&mock0, &mock1, &mock2, &mock3],
            vec![&secrets[0], &secrets[1], &secrets[2], &secrets[3]],
            None,
            true,
        );

        let action = action_certificate.data().clone();
        let arg = DUMMY_MUTABLE_BRIDGE_OBJECT_ARG;
        let tx_data = build_iota_transaction(
            iota_address,
            &gas_object_ref,
            action_certificate.clone(),
            arg,
            &id_token_map,
            1000,
        )
        .unwrap();
        let tx_digest = get_tx_digest(tx_data, &dummy_iota_key);
        mock_transaction_error(
            &iota_client_mock,
            tx_digest,
            BridgeError::Generic("some random error".to_string()),
            true,
        );

        let gas_coin = GasCoin::new_for_testing(1_000_000_000_000); // dummy gas coin
        iota_client_mock.add_gas_object_info(
            gas_coin.clone(),
            gas_object_ref,
            Owner::AddressOwner(iota_address),
        );
        let action_digest = action.digest();
        iota_client_mock.set_action_onchain_status(&action, BridgeActionStatus::Pending);

        // assert bridge is unpaused now
        assert!(!*bridge_pause_tx.borrow());

        store.insert_pending_actions(&[action.clone()]).unwrap();
        assert_eq!(
            store.get_all_pending_actions()[&action.digest()],
            action.clone()
        );

        // Kick it (send to the execution queue, skipping the signing queue)
        execution_tx
            .send(CertifiedBridgeActionExecutionWrapper(
                action_certificate.clone(),
                0,
            ))
            .await
            .unwrap();

        // Some requests come in
        tx_subscription.recv().await.unwrap();

        // Pause the bridge
        bridge_pause_tx.send(BRIDGE_PAUSED).unwrap();

        // Kick it again
        execution_tx
            .send(CertifiedBridgeActionExecutionWrapper(action_certificate, 0))
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        // Nothing is sent to execute
        assert_eq!(
            tx_subscription.try_recv().unwrap_err(),
            tokio::sync::broadcast::error::TryRecvError::Empty
        );
        // Still in WAL
        assert_eq!(
            store.get_all_pending_actions()[&action_digest],
            action.clone()
        );
    }

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_action_executor_handle_new_token() {
        let new_token_id = 255u8; // token id that does not exist
        let new_type_tag = TypeTag::from_str("0xbeef::beef::BEEF").unwrap();
        let SetupData {
            execution_tx,
            iota_client_mock,
            mut tx_subscription,
            secrets,
            dummy_iota_key,
            mock0,
            mock1,
            mock2,
            mock3,
            gas_object_ref,
            iota_address,
            iota_token_type_tags,
            ..
        } = setup().await;
        let mut id_token_map: HashMap<u8, TypeTag> = (*iota_token_type_tags.load().clone()).clone();
        let (action_certificate, _, _) = get_bridge_authority_approved_action(
            vec![&mock0, &mock1, &mock2, &mock3],
            vec![&secrets[0], &secrets[1], &secrets[2], &secrets[3]],
            Some(new_token_id),
            false, /* we need an eth -> iota action that entails the new token type tag in
                    * transaction building */
        );

        let action = action_certificate.data().clone();
        let arg = DUMMY_MUTABLE_BRIDGE_OBJECT_ARG;
        let tx_data = build_iota_transaction(
            iota_address,
            &gas_object_ref,
            action_certificate.clone(),
            arg,
            &maplit::hashmap! {
                new_token_id => new_type_tag.clone()
            },
            1000,
        )
        .unwrap();
        let tx_digest = get_tx_digest(tx_data, &dummy_iota_key);
        mock_transaction_error(
            &iota_client_mock,
            tx_digest,
            BridgeError::Generic("some random error".to_string()),
            true,
        );

        let gas_coin = GasCoin::new_for_testing(1_000_000_000_000); // dummy gas coin
        iota_client_mock.add_gas_object_info(
            gas_coin.clone(),
            gas_object_ref,
            Owner::AddressOwner(iota_address),
        );
        iota_client_mock.set_action_onchain_status(&action, BridgeActionStatus::Pending);

        // Kick it (send to the execution queue, skipping the signing queue)
        execution_tx
            .send(CertifiedBridgeActionExecutionWrapper(
                action_certificate.clone(),
                0,
            ))
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        // Nothing is sent to execute, because the token id does not exist yet
        assert_eq!(
            tx_subscription.try_recv().unwrap_err(),
            tokio::sync::broadcast::error::TryRecvError::Empty
        );

        // Now insert the new token id
        id_token_map.insert(new_token_id, new_type_tag);
        iota_token_type_tags.store(Arc::new(id_token_map));

        // Kick it again
        execution_tx
            .send(CertifiedBridgeActionExecutionWrapper(
                action_certificate.clone(),
                0,
            ))
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        // The action is sent to execution
        assert_eq!(tx_subscription.recv().await.unwrap(), tx_digest);
    }

    fn mock_bridge_authority_sigs(
        mocks: Vec<&BridgeRequestMockHandler>,
        action: &BridgeAction,
        secrets: Vec<&BridgeAuthorityKeyPair>,
        iota_tx_digest: TransactionDigest,
        iota_tx_event_index: u16,
    ) -> BTreeMap<BridgeAuthorityPublicKeyBytes, BridgeAuthorityRecoverableSignature> {
        assert_eq!(mocks.len(), secrets.len());
        let mut signed_actions = BTreeMap::new();
        for (mock, secret) in mocks.iter().zip(secrets.iter()) {
            let signed_action = sign_action_with_key(action, secret);
            mock.add_iota_event_response(
                iota_tx_digest,
                iota_tx_event_index,
                Ok(signed_action.clone()),
            );
            signed_actions.insert(secret.public().into(), signed_action.into_sig().signature);
        }
        signed_actions
    }

    fn mock_bridge_authority_signing_errors(
        mocks: Vec<&BridgeRequestMockHandler>,
        iota_tx_digest: TransactionDigest,
        iota_tx_event_index: u16,
    ) {
        for mock in mocks {
            mock.add_iota_event_response(
                iota_tx_digest,
                iota_tx_event_index,
                Err(BridgeError::RestAPI("small issue".into())),
            );
        }
    }

    /// Create a BridgeAction and mock authorities to return signatures
    fn get_bridge_authority_approved_action(
        mocks: Vec<&BridgeRequestMockHandler>,
        secrets: Vec<&BridgeAuthorityKeyPair>,
        token_id: Option<u8>,
        iota_to_eth: bool,
    ) -> (VerifiedCertifiedBridgeAction, TransactionDigest, u16) {
        let iota_tx_digest = TransactionDigest::random();
        let iota_tx_event_index = 1;
        let action = if iota_to_eth {
            get_test_iota_to_eth_bridge_action(
                Some(iota_tx_digest),
                Some(iota_tx_event_index),
                None,
                None,
                None,
                None,
                token_id,
            )
        } else {
            get_test_eth_to_iota_bridge_action(None, None, None, token_id)
        };

        let sigs = mock_bridge_authority_sigs(
            mocks,
            &action,
            secrets,
            iota_tx_digest,
            iota_tx_event_index,
        );
        let certified_action = CertifiedBridgeAction::new_from_data_and_sig(
            action,
            BridgeCommitteeValiditySignInfo { signatures: sigs },
        );
        (
            VerifiedCertifiedBridgeAction::new_from_verified(certified_action),
            iota_tx_digest,
            iota_tx_event_index,
        )
    }

    fn get_tx_digest(tx_data: TransactionData, dummy_iota_key: &IotaKeyPair) -> TransactionDigest {
        let sig = Signature::new_secure(
            &IntentMessage::new(Intent::iota_transaction(), &tx_data),
            dummy_iota_key,
        );
        let signed_tx = Transaction::from_data(tx_data, vec![sig]);
        *signed_tx.digest()
    }

    /// Why is `wildcard` needed? This is because authority signatures
    /// are part of transaction data. Depending on whose signatures
    /// are included in what order, this may change the tx digest.
    fn mock_transaction_response(
        iota_client_mock: &IotaMockClient,
        tx_digest: TransactionDigest,
        status: IotaExecutionStatus,
        events: Option<Vec<IotaEvent>>,
        wildcard: bool,
    ) {
        let mut response = IotaTransactionBlockResponse::new(tx_digest);
        let effects = IotaTransactionBlockEffects::new_for_testing(tx_digest, status);
        if let Some(events) = events {
            response.events = Some(IotaTransactionBlockEvents { data: events });
        }
        response.effects = Some(effects);
        if wildcard {
            iota_client_mock.set_wildcard_transaction_response(Ok(response));
        } else {
            iota_client_mock.add_transaction_response(tx_digest, Ok(response));
        }
    }

    fn mock_transaction_error(
        iota_client_mock: &IotaMockClient,
        tx_digest: TransactionDigest,
        error: BridgeError,
        wildcard: bool,
    ) {
        if wildcard {
            iota_client_mock.set_wildcard_transaction_response(Err(error));
        } else {
            iota_client_mock.add_transaction_response(tx_digest, Err(error));
        }
    }

    struct SetupData {
        signing_tx: iota_metrics::metered_channel::Sender<BridgeActionExecutionWrapper>,
        execution_tx: iota_metrics::metered_channel::Sender<CertifiedBridgeActionExecutionWrapper>,
        iota_client_mock: IotaMockClient,
        tx_subscription: tokio::sync::broadcast::Receiver<TransactionDigest>,
        store: Arc<BridgeOrchestratorTables>,
        secrets: Vec<BridgeAuthorityKeyPair>,
        dummy_iota_key: IotaKeyPair,
        mock0: BridgeRequestMockHandler,
        mock1: BridgeRequestMockHandler,
        mock2: BridgeRequestMockHandler,
        mock3: BridgeRequestMockHandler,
        #[expect(unused)]
        handles: Vec<tokio::task::JoinHandle<()>>,
        gas_object_ref: ObjectRef,
        iota_address: IotaAddress,
        iota_token_type_tags: Arc<ArcSwap<HashMap<u8, TypeTag>>>,
        bridge_pause_tx: tokio::sync::watch::Sender<IsBridgePaused>,
    }

    async fn setup() -> SetupData {
        telemetry_subscribers::init_for_testing();
        let registry = Registry::new();
        iota_metrics::init_metrics(&registry);
        init_all_struct_tags();

        let (iota_address, kp): (_, fastcrypto::secp256k1::Secp256k1KeyPair) = get_key_pair();
        let iota_key = IotaKeyPair::from(kp);
        let gas_object_ref = random_object_ref();
        let temp_dir = tempfile::tempdir().unwrap();
        let store = BridgeOrchestratorTables::new(temp_dir.path());
        let iota_client_mock = IotaMockClient::default();
        let tx_subscription = iota_client_mock.subscribe_to_requested_transactions();
        let iota_client = Arc::new(IotaClient::new_for_testing(iota_client_mock.clone()));

        // The dummy key is used to sign transaction so we can get TransactionDigest
        // easily. User signature is not part of the transaction so it does not
        // matter which key it is.
        let (_, dummy_kp): (_, fastcrypto::secp256k1::Secp256k1KeyPair) = get_key_pair();
        let dummy_iota_key = IotaKeyPair::from(dummy_kp);

        let mock0 = BridgeRequestMockHandler::new();
        let mock1 = BridgeRequestMockHandler::new();
        let mock2 = BridgeRequestMockHandler::new();
        let mock3 = BridgeRequestMockHandler::new();

        let (mut handles, authorities, secrets) = get_test_authorities_and_run_mock_bridge_server(
            vec![2500, 2500, 2500, 2500],
            vec![mock0.clone(), mock1.clone(), mock2.clone(), mock3.clone()],
        );

        let committee = BridgeCommittee::new(authorities).unwrap();

        let agg = Arc::new(ArcSwap::new(Arc::new(BridgeAuthorityAggregator::new(
            Arc::new(committee),
        ))));
        let metrics = Arc::new(BridgeMetrics::new(&registry));
        let iota_token_type_tags = iota_client.get_token_id_map().await.unwrap();
        let iota_token_type_tags = Arc::new(ArcSwap::new(Arc::new(iota_token_type_tags)));
        let (bridge_pause_tx, bridge_pause_rx) = tokio::sync::watch::channel(false);
        let executor = BridgeActionExecutor::new(
            iota_client.clone(),
            agg.clone(),
            store.clone(),
            iota_key,
            iota_address,
            gas_object_ref.0,
            iota_token_type_tags.clone(),
            bridge_pause_rx,
            metrics,
        )
        .await;

        let (executor_handle, signing_tx, execution_tx) = executor.run_inner();
        handles.extend(executor_handle);

        SetupData {
            signing_tx,
            execution_tx,
            iota_client_mock,
            tx_subscription,
            store,
            secrets,
            dummy_iota_key,
            mock0,
            mock1,
            mock2,
            mock3,
            handles,
            gas_object_ref,
            iota_address,
            iota_token_type_tags,
            bridge_pause_tx,
        }
    }
}
