// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[cfg(any(msim, test))]
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::{cmp::min, ops::Add, sync::Arc, time::Duration};

use arc_swap::ArcSwap;
use iota_metrics::LATENCY_SEC_BUCKETS;
use iota_types::{
    base_types::{AuthorityName, TransactionDigest},
    transaction::VerifiedSignedTransaction,
};
use prometheus::{
    Histogram, IntCounter, Registry, register_histogram_with_registry,
    register_int_counter_with_registry,
};
use tokio::{select, time::Instant};
use tracing::{debug, error, trace};

use crate::{
    authority_aggregator::AuthorityAggregator, authority_client::AuthorityAPI,
    execution_cache::TransactionCacheRead,
};

struct ValidatorTxFinalizerMetrics {
    num_finalization_attempts: IntCounter,
    num_successful_finalizations: IntCounter,
    finalization_latency: Histogram,
    validator_tx_finalizer_attempt_delay: Histogram,
    #[cfg(any(msim, test))]
    num_finalization_attempts_for_testing: AtomicU64,
    #[cfg(test)]
    num_successful_finalizations_for_testing: AtomicU64,
}

impl ValidatorTxFinalizerMetrics {
    fn new(registry: &Registry) -> Self {
        Self {
            num_finalization_attempts: register_int_counter_with_registry!(
                "validator_tx_finalizer_num_finalization_attempts",
                "Total number of attempts to finalize a transaction",
                registry,
            )
            .unwrap(),
            num_successful_finalizations: register_int_counter_with_registry!(
                "validator_tx_finalizer_num_successful_finalizations",
                "Number of transactions successfully finalized",
                registry,
            )
            .unwrap(),
            finalization_latency: register_histogram_with_registry!(
                "validator_tx_finalizer_finalization_latency",
                "Latency of transaction finalization",
                LATENCY_SEC_BUCKETS.to_vec(),
                registry,
            )
            .unwrap(),
            validator_tx_finalizer_attempt_delay: register_histogram_with_registry!(
                "validator_tx_finalizer_attempt_delay",
                "Duration that a validator in the committee waited before attempting to finalize the transaction",
                vec![60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0, 160.0, 170.0, 180.0],
                registry,
            )
            .unwrap(),
            #[cfg(any(msim, test))]
            num_finalization_attempts_for_testing: AtomicU64::new(0),
            #[cfg(test)]
            num_successful_finalizations_for_testing: AtomicU64::new(0),
        }
    }

    fn start_finalization(&self) -> Instant {
        self.num_finalization_attempts.inc();
        #[cfg(any(msim, test))]
        self.num_finalization_attempts_for_testing
            .fetch_add(1, Relaxed);
        Instant::now()
    }

    fn finalization_succeeded(&self, start_time: Instant) {
        let latency = start_time.elapsed();
        self.num_successful_finalizations.inc();
        self.finalization_latency.observe(latency.as_secs_f64());
        #[cfg(test)]
        self.num_successful_finalizations_for_testing
            .fetch_add(1, Relaxed);
    }
}

pub struct ValidatorTxFinalizerConfig {
    pub tx_finalization_delay: Duration,
    pub tx_finalization_timeout: Duration,
    /// Incremental delay for validators to wake up to finalize a transaction.
    pub validator_delay_increments_sec: u64,
    pub validator_max_delay: Duration,
}

#[cfg(not(any(msim, test)))]
impl Default for ValidatorTxFinalizerConfig {
    fn default() -> Self {
        Self {
            // Only wake up the transaction finalization task for a given transaction
            // after 1 mins of seeing it. This gives plenty of time for the transaction
            // to become final in the normal way. We also don't want this delay to be too long
            // to reduce memory usage held up by the finalizer threads.
            tx_finalization_delay: Duration::from_secs(60),
            // If a transaction can not be finalized within 1 min of being woken up, give up.
            tx_finalization_timeout: Duration::from_secs(60),
            validator_delay_increments_sec: 10,
            validator_max_delay: Duration::from_secs(180),
        }
    }
}

