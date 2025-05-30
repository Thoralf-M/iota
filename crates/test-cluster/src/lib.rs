// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::{Future, StreamExt, future::join_all};
use iota_bridge::{
    crypto::{BridgeAuthorityKeyPair, BridgeAuthoritySignInfo},
    iota_transaction_builder::{
        build_add_tokens_on_iota_transaction, build_committee_register_transaction,
    },
    types::{
        BridgeCommitteeValiditySignInfo, CertifiedBridgeAction, VerifiedCertifiedBridgeAction,
    },
    utils::{publish_and_register_coins_return_add_coins_on_iota_action, wait_for_server_to_be_up},
};
use iota_config::{
    Config, IOTA_CLIENT_CONFIG, IOTA_KEYSTORE_FILENAME, IOTA_NETWORK_CONFIG, NodeConfig,
    PersistedConfig,
    genesis::Genesis,
    local_ip_utils::get_available_port,
    node::{AuthorityOverloadConfig, DBCheckpointConfig, RunWithRange},
};
use iota_core::{
    authority_aggregator::AuthorityAggregator, authority_client::NetworkAuthorityClient,
};
use iota_genesis_builder::SnapshotSource;
use iota_json_rpc_api::{
    BridgeReadApiClient, IndexerApiClient, TransactionBuilderClient, WriteApiClient,
    error_object_from_rpc,
};
use iota_json_rpc_types::{
    IotaExecutionStatus, IotaObjectDataOptions, IotaObjectResponse, IotaObjectResponseQuery,
    IotaTransactionBlockEffectsAPI, IotaTransactionBlockResponse,
    IotaTransactionBlockResponseOptions, TransactionFilter,
};
use iota_keys::keystore::{AccountKeystore, FileBasedKeystore, Keystore};
use iota_node::IotaNodeHandle;
use iota_protocol_config::ProtocolVersion;
use iota_sdk::{
    IotaClient, IotaClientBuilder,
    apis::QuorumDriverApi,
    iota_client_config::{IotaClientConfig, IotaEnv},
    wallet_context::WalletContext,
};
use iota_swarm::memory::{Swarm, SwarmBuilder};
use iota_swarm_config::{
    genesis_config::{AccountConfig, DEFAULT_GAS_AMOUNT, GenesisConfig, ValidatorGenesisConfig},
    network_config::{NetworkConfig, NetworkConfigLight},
    network_config_builder::{
        ProtocolVersionsConfig, StateAccumulatorEnabledCallback, StateAccumulatorV1EnabledConfig,
        SupportedProtocolVersionsCallback,
    },
    node_config_builder::{FullnodeConfigBuilder, ValidatorConfigBuilder},
};
use iota_test_transaction_builder::TestTransactionBuilder;
use iota_types::{
    IOTA_BRIDGE_OBJECT_ID,
    base_types::{AuthorityName, ConciseableName, IotaAddress, ObjectID, ObjectRef},
    bridge::{
        BridgeSummary, BridgeTrait, TOKEN_ID_BTC, TOKEN_ID_ETH, TOKEN_ID_USDC, TOKEN_ID_USDT,
        get_bridge, get_bridge_obj_initial_shared_version,
    },
    committee::{Committee, CommitteeTrait, EpochId},
    crypto::{AccountKeyPair, IotaKeyPair, KeypairTraits, ToFromBytes, get_key_pair},
    effects::{TransactionEffects, TransactionEvents},
    error::IotaResult,
    governance::MIN_VALIDATOR_JOINING_STAKE_NANOS,
    iota_system_state::{
        IotaSystemState, IotaSystemStateTrait,
        epoch_start_iota_system_state::EpochStartSystemStateTrait,
    },
    message_envelope::Message,
    messages_grpc::HandleCertificateRequestV1,
    object::Object,
    quorum_driver_types::ExecuteTransactionRequestType,
    supported_protocol_versions::SupportedProtocolVersions,
    traffic_control::{PolicyConfig, RemoteFirewallConfig},
    transaction::{
        CertifiedTransaction, ObjectArg, Transaction, TransactionData, TransactionDataAPI,
        TransactionKind,
    },
    utils::to_sender_signed_transaction,
};
use jsonrpsee::{
    core::RpcResult,
    http_client::{HttpClient, HttpClientBuilder},
};
use rand::{distributions::*, rngs::OsRng, seq::SliceRandom};
use tokio::{
    task::JoinHandle,
    time::{Instant, sleep, timeout},
};
use tracing::{error, info};

const NUM_VALIDATOR: usize = 4;

pub struct FullNodeHandle {
    pub iota_node: IotaNodeHandle,
    pub iota_client: IotaClient,
    pub rpc_client: HttpClient,
    pub rpc_url: String,
}

impl FullNodeHandle {
    pub async fn new(iota_node: IotaNodeHandle, json_rpc_address: SocketAddr) -> Self {
        let rpc_url = format!("http://{}", json_rpc_address);
        let rpc_client = HttpClientBuilder::default().build(&rpc_url).unwrap();

        let iota_client = IotaClientBuilder::default().build(&rpc_url).await.unwrap();

        Self {
            iota_node,
            iota_client,
            rpc_client,
            rpc_url,
        }
    }
}

struct Faucet {
    address: IotaAddress,
    keypair: Arc<tokio::sync::Mutex<IotaKeyPair>>,
}

pub struct TestCluster {
    pub swarm: Swarm,
    pub wallet: WalletContext,
    pub fullnode_handle: FullNodeHandle,
    pub bridge_authority_keys: Option<Vec<BridgeAuthorityKeyPair>>,
    pub bridge_server_ports: Option<Vec<u16>>,
    faucet: Option<Faucet>,
}

impl TestCluster {
    pub fn rpc_client(&self) -> &HttpClient {
        &self.fullnode_handle.rpc_client
    }

    pub fn iota_client(&self) -> &IotaClient {
        &self.fullnode_handle.iota_client
    }

    pub fn quorum_driver_api(&self) -> &QuorumDriverApi {
        self.iota_client().quorum_driver_api()
    }

    pub fn rpc_url(&self) -> &str {
        &self.fullnode_handle.rpc_url
    }

    pub fn wallet(&mut self) -> &WalletContext {
        &self.wallet
    }

    pub fn wallet_mut(&mut self) -> &mut WalletContext {
        &mut self.wallet
    }

    pub fn get_addresses(&self) -> Vec<IotaAddress> {
        self.wallet.get_addresses()
    }

    // Helper function to get the 0th address in WalletContext
    pub fn get_address_0(&self) -> IotaAddress {
        self.get_addresses()[0]
    }

    // Helper function to get the 1st address in WalletContext
    pub fn get_address_1(&self) -> IotaAddress {
        self.get_addresses()[1]
    }

    // Helper function to get the 2nd address in WalletContext
    pub fn get_address_2(&self) -> IotaAddress {
        self.get_addresses()[2]
    }

    pub fn fullnode_config_builder(&self) -> FullnodeConfigBuilder {
        self.swarm.get_fullnode_config_builder()
    }

