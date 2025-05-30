// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{path::PathBuf, str::FromStr, sync::Arc};

use anyhow::anyhow;
use clap::*;
use ethers::{
    providers::Middleware,
    types::{Address as EthAddress, U256},
};
use fastcrypto::{
    encoding::{Encoding, Hex},
    hash::{HashFunction, Keccak256},
};
use iota_bridge::{
    abi::{EthBridgeCommittee, EthIotaBridge, eth_iota_bridge},
    crypto::BridgeAuthorityPublicKeyBytes,
    error::BridgeResult,
    iota_client::IotaBridgeClient,
    types::{
        AddTokensOnEvmAction, AddTokensOnIotaAction, AssetPriceUpdateAction,
        BlocklistCommitteeAction, BlocklistType, BridgeAction, EmergencyAction,
        EmergencyActionType, EvmContractUpgradeAction, LimitUpdateAction,
    },
    utils::{EthSigner, get_eth_signer_client},
};
use iota_config::Config;
use iota_json_rpc_types::IotaObjectDataOptions;
use iota_keys::keypair_file::read_key;
use iota_sdk::IotaClientBuilder;
use iota_types::{
    BRIDGE_PACKAGE_ID, TypeTag,
    base_types::{IotaAddress, ObjectID, ObjectRef},
    bridge::{BRIDGE_MODULE_NAME, BridgeChainId},
    crypto::{IotaKeyPair, Signature},
    programmable_transaction_builder::ProgrammableTransactionBuilder,
    transaction::{ObjectArg, Transaction, TransactionData},
};
use move_core_types::ident_str;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use shared_crypto::intent::{Intent, IntentMessage};
use tracing::info;

pub const SEPOLIA_BRIDGE_PROXY_ADDR: &str = "0xAE68F87938439afEEDd6552B0E83D2CbC2473623";

#[derive(Parser)]
pub struct Args {
    #[command(subcommand)]
    pub command: BridgeCommand,
}

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum Network {
    Testnet,
}

#[derive(Parser)]
pub enum BridgeCommand {
    CreateBridgeValidatorKey {
        path: PathBuf,
    },
    CreateBridgeClientKey {
        path: PathBuf,
        #[arg(long, default_value = "false")]
        use_ecdsa: bool,
    },
    /// Read bridge key from a file and print related information
    /// If `is-validator-key` is true, the key must be a secp256k1 key
    ExamineKey {
        path: PathBuf,
        #[arg(long)]
        is_validator_key: bool,
    },
    CreateBridgeNodeConfigTemplate {
        path: PathBuf,
        #[arg(long)]
        run_client: bool,
    },
    /// Governance client to facilitate and execute Bridge governance actions
    Governance {
        /// Path of BridgeCliConfig
        #[arg(long)]
        config_path: PathBuf,
        #[arg(long)]
        chain_id: u8,
        #[command(subcommand)]
        cmd: GovernanceClientCommands,
        /// If true, only collect signatures but not execute on chain
        #[arg(long)]
        dry_run: bool,
    },
    /// View current status of Eth bridge
    ViewEthBridge {
        #[arg(long)]
        network: Option<Network>,
        #[arg(long)]
        bridge_proxy: Option<EthAddress>,
        #[arg(long)]
        eth_rpc_url: String,
    },
    /// View current list of registered validators
    ViewBridgeRegistration {
        #[arg(long)]
        iota_rpc_url: String,
    },
    /// View current status of IOTA bridge
    ViewIotaBridge {
        #[arg(long)]
        iota_rpc_url: String,
        #[arg(long, default_value = "false")]
        hex: bool,
        #[arg(long, default_value = "false")]
        ping: bool,
    },
    /// Client to facilitate and execute Bridge actions
    Client {
        /// Path of BridgeCliConfig
        #[arg(long)]
        config_path: PathBuf,
        #[command(subcommand)]
        cmd: BridgeClientCommands,
    },
}