#[cfg(any(msim, test))]
impl Default for ValidatorTxFinalizerConfig {
    fn default() -> Self {
        Self {
            tx_finalization_delay: Duration::from_secs(5),
            tx_finalization_timeout: Duration::from_secs(60),
            validator_delay_increments_sec: 1,
            validator_max_delay: Duration::from_secs(15),
        }
    }
}

/// The `ValidatorTxFinalizer` is responsible for finalizing transactions that
/// have been signed by the validator. It does this by waiting for a delay
/// after the transaction has been signed, and then attempting to finalize
/// the transaction if it has not yet been done by a fullnode.
pub struct ValidatorTxFinalizer<C: Clone> {
    agg: Arc<ArcSwap<AuthorityAggregator<C>>>,
    name: AuthorityName,
    config: Arc<ValidatorTxFinalizerConfig>,
    metrics: Arc<ValidatorTxFinalizerMetrics>,
}

impl<C: Clone> ValidatorTxFinalizer<C> {
    pub fn new(
        agg: Arc<ArcSwap<AuthorityAggregator<C>>>,
        name: AuthorityName,
        registry: &Registry,
    ) -> Self {
        Self {
            agg,
            name,
            config: Arc::new(ValidatorTxFinalizerConfig::default()),
            metrics: Arc::new(ValidatorTxFinalizerMetrics::new(registry)),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_testing(
        agg: Arc<ArcSwap<AuthorityAggregator<C>>>,
        name: AuthorityName,
    ) -> Self {
        Self::new(agg, name, &Registry::new())
    }

    #[cfg(test)]
    pub(crate) fn auth_agg(&self) -> &Arc<ArcSwap<AuthorityAggregator<C>>> {
        &self.agg
    }

    #[cfg(any(msim, test))]
    pub fn num_finalization_attempts_for_testing(&self) -> u64 {
        self.metrics
            .num_finalization_attempts_for_testing
            .load(Relaxed)
    }
}

impl<C> ValidatorTxFinalizer<C>
where
    C: Clone + AuthorityAPI + Send + Sync + 'static,
{
    pub async fn track_signed_tx(
        &self,
        cache_read: Arc<dyn TransactionCacheRead>,
        tx: VerifiedSignedTransaction,
    ) {
        let tx_digest = *tx.digest();
        trace!(?tx_digest, "Tracking signed transaction");
        match self.delay_and_finalize_tx(cache_read, tx).await {
            Ok(did_run) => {
                if did_run {
                    debug!(?tx_digest, "Transaction finalized");
                }
            }
            Err(err) => {
                error!(?tx_digest, ?err, "Failed to finalize transaction");
            }
        }
    }

    async fn delay_and_finalize_tx(
        &self,
        cache_read: Arc<dyn TransactionCacheRead>,
        tx: VerifiedSignedTransaction,
    ) -> anyhow::Result<bool> {
        let tx_digest = *tx.digest();
        let Some(tx_finalization_delay) = self.determine_finalization_delay(&tx_digest) else {
            return Ok(false);
        };
        let digests = [tx_digest];
        select! {
            _ = tokio::time::sleep(tx_finalization_delay) => {
                trace!(?tx_digest, "Waking up to finalize transaction");
            }
            _ = cache_read.notify_read_executed_effects_digests(&digests) => {
                trace!(?tx_digest, "Transaction already finalized");
                return Ok(false);
            }
        }

        self.metrics
            .validator_tx_finalizer_attempt_delay
            .observe(tx_finalization_delay.as_secs_f64());
        let start_time = self.metrics.start_finalization();
        debug!(
            ?tx_digest,
            "Invoking authority aggregator to finalize transaction"
        );
        tokio::time::timeout(
            self.config.tx_finalization_timeout,
            self.agg
                .load()
                .execute_transaction_block(tx.into_unsigned().inner(), None),
        )
        .await??;
        self.metrics.finalization_succeeded(start_time);
        Ok(true)
    }

    // We want to avoid all validators waking up at the same time to finalize the
    // same transaction. That can lead to a waste of resource and flood the
    // network unnecessarily. Here we use the transaction digest to determine an
    // order of all validators. Validators will wake up one by one with
    // incremental delays to finalize the transaction. The hope is that the
    // first few should be able to finalize the transaction, and the rest will
    // see it already executed and do not need to do anything.
    fn determine_finalization_delay(&self, tx_digest: &TransactionDigest) -> Option<Duration> {
        let agg = self.agg.load();
        let order = agg.committee.shuffle_by_stake_from_tx_digest(tx_digest);
        let Some(position) = order.iter().position(|&name| name == self.name) else {
            // Somehow the validator is not found in the committee. This should never
            // happen. TODO: This is where we should report system invariant
            // violation.
            error!("Validator {} not found in the committee", self.name);
            return None;
        };
        // TODO: As an optimization, we could also limit the number of validators that
        // would do this.
        let extra_delay = position as u64 * self.config.validator_delay_increments_sec;
        let delay = self
            .config
            .tx_finalization_delay
            .add(Duration::from_secs(extra_delay));
        Some(min(delay, self.config.validator_max_delay))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cmp::min,
        collections::BTreeMap,
        iter,
        net::SocketAddr,
        num::NonZeroUsize,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering::Relaxed},
        },
    };

