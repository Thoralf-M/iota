// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, sync::Arc};

use futures::future::join_all;
use iota_types::{
    base_types::{IotaAddress, ObjectID, ObjectRef, dbg_addr},
    crypto::{AccountKeyPair, get_key_pair},
    effects::{SignedTransactionEffects, TransactionEffectsAPI},
    error::{IotaError, UserInputError},
    execution_status::{ExecutionFailureStatus, ExecutionStatus},
    gas_coin::GasCoin,
    object::Object,
    programmable_transaction_builder::ProgrammableTransactionBuilder,
    transaction::TransactionData,
    utils::to_sender_signed_transaction,
};

use crate::authority::{
    AuthorityState,
    authority_tests::{init_state_with_committee, send_and_confirm_transaction},
    test_authority_builder::TestAuthorityBuilder,
};

#[tokio::test]
async fn test_pay_iota_failure_empty_recipients() {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let coin_id = ObjectID::random();
    let coin1 = Object::with_id_owner_gas_for_testing(coin_id, sender, 2000000);

    // an empty set of programmable transaction commands will still charge gas
    let res = execute_pay_iota(vec![coin1], vec![], vec![], sender, sender_key, 2000000).await;

    let effects = res.txn_result.unwrap().into_data();
    assert_eq!(effects.status(), &ExecutionStatus::Success);
    assert_eq!(effects.mutated().len(), 1);
    assert_eq!(effects.mutated()[0].0.0, coin_id);
    assert!(effects.deleted().is_empty());
    assert!(effects.created().is_empty());
}

#[tokio::test]
async fn test_pay_iota_failure_insufficient_gas_balance_one_input_coin() {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let coin1 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 2000);
    let recipient1 = dbg_addr(1);
    let recipient2 = dbg_addr(2);

    let res = execute_pay_iota(
        vec![coin1],
        vec![recipient1, recipient2],
        vec![100, 100],
        sender,
        sender_key,
        2200000,
    )
    .await;

    assert_eq!(
        UserInputError::try_from(res.txn_result.unwrap_err()).unwrap(),
        UserInputError::GasBalanceTooLow {
            gas_balance: 2000,
            needed_gas_amount: 2200000,
        }
    );
}

#[tokio::test]
async fn test_pay_iota_failure_insufficient_total_balance_one_input_coin() {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let coin1 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 1000100);
    let recipient1 = dbg_addr(1);
    let recipient2 = dbg_addr(2);

    let res = execute_pay_iota(
        vec![coin1],
        vec![recipient1, recipient2],
        vec![100, 100],
        sender,
        sender_key,
        1000000,
    )
    .await;

    assert_eq!(
        res.txn_result.as_ref().unwrap().status(),
        &ExecutionStatus::Failure {
            error: ExecutionFailureStatus::InsufficientCoinBalance,
            command: Some(0) // SplitCoins is the first command in the implementation of pay
        },
    );
}

#[tokio::test]
async fn test_pay_iota_failure_insufficient_gas_balance_multiple_input_coins() {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let coin1 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 800);
    let coin2 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 700);
    let recipient1 = dbg_addr(1);
    let recipient2 = dbg_addr(2);

    let res = execute_pay_iota(
        vec![coin1, coin2],
        vec![recipient1, recipient2],
        vec![100, 100],
        sender,
        sender_key,
        2000000,
    )
    .await;

    assert_eq!(
        UserInputError::try_from(res.txn_result.unwrap_err()).unwrap(),
        UserInputError::GasBalanceTooLow {
            gas_balance: 1500,
            needed_gas_amount: 2000000,
        }
    );
}

#[tokio::test]
async fn test_pay_iota_failure_insufficient_total_balance_multiple_input_coins() {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let coin1 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 404000);
    let coin2 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 603000);
    let recipient1 = dbg_addr(1);
    let recipient2 = dbg_addr(2);

    let res = execute_pay_iota(
        vec![coin1, coin2],
        vec![recipient1, recipient2],
        vec![8000, 8000],
        sender,
        sender_key,
        1000000,
    )
    .await;
    assert_eq!(
        res.txn_result.as_ref().unwrap().status(),
        &ExecutionStatus::Failure {
            error: ExecutionFailureStatus::InsufficientCoinBalance,
            command: Some(0) // SplitCoins is the first command in the implementation of pay
        },
    );
}

