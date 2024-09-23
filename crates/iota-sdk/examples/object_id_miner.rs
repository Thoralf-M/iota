// Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! This example shows how to mine an object id with specific starting bytes.
//!
//! cargo run -r --example object_id_miner

mod utils;

use iota_sdk::IotaClient;
use iota_types::{
    base_types::{IotaAddress, ObjectID, ObjectRef},
    crypto::default_hash,
    digests::TransactionDigest,
    programmable_transaction_builder::ProgrammableTransactionBuilder,
    transaction::{
        GasData, TransactionData, TransactionDataV1, TransactionExpiration, TransactionKind,
    },
};
use shared_crypto::intent::{AppId, Intent, IntentMessage, IntentScope, IntentVersion};
use utils::{setup_for_write, sign_and_execute_transaction};

const NUMBER_OF_EQUAL_START_BYTES: usize = 3;
const PARALLEL_TASKS: usize = 14;
// Time with 14 tasks on my machine: ~21 seconds
const TOTAL_MINING_ROUNDS: usize = 1_000_000_000;
// Increase this by TOTAL_MINING_ROUNDS when running again, to check new ids
const MINING_OFFSET_ROUNDS: usize = 0;
// Set to true, to actually create a mined object, only the first will work
const SEND_TRANSACTION: bool = false;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, sender, _recipient) = setup_for_write().await?;

    let coins = client
        .coin_read_api()
        .get_coins(sender, None, None, None)
        .await?;
    let gas_coin = coins
        .data
        .into_iter()
        // Gas coin should have enough IOTA
        .find(|c| c.balance >= 2_000_000_000)
        .unwrap();
    let gas_coin_object_ref = gas_coin.object_ref();

    let gas_budget = 10_000_000;
    let gas_price = client.read_api().get_reference_gas_price().await?;

    let start = std::time::Instant::now();

    let amount = TOTAL_MINING_ROUNDS / PARALLEL_TASKS;

    let mut tasks = Vec::new();

    for i in 0..PARALLEL_TASKS {
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            if i == PARALLEL_TASKS - 1 {
                mine(
                    MINING_OFFSET_ROUNDS + i * amount,
                    MINING_OFFSET_ROUNDS
                        + ((i + 1) * amount)
                        + TOTAL_MINING_ROUNDS % PARALLEL_TASKS,
                    sender,
                    gas_budget,
                    gas_price,
                    gas_coin_object_ref,
                    client,
                )
                .await
            } else {
                mine(
                    MINING_OFFSET_ROUNDS + i * amount,
                    MINING_OFFSET_ROUNDS + (i + 1) * amount,
                    sender,
                    gas_budget,
                    gas_price,
                    gas_coin_object_ref,
                    client,
                )
                .await
            }
        }));
    }

    let results = futures::future::try_join_all(tasks).await?;

    for res in results {
        res?;
    }
    println!("Time to run: {:?}", start.elapsed());

    Ok(())
}

async fn mine(
    start: usize,
    end: usize,
    sender: IotaAddress,
    gas_budget: u64,
    gas_price: u64,
    gas_coin_object_ref: ObjectRef,
    client: IotaClient,
) -> anyhow::Result<()> {
    'outer: for i in start..end {
        let programmable_transaction = {
            let mut builder = ProgrammableTransactionBuilder::new();
            builder.pure(i)?;
            builder.pay_iota(vec![sender], vec![1_000_000_000]).unwrap();
            builder.finish()
        };

        let transaction = TransactionData::V1(TransactionDataV1 {
            kind: TransactionKind::ProgrammableTransaction(programmable_transaction.clone()),
            sender,
            gas_data: GasData {
                payment: vec![gas_coin_object_ref],
                owner: sender,
                price: gas_price,
                budget: gas_budget,
            },
            expiration: TransactionExpiration::None,
        });

        let intent_msg = IntentMessage::new(
            Intent {
                version: IntentVersion::V0,
                scope: IntentScope::TransactionData,
                app_id: AppId::Iota,
            },
            transaction,
        );
        let transaction_digest = TransactionDigest::new(default_hash(&intent_msg.value));

        let precomputed_id = ObjectID::derive_id(transaction_digest, 0);

        for i in 0..NUMBER_OF_EQUAL_START_BYTES {
            if precomputed_id[i] != 0 {
                continue 'outer;
            }
        }
        println!("{i}: {}", precomputed_id);

        if SEND_TRANSACTION {
            let tx_data = TransactionData::new_programmable(
                sender,
                vec![gas_coin_object_ref],
                programmable_transaction,
                gas_budget,
                gas_price,
            );
            let transaction_response =
                sign_and_execute_transaction(&client, &sender, tx_data).await?;
            println!("{:?}", transaction_response);
            return Ok(());
        }
    }
    Ok(())
}
