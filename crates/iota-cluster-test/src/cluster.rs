// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{net::SocketAddr, path::Path};

use async_trait::async_trait;
use iota_config::{
    Config, IOTA_GENESIS_FILENAME, IOTA_KEYSTORE_FILENAME, IOTA_NETWORK_CONFIG, PersistedConfig,
    genesis::Genesis,
};
use iota_genesis_builder::SnapshotSource;
use iota_graphql_rpc::{
    config::ConnectionConfig, test_infra::cluster::start_graphql_server_with_fn_rpc,
};
use iota_indexer::test_utils::{IndexerTypeConfig, start_test_indexer};
use iota_keys::keystore::{AccountKeystore, FileBasedKeystore, Keystore};
use iota_sdk::{
    iota_client_config::{IotaClientConfig, IotaEnv},
    wallet_context::WalletContext,
};
use iota_swarm::memory::Swarm;
use iota_swarm_config::{
    genesis_config::GenesisConfig,
    network_config::{NetworkConfig, NetworkConfigLight},
};
use iota_types::{
    base_types::IotaAddress,
    crypto::{AccountKeyPair, IotaKeyPair, KeypairTraits, get_key_pair},
};
use tempfile::tempdir;
use test_cluster::{TestCluster, TestClusterBuilder};
use tracing::info;

use super::config::{ClusterTestOpt, Env};

const DEVNET_FAUCET_ADDR: &str = "https://faucet.devnet.iota.cafe:443";
const TESTNET_FAUCET_ADDR: &str = "https://faucet.testnet.iota.cafe:443";
const DEVNET_FULLNODE_ADDR: &str = "https://api.devnet.iota.cafe:443";
const TESTNET_FULLNODE_ADDR: &str = "https://api.testnet.iota.cafe:443";

pub struct ClusterFactory;

impl ClusterFactory {
    pub async fn start(
        options: &ClusterTestOpt,
    ) -> Result<Box<dyn Cluster + Sync + Send>, anyhow::Error> {
        Ok(match &options.env {
            Env::NewLocal => Box::new(LocalNewCluster::start(options).await?),
            _ => Box::new(RemoteRunningCluster::start(options).await?),
        })
    }
}

/// Cluster Abstraction
#[async_trait]
pub trait Cluster {
    async fn start(options: &ClusterTestOpt) -> Result<Self, anyhow::Error>
    where
        Self: Sized;

    fn fullnode_url(&self) -> &str;
    fn user_key(&self) -> AccountKeyPair;
    fn indexer_url(&self) -> &Option<String>;

    /// Returns faucet url in a remote cluster.
    fn remote_faucet_url(&self) -> Option<&str>;

    /// Returns faucet key in a local cluster.
    fn local_faucet_key(&self) -> Option<&AccountKeyPair>;

    /// Place to put config for the wallet, and any locally running services.
    fn config_directory(&self) -> &Path;
}

/// Represents an up and running cluster deployed remotely.
pub struct RemoteRunningCluster {
    fullnode_url: String,
    faucet_url: String,
    config_directory: tempfile::TempDir,
}

#[async_trait]
impl Cluster for RemoteRunningCluster {
    async fn start(options: &ClusterTestOpt) -> Result<Self, anyhow::Error> {
        let (fullnode_url, faucet_url) = match options.env {
            Env::Devnet => (
                String::from(DEVNET_FULLNODE_ADDR),
                String::from(DEVNET_FAUCET_ADDR),
            ),
            Env::Testnet => (
                String::from(TESTNET_FULLNODE_ADDR),
                String::from(TESTNET_FAUCET_ADDR),
            ),
            Env::CustomRemote => (
                options
                    .fullnode_address
                    .clone()
                    .expect("Expect 'fullnode_address' for Env::Custom"),
                options
                    .faucet_address
                    .clone()
                    .expect("Expect 'faucet_address' for Env::Custom"),
            ),
            Env::NewLocal => unreachable!("NewLocal shouldn't use RemoteRunningCluster"),
        };

        // TODO: test connectivity before proceeding?

        Ok(Self {
            fullnode_url,
            faucet_url,
            config_directory: tempfile::tempdir()?,
        })
    }