#[derive(Parser)]
pub enum GovernanceClientCommands {
    EmergencyButton {
        #[arg(name = "nonce", long)]
        nonce: u64,
        #[arg(name = "action-type", long)]
        action_type: EmergencyActionType,
    },
    UpdateCommitteeBlocklist {
        #[arg(name = "nonce", long)]
        nonce: u64,
        #[arg(name = "blocklist-type", long)]
        blocklist_type: BlocklistType,
        #[arg(name = "pubkey-hex", use_value_delimiter = true, long)]
        pubkeys_hex: Vec<BridgeAuthorityPublicKeyBytes>,
    },
    UpdateLimit {
        #[arg(name = "nonce", long)]
        nonce: u64,
        #[arg(name = "sending-chain", long)]
        sending_chain: u8,
        #[arg(name = "new-usd-limit", long)]
        new_usd_limit: u64,
    },
    UpdateAssetPrice {
        #[arg(name = "nonce", long)]
        nonce: u64,
        #[arg(name = "token-id", long)]
        token_id: u8,
        #[arg(name = "new-usd-price", long)]
        new_usd_price: u64,
    },
    AddTokensOnIota {
        #[arg(name = "nonce", long)]
        nonce: u64,
        #[arg(name = "token-ids", use_value_delimiter = true, long)]
        token_ids: Vec<u8>,
        #[arg(name = "token-type-names", use_value_delimiter = true, long)]
        token_type_names: Vec<TypeTag>,
        #[arg(name = "token-prices", use_value_delimiter = true, long)]
        token_prices: Vec<u64>,
    },
    AddTokensOnEvm {
        #[arg(name = "nonce", long)]
        nonce: u64,
        #[arg(name = "token-ids", use_value_delimiter = true, long)]
        token_ids: Vec<u8>,
        #[arg(name = "token-type-names", use_value_delimiter = true, long)]
        token_addresses: Vec<EthAddress>,
        #[arg(name = "token-prices", use_value_delimiter = true, long)]
        token_prices: Vec<u64>,
        #[arg(name = "token-iota-decimals", use_value_delimiter = true, long)]
        token_iota_decimals: Vec<u8>,
    },
    #[command(name = "upgrade-evm-contract")]
    UpgradeEVMContract {
        #[arg(name = "nonce", long)]
        nonce: u64,
        #[arg(name = "proxy-address", long)]
        proxy_address: EthAddress,
        /// The address of the new implementation contract
        #[arg(name = "implementation-address", long)]
        implementation_address: EthAddress,
        /// Function selector with params types, e.g. `foo(uint256,bool,string)`
        #[arg(name = "function-selector", long)]
        function_selector: Option<String>,
        /// Params to be passed to the function, e.g. `420,false,hello`
        #[arg(name = "params", use_value_delimiter = true, long)]
        params: Vec<String>,
    },
}

pub fn make_action(chain_id: BridgeChainId, cmd: &GovernanceClientCommands) -> BridgeAction {
    match cmd {
        GovernanceClientCommands::EmergencyButton { nonce, action_type } => {
            BridgeAction::EmergencyAction(EmergencyAction {
                nonce: *nonce,
                chain_id,
                action_type: *action_type,
            })
        }
        GovernanceClientCommands::UpdateCommitteeBlocklist {
            nonce,
            blocklist_type,
            pubkeys_hex,
        } => BridgeAction::BlocklistCommitteeAction(BlocklistCommitteeAction {
            nonce: *nonce,
            chain_id,
            blocklist_type: *blocklist_type,
            members_to_update: pubkeys_hex.clone(),
        }),
        GovernanceClientCommands::UpdateLimit {
            nonce,
            sending_chain,
            new_usd_limit,
        } => {
            let sending_chain_id =
                BridgeChainId::try_from(*sending_chain).expect("Invalid sending chain id");
            BridgeAction::LimitUpdateAction(LimitUpdateAction {
                nonce: *nonce,
                chain_id,
                sending_chain_id,
                new_usd_limit: *new_usd_limit,
            })
        }
        GovernanceClientCommands::UpdateAssetPrice {
            nonce,
            token_id,
            new_usd_price,
        } => BridgeAction::AssetPriceUpdateAction(AssetPriceUpdateAction {
            nonce: *nonce,
            chain_id,
            token_id: *token_id,
            new_usd_price: *new_usd_price,
        }),
        GovernanceClientCommands::AddTokensOnIota {
            nonce,
            token_ids,
            token_type_names,
            token_prices,
        } => {
            assert_eq!(token_ids.len(), token_type_names.len());
            assert_eq!(token_ids.len(), token_prices.len());
            BridgeAction::AddTokensOnIotaAction(AddTokensOnIotaAction {
                nonce: *nonce,
                chain_id,
                native: false, // only foreign tokens are supported now
                token_ids: token_ids.clone(),
                token_type_names: token_type_names.clone(),
                token_prices: token_prices.clone(),
            })
        }
        GovernanceClientCommands::AddTokensOnEvm {
            nonce,
            token_ids,
            token_addresses,
            token_prices,
            token_iota_decimals,
        } => {
            assert_eq!(token_ids.len(), token_addresses.len());
            assert_eq!(token_ids.len(), token_prices.len());
            assert_eq!(token_ids.len(), token_iota_decimals.len());
            BridgeAction::AddTokensOnEvmAction(AddTokensOnEvmAction {
                nonce: *nonce,
                native: true, // only eth native tokens are supported now
                chain_id,
                token_ids: token_ids.clone(),
                token_addresses: token_addresses.clone(),
                token_prices: token_prices.clone(),
                token_iota_decimals: token_iota_decimals.clone(),
            })
        }
        GovernanceClientCommands::UpgradeEVMContract {
            nonce,
            proxy_address,
            implementation_address,
            function_selector,
            params,
        } => {
            let call_data = match function_selector {
                Some(function_selector) => encode_call_data(function_selector, params),
                None => vec![],
            };
            BridgeAction::EvmContractUpgradeAction(EvmContractUpgradeAction {
                nonce: *nonce,
                chain_id,
                proxy_address: *proxy_address,
                new_impl_address: *implementation_address,
                call_data,
            })
        }
    }
}