    use arc_swap::ArcSwap;
    use async_trait::async_trait;
    use iota_macros::sim_test;
    use iota_swarm_config::network_config_builder::ConfigBuilder;
    use iota_test_transaction_builder::TestTransactionBuilder;
    use iota_types::{
        base_types::{AuthorityName, IotaAddress, ObjectID, TransactionDigest},
        committee::{CommitteeTrait, StakeUnit},
        crypto::{AccountKeyPair, get_account_key_pair},
        effects::{TransactionEffectsAPI, TransactionEvents},
        error::IotaError,
        executable_transaction::VerifiedExecutableTransaction,
        iota_system_state::IotaSystemState,
        messages_checkpoint::{CheckpointRequest, CheckpointResponse},
        messages_grpc::{
            HandleCertificateRequestV1, HandleCertificateResponseV1,
            HandleSoftBundleCertificatesRequestV1, HandleSoftBundleCertificatesResponseV1,
            HandleTransactionResponse, ObjectInfoRequest, ObjectInfoResponse, SystemStateRequest,
            TransactionInfoRequest, TransactionInfoResponse,
        },
        object::Object,
        transaction::{
            SignedTransaction, Transaction, VerifiedCertificate, VerifiedSignedTransaction,
            VerifiedTransaction,
        },
        utils::to_sender_signed_transaction,
    };

    use crate::{
        authority::{AuthorityState, test_authority_builder::TestAuthorityBuilder},
        authority_aggregator::{AuthorityAggregator, AuthorityAggregatorBuilder},
        authority_client::AuthorityAPI,
        validator_tx_finalizer::ValidatorTxFinalizer,
    };

    #[derive(Clone)]
    struct MockAuthorityClient {
        authority: Arc<AuthorityState>,
        inject_fault: Arc<AtomicBool>,
    }

    #[async_trait]
    impl AuthorityAPI for MockAuthorityClient {
        async fn handle_transaction(
            &self,
            transaction: Transaction,
            _client_addr: Option<SocketAddr>,
        ) -> Result<HandleTransactionResponse, IotaError> {
            if self.inject_fault.load(Relaxed) {
                return Err(IotaError::Timeout);
            }
            let epoch_store = self.authority.epoch_store_for_testing();
            self.authority
                .handle_transaction(
                    &epoch_store,
                    VerifiedTransaction::new_unchecked(transaction),
                )
                .await
        }