    fn fullnode_url(&self) -> &str {
        &self.fullnode_url
    }

    fn indexer_url(&self) -> &Option<String> {
        &None
    }

    fn user_key(&self) -> AccountKeyPair {
        get_key_pair().1
    }

    fn remote_faucet_url(&self) -> Option<&str> {
        Some(&self.faucet_url)
    }

    fn local_faucet_key(&self) -> Option<&AccountKeyPair> {
        None
    }

    fn config_directory(&self) -> &Path {
        self.config_directory.path()
    }
}

/// Represents a local Cluster which starts per cluster test run.
pub struct LocalNewCluster {
    test_cluster: TestCluster,
    fullnode_url: String,
    indexer_url: Option<String>,
    faucet_key: AccountKeyPair,
    config_directory: tempfile::TempDir,
}

impl LocalNewCluster {
    #[allow(unused)]
    pub fn swarm(&self) -> &Swarm {
        &self.test_cluster.swarm
    }
}

#[async_trait]
impl Cluster for LocalNewCluster {
    async fn start(options: &ClusterTestOpt) -> Result<Self, anyhow::Error> {
        let data_ingestion_path = tempdir()?.into_path();
        // TODO: options should contain port instead of address
        let fullnode_rpc_addr = options.fullnode_address.as_ref().map(|addr| {
            addr.parse::<SocketAddr>()
                .expect("Unable to parse fullnode address")
        });

        let indexer_address = options.indexer_address.as_ref().map(|addr| {
            addr.parse::<SocketAddr>()
                .expect("Unable to parse indexer address")
        });

        let mut cluster_builder = TestClusterBuilder::new()
            .enable_fullnode_events()
            .with_data_ingestion_dir(data_ingestion_path.clone());

        // Check if we already have a config directory that is passed
        if let Some(config_dir) = options.config_dir.clone() {
            assert!(options.epoch_duration_ms.is_none());
            // Load the config of the IOTA authority.
            let network_config_path = config_dir.join(IOTA_NETWORK_CONFIG);
            let NetworkConfigLight {
                validator_configs,
                account_keys,
                committee_with_network: _,
            } = PersistedConfig::read(&network_config_path).map_err(|err| {
                err.context(format!(
                    "Cannot open IOTA network config file at {:?}",
                    network_config_path
                ))
            })?;

            // Add genesis objects
            let genesis_path = config_dir.join(IOTA_GENESIS_FILENAME);
            let genesis = Genesis::load(genesis_path)?;
            let network_config = NetworkConfig {
                validator_configs,
                account_keys,
                genesis,
            };
            cluster_builder = cluster_builder.set_network_config(network_config);

            cluster_builder = cluster_builder.with_config_dir(config_dir);
        } else {
            // Let the faucet account hold 1000 gas objects on genesis
            let mut genesis_config = GenesisConfig::custom_genesis(1, 100);
            // Add any migration sources
            let local_snapshots = options
                .local_migration_snapshots
                .iter()
                .cloned()
                .map(SnapshotSource::Local);
            let remote_snapshots = options
                .remote_migration_snapshots
                .iter()
                .cloned()
                .map(SnapshotSource::S3);
            genesis_config.migration_sources = local_snapshots.chain(remote_snapshots).collect();
            // Custom genesis should be build here where we add the extra accounts
            cluster_builder = cluster_builder.set_genesis_config(genesis_config);

            if let Some(epoch_duration_ms) = options.epoch_duration_ms {
                cluster_builder = cluster_builder.with_epoch_duration_ms(epoch_duration_ms);
            }
        }

        if let Some(fullnode_rpc_addr) = fullnode_rpc_addr {
            cluster_builder = cluster_builder.with_fullnode_rpc_addr(fullnode_rpc_addr);
        }

        let mut test_cluster = cluster_builder.build().await;

        // Use the wealthy account for faucet
        let faucet_key = test_cluster.swarm.config_mut().account_keys.swap_remove(0);
        let faucet_address = IotaAddress::from(faucet_key.public());
        info!(?faucet_address, "faucet_address");

        // This cluster has fullnode handle, safe to unwrap
        let fullnode_url = test_cluster.fullnode_handle.rpc_url.clone();

        if let (Some(pg_address), Some(indexer_address)) =
            (options.pg_address.clone(), indexer_address)
        {
            // Start in writer mode
            start_test_indexer(
                pg_address.clone(),
                // reset the existing db
                true,
                None,
                fullnode_url.clone(),
                IndexerTypeConfig::writer_mode(None, None),
                Some(data_ingestion_path.clone()),
            )
            .await;

            // Start in reader mode
            start_test_indexer(
                pg_address,
                false,
                None,
                fullnode_url.clone(),
                IndexerTypeConfig::reader_mode(indexer_address.to_string()),
                Some(data_ingestion_path),
            )
            .await;
        }

        if let Some(graphql_address) = &options.graphql_address {
            let graphql_address = graphql_address.parse::<SocketAddr>()?;
            let graphql_connection_config = ConnectionConfig::new(
                Some(graphql_address.port()),
                Some(graphql_address.ip().to_string()),
                options.pg_address.clone(),
                None,
                None,
                None,
            );

            start_graphql_server_with_fn_rpc(
                graphql_connection_config.clone(),
                Some(fullnode_url.clone()),
                // cancellation_token
                None,
            )
            .await;
        }

        // Let nodes connect to one another
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // TODO: test connectivity before proceeding?
        Ok(Self {
            test_cluster,
            fullnode_url,
            faucet_key,
            config_directory: tempfile::tempdir()?,
            indexer_url: options.indexer_address.clone(),
        })
    }

