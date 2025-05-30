// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use arc_swap::ArcSwap;
use ethers::types::Address as EthAddress;
use iota_metrics::spawn_logged_monitored_task;
use iota_types::{
    Identifier,
    bridge::{
        BRIDGE_COMMITTEE_MODULE_NAME, BRIDGE_LIMITER_MODULE_NAME, BRIDGE_MODULE_NAME,
        BRIDGE_TREASURY_MODULE_NAME,
    },
    event::EventID,
};
use tokio::task::JoinHandle;
use tracing::info;

use crate::{
    action_executor::BridgeActionExecutor,
    client::bridge_authority_aggregator::BridgeAuthorityAggregator,
    config::{BridgeClientConfig, BridgeNodeConfig},
    eth_syncer::EthSyncer,
    events::init_all_struct_tags,
    iota_syncer::IotaSyncer,
    metrics::BridgeMetrics,
    monitor::BridgeMonitor,
    orchestrator::BridgeOrchestrator,
    server::{BridgeNodePublicMetadata, handler::BridgeRequestHandler, run_server},
    storage::BridgeOrchestratorTables,
};

pub async fn run_bridge_node(
    config: BridgeNodeConfig,
    metadata: BridgeNodePublicMetadata,
    prometheus_registry: prometheus::Registry,
) -> anyhow::Result<JoinHandle<()>> {
    init_all_struct_tags();
    let metrics = Arc::new(BridgeMetrics::new(&prometheus_registry));
    let (server_config, client_config) = config.validate(metrics.clone()).await?;

    // Start Client
    let _handles = if let Some(client_config) = client_config {
        start_client_components(client_config, metrics.clone()).await
    } else {
        Ok(vec![])
    }?;

    // Start Server
    let socket_address = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        server_config.server_listen_port,
    );
    Ok(run_server(
        &socket_address,
        BridgeRequestHandler::new(
            server_config.key,
            server_config.iota_client,
            server_config.eth_client,
            server_config.approved_governance_actions,
            metrics.clone(),
        ),
        metrics,
        Arc::new(metadata),
    ))
}

// TODO: is there a way to clean up the overrides after it's stored in DB?
async fn start_client_components(
    client_config: BridgeClientConfig,
    metrics: Arc<BridgeMetrics>,
) -> anyhow::Result<Vec<JoinHandle<()>>> {
    let store: std::sync::Arc<BridgeOrchestratorTables> =
        BridgeOrchestratorTables::new(&client_config.db_path.join("client"));
    let iota_modules_to_watch = get_iota_modules_to_watch(
        &store,
        client_config.iota_bridge_module_last_processed_event_id_override,
    );
    let eth_contracts_to_watch = get_eth_contracts_to_watch(
        &store,
        &client_config.eth_contracts,
        client_config.eth_contracts_start_block_fallback,
        client_config.eth_contracts_start_block_override,
    );

    let iota_client = client_config.iota_client.clone();

    let mut all_handles = vec![];
    let (task_handles, eth_events_rx, _) =
        EthSyncer::new(client_config.eth_client.clone(), eth_contracts_to_watch)
            .run(metrics.clone())
            .await
            .expect("Failed to start eth syncer");
    all_handles.extend(task_handles);

    let (task_handles, iota_events_rx) =
        IotaSyncer::new(client_config.iota_client, iota_modules_to_watch)
            .run(Duration::from_secs(2))
            .await
            .expect("Failed to start iota syncer");
    all_handles.extend(task_handles);

    let committee = Arc::new(
        iota_client
            .get_bridge_committee()
            .await
            .expect("Failed to get committee"),
    );
    let bridge_auth_agg = Arc::new(ArcSwap::from(Arc::new(BridgeAuthorityAggregator::new(
        committee,
    ))));
    // TODO: should we use one query instead of two?
    let iota_token_type_tags = iota_client.get_token_id_map().await.unwrap();
    let is_bridge_paused = iota_client.is_bridge_paused().await.unwrap();

    let (bridge_pause_tx, bridge_pause_rx) = tokio::sync::watch::channel(is_bridge_paused);

    let (monitor_tx, monitor_rx) = iota_metrics::metered_channel::channel(
        10000,
        &iota_metrics::get_metrics()
            .unwrap()
            .channel_inflight
            .with_label_values(&["monitor_queue"]),
    );
    let iota_token_type_tags = Arc::new(ArcSwap::from(Arc::new(iota_token_type_tags)));
    let bridge_action_executor = BridgeActionExecutor::new(
        iota_client.clone(),
        bridge_auth_agg.clone(),
        store.clone(),
        client_config.key,
        client_config.iota_address,
        client_config.gas_object_ref.0,
        iota_token_type_tags.clone(),
        bridge_pause_rx,
        metrics.clone(),
    )
    .await;

    let monitor = BridgeMonitor::new(
        iota_client.clone(),
        monitor_rx,
        bridge_auth_agg.clone(),
        bridge_pause_tx,
        iota_token_type_tags,
    );
    all_handles.push(spawn_logged_monitored_task!(monitor.run()));

    let orchestrator = BridgeOrchestrator::new(
        iota_client,
        iota_events_rx,
        eth_events_rx,
        store.clone(),
        monitor_tx,
        metrics,
    );

    all_handles.extend(orchestrator.run(bridge_action_executor).await);
    Ok(all_handles)
}