        async fn handle_certificate_v1(
            &self,
            request: HandleCertificateRequestV1,
            _client_addr: Option<SocketAddr>,
        ) -> Result<HandleCertificateResponseV1, IotaError> {
            let epoch_store = self.authority.epoch_store_for_testing();
            let (effects, _) = self
                .authority
                .try_execute_immediately(
                    &VerifiedExecutableTransaction::new_from_certificate(
                        VerifiedCertificate::new_unchecked(request.certificate),
                    ),
                    None,
                    &epoch_store,
                )
                .await?;
            let events = match effects.events_digest() {
                None => TransactionEvents::default(),
                Some(digest) => self.authority.get_transaction_events(digest)?,
            };
            let signed_effects = self
                .authority
                .sign_effects(effects, &epoch_store)?
                .into_inner();
            Ok(HandleCertificateResponseV1 {
                signed_effects,
                events: Some(events),
                input_objects: None,
                output_objects: None,
                auxiliary_data: None,
            })
        }

        async fn handle_soft_bundle_certificates_v1(
            &self,
            _request: HandleSoftBundleCertificatesRequestV1,
            _client_addr: Option<SocketAddr>,
        ) -> Result<HandleSoftBundleCertificatesResponseV1, IotaError> {
            unimplemented!()
        }

        async fn handle_object_info_request(
            &self,
            _request: ObjectInfoRequest,
        ) -> Result<ObjectInfoResponse, IotaError> {
            unimplemented!()
        }

        async fn handle_transaction_info_request(
            &self,
            _request: TransactionInfoRequest,
        ) -> Result<TransactionInfoResponse, IotaError> {
            unimplemented!()
        }

        async fn handle_checkpoint(
            &self,
            _request: CheckpointRequest,
        ) -> Result<CheckpointResponse, IotaError> {
            unimplemented!()
        }

        async fn handle_system_state_object(
            &self,
            _request: SystemStateRequest,
        ) -> Result<IotaSystemState, IotaError> {
            unimplemented!()
        }
    }