    fn fullnode_url(&self) -> &str {
        &self.fullnode_url
    }

    fn indexer_url(&self) -> &Option<String> {
        &self.indexer_url
    }

    fn user_key(&self) -> AccountKeyPair {
        get_key_pair().1
    }

    fn remote_faucet_url(&self) -> Option<&str> {
        None
    }

    fn local_faucet_key(&self) -> Option<&AccountKeyPair> {
        Some(&self.faucet_key)
    }

    fn config_directory(&self) -> &Path {
        self.config_directory.path()
    }
}

// Make linter happy
#[async_trait]
impl Cluster for Box<dyn Cluster + Send + Sync> {
    async fn start(_options: &ClusterTestOpt) -> Result<Self, anyhow::Error> {
        unreachable!(
            "If we already have a boxed Cluster trait object we wouldn't have to call this function"
        );
    }
    fn fullnode_url(&self) -> &str {
        (**self).fullnode_url()
    }
    fn indexer_url(&self) -> &Option<String> {
        (**self).indexer_url()
    }

    fn user_key(&self) -> AccountKeyPair {
        (**self).user_key()
    }

    fn remote_faucet_url(&self) -> Option<&str> {
        (**self).remote_faucet_url()
    }

    fn local_faucet_key(&self) -> Option<&AccountKeyPair> {
        (**self).local_faucet_key()
    }

    fn config_directory(&self) -> &Path {
        (**self).config_directory()
    }
}

pub fn new_wallet_context_from_cluster(
    cluster: &(dyn Cluster + Sync + Send),
    key_pair: AccountKeyPair,
) -> WalletContext {
    let config_dir = cluster.config_directory();
    let wallet_config_path = config_dir.join("client.yaml");
    let fullnode_url = cluster.fullnode_url();
    info!("Use RPC: {}", &fullnode_url);
    let keystore_path = config_dir.join(IOTA_KEYSTORE_FILENAME);
    let mut keystore = Keystore::from(FileBasedKeystore::new(&keystore_path).unwrap());
    let address: IotaAddress = key_pair.public().into();
    keystore
        .add_key(None, IotaKeyPair::Ed25519(key_pair))
        .unwrap();
    IotaClientConfig::new(keystore)
        .with_envs([IotaEnv::new("localnet", fullnode_url)])
        .with_active_address(address)
        .with_active_env("localnet".to_string())
        .persisted(&wallet_config_path)
        .save()
        .unwrap();

    info!(
        "Initialize wallet from config path: {:?}",
        wallet_config_path
    );

    WalletContext::new(&wallet_config_path, None, None).unwrap_or_else(|e| {
        panic!(
            "Failed to init wallet context from path {:?}, error: {e}",
            wallet_config_path
        )
    })
}