    pub fn committee(&self) -> Arc<Committee> {
        self.fullnode_handle
            .iota_node
            .with(|node| node.state().epoch_store_for_testing().committee().clone())
    }

    /// Convenience method to start a new fullnode in the test cluster.
    pub async fn spawn_new_fullnode(&mut self) -> FullNodeHandle {
        self.start_fullnode_from_config(
            self.fullnode_config_builder()
                .build(&mut OsRng, self.swarm.config()),
        )
        .await
    }

    pub async fn start_fullnode_from_config(&mut self, config: NodeConfig) -> FullNodeHandle {
        let json_rpc_address = config.json_rpc_address;
        let node = self.swarm.spawn_new_node(config).await;
        FullNodeHandle::new(node, json_rpc_address).await
    }

    pub fn all_node_handles(&self) -> Vec<IotaNodeHandle> {
        self.swarm
            .all_nodes()
            .flat_map(|n| n.get_node_handle())
            .collect()
    }

    pub fn all_validator_handles(&self) -> Vec<IotaNodeHandle> {
        self.swarm
            .validator_nodes()
            .map(|n| n.get_node_handle().unwrap())
            .collect()
    }

    pub fn get_validator_pubkeys(&self) -> Vec<AuthorityName> {
        self.swarm.active_validators().map(|v| v.name()).collect()
    }

    pub fn get_genesis(&self) -> Genesis {
        self.swarm.config().genesis.clone()
    }

    pub fn stop_node(&self, name: &AuthorityName) {
        self.swarm.node(name).unwrap().stop();
    }

