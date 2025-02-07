// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! This example requests coins from the faucet and spams with simple transfers
//! to the own address.
//! cargo run -r --example spammer

mod utils;
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

use futures::future;
use iota_config::{iota_config_dir, IOTA_KEYSTORE_FILENAME};
use iota_keys::keystore::{AccountKeystore, FileBasedKeystore};
use iota_sdk::{
    rpc_types::IotaTransactionBlockResponseOptions,
    types::transaction::{Transaction, TransactionData},
    IotaClientBuilder,
};
use iota_types::{
    base_types::IotaAddress,
    programmable_transaction_builder::ProgrammableTransactionBuilder,
    transaction::{Argument, Command, ObjectArg, TransactionKind},
};
use reqwest::Client;
use serde_json::json;
use shared_crypto::intent::Intent;
use tokio::time::{sleep, Instant};
use utils::retrieve_wallet;

const API_URL: &str = &"https://api.testnet.iota.cafe";
const FAUCET_URL: &str = &"https://faucet.testnet.iota.cafe/gas";
// const API_URL: &str = &"http://127.0.0.1:9000";
// const FAUCET_URL: &str = &"http://127.0.0.1:9123/gas";
const TX_AMOUNT: usize = 100_000;
const MIN_AMOUNT: u64 = 4_000_000_000;
const GAS_BUDGET: u64 = 2_000_000;
// Times I got with a remote node, may be different for you:
// 100_000 txs 100 tasks: 93.104808472s
// 100_000 txs 125 tasks: 78.191121339s
// 100_000 txs 150 tasks: 65.607548305s
// 100_000 txs 175 tasks: 61.040636164s
// 100_000 txs 200 tasks: 56.687619343s
// 100_000 txs 225 tasks: 56.832966901s
// 100_000 txs 250 tasks: 56.737081717s
// 100_000 txs 275 tasks: 55.965424063s
// 100_000 txs 300 tasks: 58.517830051s
// 100_000 txs 400 tasks: 59.072423557s
// 100_000 txs 500 tasks: 60.934076263s
const PARALLEL_TASKS: usize = 260;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // tracing_subscriber::fmt::init();
    // // Optionally to log panics as error
    // std::panic::set_hook(Box::new(|p| tracing::error!("{p}")));

    let client = IotaClientBuilder::default().build(API_URL).await?;
    println!("Iota testnet version is: {}", client.api_version());
    let address = retrieve_wallet()?.active_address()?;
    println!("Wallet address is: {address}");
    let gas_price = client.read_api().get_reference_gas_price().await?;

    let keystore = FileBasedKeystore::new(&iota_config_dir()?.join(IOTA_KEYSTORE_FILENAME))?;

    // get enough coins
    let coins_with_enough_balance = loop {
        let mut page = client
            .coin_read_api()
            .get_coins(address, None, None, None)
            .await?;
        let mut coins = page.data;
        while let Some(cursor) = page.next_cursor {
            page = client
                .coin_read_api()
                .get_coins(address, None, Some(cursor), None)
                .await?;
            coins.extend(page.data);
        }

        let (mut coins_with_enough_balance, small_coins): (
            Vec<iota_json_rpc_types::Coin>,
            Vec<iota_json_rpc_types::Coin>,
        ) = coins.into_iter().partition(|c| c.balance > MIN_AMOUNT);
        println!(
            "coins_with_enough_balance: {}",
            coins_with_enough_balance.len()
        );
        println!("small_coins: {}", small_coins.len());
        if coins_with_enough_balance.len() >= PARALLEL_TASKS {
            break coins_with_enough_balance;
        }

        if coins_with_enough_balance.is_empty() {
            request_tokens_from_faucet(address).await?;
        }

        // merge the small coins
        if !small_coins.is_empty() {
            println!("Merging small coins...");

            let pt = {
                let mut builder = ProgrammableTransactionBuilder::new();

                let coin_args = small_coins
                    .into_iter()
                    .map(|c| {
                        builder
                            .obj(ObjectArg::ImmOrOwnedObject(c.object_ref()))
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                builder.command(Command::MergeCoins(Argument::GasCoin, coin_args));
                builder.finish()
            };
            let tx_data = TransactionData::new(
                TransactionKind::programmable(pt),
                address,
                coins_with_enough_balance.pop().unwrap().object_ref(),
                GAS_BUDGET * 10,
                gas_price,
            );

            let signature = keystore.sign_secure(&address, &tx_data, Intent::iota_transaction())?;

            match client
                .quorum_driver_api()
                .execute_transaction_block(
                    Transaction::from_data(tx_data, vec![signature]),
                    IotaTransactionBlockResponseOptions::new().with_object_changes(),
                    None,
                )
                .await
            {
                Ok(tx_response) => {
                    println!("Merging tx: {:?}", tx_response.digest);
                    sleep(Duration::from_millis(2000)).await;
                }
                Err(err) => println!("Err: {err}"),
            };
        }

        request_tokens_from_faucet(address).await?;
        sleep(Duration::from_millis(100)).await;
    };

    let spam_start = Instant::now();

    let mut tasks = Vec::new();
    let error_count = Arc::new(AtomicU32::new(0));
    let arced_keystore = Arc::new(keystore);

    let txs_per_task = TX_AMOUNT / PARALLEL_TASKS;
    for (task, coin) in coins_with_enough_balance.into_iter().enumerate() {
        if task >= PARALLEL_TASKS {
            break;
        }
        let client = client.clone();
        let keystore = arced_keystore.clone();
        let error_count = error_count.clone();
        tasks.push(async move {
            tokio::task::spawn(async move {
                let mut object_ref = coin.object_ref();

                for i in 0..txs_per_task {
                    let tx_data = TransactionData::new_transfer_iota(
                        address, address, None, // Some(1_000_000),
                        object_ref, GAS_BUDGET, gas_price,
                    );

                    let signature =
                        keystore.sign_secure(&address, &tx_data, Intent::iota_transaction())?;

                    match client
                        .quorum_driver_api()
                        .execute_transaction_block(
                            Transaction::from_data(tx_data, vec![signature]),
                            IotaTransactionBlockResponseOptions::new().with_object_changes(),
                            None,
                        )
                        .await
                    {
                        Ok(tx_response) => {
                            // Only print every 30th task and every 50th tx to not spam the console
                            if task % 30 == 0 && i % 50 == 0 {
                                println!(
                                    "Task {task} tx {i}/{txs_per_task}: {:?}",
                                    tx_response.digest
                                );
                            }
                            object_ref = tx_response
                                .object_changes
                                .expect("missing object changes")
                                .first()
                                .expect("missing created object")
                                .object_ref();
                        }
                        Err(err) => {
                            error_count.fetch_add(1, Ordering::SeqCst);
                            println!("Err: {err}");
                            // Error message when coin has not enough gas
                            if err.to_string().contains("lower than the needed amount") {
                                let remaining_rounds = txs_per_task - i - 1;
                                println!("Skipping remaining {remaining_rounds} rounds because {coin:?} is out of gas");
                                error_count
                                    .fetch_add((remaining_rounds) as u32, Ordering::SeqCst);
                                // Without enough gas, no need to try sending a tx again
                                break;
                            }
                        }
                    };
                }
                Ok(())
            })
            .await
        });
    }
    let results: Vec<Result<_, anyhow::Error>> = future::try_join_all(tasks).await?;
    let _ = results.into_iter().collect::<Result<Vec<_>, _>>()?;

    let spam_duration = spam_start.elapsed();
    println!(
        "Spamming {TX_AMOUNT} txs with {PARALLEL_TASKS} tasks took {:?} ({} tps), errors: {error_count:?}",
        spam_duration,
        TX_AMOUNT as u64 / spam_duration.as_secs()
    );

    Ok(())
}

/// Request tokens from the Faucet for the given address
pub async fn request_tokens_from_faucet(address: IotaAddress) -> Result<(), anyhow::Error> {
    let address_str = address.to_string();
    let json_body = json![{
        "FixedAmountRequest": {
            "recipient": &address_str
        }
    }];

    let resp = Client::new()
        .post(FAUCET_URL)
        .header("Content-Type", "application/json")
        .json(&json_body)
        .send()
        .await?;

    println!("{:?}", resp.text().await?);

    Ok(())
}