#[tokio::test]
async fn test_pay_iota_success_one_input_coin() -> anyhow::Result<()> {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let object_id = ObjectID::random();
    let coin_amount = 50000000;
    let coin_obj = Object::with_id_owner_gas_for_testing(object_id, sender, 50000000);
    let recipient1 = dbg_addr(1);
    let recipient2 = dbg_addr(2);
    let recipient3 = dbg_addr(3);
    let recipient_amount_map: HashMap<_, u64> =
        HashMap::from([(recipient1, 100), (recipient2, 200), (recipient3, 300)]);
    let res = execute_pay_iota(
        vec![coin_obj],
        vec![recipient1, recipient2, recipient3],
        vec![100, 200, 300],
        sender,
        sender_key,
        coin_amount - 300 - 200 - 100,
    )
    .await;

    let effects = res.txn_result.unwrap().into_data();
    assert_eq!(*effects.status(), ExecutionStatus::Success);
    // make sure each recipient receives the specified amount
    assert_eq!(effects.created().len(), 3);
    let created_obj_id1 = effects.created()[0].0.0;
    let created_obj_id2 = effects.created()[1].0.0;
    let created_obj_id3 = effects.created()[2].0.0;
    let created_obj1 = res
        .authority_state
        .get_object(&created_obj_id1)
        .await
        .unwrap()
        .unwrap();
    let created_obj2 = res
        .authority_state
        .get_object(&created_obj_id2)
        .await
        .unwrap()
        .unwrap();
    let created_obj3 = res
        .authority_state
        .get_object(&created_obj_id3)
        .await
        .unwrap()
        .unwrap();

    let addr1 = effects.created()[0].1.get_owner_address()?;
    let addr2 = effects.created()[1].1.get_owner_address()?;
    let addr3 = effects.created()[2].1.get_owner_address()?;
    let coin_val1 = *recipient_amount_map
        .get(&addr1)
        .ok_or(IotaError::InvalidAddress)?;
    let coin_val2 = *recipient_amount_map
        .get(&addr2)
        .ok_or(IotaError::InvalidAddress)?;
    let coin_val3 = *recipient_amount_map
        .get(&addr3)
        .ok_or(IotaError::InvalidAddress)?;
    assert_eq!(GasCoin::try_from(&created_obj1)?.value(), coin_val1);
    assert_eq!(GasCoin::try_from(&created_obj2)?.value(), coin_val2);
    assert_eq!(GasCoin::try_from(&created_obj3)?.value(), coin_val3);

    // make sure the first object still belongs to the sender,
    // the value is equal to all residual values after amounts transferred and gas
    // payment.
    assert_eq!(effects.mutated()[0].0.0, object_id);
    assert_eq!(effects.mutated()[0].1, sender);
    let gas_used = effects.gas_cost_summary().net_gas_usage() as u64;
    let gas_object = res.authority_state.get_object(&object_id).await?.unwrap();
    assert_eq!(
        GasCoin::try_from(&gas_object)?.value(),
        coin_amount - 100 - 200 - 300 - gas_used,
    );

    Ok(())
}