    pub async fn stop_all_validators(&self) {
        info!("Stopping all validators in the cluster");
        self.swarm.active_validators().for_each(|v| v.stop());
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    pub async fn start_all_validators(&self) {
        info!("Starting all validators in the cluster");
        for v in self.swarm.validator_nodes() {
            if v.is_running() {
                continue;
            }
            v.start().await.unwrap();
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    pub async fn start_node(&self, name: &AuthorityName) {
        let node = self.swarm.node(name).unwrap();
        if node.is_running() {
            return;
        }
        node.start().await.unwrap();
    }

    pub async fn spawn_new_validator(
        &mut self,
        genesis_config: ValidatorGenesisConfig,
    ) -> IotaNodeHandle {
        let node_config = ValidatorConfigBuilder::new()
            .build(genesis_config, self.swarm.config().genesis.clone());
        self.swarm.spawn_new_node(node_config).await
    }

    pub fn random_node_restarter(self: &Arc<Self>) -> RandomNodeRestarter {
        RandomNodeRestarter::new(self.clone())
    }

    pub async fn get_reference_gas_price(&self) -> u64 {
        self.iota_client()
            .governance_api()
            .get_reference_gas_price()
            .await
            .expect("failed to get reference gas price")
    }

    pub async fn get_object_from_fullnode_store(&self, object_id: &ObjectID) -> Option<Object> {
        self.fullnode_handle
            .iota_node
            .with_async(|node| async { node.state().get_object(object_id).await.unwrap() })
            .await
    }

    pub async fn get_latest_object_ref(&self, object_id: &ObjectID) -> ObjectRef {
        self.get_object_from_fullnode_store(object_id)
            .await
            .unwrap()
            .compute_object_reference()
    }

    pub async fn get_bridge_summary(&self) -> RpcResult<BridgeSummary> {
        self.iota_client()
            .http()
            .get_latest_bridge()
            .await
            .map_err(error_object_from_rpc)
    }

    pub async fn get_object_or_tombstone_from_fullnode_store(
        &self,
        object_id: ObjectID,
    ) -> ObjectRef {
        self.fullnode_handle
            .iota_node
            .state()
            .get_object_cache_reader()
            .get_latest_object_ref_or_tombstone(object_id)
            .unwrap()
            .unwrap()
    }

    pub async fn wait_for_run_with_range_shutdown_signal(&self) -> Option<RunWithRange> {
        self.wait_for_run_with_range_shutdown_signal_with_timeout(Duration::from_secs(60))
            .await
    }

    pub async fn wait_for_run_with_range_shutdown_signal_with_timeout(
        &self,
        timeout_dur: Duration,
    ) -> Option<RunWithRange> {
        let mut shutdown_channel_rx = self
            .fullnode_handle
            .iota_node
            .with(|node| node.subscribe_to_shutdown_channel());

        timeout(timeout_dur, async move {
            tokio::select! {
                msg = shutdown_channel_rx.recv() =>
                {
                    match msg {
                        Ok(Some(run_with_range)) => Some(run_with_range),
                        Ok(None) => None,
                        Err(e) => {
                            error!("failed recv from iota-node shutdown channel: {}", e);
                            None
                        },
                    }
                },
            }
        })
            .await
            .expect("Timed out waiting for cluster to hit target epoch and recv shutdown signal from iota-node")
    }

    pub async fn wait_for_protocol_version(
        &self,
        target_protocol_version: ProtocolVersion,
    ) -> IotaSystemState {
        self.wait_for_protocol_version_with_timeout(
            target_protocol_version,
            Duration::from_secs(60),
        )
        .await
    }

    pub async fn wait_for_protocol_version_with_timeout(
        &self,
        target_protocol_version: ProtocolVersion,
        timeout_dur: Duration,
    ) -> IotaSystemState {
        timeout(timeout_dur, async move {
            loop {
                let system_state = self.wait_for_epoch(None).await;
                if system_state.protocol_version() >= target_protocol_version.as_u64() {
                    return system_state;
                }
            }
        })
        .await
        .expect("Timed out waiting for cluster to target protocol version")
    }

    /// Ask 2f+1 validators to close epoch actively, and wait for the entire
    /// network to reach the next epoch. This requires waiting for both the
    /// fullnode and all validators to reach the next epoch.
    pub async fn force_new_epoch(&self) {
        info!("Starting reconfiguration");
        let start = Instant::now();

        // Close epoch on 2f+1 validators.
        let cur_committee = self
            .fullnode_handle
            .iota_node
            .with(|node| node.state().clone_committee_for_testing());
        let mut cur_stake = 0;
        for node in self.swarm.active_validators() {
            node.get_node_handle()
                .unwrap()
                .with_async(|node| async {
                    node.close_epoch_for_testing().await.unwrap();
                    cur_stake += cur_committee.weight(&node.state().name);
                })
                .await;
            if cur_stake >= cur_committee.quorum_threshold() {
                break;
            }
        }
        info!("close_epoch complete after {:?}", start.elapsed());

        self.wait_for_epoch(Some(cur_committee.epoch + 1)).await;
        self.wait_for_epoch_all_nodes(cur_committee.epoch + 1).await;

        info!("reconfiguration complete after {:?}", start.elapsed());
    }

    /// To detect whether the network has reached such state, we use the
    /// fullnode as the source of truth, since a fullnode only does epoch
    /// transition when the network has done so.
    /// If target_epoch is specified, wait until the cluster reaches that epoch.
    /// If target_epoch is None, wait until the cluster reaches the next epoch.
    /// Note that this function does not guarantee that every node is at the
    /// target epoch.
    pub async fn wait_for_epoch(&self, target_epoch: Option<EpochId>) -> IotaSystemState {
        self.wait_for_epoch_with_timeout(target_epoch, Duration::from_secs(60))
            .await
    }

    pub async fn wait_for_epoch_on_node(
        &self,
        handle: &IotaNodeHandle,
        target_epoch: Option<EpochId>,
        timeout_dur: Duration,
    ) -> IotaSystemState {
        let mut epoch_rx = handle.with(|node| node.subscribe_to_epoch_change());

        let mut state = None;
        timeout(timeout_dur, async {
            let epoch = handle.with(|node| node.state().epoch_store_for_testing().epoch());
            if Some(epoch) == target_epoch {
                return handle.with(|node| node.state().get_iota_system_state_object_for_testing().unwrap());
            }
            while let Ok(system_state) = epoch_rx.recv().await {
                info!("received epoch {}", system_state.epoch());
                state = Some(system_state.clone());
                match target_epoch {
                    Some(target_epoch) if system_state.epoch() >= target_epoch => {
                        return system_state;
                    }
                    None => {
                        return system_state;
                    }
                    _ => (),
                }
            }
            unreachable!("Broken reconfig channel");
        })
            .await
            .unwrap_or_else(|_| {
                error!("Timed out waiting for cluster to reach epoch {target_epoch:?}");
                if let Some(state) = state {
                    panic!("Timed out waiting for cluster to reach epoch {target_epoch:?}. Current epoch: {}", state.epoch());
                }
                panic!("Timed out waiting for cluster to target epoch {target_epoch:?}")
            })
    }

    pub async fn wait_for_epoch_with_timeout(
        &self,
        target_epoch: Option<EpochId>,
        timeout_dur: Duration,
    ) -> IotaSystemState {
        self.wait_for_epoch_on_node(&self.fullnode_handle.iota_node, target_epoch, timeout_dur)
            .await
    }

    pub async fn wait_for_epoch_all_nodes(&self, target_epoch: EpochId) {
        let handles: Vec<_> = self
            .swarm
            .all_nodes()
            .map(|node| node.get_node_handle().unwrap())
            .collect();
        let tasks: Vec<_> = handles
            .iter()
            .map(|handle| {
                handle.with_async(|node| async {
                    let mut retries = 0;
                    loop {
                        let epoch = node.state().epoch_store_for_testing().epoch();
                        if epoch == target_epoch {
                            if let Some(agg) = node.clone_authority_aggregator() {
                                // This is a fullnode, we need to wait for its auth aggregator to reconfigure as well.
                                if agg.committee.epoch() == target_epoch {
                                    break;
                                }
                            } else {
                                // This is a validator, we don't need to check the auth aggregator.
                                break;
                            }
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        retries += 1;
                        if retries % 5 == 0 {
                            tracing::warn!(validator=?node.state().name.concise(), "Waiting for {:?} seconds to reach epoch {:?}. Currently at epoch {:?}", retries, target_epoch, epoch);
                        }
                    }
                })
            })
            .collect();

        timeout(Duration::from_secs(40), join_all(tasks))
            .await
            .expect("timed out waiting for reconfiguration to complete");
    }

    /// Upgrade the network protocol version, by restarting every validator with
    /// a new supported versions.
    /// Note that we don't restart the fullnode here, and it is assumed that the
    /// fulnode supports the entire version range.
    pub async fn update_validator_supported_versions(
        &self,
        new_supported_versions: SupportedProtocolVersions,
    ) {
        for authority in self.get_validator_pubkeys() {
            self.stop_node(&authority);
            tokio::time::sleep(Duration::from_millis(1000)).await;
            self.swarm
                .node(&authority)
                .unwrap()
                .config()
                .supported_protocol_versions = Some(new_supported_versions);
            self.start_node(&authority).await;
            info!("Restarted validator {}", authority);
        }
    }

    /// Wait for all nodes in the network to upgrade to `protocol_version`.
    pub async fn wait_for_all_nodes_upgrade_to(&self, protocol_version: u64) {
        for h in self.all_node_handles() {
            h.with_async(|node| async {
                while node
                    .state()
                    .epoch_store_for_testing()
                    .epoch_start_state()
                    .protocol_version()
                    .as_u64()
                    != protocol_version
                {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            })
            .await;
        }
    }

    pub async fn trigger_reconfiguration_if_not_yet_and_assert_bridge_committee_initialized(&self) {
        let mut bridge =
            get_bridge(self.fullnode_handle.iota_node.state().get_object_store()).unwrap();
        if !bridge.committee().members.contents.is_empty() {
            assert_eq!(
                self.swarm.active_validators().count(),
                bridge.committee().members.contents.len()
            );
            return;
        }
        // wait for next epoch
        self.force_new_epoch().await;
        bridge = get_bridge(self.fullnode_handle.iota_node.state().get_object_store()).unwrap();
        // Committee should be initiated
        assert!(bridge.committee().member_registrations.contents.is_empty());
        assert_eq!(
            self.swarm.active_validators().count(),
            bridge.committee().members.contents.len()
        );
    }

    // Wait for bridge node in the cluster to be up and running.
    pub async fn wait_for_bridge_cluster_to_be_up(&self, timeout_sec: u64) {
        let bridge_ports = self.bridge_server_ports.as_ref().unwrap();
        let mut tasks = vec![];
        for port in bridge_ports.iter() {
            let server_url = format!("http://127.0.0.1:{}", port);
            tasks.push(wait_for_server_to_be_up(server_url, timeout_sec));
        }
        join_all(tasks)
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()
            .unwrap();
    }

    pub async fn get_mut_bridge_arg(&self) -> Option<ObjectArg> {
        get_bridge_obj_initial_shared_version(
            self.fullnode_handle.iota_node.state().get_object_store(),
        )
        .unwrap()
        .map(|seq| ObjectArg::SharedObject {
            id: IOTA_BRIDGE_OBJECT_ID,
            initial_shared_version: seq,
            mutable: true,
        })
    }

    pub async fn wait_for_authenticator_state_update(&self) {
        timeout(
            Duration::from_secs(60),
            self.fullnode_handle
                .iota_node
                .with_async(|node| async move {
                    let mut txns = node.state().subscription_handler.subscribe_transactions(
                        TransactionFilter::ChangedObject(
                            ObjectID::from_hex_literal("0x7").unwrap(),
                        ),
                    );
                    let state = node.state();

                    while let Some(tx) = txns.next().await {
                        let digest = *tx.transaction_digest();
                        let tx = state
                            .get_transaction_cache_reader()
                            .get_transaction_block(&digest)
                            .unwrap()
                            .unwrap();
                        match &tx.data().intent_message().value.kind() {
                            TransactionKind::EndOfEpochTransaction(_) => (),
                            TransactionKind::AuthenticatorStateUpdateV1(_) => break,
                            _ => panic!("{:?}", tx),
                        }
                    }
                }),
        )
        .await
        .expect("Timed out waiting for authenticator state update");
    }

    /// Return the highest observed protocol version in the test cluster.
    pub fn highest_protocol_version(&self) -> ProtocolVersion {
        self.all_node_handles()
            .into_iter()
            .map(|h| {
                h.with(|node| {
                    node.state()
                        .epoch_store_for_testing()
                        .epoch_start_state()
                        .protocol_version()
                })
            })
            .max()
            .expect("at least one node must be up to get highest protocol version")
    }

    pub async fn test_transaction_builder(&self) -> TestTransactionBuilder {
        let (sender, gas) = self.wallet.get_one_gas_object().await.unwrap().unwrap();
        self.test_transaction_builder_with_gas_object(sender, gas)
            .await
    }

    pub async fn test_transaction_builder_with_sender(
        &self,
        sender: IotaAddress,
    ) -> TestTransactionBuilder {
        let gas = self
            .wallet
            .get_one_gas_object_owned_by_address(sender)
            .await
            .unwrap()
            .unwrap();
        self.test_transaction_builder_with_gas_object(sender, gas)
            .await
    }

    pub async fn test_transaction_builder_with_gas_object(
        &self,
        sender: IotaAddress,
        gas: ObjectRef,
    ) -> TestTransactionBuilder {
        let rgp = self.get_reference_gas_price().await;
        TestTransactionBuilder::new(sender, gas, rgp)
    }

    pub fn sign_transaction(&self, tx_data: &TransactionData) -> Transaction {
        self.wallet.sign_transaction(tx_data)
    }

    pub async fn sign_and_execute_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> IotaTransactionBlockResponse {
        let tx = self.wallet.sign_transaction(tx_data);
        self.execute_transaction(tx).await
    }

    /// Execute a transaction on the network and wait for it to be executed on
    /// the rpc fullnode. Also expects the effects status to be
    /// ExecutionStatus::Success. This function is recommended for
    /// transaction execution since it most resembles the production path.
    pub async fn execute_transaction(&self, tx: Transaction) -> IotaTransactionBlockResponse {
        self.wallet.execute_transaction_must_succeed(tx).await
    }

    /// Different from `execute_transaction` which returns RPC effects types,
    /// this function returns raw effects, events and extra objects returned
    /// by the validators, aggregated manually (without authority
    /// aggregator). It also does not check whether the transaction is
    /// executed successfully. In order to keep the fullnode up-to-date so
    /// that latter queries can read consistent results, it calls
    /// execute_transaction_may_fail again which goes through fullnode. This
    /// is less efficient and verbose, but can be used if more details are
    /// needed from the execution results, and if the transaction is
    /// expected to fail.
    pub async fn execute_transaction_return_raw_effects(
        &self,
        tx: Transaction,
    ) -> anyhow::Result<(TransactionEffects, TransactionEvents)> {
        let results = self
            .submit_transaction_to_validators(tx.clone(), &self.get_validator_pubkeys())
            .await?;
        self.wallet.execute_transaction_may_fail(tx).await.unwrap();
        Ok(results)
    }

    pub fn authority_aggregator(&self) -> Arc<AuthorityAggregator<NetworkAuthorityClient>> {
        self.fullnode_handle
            .iota_node
            .with(|node| node.clone_authority_aggregator().unwrap())
    }

    pub async fn create_certificate(
        &self,
        tx: Transaction,
        client_addr: Option<SocketAddr>,
    ) -> anyhow::Result<CertifiedTransaction> {
        let agg = self.authority_aggregator();
        Ok(agg
            .process_transaction(tx, client_addr)
            .await?
            .into_cert_for_testing())
    }

    /// Execute a transaction on specified list of validators, and bypassing
    /// authority aggregator. This allows us to obtain the return value
    /// directly from validators, so that we can access more information
    /// directly such as the original effects, events and extra objects
    /// returned. This also allows us to control which validator to send
    /// certificates to, which is useful in some tests.
    pub async fn submit_transaction_to_validators(
        &self,
        tx: Transaction,
        pubkeys: &[AuthorityName],
    ) -> anyhow::Result<(TransactionEffects, TransactionEvents)> {
        let agg = self.authority_aggregator();
        let certificate = agg
            .process_transaction(tx, None)
            .await?
            .into_cert_for_testing();
        let replies = loop {
            let futures: Vec<_> = agg
                .authority_clients
                .iter()
                .filter_map(|(name, client)| {
                    if pubkeys.contains(name) {
                        Some(client)
                    } else {
                        None
                    }
                })
                .map(|client| {
                    let cert = certificate.clone();
                    async move {
                        client
                            .handle_certificate_v1(
                                HandleCertificateRequestV1::new(cert).with_events(),
                                None,
                            )
                            .await
                    }
                })
                .collect();

            let replies: Vec<_> = futures::future::join_all(futures)
                .await
                .into_iter()
                .filter(|result| match result {
                    Err(e) => !e.to_string().contains("deadline has elapsed"),
                    _ => true,
                })
                .collect();

            if !replies.is_empty() {
                break replies;
            }
        };
        let replies: IotaResult<Vec<_>> = replies.into_iter().collect();
        let replies = replies?;
        let mut all_effects = HashMap::new();
        let mut all_events = HashMap::new();
        for reply in replies {
            let effects = reply.signed_effects.into_data();
            let events = reply.events.unwrap_or_default();
            all_effects.insert(effects.digest(), effects);
            all_events.insert(events.digest(), events);
        }
        assert_eq!(all_effects.len(), 1);
        assert_eq!(all_events.len(), 1);
        Ok((
            all_effects.into_values().next().unwrap(),
            all_events.into_values().next().unwrap(),
        ))
    }

    /// This call sends some funds from the seeded faucet address to the funding
    /// address for the given amount and returns the gas object ref. This
    /// is useful to construct transactions from the funding address.
    pub async fn fund_address_and_return_gas(
        &self,
        rgp: u64,
        amount: Option<u64>,
        funding_address: IotaAddress,
    ) -> ObjectRef {
        let Faucet { address, keypair } = &self
            .faucet
            .as_ref()
            .expect("Faucet not initialized: incompatible with `NetworkConfig`.");

        let keypair = &*keypair.lock().await;

        let gas_ref = *self
            .wallet
            .get_gas_objects_owned_by_address(*address, None)
            .await
            .unwrap()
            .first()
            .unwrap();

        let tx_data = TestTransactionBuilder::new(*address, gas_ref, rgp)
            .transfer_iota(amount, funding_address)
            .build();

        let signed_transaction = to_sender_signed_transaction(tx_data, keypair);

        let response = self
            .iota_client()
            .quorum_driver_api()
            .execute_transaction_block(
                signed_transaction,
                IotaTransactionBlockResponseOptions::new().with_effects(),
                Some(ExecuteTransactionRequestType::WaitForLocalExecution),
            )
            .await
            .unwrap();

        response
            .effects
            .unwrap()
            .created()
            .first()
            .unwrap()
            .reference
            .to_object_ref()
    }

    pub async fn transfer_iota_must_exceed(
        &self,
        sender: IotaAddress,
        receiver: IotaAddress,
        amount: u64,
    ) -> ObjectID {
        let tx = self
            .test_transaction_builder_with_sender(sender)
            .await
            .transfer_iota(Some(amount), receiver)
            .build();
        let effects = self
            .sign_and_execute_transaction(&tx)
            .await
            .effects
            .unwrap();
        assert_eq!(&IotaExecutionStatus::Success, effects.status());
        effects.created().first().unwrap().object_id()
    }

    /// Wait to catch up to the given checkpoint sequence
    /// number with a default timeout of 60 sec
    pub async fn wait_for_checkpoint(
        &self,
        checkpoint_sequence_number: u64,
        timeout: Option<Duration>,
    ) {
        let timeout = timeout.unwrap_or(Duration::from_secs(60));
        tokio::time::timeout(timeout, async {
            loop {
                let fullnode_checkpoint = self
                    .fullnode_handle
                    .iota_node
                    .with(|node| {
                        node.state()
                            .get_checkpoint_store()
                            .get_highest_executed_checkpoint_seq_number()
                    })
                    .unwrap();

                match fullnode_checkpoint {
                    Some(c) if c >= checkpoint_sequence_number => break,
                    _ => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            }
        })
        .await
        .expect("Timeout waiting for indexer to catchup to checkpoint");
    }

    /// Get all objects owned by an address
    pub async fn get_owned_objects(
        &self,
        address: IotaAddress,
        options: Option<IotaObjectDataOptions>,
    ) -> anyhow::Result<Vec<IotaObjectResponse>> {
        let page = self
            .rpc_client()
            .get_owned_objects(
                address,
                options.map(IotaObjectResponseQuery::new_with_options),
                None,
                None,
            )
            .await?;

        Ok(page.data)
    }

    /// Create transactions based on provided object ids
    /// by transferring them from one address to another
    pub async fn transfer_objects(
        &self,
        sender: IotaAddress,
        receiver: IotaAddress,
        object_ids: Vec<ObjectID>,
        gas: ObjectID,
        options: Option<IotaTransactionBlockResponseOptions>,
    ) -> anyhow::Result<Vec<IotaTransactionBlockResponse>> {
        let mut transaction_block_resp: Vec<IotaTransactionBlockResponse> = Vec::new();

        for id in object_ids {
            let response = self
                .transfer_object(sender, receiver, id, gas, options.clone())
                .await?;

            transaction_block_resp.push(response);
        }

        Ok(transaction_block_resp)
    }

    /// Create a transaction to transfer an object from one address to another.
    /// The object's type must allow public transfers
    pub async fn transfer_object(
        &self,
        sender: IotaAddress,
        receiver: IotaAddress,
        object_id: ObjectID,
        gas: ObjectID,
        options: Option<IotaTransactionBlockResponseOptions>,
    ) -> anyhow::Result<IotaTransactionBlockResponse> {
        let http_client = self.rpc_client();
        let transaction_bytes = http_client
            .transfer_object(sender, object_id, Some(gas), 10_000_000.into(), receiver)
            .await?;

        let tx = self
            .wallet
            .sign_transaction(&transaction_bytes.to_data().unwrap());

        let (tx_bytes, signatures) = tx.to_tx_bytes_and_signatures();

        let response = http_client
            .execute_transaction_block(
                tx_bytes,
                signatures,
                options,
                Some(ExecuteTransactionRequestType::WaitForLocalExecution),
            )
            .await?;

        Ok(response)
    }

    #[cfg(msim)]
    pub fn set_safe_mode_expected(&self, value: bool) {
        for n in self.all_node_handles() {
            n.with(|node| node.set_safe_mode_expected(value));
        }
    }
}

pub struct RandomNodeRestarter {
    test_cluster: Arc<TestCluster>,

    // How frequently should we kill nodes
    kill_interval: Uniform<Duration>,
    // How long should we wait before restarting them.
    restart_delay: Uniform<Duration>,

    task_handle: Mutex<Option<JoinHandle<()>>>,
}

impl RandomNodeRestarter {
    fn new(test_cluster: Arc<TestCluster>) -> Self {
        Self {
            test_cluster,
            kill_interval: Uniform::new(Duration::from_secs(10), Duration::from_secs(11)),
            restart_delay: Uniform::new(Duration::from_secs(1), Duration::from_secs(2)),
            task_handle: Default::default(),
        }
    }

    pub fn with_kill_interval_secs(mut self, a: u64, b: u64) -> Self {
        self.kill_interval = Uniform::new(Duration::from_secs(a), Duration::from_secs(b));
        self
    }

    pub fn with_restart_delay_secs(mut self, a: u64, b: u64) -> Self {
        self.restart_delay = Uniform::new(Duration::from_secs(a), Duration::from_secs(b));
        self
    }

    pub fn run(&self) {
        let test_cluster = self.test_cluster.clone();
        let kill_interval = self.kill_interval;
        let restart_delay = self.restart_delay;
        let validators = self.test_cluster.get_validator_pubkeys();
        let mut task_handle = self.task_handle.lock().unwrap();
        assert!(task_handle.is_none());
        task_handle.replace(tokio::task::spawn(async move {
            loop {
                let delay = kill_interval.sample(&mut OsRng);
                info!("Sleeping {delay:?} before killing a validator");
                sleep(delay).await;

                let validator = validators.choose(&mut OsRng).unwrap();
                info!("Killing validator {:?}", validator.concise());
                test_cluster.stop_node(validator);

                let delay = restart_delay.sample(&mut OsRng);
                info!("Sleeping {delay:?} before restarting");
                sleep(delay).await;
                info!("Starting validator {:?}", validator.concise());
                test_cluster.start_node(validator).await;
            }
        }));
    }
}

impl Drop for RandomNodeRestarter {
    fn drop(&mut self) {
        if let Some(handle) = self.task_handle.lock().unwrap().take() {
            handle.abort();
        }
    }
}

pub struct TestClusterBuilder {
    genesis_config: Option<GenesisConfig>,
    network_config: Option<NetworkConfig>,
    additional_objects: Vec<Object>,
    num_validators: Option<usize>,
    fullnode_rpc_port: Option<u16>,
    fullnode_rpc_addr: Option<SocketAddr>,
    enable_fullnode_events: bool,
    validator_supported_protocol_versions_config: ProtocolVersionsConfig,
    // Default to validator_supported_protocol_versions_config, but can be overridden.
    fullnode_supported_protocol_versions_config: Option<ProtocolVersionsConfig>,
    db_checkpoint_config_validators: DBCheckpointConfig,
    db_checkpoint_config_fullnodes: DBCheckpointConfig,
    num_unpruned_validators: Option<usize>,
    jwk_fetch_interval: Option<Duration>,
    config_dir: Option<PathBuf>,
    default_jwks: bool,
    authority_overload_config: Option<AuthorityOverloadConfig>,
    data_ingestion_dir: Option<PathBuf>,
    fullnode_run_with_range: Option<RunWithRange>,
    fullnode_policy_config: Option<PolicyConfig>,
    fullnode_fw_config: Option<RemoteFirewallConfig>,

    max_submit_position: Option<usize>,
    submit_delay_step_override_millis: Option<u64>,
    validator_state_accumulator_config: StateAccumulatorV1EnabledConfig,
}

impl TestClusterBuilder {
    pub fn new() -> Self {
        TestClusterBuilder {
            genesis_config: None,
            network_config: None,
            additional_objects: vec![],
            fullnode_rpc_port: None,
            fullnode_rpc_addr: None,
            num_validators: None,
            enable_fullnode_events: false,
            validator_supported_protocol_versions_config: ProtocolVersionsConfig::Default,
            fullnode_supported_protocol_versions_config: None,
            db_checkpoint_config_validators: DBCheckpointConfig::default(),
            db_checkpoint_config_fullnodes: DBCheckpointConfig::default(),
            num_unpruned_validators: None,
            jwk_fetch_interval: None,
            config_dir: None,
            default_jwks: false,
            authority_overload_config: None,
            data_ingestion_dir: None,
            fullnode_run_with_range: None,
            fullnode_policy_config: None,
            fullnode_fw_config: None,
            max_submit_position: None,
            submit_delay_step_override_millis: None,
            validator_state_accumulator_config: StateAccumulatorV1EnabledConfig::Global(true),
        }
    }

    pub fn with_fullnode_run_with_range(mut self, run_with_range: Option<RunWithRange>) -> Self {
        if let Some(run_with_range) = run_with_range {
            self.fullnode_run_with_range = Some(run_with_range);
        }
        self
    }

    pub fn with_fullnode_policy_config(mut self, config: Option<PolicyConfig>) -> Self {
        self.fullnode_policy_config = config;
        self
    }

    pub fn with_fullnode_fw_config(mut self, config: Option<RemoteFirewallConfig>) -> Self {
        self.fullnode_fw_config = config;
        self
    }

    pub fn with_fullnode_rpc_port(mut self, rpc_port: u16) -> Self {
        self.fullnode_rpc_port = Some(rpc_port);
        self
    }

    pub fn with_fullnode_rpc_addr(mut self, addr: SocketAddr) -> Self {
        self.fullnode_rpc_addr = Some(addr);
        self
    }

    pub fn set_genesis_config(mut self, genesis_config: GenesisConfig) -> Self {
        assert!(self.genesis_config.is_none() && self.network_config.is_none());
        self.genesis_config = Some(genesis_config);
        self
    }

    pub fn set_network_config(mut self, network_config: NetworkConfig) -> Self {
        assert!(self.genesis_config.is_none() && self.network_config.is_none());
        self.network_config = Some(network_config);
        self
    }

    pub fn with_objects<I: IntoIterator<Item = Object>>(mut self, objects: I) -> Self {
        self.additional_objects.extend(objects);
        self
    }

    pub fn with_num_validators(mut self, num: usize) -> Self {
        self.num_validators = Some(num);
        self
    }

    pub fn enable_fullnode_events(mut self) -> Self {
        self.enable_fullnode_events = true;
        self
    }

    pub fn with_enable_db_checkpoints_validators(mut self) -> Self {
        self.db_checkpoint_config_validators = DBCheckpointConfig {
            perform_db_checkpoints_at_epoch_end: true,
            checkpoint_path: None,
            object_store_config: None,
            perform_index_db_checkpoints_at_epoch_end: None,
            prune_and_compact_before_upload: None,
        };
        self
    }

    pub fn with_enable_db_checkpoints_fullnodes(mut self) -> Self {
        self.db_checkpoint_config_fullnodes = DBCheckpointConfig {
            perform_db_checkpoints_at_epoch_end: true,
            checkpoint_path: None,
            object_store_config: None,
            perform_index_db_checkpoints_at_epoch_end: None,
            prune_and_compact_before_upload: Some(true),
        };
        self
    }

    pub fn with_epoch_duration_ms(mut self, epoch_duration_ms: u64) -> Self {
        self.get_or_init_genesis_config()
            .parameters
            .epoch_duration_ms = epoch_duration_ms;
        self
    }

    pub fn with_supported_protocol_versions(mut self, c: SupportedProtocolVersions) -> Self {
        self.validator_supported_protocol_versions_config = ProtocolVersionsConfig::Global(c);
        self
    }

    pub fn with_jwk_fetch_interval(mut self, i: Duration) -> Self {
        self.jwk_fetch_interval = Some(i);
        self
    }

    pub fn with_fullnode_supported_protocol_versions_config(
        mut self,
        c: SupportedProtocolVersions,
    ) -> Self {
        self.fullnode_supported_protocol_versions_config = Some(ProtocolVersionsConfig::Global(c));
        self
    }

    pub fn with_protocol_version(mut self, v: ProtocolVersion) -> Self {
        self.get_or_init_genesis_config()
            .parameters
            .protocol_version = v;
        self
    }

    pub fn with_supported_protocol_version_callback(
        mut self,
        func: SupportedProtocolVersionsCallback,
    ) -> Self {
        self.validator_supported_protocol_versions_config =
            ProtocolVersionsConfig::PerValidator(func);
        self
    }

    pub fn with_state_accumulator_callback(
        mut self,
        func: StateAccumulatorEnabledCallback,
    ) -> Self {
        self.validator_state_accumulator_config =
            StateAccumulatorV1EnabledConfig::PerValidator(func);
        self
    }

    pub fn with_validator_candidates(
        mut self,
        addresses: impl IntoIterator<Item = IotaAddress>,
    ) -> Self {
        self.get_or_init_genesis_config()
            .accounts
            .extend(addresses.into_iter().map(|address| AccountConfig {
                address: Some(address),
                gas_amounts: vec![DEFAULT_GAS_AMOUNT, MIN_VALIDATOR_JOINING_STAKE_NANOS],
            }));
        self
    }

    pub fn with_num_unpruned_validators(mut self, n: usize) -> Self {
        self.num_unpruned_validators = Some(n);
        self
    }

    pub fn with_accounts(mut self, accounts: Vec<AccountConfig>) -> Self {
        self.get_or_init_genesis_config().accounts = accounts;
        self
    }

    pub fn with_migration_data(mut self, migration_sources: Vec<SnapshotSource>) -> Self {
        self.get_or_init_genesis_config().migration_sources = migration_sources;
        self
    }

    pub fn with_additional_accounts(mut self, accounts: Vec<AccountConfig>) -> Self {
        self.get_or_init_genesis_config().accounts.extend(accounts);
        self
    }

    pub fn with_delegator(mut self, delegator: IotaAddress) -> Self {
        self.get_or_init_genesis_config().delegator = Some(delegator);
        self
    }

    pub fn with_config_dir(mut self, config_dir: PathBuf) -> Self {
        self.config_dir = Some(config_dir);
        self
    }

    pub fn with_default_jwks(mut self) -> Self {
        self.default_jwks = true;
        self
    }

    pub fn with_authority_overload_config(mut self, config: AuthorityOverloadConfig) -> Self {
        assert!(self.network_config.is_none());
        self.authority_overload_config = Some(config);
        self
    }

    pub fn with_data_ingestion_dir(mut self, path: PathBuf) -> Self {
        self.data_ingestion_dir = Some(path);
        self
    }

    pub fn with_max_submit_position(mut self, max_submit_position: usize) -> Self {
        self.max_submit_position = Some(max_submit_position);
        self
    }

    pub fn with_submit_delay_step_override_millis(
        mut self,
        submit_delay_step_override_millis: u64,
    ) -> Self {
        self.submit_delay_step_override_millis = Some(submit_delay_step_override_millis);
        self
    }

    pub async fn build(mut self) -> TestCluster {
        // We can add a faucet account to the `GenesisConfig` if there was no
        // `NetworkConfig` provided. Only either a `GenesisConfig` or a
        // `NetworkConfig` can be used to configure and build the cluster.
        let faucet = self.network_config.is_none().then(|| {
            let (faucet_address, faucet_keypair): (IotaAddress, AccountKeyPair) = get_key_pair();
            let accounts = &mut self.get_or_init_genesis_config().accounts;
            accounts.push(AccountConfig {
                address: Some(faucet_address),
                gas_amounts: vec![DEFAULT_GAS_AMOUNT],
            });
            Faucet {
                address: faucet_address,
                keypair: Arc::new(tokio::sync::Mutex::new(IotaKeyPair::Ed25519(
                    faucet_keypair,
                ))),
            }
        });

        // All test clusters receive a continuous stream of random JWKs.
        // If we later use zklogin authenticated transactions in tests we will need to
        // supply valid JWKs as well.
        #[cfg(msim)]
        if !self.default_jwks {
            iota_node::set_jwk_injector(Arc::new(|_authority, provider| {
                use fastcrypto_zkp::bn254::zk_login::{JWK, JwkId};
                use rand::Rng;

                // generate random (and possibly conflicting) id/key pairings.
                let id_num = rand::thread_rng().gen_range(1..=4);
                let key_num = rand::thread_rng().gen_range(1..=4);

                let id = JwkId {
                    iss: provider.get_config().iss,
                    kid: format!("kid{}", id_num),
                };

                let jwk = JWK {
                    kty: "kty".to_string(),
                    e: "e".to_string(),
                    n: format!("n{}", key_num),
                    alg: "alg".to_string(),
                };

                Ok(vec![(id, jwk)])
            }));
        }

        let swarm = self.start_swarm().await.unwrap();
        let working_dir = swarm.dir();

        let mut wallet_conf: IotaClientConfig =
            PersistedConfig::read(&working_dir.join(IOTA_CLIENT_CONFIG)).unwrap();

        let fullnode = swarm.fullnodes().next().unwrap();
        let json_rpc_address = fullnode.config().json_rpc_address;
        let fullnode_handle =
            FullNodeHandle::new(fullnode.get_node_handle().unwrap(), json_rpc_address).await;

        wallet_conf.add_env(IotaEnv::new("localnet", fullnode_handle.rpc_url.clone()));
        wallet_conf.set_active_env(Some("localnet".to_string()));

        wallet_conf
            .persisted(&working_dir.join(IOTA_CLIENT_CONFIG))
            .save()
            .unwrap();

        let wallet_conf = swarm.dir().join(IOTA_CLIENT_CONFIG);
        let wallet = WalletContext::new(&wallet_conf, None, None).unwrap();

        TestCluster {
            swarm,
            wallet,
            fullnode_handle,
            bridge_authority_keys: None,
            bridge_server_ports: None,
            faucet,
        }
    }

    pub async fn build_with_bridge(
        self,
        bridge_authority_keys: Vec<BridgeAuthorityKeyPair>,
        deploy_tokens: bool,
    ) -> TestCluster {
        let timer = Instant::now();
        let gas_objects_for_authority_keys = bridge_authority_keys
            .iter()
            .map(|k| {
                let address = IotaAddress::from(k.public());
                Object::with_id_owner_for_testing(ObjectID::random(), address)
            })
            .collect::<Vec<_>>();
        let mut test_cluster = self
            .with_objects(gas_objects_for_authority_keys)
            .build()
            .await;
        info!(
            "TestCluster build took {:?} secs",
            timer.elapsed().as_secs()
        );
        let ref_gas_price = test_cluster.get_reference_gas_price().await;
        let bridge_arg = test_cluster.get_mut_bridge_arg().await.unwrap();
        assert_eq!(
            bridge_authority_keys.len(),
            test_cluster.swarm.active_validators().count()
        );

        // Committee registers themselves
        let mut server_ports = vec![];
        let mut tasks = vec![];
        let quorum_driver_api = test_cluster.quorum_driver_api().clone();
        for (node, kp) in test_cluster
            .swarm
            .active_validators()
            .zip(bridge_authority_keys.iter())
        {
            let validator_address = node.config().iota_address();
            // create committee registration tx
            let gas = test_cluster
                .wallet
                .get_one_gas_object_owned_by_address(validator_address)
                .await
                .unwrap()
                .unwrap();

            let server_port = get_available_port("127.0.0.1");
            let server_url = format!("http://127.0.0.1:{}", server_port);
            server_ports.push(server_port);
            let data = build_committee_register_transaction(
                validator_address,
                &gas,
                bridge_arg,
                kp.public().as_bytes().to_vec(),
                &server_url,
                ref_gas_price,
                1000000000,
            )
            .unwrap();

            let tx = Transaction::from_data_and_signer(
                data,
                vec![node.config().account_key_pair.keypair()],
            );
            let api_clone = quorum_driver_api.clone();
            tasks.push(async move {
                api_clone
                    .execute_transaction_block(
                        tx,
                        IotaTransactionBlockResponseOptions::new().with_effects(),
                        None,
                    )
                    .await
            });
        }

        if deploy_tokens {
            let timer = Instant::now();
            let token_ids = vec![TOKEN_ID_BTC, TOKEN_ID_ETH, TOKEN_ID_USDC, TOKEN_ID_USDT];
            let token_prices = vec![500_000_000u64, 30_000_000u64, 1_000u64, 1_000u64];
            let action = publish_and_register_coins_return_add_coins_on_iota_action(
                test_cluster.wallet(),
                bridge_arg,
                vec![
                    Path::new("../../bridge/move/tokens/btc").into(),
                    Path::new("../../bridge/move/tokens/eth").into(),
                    Path::new("../../bridge/move/tokens/usdc").into(),
                    Path::new("../../bridge/move/tokens/usdt").into(),
                ],
                token_ids,
                token_prices,
                0,
            );
            let action = action.await;
            info!("register tokens took {:?} secs", timer.elapsed().as_secs());
            let sig_map = bridge_authority_keys
                .iter()
                .map(|key| {
                    (
                        key.public().into(),
                        BridgeAuthoritySignInfo::new(&action, key).signature,
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let certified_action = CertifiedBridgeAction::new_from_data_and_sig(
                action,
                BridgeCommitteeValiditySignInfo {
                    signatures: sig_map.clone(),
                },
            );
            let verifired_action_cert =
                VerifiedCertifiedBridgeAction::new_from_verified(certified_action);
            let sender_address = test_cluster.get_address_0();

            await_committee_register_tasks(&test_cluster, tasks).await;

            // Wait until committee is set up
            test_cluster
                .trigger_reconfiguration_if_not_yet_and_assert_bridge_committee_initialized()
                .await;

            let tx = build_add_tokens_on_iota_transaction(
                sender_address,
                &test_cluster
                    .wallet
                    .get_one_gas_object_owned_by_address(sender_address)
                    .await
                    .unwrap()
                    .unwrap(),
                verifired_action_cert,
                bridge_arg,
                ref_gas_price,
            )
            .unwrap();

            let response = test_cluster.sign_and_execute_transaction(&tx).await;
            assert_eq!(
                response.effects.unwrap().status(),
                &IotaExecutionStatus::Success
            );
            info!("Deploy tokens took {:?} secs", timer.elapsed().as_secs());
        } else {
            await_committee_register_tasks(&test_cluster, tasks).await;
        }

        async fn await_committee_register_tasks(
            test_cluster: &TestCluster,
            tasks: Vec<
                impl Future<Output = Result<IotaTransactionBlockResponse, iota_sdk::error::Error>>,
            >,
        ) {
            // The tx may fail if a member tries to register when the committee is already
            // finalized. In that case, we just need to check the committee
            // members is not empty since once the committee is finalized, it
            // should not be empty.
            let responses = join_all(tasks).await;
            let mut has_failure = false;
            for response in responses {
                if response.unwrap().effects.unwrap().status() != &IotaExecutionStatus::Success {
                    has_failure = true;
                }
            }
            if has_failure {
                let bridge_summary = test_cluster.get_bridge_summary().await.unwrap();
                assert_ne!(bridge_summary.committee.members.len(), 0);
            }
        }

        info!(
            "TestCluster build_with_bridge took {:?} secs",
            timer.elapsed().as_secs()
        );
        test_cluster.bridge_authority_keys = Some(bridge_authority_keys);
        test_cluster.bridge_server_ports = Some(server_ports);
        test_cluster
    }

    /// Start a Swarm and set up WalletConfig
    async fn start_swarm(&mut self) -> Result<Swarm, anyhow::Error> {
        let mut builder: SwarmBuilder = Swarm::builder()
            .committee_size(
                NonZeroUsize::new(self.num_validators.unwrap_or(NUM_VALIDATOR)).unwrap(),
            )
            .with_objects(self.additional_objects.clone())
            .with_db_checkpoint_config(self.db_checkpoint_config_validators.clone())
            .with_supported_protocol_versions_config(
                self.validator_supported_protocol_versions_config.clone(),
            )
            .with_state_accumulator_config(self.validator_state_accumulator_config.clone())
            .with_fullnode_count(1)
            .with_fullnode_supported_protocol_versions_config(
                self.fullnode_supported_protocol_versions_config
                    .clone()
                    .unwrap_or(self.validator_supported_protocol_versions_config.clone()),
            )
            .with_db_checkpoint_config(self.db_checkpoint_config_fullnodes.clone())
            .with_fullnode_run_with_range(self.fullnode_run_with_range)
            .with_fullnode_policy_config(self.fullnode_policy_config.clone())
            .with_fullnode_fw_config(self.fullnode_fw_config.clone());

        if let Some(genesis_config) = self.genesis_config.take() {
            builder = builder.with_genesis_config(genesis_config);
        }

        if let Some(network_config) = self.network_config.take() {
            builder = builder.with_network_config(network_config);
        }

        if let Some(authority_overload_config) = self.authority_overload_config.take() {
            builder = builder.with_authority_overload_config(authority_overload_config);
        }

        if let Some(fullnode_rpc_addr) = self.fullnode_rpc_addr {
            builder = builder.with_fullnode_rpc_addr(fullnode_rpc_addr);
        } else if let Some(fullnode_rpc_port) = self.fullnode_rpc_port {
            builder = builder.with_fullnode_rpc_port(fullnode_rpc_port);
        }

        if let Some(num_unpruned_validators) = self.num_unpruned_validators {
            builder = builder.with_num_unpruned_validators(num_unpruned_validators);
        }

        if let Some(jwk_fetch_interval) = self.jwk_fetch_interval {
            builder = builder.with_jwk_fetch_interval(jwk_fetch_interval);
        }

        if let Some(config_dir) = self.config_dir.take() {
            builder = builder.dir(config_dir);
        }

        if let Some(data_ingestion_dir) = self.data_ingestion_dir.take() {
            builder = builder.with_data_ingestion_dir(data_ingestion_dir);
        }

        if let Some(max_submit_position) = self.max_submit_position {
            builder = builder.with_max_submit_position(max_submit_position);
        }

        if let Some(submit_delay_step_override_millis) = self.submit_delay_step_override_millis {
            builder =
                builder.with_submit_delay_step_override_millis(submit_delay_step_override_millis);
        }

        let mut swarm = builder.build();
        swarm.launch().await?;

        let dir = swarm.dir();

        let network_path = dir.join(IOTA_NETWORK_CONFIG);
        let wallet_path = dir.join(IOTA_CLIENT_CONFIG);
        let keystore_path = dir.join(IOTA_KEYSTORE_FILENAME);

        let network_config = swarm.config();
        // Create light config to save
        let account_keys = network_config
            .account_keys
            .iter()
            .map(|kp| kp.copy())
            .collect();
        let network_config_light = NetworkConfigLight::new(
            network_config.validator_configs.clone(),
            account_keys,
            &network_config.genesis,
        );
        network_config_light.save(network_path)?;

        let mut keystore = Keystore::from(FileBasedKeystore::new(&keystore_path)?);
        for key in &swarm.config().account_keys {
            keystore.add_key(None, IotaKeyPair::Ed25519(key.copy()))?;
        }

        let active_address = keystore.addresses().first().cloned();

        // Create wallet config with stated authorities port
        IotaClientConfig::new(FileBasedKeystore::new(&keystore_path)?)
            .with_active_address(active_address)
            .save(wallet_path)?;

        // Return network handle
        Ok(swarm)
    }

    fn get_or_init_genesis_config(&mut self) -> &mut GenesisConfig {
        if self.genesis_config.is_none() {
            self.genesis_config = Some(GenesisConfig::for_local_testing());
        }
        self.genesis_config.as_mut().unwrap()
    }
}

impl Default for TestClusterBuilder {
    fn default() -> Self {
        Self::new()
    }
}