fn encode_call_data(function_selector: &str, params: &[String]) -> Vec<u8> {
    let left = function_selector
        .find('(')
        .expect("Invalid function selector, no left parentheses");
    let right = function_selector
        .find(')')
        .expect("Invalid function selector, no right parentheses");
    let param_types = function_selector[left + 1..right]
        .split(',')
        .map(|x| x.trim())
        .collect::<Vec<&str>>();

    assert_eq!(param_types.len(), params.len(), "Invalid number of params");

    let mut call_data = Keccak256::digest(function_selector).digest[0..4].to_vec();
    let mut tokens = vec![];
    for (param, param_type) in params.iter().zip(param_types.iter()) {
        match param_type.to_lowercase().as_str() {
            "uint256" => {
                tokens.push(ethers::abi::Token::Uint(
                    ethers::types::U256::from_dec_str(param).expect("Invalid U256"),
                ));
            }
            "bool" => {
                tokens.push(ethers::abi::Token::Bool(match param.as_str() {
                    "true" => true,
                    "false" => false,
                    _ => panic!("Invalid bool in params"),
                }));
            }
            "string" => {
                tokens.push(ethers::abi::Token::String(param.clone()));
            }
            // TODO: need to support more types if needed
            _ => panic!("Invalid param type"),
        }
    }
    if !tokens.is_empty() {
        call_data.extend(ethers::abi::encode(&tokens));
    }
    call_data
}