#[tokio::test]
async fn test_pay_iota_success_multiple_input_coins() -> anyhow::Result<()> {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let object_id1 = ObjectID::random();
    let object_id2 = ObjectID::random();
    let object_id3 = ObjectID::random();
    let coin_obj1 = Object::with_id_owner_gas_for_testing(object_id1, sender, 5000000);
    let coin_obj2 = Object::with_id_owner_gas_for_testing(object_id2, sender, 1000);
    let coin_obj3 = Object::with_id_owner_gas_for_testing(object_id3, sender, 1000);
    let recipient1 = dbg_addr(1);
    let recipient2 = dbg_addr(2);

    let res = execute_pay_iota(
        vec![coin_obj1, coin_obj2, coin_obj3],
        vec![recipient1, recipient2],
        vec![500, 1500],
        sender,
        sender_key,
        5000000,
    )
    .await;
    let recipient_amount_map: HashMap<_, u64> =
        HashMap::from([(recipient1, 500), (recipient2, 1500)]);
    let effects = res.txn_result.unwrap().into_data();
    assert_eq!(*effects.status(), ExecutionStatus::Success);

    // make sure each recipient receives the specified amount
    assert_eq!(effects.created().len(), 2);
    let created_obj_id1 = effects.created()[0].0.0;
    let created_obj_id2 = effects.created()[1].0.0;
    let created_obj1 = res
        .authority_state
        .get_object(&created_obj_id1)
        .await
        .unwrap()
        .unwrap();
    let created_obj2 = res
        .authority_state
        .get_object(&created_obj_id2)
        .await
        .unwrap()
        .unwrap();
    let addr1 = effects.created()[0].1.get_owner_address()?;
    let addr2 = effects.created()[1].1.get_owner_address()?;
    let coin_val1 = *recipient_amount_map
        .get(&addr1)
        .ok_or(IotaError::InvalidAddress)?;
    let coin_val2 = *recipient_amount_map
        .get(&addr2)
        .ok_or(IotaError::InvalidAddress)?;
    assert_eq!(GasCoin::try_from(&created_obj1)?.value(), coin_val1);
    assert_eq!(GasCoin::try_from(&created_obj2)?.value(), coin_val2);
    // make sure the first input coin still belongs to the sender,
    // the value is equal to all residual values after amounts transferred and gas
    // payment.
    assert_eq!(effects.mutated()[0].0.0, object_id1);
    assert_eq!(effects.mutated()[0].1, sender);
    let gas_used = effects.gas_cost_summary().net_gas_usage() as u64;
    let gas_object = res.authority_state.get_object(&object_id1).await?.unwrap();
    assert_eq!(
        GasCoin::try_from(&gas_object)?.value(),
        5002000 - 500 - 1500 - gas_used,
    );

    // make sure the second and third input coins are deleted
    let deleted_ids: Vec<ObjectID> = effects.deleted().iter().map(|d| d.0).collect();
    assert!(deleted_ids.contains(&object_id2));
    assert!(deleted_ids.contains(&object_id3));
    Ok(())
}

#[tokio::test]
async fn test_pay_all_iota_failure_insufficient_gas_one_input_coin() {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let coin1 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 1800);
    let recipient = dbg_addr(2);

    let res = execute_pay_all_iota(vec![&coin1], recipient, sender, sender_key, 2000000).await;

    assert_eq!(
        UserInputError::try_from(res.txn_result.unwrap_err()).unwrap(),
        UserInputError::GasBalanceTooLow {
            gas_balance: 1800,
            needed_gas_amount: 2000000,
        }
    );
}

#[tokio::test]
async fn test_pay_all_iota_failure_insufficient_gas_budget_multiple_input_coins() {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let coin1 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 1000);
    let coin2 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 1000);
    let recipient = dbg_addr(2);
    let res =
        execute_pay_all_iota(vec![&coin1, &coin2], recipient, sender, sender_key, 2500000).await;

    assert_eq!(
        UserInputError::try_from(res.txn_result.unwrap_err()).unwrap(),
        UserInputError::GasBalanceTooLow {
            gas_balance: 2000,
            needed_gas_amount: 2500000,
        }
    );
}

#[tokio::test]
async fn test_pay_all_iota_success_one_input_coin() -> anyhow::Result<()> {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let object_id = ObjectID::random();
    let coin_obj = Object::with_id_owner_gas_for_testing(object_id, sender, 3000000);
    let recipient = dbg_addr(2);
    let res = execute_pay_all_iota(vec![&coin_obj], recipient, sender, sender_key, 2000000).await;

    let effects = res.txn_result.unwrap().into_data();
    assert_eq!(*effects.status(), ExecutionStatus::Success);

    // make sure the first object now belongs to the recipient,
    // the value is equal to all residual values after gas payment.
    let obj_ref = &effects.mutated()[0].0;
    assert_eq!(obj_ref.0, object_id);
    assert_eq!(effects.mutated()[0].1, recipient);

    let gas_used = effects.gas_cost_summary().gas_used();
    let gas_object = res.authority_state.get_object(&object_id).await?.unwrap();
    assert_eq!(GasCoin::try_from(&gas_object)?.value(), 3000000 - gas_used,);
    Ok(())
}