    #[sim_test]
    async fn test_validator_tx_finalizer_basic_flow() {
        telemetry_subscribers::init_for_testing();
        let (sender, keypair) = get_account_key_pair();
        let gas_object = Object::with_owner_for_testing(sender);
        let gas_object_id = gas_object.id();
        let (states, auth_agg, clients) = create_validators(gas_object).await;
        let finalizer1 = ValidatorTxFinalizer::new_for_testing(auth_agg.clone(), states[0].name);
        let signed_tx = create_tx(&clients, &states[0], sender, &keypair, gas_object_id).await;
        let tx_digest = *signed_tx.digest();
        let cache_read = states[0].get_transaction_cache_reader().clone();
        let metrics = finalizer1.metrics.clone();
        let handle = tokio::spawn(async move {
            finalizer1.track_signed_tx(cache_read, signed_tx).await;
        });
        handle.await.unwrap();
        check_quorum_execution(&auth_agg.load(), &clients, &tx_digest, true);
        assert_eq!(
            metrics.num_finalization_attempts_for_testing.load(Relaxed),
            1
        );
        assert_eq!(
            metrics
                .num_successful_finalizations_for_testing
                .load(Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn test_validator_tx_finalizer_new_epoch() {
        let (sender, keypair) = get_account_key_pair();
        let gas_object = Object::with_owner_for_testing(sender);
        let gas_object_id = gas_object.id();
        let (states, auth_agg, clients) = create_validators(gas_object).await;
        let finalizer1 = ValidatorTxFinalizer::new_for_testing(auth_agg.clone(), states[0].name);
        let signed_tx = create_tx(&clients, &states[0], sender, &keypair, gas_object_id).await;
        let tx_digest = *signed_tx.digest();
        let epoch_store = states[0].epoch_store_for_testing();
        let cache_read = states[0].get_transaction_cache_reader().clone();

        let metrics = finalizer1.metrics.clone();
        let handle = tokio::spawn(async move {
            let _ = epoch_store
                .within_alive_epoch(finalizer1.track_signed_tx(cache_read, signed_tx))
                .await;
        });
        states[0].reconfigure_for_testing().await;
        handle.await.unwrap();
        check_quorum_execution(&auth_agg.load(), &clients, &tx_digest, false);
        assert_eq!(
            metrics.num_finalization_attempts_for_testing.load(Relaxed),
            0
        );
        assert_eq!(
            metrics
                .num_successful_finalizations_for_testing
                .load(Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn test_validator_tx_finalizer_auth_agg_reconfig() {
        let (sender, _) = get_account_key_pair();
        let gas_object = Object::with_owner_for_testing(sender);
        let (states, auth_agg, _clients) = create_validators(gas_object).await;
        let finalizer1 = ValidatorTxFinalizer::new_for_testing(auth_agg.clone(), states[0].name);
        let mut new_auth_agg = (**auth_agg.load()).clone();
        let mut new_committee = (*new_auth_agg.committee).clone();
        new_committee.epoch = 100;
        new_auth_agg.committee = Arc::new(new_committee);
        auth_agg.store(Arc::new(new_auth_agg));
        assert_eq!(
            finalizer1.auth_agg().load().committee.epoch,
            100,
            "AuthorityAggregator not updated"
        );
    }

    #[tokio::test]
    async fn test_validator_tx_finalizer_already_executed() {
        telemetry_subscribers::init_for_testing();
        let (sender, keypair) = get_account_key_pair();
        let gas_object = Object::with_owner_for_testing(sender);
        let gas_object_id = gas_object.id();
        let (states, auth_agg, clients) = create_validators(gas_object).await;
        let finalizer1 = ValidatorTxFinalizer::new_for_testing(auth_agg.clone(), states[0].name);
        let signed_tx = create_tx(&clients, &states[0], sender, &keypair, gas_object_id).await;
        let tx_digest = *signed_tx.digest();
        let cache_read = states[0].get_transaction_cache_reader().clone();

        let metrics = finalizer1.metrics.clone();
        let signed_tx_clone = signed_tx.clone();
        let handle = tokio::spawn(async move {
            finalizer1
                .track_signed_tx(cache_read, signed_tx_clone)
                .await;
        });
        auth_agg
            .load()
            .execute_transaction_block(&signed_tx.into_inner().into_unsigned(), None)
            .await
            .unwrap();
        handle.await.unwrap();
        check_quorum_execution(&auth_agg.load(), &clients, &tx_digest, true);
        assert_eq!(
            metrics.num_finalization_attempts_for_testing.load(Relaxed),
            0
        );
        assert_eq!(
            metrics
                .num_successful_finalizations_for_testing
                .load(Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn test_validator_tx_finalizer_timeout() {
        telemetry_subscribers::init_for_testing();
        let (sender, keypair) = get_account_key_pair();
        let gas_object = Object::with_owner_for_testing(sender);
        let gas_object_id = gas_object.id();
        let (states, auth_agg, clients) = create_validators(gas_object).await;
        let finalizer1 = ValidatorTxFinalizer::new_for_testing(auth_agg.clone(), states[0].name);
        let signed_tx = create_tx(&clients, &states[0], sender, &keypair, gas_object_id).await;
        let tx_digest = *signed_tx.digest();
        let cache_read = states[0].get_transaction_cache_reader().clone();
        for client in clients.values() {
            client.inject_fault.store(true, Relaxed);
        }

        let metrics = finalizer1.metrics.clone();
        let signed_tx_clone = signed_tx.clone();
        let handle = tokio::spawn(async move {
            finalizer1
                .track_signed_tx(cache_read, signed_tx_clone)
                .await;
        });
        handle.await.unwrap();
        check_quorum_execution(&auth_agg.load(), &clients, &tx_digest, false);
        assert_eq!(
            metrics.num_finalization_attempts_for_testing.load(Relaxed),
            1
        );
        assert_eq!(
            metrics
                .num_successful_finalizations_for_testing
                .load(Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn test_validator_tx_finalizer_determine_finalization_delay() {
        const COMMITTEE_SIZE: usize = 15;
        let network_config = ConfigBuilder::new_with_temp_dir()
            .committee_size(NonZeroUsize::new(COMMITTEE_SIZE).unwrap())
            .build();
        let (auth_agg, _) = AuthorityAggregatorBuilder::from_network_config(&network_config)
            .build_network_clients();
        let auth_agg = Arc::new(auth_agg);
        let finalizers = (0..COMMITTEE_SIZE)
            .map(|idx| {
                ValidatorTxFinalizer::new_for_testing(
                    Arc::new(ArcSwap::new(auth_agg.clone())),
                    auth_agg.committee.voting_rights[idx].0,
                )
            })
            .collect::<Vec<_>>();
        let config = finalizers[0].config.clone();
        for _ in 0..100 {
            let tx_digest = TransactionDigest::random();
            let mut delays: Vec<_> = finalizers
                .iter()
                .map(|finalizer| {
                    finalizer
                        .determine_finalization_delay(&tx_digest)
                        .map(|delay| delay.as_secs())
                        .unwrap()
                })
                .collect();
            delays.sort();
            for (idx, delay) in delays.iter().enumerate() {
                assert_eq!(
                    *delay,
                    min(
                        config.validator_max_delay.as_secs(),
                        config.tx_finalization_delay.as_secs()
                            + idx as u64 * config.validator_delay_increments_sec
                    )
                );
            }
        }
    }

    async fn create_validators(
        gas_object: Object,
    ) -> (
        Vec<Arc<AuthorityState>>,
        Arc<ArcSwap<AuthorityAggregator<MockAuthorityClient>>>,
        BTreeMap<AuthorityName, MockAuthorityClient>,
    ) {
        let network_config = ConfigBuilder::new_with_temp_dir()
            .committee_size(NonZeroUsize::new(4).unwrap())
            .with_objects(iter::once(gas_object))
            .build();
        let mut authority_states = vec![];
        for idx in 0..4 {
            let state = TestAuthorityBuilder::new()
                .with_network_config(&network_config, idx)
                .build()
                .await;
            authority_states.push(state);
        }
        let clients: BTreeMap<_, _> = authority_states
            .iter()
            .map(|state| {
                (
                    state.name,
                    MockAuthorityClient {
                        authority: state.clone(),
                        inject_fault: Arc::new(AtomicBool::new(false)),
                    },
                )
            })
            .collect();
        let auth_agg = AuthorityAggregatorBuilder::from_network_config(&network_config)
            .build_custom_clients(clients.clone());
        (
            authority_states,
            Arc::new(ArcSwap::new(Arc::new(auth_agg))),
            clients,
        )
    }

    async fn create_tx(
        clients: &BTreeMap<AuthorityName, MockAuthorityClient>,
        state: &Arc<AuthorityState>,
        sender: IotaAddress,
        keypair: &AccountKeyPair,
        gas_object_id: ObjectID,
    ) -> VerifiedSignedTransaction {
        let gas_object_ref = state
            .get_object(&gas_object_id)
            .await
            .unwrap()
            .unwrap()
            .compute_object_reference();
        let tx_data = TestTransactionBuilder::new(
            sender,
            gas_object_ref,
            state.reference_gas_price_for_testing().unwrap(),
        )
        .transfer_iota(None, sender)
        .build();
        let tx = to_sender_signed_transaction(tx_data, keypair);
        let response = clients
            .get(&state.name)
            .unwrap()
            .handle_transaction(tx.clone(), None)
            .await
            .unwrap();
        VerifiedSignedTransaction::new_unchecked(SignedTransaction::new_from_data_and_sig(
            tx.into_data(),
            response.status.into_signed_for_testing(),
        ))
    }

    fn check_quorum_execution(
        auth_agg: &Arc<AuthorityAggregator<MockAuthorityClient>>,
        clients: &BTreeMap<AuthorityName, MockAuthorityClient>,
        tx_digest: &TransactionDigest,
        expected: bool,
    ) {
        let quorum = auth_agg.committee.quorum_threshold();
        let executed_weight: StakeUnit = clients
            .iter()
            .filter_map(|(name, client)| {
                client
                    .authority
                    .is_tx_already_executed(tx_digest)
                    .unwrap()
                    .then_some(auth_agg.committee.weight(name))
            })
            .sum();
        assert_eq!(executed_weight >= quorum, expected);
    }
}