fn get_iota_modules_to_watch(
    store: &std::sync::Arc<BridgeOrchestratorTables>,
    iota_bridge_module_last_processed_event_id_override: Option<EventID>,
) -> HashMap<Identifier, Option<EventID>> {
    let iota_bridge_modules = vec![
        BRIDGE_MODULE_NAME.to_owned(),
        BRIDGE_COMMITTEE_MODULE_NAME.to_owned(),
        BRIDGE_TREASURY_MODULE_NAME.to_owned(),
        BRIDGE_LIMITER_MODULE_NAME.to_owned(),
    ];
    if let Some(cursor) = iota_bridge_module_last_processed_event_id_override {
        info!("Overriding cursor for iota bridge modules to {:?}", cursor);
        return HashMap::from_iter(
            iota_bridge_modules
                .iter()
                .map(|module| (module.clone(), Some(cursor))),
        );
    }

    let iota_bridge_module_stored_cursor = store
        .get_iota_event_cursors(&iota_bridge_modules)
        .expect("Failed to get eth iota event cursors from storage");
    let mut iota_modules_to_watch = HashMap::new();
    for (module_identifier, cursor) in iota_bridge_modules
        .iter()
        .zip(iota_bridge_module_stored_cursor)
    {
        if cursor.is_none() {
            info!(
                "No cursor found for iota bridge module {} in storage or config override, query start from the beginning.",
                module_identifier
            );
        }
        iota_modules_to_watch.insert(module_identifier.clone(), cursor);
    }
    iota_modules_to_watch
}

fn get_eth_contracts_to_watch(
    store: &std::sync::Arc<BridgeOrchestratorTables>,
    eth_contracts: &[EthAddress],
    eth_contracts_start_block_fallback: u64,
    eth_contracts_start_block_override: Option<u64>,
) -> HashMap<EthAddress, u64> {
    let stored_eth_cursors = store
        .get_eth_event_cursors(eth_contracts)
        .expect("Failed to get eth event cursors from storage");
    let mut eth_contracts_to_watch = HashMap::new();
    for (contract, stored_cursor) in eth_contracts.iter().zip(stored_eth_cursors) {
        // start block precedence:
        // eth_contracts_start_block_override > stored cursor >
        // eth_contracts_start_block_fallback
        match (eth_contracts_start_block_override, stored_cursor) {
            (Some(override_), _) => {
                eth_contracts_to_watch.insert(*contract, override_);
                info!(
                    "Overriding cursor for eth bridge contract {} to {}. Stored cursor: {:?}",
                    contract, override_, stored_cursor
                );
            }
            (None, Some(stored_cursor)) => {
                // +1: The stored value is the last block that was processed, so we start from
                // the next block.
                eth_contracts_to_watch.insert(*contract, stored_cursor + 1);
            }
            (None, None) => {
                // If no cursor is found, start from the fallback block.
                eth_contracts_to_watch.insert(*contract, eth_contracts_start_block_fallback);
            }
        }
    }
    eth_contracts_to_watch
}

#[cfg(test)]
mod tests {
    use ethers::types::Address as EthAddress;
    use fastcrypto::secp256k1::Secp256k1KeyPair;
    use iota_config::local_ip_utils::get_available_port;
    use iota_types::{
        base_types::IotaAddress,
        bridge::BridgeChainId,
        crypto::{EncodeDecodeBase64, IotaKeyPair, KeypairTraits, get_key_pair},
        digests::TransactionDigest,
        event::EventID,
    };
    use prometheus::Registry;
    use tempfile::tempdir;