#[tokio::test]
async fn test_pay_all_iota_success_multiple_input_coins() -> anyhow::Result<()> {
    let (sender, sender_key): (_, AccountKeyPair) = get_key_pair();
    let object_id1 = ObjectID::random();
    let coin_obj1 = Object::with_id_owner_gas_for_testing(object_id1, sender, 3000000);
    let coin_obj2 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 1000);
    let coin_obj3 = Object::with_id_owner_gas_for_testing(ObjectID::random(), sender, 1000);
    let recipient = dbg_addr(2);
    let res = execute_pay_all_iota(
        vec![&coin_obj1, &coin_obj2, &coin_obj3],
        recipient,
        sender,
        sender_key,
        3000000,
    )
    .await;

    let effects = res.txn_result.unwrap().into_data();
    assert_eq!(*effects.status(), ExecutionStatus::Success);

    // make sure the first object now belongs to the recipient,
    // the value is equal to all residual values after gas payment.
    let obj_ref = &effects.mutated()[0].0;
    assert_eq!(obj_ref.0, object_id1);
    assert_eq!(effects.mutated()[0].1, recipient);

    let gas_used = effects.gas_cost_summary().gas_used();
    let gas_object = res.authority_state.get_object(&object_id1).await?.unwrap();
    assert_eq!(GasCoin::try_from(&gas_object)?.value(), 3002000 - gas_used,);
    Ok(())
}

struct PayIotaTransactionBlockExecutionResult {
    pub authority_state: Arc<AuthorityState>,
    pub txn_result: Result<SignedTransactionEffects, IotaError>,
}

async fn execute_pay_iota(
    input_coin_objects: Vec<Object>,
    recipients: Vec<IotaAddress>,
    amounts: Vec<u64>,
    sender: IotaAddress,
    sender_key: AccountKeyPair,
    gas_budget: u64,
) -> PayIotaTransactionBlockExecutionResult {
    let authority_state = TestAuthorityBuilder::new().build().await;

    let input_coin_refs: Vec<ObjectRef> = input_coin_objects
        .iter()
        .map(|coin_obj| coin_obj.compute_object_reference())
        .collect();
    let handles: Vec<_> = input_coin_objects
        .into_iter()
        .map(|obj| authority_state.insert_genesis_object(obj))
        .collect();
    join_all(handles).await;
    let rgp = authority_state.reference_gas_price_for_testing().unwrap();

    let mut builder = ProgrammableTransactionBuilder::new();
    builder.pay_iota(recipients, amounts).unwrap();
    let pt = builder.finish();
    let data = TransactionData::new_programmable(sender, input_coin_refs, pt, gas_budget, rgp);
    let tx = to_sender_signed_transaction(data, &sender_key);
    let txn_result = send_and_confirm_transaction(&authority_state, tx)
        .await
        .map(|(_, effects)| effects);

    PayIotaTransactionBlockExecutionResult {
        authority_state,
        txn_result,
    }
}

async fn execute_pay_all_iota(
    input_coin_objects: Vec<&Object>,
    recipient: IotaAddress,
    sender: IotaAddress,
    sender_key: AccountKeyPair,
    gas_budget: u64,
) -> PayIotaTransactionBlockExecutionResult {
    let dir = tempfile::TempDir::new().unwrap();
    let network_config = iota_swarm_config::network_config_builder::ConfigBuilder::new(&dir)
        .with_reference_gas_price(700)
        .with_objects(
            input_coin_objects
                .clone()
                .into_iter()
                .map(ToOwned::to_owned),
        )
        .build();
    let genesis = network_config.genesis;
    let keypair = network_config.validator_configs[0].authority_key_pair();

    let authority_state = init_state_with_committee(&genesis, keypair).await;
    let rgp = authority_state.reference_gas_price_for_testing().unwrap();

    let mut input_coins = Vec::new();
    for coin in input_coin_objects {
        let id = coin.id();
        let object_ref = genesis
            .objects()
            .iter()
            .find(|o| o.id() == id)
            .unwrap()
            .compute_object_reference();
        input_coins.push(object_ref);
    }

    let mut builder = ProgrammableTransactionBuilder::new();
    builder.pay_all_iota(recipient);
    let pt = builder.finish();
    let data = TransactionData::new_programmable(sender, input_coins, pt, gas_budget, rgp);
    let tx = to_sender_signed_transaction(data, &sender_key);
    let txn_result = send_and_confirm_transaction(&authority_state, tx)
        .await
        .map(|(_, effects)| effects);
    PayIotaTransactionBlockExecutionResult {
        authority_state,
        txn_result,
    }
}