pub fn select_contract_address(
    config: &LoadedBridgeCliConfig,
    cmd: &GovernanceClientCommands,
) -> EthAddress {
    match cmd {
        GovernanceClientCommands::EmergencyButton { .. } => config.eth_bridge_proxy_address,
        GovernanceClientCommands::UpdateCommitteeBlocklist { .. } => {
            config.eth_bridge_committee_proxy_address
        }
        GovernanceClientCommands::UpdateLimit { .. } => config.eth_bridge_limiter_proxy_address,
        GovernanceClientCommands::UpdateAssetPrice { .. } => config.eth_bridge_config_proxy_address,
        GovernanceClientCommands::UpgradeEVMContract { proxy_address, .. } => *proxy_address,
        GovernanceClientCommands::AddTokensOnIota { .. } => unreachable!(),
        GovernanceClientCommands::AddTokensOnEvm { .. } => config.eth_bridge_config_proxy_address,
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct BridgeCliConfig {
    /// Rpc url for IOTA fullnode, used for query stuff and submit transactions.
    pub iota_rpc_url: String,
    /// Rpc url for Eth fullnode, used for query stuff.
    pub eth_rpc_url: String,
    /// Proxy address for IotaBridge deployed on Eth
    pub eth_bridge_proxy_address: EthAddress,
    /// Path of the file where private key is stored. The content could be any
    /// of the following:
    /// - Base64 encoded `flag || privkey` for ECDSA key
    /// - Base64 encoded `privkey` for Raw key
    /// - Hex encoded `privkey` for Raw key
    /// At least one of `iota_key_path` or `eth_key_path` must be provided.
    /// If only one is provided, it will be used for both IOTA and Eth.
    pub iota_key_path: Option<PathBuf>,
    /// See `iota_key_path`. Must be Secp256k1 key.
    pub eth_key_path: Option<PathBuf>,
}

impl Config for BridgeCliConfig {}

pub struct LoadedBridgeCliConfig {
    /// Rpc url for IOTA fullnode, used for query stuff and submit transactions.
    pub iota_rpc_url: String,
    /// Rpc url for Eth fullnode, used for query stuff.
    pub eth_rpc_url: String,
    /// Proxy address for IotaBridge deployed on Eth
    pub eth_bridge_proxy_address: EthAddress,
    /// Proxy address for BridgeCommittee deployed on Eth
    pub eth_bridge_committee_proxy_address: EthAddress,
    /// Proxy address for BridgeConfig deployed on Eth
    pub eth_bridge_config_proxy_address: EthAddress,
    /// Proxy address for BridgeLimiter deployed on Eth
    pub eth_bridge_limiter_proxy_address: EthAddress,
    /// Key pair for IOTA operations
    iota_key: IotaKeyPair,
    /// Key pair for Eth operations, must be Secp256k1 key
    eth_signer: EthSigner,
}

impl LoadedBridgeCliConfig {
    pub async fn load(cli_config: BridgeCliConfig) -> anyhow::Result<Self> {
        if cli_config.eth_key_path.is_none() && cli_config.iota_key_path.is_none() {
            return Err(anyhow!(
                "At least one of `iota_key_path` or `eth_key_path` must be provided"
            ));
        }
        let iota_key = if let Some(iota_key_path) = &cli_config.iota_key_path {
            Some(read_key(iota_key_path, false)?)
        } else {
            None
        };
        let eth_key = if let Some(eth_key_path) = &cli_config.eth_key_path {
            let eth_key = read_key(eth_key_path, true)?;
            Some(eth_key)
        } else {
            None
        };
        let (eth_key, iota_key) = {
            if eth_key.is_none() {
                let iota_key = iota_key.unwrap();
                if !matches!(iota_key, IotaKeyPair::Secp256k1(_)) {
                    return Err(anyhow!("Eth key must be an ECDSA key"));
                }
                (iota_key.copy(), iota_key)
            } else if iota_key.is_none() {
                let eth_key = eth_key.unwrap();
                (eth_key.copy(), eth_key)
            } else {
                (eth_key.unwrap(), iota_key.unwrap())
            }
        };

        let provider = Arc::new(
            ethers::prelude::Provider::<ethers::providers::Http>::try_from(&cli_config.eth_rpc_url)
                .unwrap()
                .interval(std::time::Duration::from_millis(2000)),
        );
        let private_key = Hex::encode(eth_key.to_bytes_no_flag());
        let eth_signer = get_eth_signer_client(&cli_config.eth_rpc_url, &private_key).await?;
        let iota_bridge = EthIotaBridge::new(cli_config.eth_bridge_proxy_address, provider.clone());
        let eth_bridge_committee_proxy_address: EthAddress = iota_bridge.committee().call().await?;
        let eth_bridge_limiter_proxy_address: EthAddress = iota_bridge.limiter().call().await?;
        let eth_committee =
            EthBridgeCommittee::new(eth_bridge_committee_proxy_address, provider.clone());
        let eth_bridge_committee_proxy_address: EthAddress = iota_bridge.committee().call().await?;
        let eth_bridge_config_proxy_address: EthAddress = eth_committee.config().call().await?;

        let eth_address = eth_signer.address();
        let eth_chain_id = provider.get_chainid().await?;
        let iota_address = IotaAddress::from(&iota_key.public());
        println!("Using IOTA address: {:?}", iota_address);
        println!("Using Eth address: {:?}", eth_address);
        println!("Using Eth chain: {:?}", eth_chain_id);

        Ok(Self {
            iota_rpc_url: cli_config.iota_rpc_url,
            eth_rpc_url: cli_config.eth_rpc_url,
            eth_bridge_proxy_address: cli_config.eth_bridge_proxy_address,
            eth_bridge_committee_proxy_address,
            eth_bridge_limiter_proxy_address,
            eth_bridge_config_proxy_address,
            iota_key,
            eth_signer,
        })
    }
}

impl LoadedBridgeCliConfig {
    pub fn eth_signer(self: &LoadedBridgeCliConfig) -> &EthSigner {
        &self.eth_signer
    }

    pub async fn get_iota_account_info(
        self: &LoadedBridgeCliConfig,
    ) -> anyhow::Result<(IotaKeyPair, IotaAddress, ObjectRef)> {
        let pubkey = self.iota_key.public();
        let iota_client_address = IotaAddress::from(&pubkey);
        let iota_sdk_client = IotaClientBuilder::default()
            .build(self.iota_rpc_url.clone())
            .await?;
        let gases = iota_sdk_client
            .coin_read_api()
            .get_coins(iota_client_address, None, None, None)
            .await?
            .data;
        // TODO: is 5 IOTA a good number?
        let gas = gases
            .into_iter()
            .find(|coin| coin.balance >= 5_000_000_000)
            .ok_or(anyhow!(
                "Did not find gas object with enough balance for {}",
                iota_client_address
            ))?;
        println!("Using Gas object: {}", gas.coin_object_id);
        Ok((self.iota_key.copy(), iota_client_address, gas.object_ref()))
    }
}
#[derive(Parser)]
pub enum BridgeClientCommands {
    DepositNativeEtherOnEth {
        #[arg(long)]
        ether_amount: f64,
        #[arg(long)]
        target_chain: u8,
        #[arg(long)]
        iota_recipient_address: IotaAddress,
    },
    DepositOnIota {
        #[arg(long)]
        coin_object_id: ObjectID,
        #[arg(long)]
        coin_type: String,
        #[arg(long)]
        target_chain: u8,
        #[arg(long)]
        recipient_address: EthAddress,
    },
    ClaimOnEth {
        #[arg(long)]
        seq_num: u64,
    },
}

impl BridgeClientCommands {
    pub async fn handle(
        self,
        config: &LoadedBridgeCliConfig,
        iota_bridge_client: IotaBridgeClient,
    ) -> anyhow::Result<()> {
        match self {
            BridgeClientCommands::DepositNativeEtherOnEth {
                ether_amount,
                target_chain,
                iota_recipient_address,
            } => {
                let eth_iota_bridge = EthIotaBridge::new(
                    config.eth_bridge_proxy_address,
                    Arc::new(config.eth_signer().clone()),
                );
                // Note: even with f64 there may still be loss of precision even there are a lot
                // of 0s
                let int_part = ether_amount.trunc() as u64;
                let frac_part = ether_amount.fract();
                let int_wei = U256::from(int_part) * U256::exp10(18);
                let frac_wei = U256::from((frac_part * 1_000_000_000_000_000_000f64) as u64);
                let amount = int_wei + frac_wei;
                let eth_tx = eth_iota_bridge
                    .bridge_eth(iota_recipient_address.to_vec().into(), target_chain)
                    .value(amount);
                let pending_tx = eth_tx.send().await.unwrap();
                let tx_receipt = pending_tx.await.unwrap().unwrap();
                info!(
                    "Deposited {ether_amount} Ethers to {:?} (target chain {target_chain}). Receipt: {:?}",
                    iota_recipient_address, tx_receipt,
                );
                Ok(())
            }
            BridgeClientCommands::ClaimOnEth { seq_num } => {
                claim_on_eth(seq_num, config, iota_bridge_client)
                    .await
                    .map_err(|e| anyhow!("{:?}", e))
            }
            BridgeClientCommands::DepositOnIota {
                coin_object_id,
                coin_type,
                target_chain,
                recipient_address,
            } => {
                let target_chain = BridgeChainId::try_from(target_chain).expect("Invalid chain id");
                let coin_type = TypeTag::from_str(&coin_type).expect("Invalid coin type");
                deposit_on_iota(
                    coin_object_id,
                    coin_type,
                    target_chain,
                    recipient_address,
                    config,
                    iota_bridge_client,
                )
                .await
            }
        }
    }
}

async fn deposit_on_iota(
    coin_object_id: ObjectID,
    coin_type: TypeTag,
    target_chain: BridgeChainId,
    recipient_address: EthAddress,
    config: &LoadedBridgeCliConfig,
    iota_bridge_client: IotaBridgeClient,
) -> anyhow::Result<()> {
    let target_chain = target_chain as u8;
    let iota_client = iota_bridge_client.iota_client();
    let bridge_object_arg = iota_bridge_client
        .get_mutable_bridge_object_arg_must_succeed()
        .await;
    let rgp = iota_client
        .governance_api()
        .get_reference_gas_price()
        .await
        .unwrap();
    let sender = IotaAddress::from(&config.iota_key.public());
    let gas_obj_ref = iota_client
        .coin_read_api()
        .select_coins(sender, None, 1_000_000_000, vec![])
        .await?
        .first()
        .ok_or(anyhow!("No coin found for address {}", sender))?
        .object_ref();
    let coin_obj_ref = iota_client
        .read_api()
        .get_object_with_options(coin_object_id, IotaObjectDataOptions::default())
        .await?
        .data
        .unwrap()
        .object_ref();

    let mut builder = ProgrammableTransactionBuilder::new();
    let arg_target_chain = builder.pure(target_chain).unwrap();
    let arg_target_address = builder.pure(recipient_address.as_bytes()).unwrap();
    let arg_token = builder
        .obj(ObjectArg::ImmOrOwnedObject(coin_obj_ref))
        .unwrap();
    let arg_bridge = builder.obj(bridge_object_arg).unwrap();

    builder.programmable_move_call(
        BRIDGE_PACKAGE_ID,
        BRIDGE_MODULE_NAME.to_owned(),
        ident_str!("send_token").to_owned(),
        vec![coin_type],
        vec![arg_bridge, arg_target_chain, arg_target_address, arg_token],
    );
    let pt = builder.finish();
    let tx_data =
        TransactionData::new_programmable(sender, vec![gas_obj_ref], pt, 500_000_000, rgp);
    let sig = Signature::new_secure(
        &IntentMessage::new(Intent::iota_transaction(), tx_data.clone()),
        &config.iota_key,
    );
    let signed_tx = Transaction::from_data(tx_data, vec![sig]);
    let tx_digest = *signed_tx.digest();
    info!(?tx_digest, "Sending deposit transaction to IOTA.");
    let resp = iota_bridge_client
        .execute_transaction_block_with_effects(signed_tx)
        .await
        .expect("Failed to execute transaction block");
    if !resp.status_ok().unwrap() {
        return Err(anyhow!("Transaction {:?} failed: {:?}", tx_digest, resp));
    }
    let events = resp.events.unwrap();
    info!(
        ?tx_digest,
        "Deposit transaction succeeded. Events: {:?}", events
    );
    Ok(())
}

async fn claim_on_eth(
    seq_num: u64,
    config: &LoadedBridgeCliConfig,
    iota_bridge_client: IotaBridgeClient,
) -> BridgeResult<()> {
    let iota_chain_id = iota_bridge_client.get_bridge_summary().await?.chain_id;
    let parsed_message = iota_bridge_client
        .get_parsed_token_transfer_message(iota_chain_id, seq_num)
        .await?;
    if parsed_message.is_none() {
        println!("No record found for seq_num: {seq_num}, chain id: {iota_chain_id}");
        return Ok(());
    }
    let parsed_message = parsed_message.unwrap();
    let sigs = iota_bridge_client
        .get_token_transfer_action_onchain_signatures_until_success(iota_chain_id, seq_num)
        .await;
    if sigs.is_none() {
        println!("No signatures found for seq_num: {seq_num}, chain id: {iota_chain_id}");
        return Ok(());
    }
    let signatures = sigs
        .unwrap()
        .into_iter()
        .map(|sig: Vec<u8>| ethers::types::Bytes::from(sig))
        .collect::<Vec<_>>();

    let eth_iota_bridge = EthIotaBridge::new(
        config.eth_bridge_proxy_address,
        Arc::new(config.eth_signer().clone()),
    );
    let message = eth_iota_bridge::Message::from(parsed_message);
    let tx = eth_iota_bridge.transfer_bridged_tokens_with_signatures(signatures, message);
    let _eth_claim_tx_receipt = tx.send().await.unwrap().await.unwrap().unwrap();
    info!("IOTA to Eth bridge transfer claimed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use ethers::abi::FunctionExt;

    use super::*;

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_encode_call_data() {
        let abi_json =
            std::fs::read_to_string("../iota-bridge/abi/tests/mock_iota_bridge_v2.json").unwrap();
        let abi: ethers::abi::Abi = serde_json::from_str(&abi_json).unwrap();

        let function_selector = "initializeV2Params(uint256,bool,string)";
        let params = vec!["420".to_string(), "false".to_string(), "hello".to_string()];
        let call_data = encode_call_data(function_selector, &params);

        let function = abi
            .functions()
            .find(|f| {
                let selector = f.selector();
                call_data.starts_with(selector.as_ref())
            })
            .expect("Function not found");

        // Decode the data excluding the selector
        let tokens = function.decode_input(&call_data[4..]).unwrap();
        assert_eq!(
            tokens,
            vec![
                ethers::abi::Token::Uint(ethers::types::U256::from_dec_str("420").unwrap()),
                ethers::abi::Token::Bool(false),
                ethers::abi::Token::String("hello".to_string())
            ]
        )
    }
}