    use super::*;
    use crate::{
        config::{BridgeNodeConfig, EthConfig, IotaConfig},
        e2e_tests::test_utils::{BridgeTestCluster, BridgeTestClusterBuilder},
        utils::wait_for_server_to_be_up,
    };

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_get_eth_contracts_to_watch() {
        telemetry_subscribers::init_for_testing();
        let temp_dir = tempfile::tempdir().unwrap();
        let eth_contracts = vec![
            EthAddress::from_low_u64_be(1),
            EthAddress::from_low_u64_be(2),
        ];
        let store = BridgeOrchestratorTables::new(temp_dir.path());

        // No override, no watermark found in DB, use fallback
        let contracts = get_eth_contracts_to_watch(&store, &eth_contracts, 10, None);
        assert_eq!(
            contracts,
            vec![(eth_contracts[0], 10), (eth_contracts[1], 10)]
                .into_iter()
                .collect::<HashMap<_, _>>()
        );

        // no watermark found in DB, use override
        let contracts = get_eth_contracts_to_watch(&store, &eth_contracts, 10, Some(420));
        assert_eq!(
            contracts,
            vec![(eth_contracts[0], 420), (eth_contracts[1], 420)]
                .into_iter()
                .collect::<HashMap<_, _>>()
        );

        store
            .update_eth_event_cursor(eth_contracts[0], 100)
            .unwrap();
        store
            .update_eth_event_cursor(eth_contracts[1], 102)
            .unwrap();

        // No override, found watermarks in DB, use +1
        let contracts = get_eth_contracts_to_watch(&store, &eth_contracts, 10, None);
        assert_eq!(
            contracts,
            vec![(eth_contracts[0], 101), (eth_contracts[1], 103)]
                .into_iter()
                .collect::<HashMap<_, _>>()
        );

        // use override
        let contracts = get_eth_contracts_to_watch(&store, &eth_contracts, 10, Some(200));
        assert_eq!(
            contracts,
            vec![(eth_contracts[0], 200), (eth_contracts[1], 200)]
                .into_iter()
                .collect::<HashMap<_, _>>()
        );
    }

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_get_iota_modules_to_watch() {
        telemetry_subscribers::init_for_testing();
        let temp_dir = tempfile::tempdir().unwrap();

        let store = BridgeOrchestratorTables::new(temp_dir.path());
        let bridge_module = BRIDGE_MODULE_NAME.to_owned();
        let committee_module = BRIDGE_COMMITTEE_MODULE_NAME.to_owned();
        let treasury_module = BRIDGE_TREASURY_MODULE_NAME.to_owned();
        let limiter_module = BRIDGE_LIMITER_MODULE_NAME.to_owned();
        // No override, no stored watermark, use None
        let iota_modules_to_watch = get_iota_modules_to_watch(&store, None);
        assert_eq!(
            iota_modules_to_watch,
            vec![
                (bridge_module.clone(), None),
                (committee_module.clone(), None),
                (treasury_module.clone(), None),
                (limiter_module.clone(), None)
            ]
            .into_iter()
            .collect::<HashMap<_, _>>()
        );

        // no stored watermark, use override
        let override_cursor = EventID {
            tx_digest: TransactionDigest::random(),
            event_seq: 42,
        };
        let iota_modules_to_watch = get_iota_modules_to_watch(&store, Some(override_cursor));
        assert_eq!(
            iota_modules_to_watch,
            vec![
                (bridge_module.clone(), Some(override_cursor)),
                (committee_module.clone(), Some(override_cursor)),
                (treasury_module.clone(), Some(override_cursor)),
                (limiter_module.clone(), Some(override_cursor))
            ]
            .into_iter()
            .collect::<HashMap<_, _>>()
        );

        // No override, found stored watermark for `bridge` module, use stored watermark
        // for `bridge` and None for `committee`
        let stored_cursor = EventID {
            tx_digest: TransactionDigest::random(),
            event_seq: 100,
        };
        store
            .update_iota_event_cursor(bridge_module.clone(), stored_cursor)
            .unwrap();
        let iota_modules_to_watch = get_iota_modules_to_watch(&store, None);
        assert_eq!(
            iota_modules_to_watch,
            vec![
                (bridge_module.clone(), Some(stored_cursor)),
                (committee_module.clone(), None),
                (treasury_module.clone(), None),
                (limiter_module.clone(), None)
            ]
            .into_iter()
            .collect::<HashMap<_, _>>()
        );

        // found stored watermark, use override
        let stored_cursor = EventID {
            tx_digest: TransactionDigest::random(),
            event_seq: 100,
        };
        store
            .update_iota_event_cursor(committee_module.clone(), stored_cursor)
            .unwrap();
        let iota_modules_to_watch = get_iota_modules_to_watch(&store, Some(override_cursor));
        assert_eq!(
            iota_modules_to_watch,
            vec![
                (bridge_module.clone(), Some(override_cursor)),
                (committee_module.clone(), Some(override_cursor)),
                (treasury_module.clone(), Some(override_cursor)),
                (limiter_module.clone(), Some(override_cursor))
            ]
            .into_iter()
            .collect::<HashMap<_, _>>()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_starting_bridge_node() {
        telemetry_subscribers::init_for_testing();
        let bridge_test_cluster = setup().await;
        let kp = bridge_test_cluster.bridge_authority_key(0);

        // prepare node config (server only)
        let tmp_dir = tempdir().unwrap().into_path();
        let authority_key_path = "test_starting_bridge_node_bridge_authority_key";
        let server_listen_port = get_available_port("127.0.0.1");
        let base64_encoded = kp.encode_base64();
        std::fs::write(tmp_dir.join(authority_key_path), base64_encoded).unwrap();

        let config = BridgeNodeConfig {
            server_listen_port,
            metrics_port: get_available_port("127.0.0.1"),
            bridge_authority_key_path: tmp_dir.join(authority_key_path),
            iota: IotaConfig {
                iota_rpc_url: bridge_test_cluster.iota_rpc_url(),
                iota_bridge_chain_id: BridgeChainId::IotaCustom as u8,
                bridge_client_key_path: None,
                bridge_client_gas_object: None,
                iota_bridge_module_last_processed_event_id_override: None,
            },
            eth: EthConfig {
                eth_rpc_url: bridge_test_cluster.eth_rpc_url(),
                eth_bridge_proxy_address: bridge_test_cluster.iota_bridge_address(),
                eth_bridge_chain_id: BridgeChainId::EthCustom as u8,
                eth_contracts_start_block_fallback: None,
                eth_contracts_start_block_override: None,
            },
            approved_governance_actions: vec![],
            run_client: false,
            db_path: None,
        };
        // Spawn bridge node in memory
        let _handle = run_bridge_node(
            config,
            BridgeNodePublicMetadata::empty_for_testing(),
            Registry::new(),
        )
        .await
        .unwrap();

        let server_url = format!("http://127.0.0.1:{}", server_listen_port);
        // Now we expect to see the server to be up and running.
        let res = wait_for_server_to_be_up(server_url, 5).await;
        res.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_starting_bridge_node_with_client() {
        telemetry_subscribers::init_for_testing();
        let bridge_test_cluster = setup().await;
        let kp = bridge_test_cluster.bridge_authority_key(0);

        // prepare node config (server + client)
        let tmp_dir = tempdir().unwrap().into_path();
        let db_path = tmp_dir.join("test_starting_bridge_node_with_client_db");
        let authority_key_path = "test_starting_bridge_node_with_client_bridge_authority_key";
        let server_listen_port = get_available_port("127.0.0.1");

        let base64_encoded = kp.encode_base64();
        std::fs::write(tmp_dir.join(authority_key_path), base64_encoded).unwrap();

        let client_iota_address = IotaAddress::from(kp.public());
        let sender_address = bridge_test_cluster.iota_user_address();
        // send some gas to this address
        bridge_test_cluster
            .test_cluster
            .transfer_iota_must_exceed(sender_address, client_iota_address, 1000000000)
            .await;

        let config = BridgeNodeConfig {
            server_listen_port,
            metrics_port: get_available_port("127.0.0.1"),
            bridge_authority_key_path: tmp_dir.join(authority_key_path),
            iota: IotaConfig {
                iota_rpc_url: bridge_test_cluster.iota_rpc_url(),
                iota_bridge_chain_id: BridgeChainId::IotaCustom as u8,
                bridge_client_key_path: None,
                bridge_client_gas_object: None,
                iota_bridge_module_last_processed_event_id_override: Some(EventID {
                    tx_digest: TransactionDigest::random(),
                    event_seq: 0,
                }),
            },
            eth: EthConfig {
                eth_rpc_url: bridge_test_cluster.eth_rpc_url(),
                eth_bridge_proxy_address: bridge_test_cluster.iota_bridge_address(),
                eth_bridge_chain_id: BridgeChainId::EthCustom as u8,
                eth_contracts_start_block_fallback: Some(0),
                eth_contracts_start_block_override: None,
            },
            approved_governance_actions: vec![],
            run_client: true,
            db_path: Some(db_path),
        };
        // Spawn bridge node in memory
        let _handle = run_bridge_node(
            config,
            BridgeNodePublicMetadata::empty_for_testing(),
            Registry::new(),
        )
        .await
        .unwrap();

        let server_url = format!("http://127.0.0.1:{}", server_listen_port);
        // Now we expect to see the server to be up and running.
        // client components are spawned earlier than server, so as long as the server
        // is up, we know the client components are already running.
        let res = wait_for_server_to_be_up(server_url, 5).await;
        res.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_starting_bridge_node_with_client_and_separate_client_key() {
        telemetry_subscribers::init_for_testing();
        let bridge_test_cluster = setup().await;
        let kp = bridge_test_cluster.bridge_authority_key(0);

        // prepare node config (server + client)
        let tmp_dir = tempdir().unwrap().into_path();
        let db_path =
            tmp_dir.join("test_starting_bridge_node_with_client_and_separate_client_key_db");
        let authority_key_path =
            "test_starting_bridge_node_with_client_and_separate_client_key_bridge_authority_key";
        let server_listen_port = get_available_port("127.0.0.1");

        // prepare bridge authority key
        let base64_encoded = kp.encode_base64();
        std::fs::write(tmp_dir.join(authority_key_path), base64_encoded).unwrap();

        // prepare bridge client key
        let (_, kp): (_, Secp256k1KeyPair) = get_key_pair();
        let kp = IotaKeyPair::from(kp);
        let client_key_path =
            "test_starting_bridge_node_with_client_and_separate_client_key_bridge_client_key";
        std::fs::write(tmp_dir.join(client_key_path), kp.encode_base64()).unwrap();
        let client_iota_address = IotaAddress::from(&kp.public());
        let sender_address = bridge_test_cluster.iota_user_address();
        // send some gas to this address
        let gas_obj = bridge_test_cluster
            .test_cluster
            .transfer_iota_must_exceed(sender_address, client_iota_address, 1000000000)
            .await;

        let config = BridgeNodeConfig {
            server_listen_port,
            metrics_port: get_available_port("127.0.0.1"),
            bridge_authority_key_path: tmp_dir.join(authority_key_path),
            iota: IotaConfig {
                iota_rpc_url: bridge_test_cluster.iota_rpc_url(),
                iota_bridge_chain_id: BridgeChainId::IotaCustom as u8,
                bridge_client_key_path: Some(tmp_dir.join(client_key_path)),
                bridge_client_gas_object: Some(gas_obj),
                iota_bridge_module_last_processed_event_id_override: Some(EventID {
                    tx_digest: TransactionDigest::random(),
                    event_seq: 0,
                }),
            },
            eth: EthConfig {
                eth_rpc_url: bridge_test_cluster.eth_rpc_url(),
                eth_bridge_proxy_address: bridge_test_cluster.iota_bridge_address(),
                eth_bridge_chain_id: BridgeChainId::EthCustom as u8,
                eth_contracts_start_block_fallback: Some(0),
                eth_contracts_start_block_override: Some(0),
            },
            approved_governance_actions: vec![],
            run_client: true,
            db_path: Some(db_path),
        };
        // Spawn bridge node in memory
        let _handle = run_bridge_node(
            config,
            BridgeNodePublicMetadata::empty_for_testing(),
            Registry::new(),
        )
        .await
        .unwrap();

        let server_url = format!("http://127.0.0.1:{}", server_listen_port);
        // Now we expect to see the server to be up and running.
        // client components are spawned earlier than server, so as long as the server
        // is up, we know the client components are already running.
        let res = wait_for_server_to_be_up(server_url, 5).await;
        res.unwrap();
    }

    async fn setup() -> BridgeTestCluster {
        BridgeTestClusterBuilder::new()
            .with_eth_env(true)
            .with_bridge_cluster(false)
            .with_num_validators(2)
            .build()
            .await
    }
}
