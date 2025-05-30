// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, ops::Not, str::FromStr, vec};

use anyhow::anyhow;
use iota_json_rpc_types::{
    BalanceChange, IotaArgument, IotaCallArg, IotaCommand, IotaProgrammableMoveCall,
    IotaProgrammableTransactionBlock,
};
use iota_sdk::rpc_types::{
    IotaTransactionBlockData, IotaTransactionBlockDataAPI, IotaTransactionBlockEffectsAPI,
    IotaTransactionBlockKind, IotaTransactionBlockResponse,
};
use iota_types::{
    IOTA_SYSTEM_ADDRESS, IOTA_SYSTEM_PACKAGE_ID,
    base_types::{IotaAddress, ObjectID, SequenceNumber},
    digests::TransactionDigest,
    gas_coin::{GAS, GasCoin},
    governance::{ADD_STAKE_FUN_NAME, WITHDRAW_STAKE_FUN_NAME},
    iota_system_state::IOTA_SYSTEM_MODULE_NAME,
    object::Owner,
    transaction::TransactionData,
};
use move_core_types::{
    ident_str,
    language_storage::{ModuleId, StructTag},
    resolver::ModuleResolver,
};
use serde::{Deserialize, Serialize};

use crate::{
    Error,
    types::{
        AccountIdentifier, Amount, CoinAction, CoinChange, CoinID, CoinIdentifier,
        InternalOperation, OperationIdentifier, OperationStatus, OperationType,
    },
};

#[cfg(test)]
#[path = "unit_tests/operations_tests.rs"]
mod operations_tests;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Operations(Vec<Operation>);

impl FromIterator<Operation> for Operations {
    fn from_iter<T: IntoIterator<Item = Operation>>(iter: T) -> Self {
        Operations::new(iter.into_iter().collect())
    }
}

impl FromIterator<Vec<Operation>> for Operations {
    fn from_iter<T: IntoIterator<Item = Vec<Operation>>>(iter: T) -> Self {
        iter.into_iter().flatten().collect()
    }
}

impl IntoIterator for Operations {
    type Item = Operation;
    type IntoIter = vec::IntoIter<Operation>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl Operations {
    pub fn new(mut ops: Vec<Operation>) -> Self {
        for (index, op) in ops.iter_mut().enumerate() {
            op.operation_identifier = (index as u64).into()
        }
        Self(ops)
    }

    pub fn contains(&self, other: &Operations) -> bool {
        for (i, other_op) in other.0.iter().enumerate() {
            if let Some(op) = self.0.get(i) {
                if op != other_op {
                    return false;
                }
            } else {
                return false;
            }
        }
        true
    }

    pub fn set_status(mut self, status: Option<OperationStatus>) -> Self {
        for op in &mut self.0 {
            op.status = status
        }
        self
    }

    pub fn type_(&self) -> Option<OperationType> {
        self.0.first().map(|op| op.type_)
    }

    /// Parse operation input from rosetta operation to intermediate internal
    /// operation;
    pub fn into_internal(self) -> Result<InternalOperation, Error> {
        let type_ = self
            .type_()
            .ok_or_else(|| Error::MissingInput("Operation type".into()))?;
        match type_ {
            OperationType::PayIota => self.pay_iota_ops_to_internal(),
            OperationType::Stake => self.stake_ops_to_internal(),
            OperationType::WithdrawStake => self.withdraw_stake_ops_to_internal(),
            op => Err(Error::UnsupportedOperation(op)),
        }
    }

    fn pay_iota_ops_to_internal(self) -> Result<InternalOperation, Error> {
        let mut recipients = vec![];
        let mut amounts = vec![];
        let mut sender = None;
        for op in self {
            if let (Some(amount), Some(account)) = (op.amount.clone(), op.account.clone()) {
                if amount.value.is_negative() {
                    sender = Some(account.address)
                } else {
                    recipients.push(account.address);
                    let amount = amount.value.abs();
                    if amount > u64::MAX as i128 {
                        return Err(Error::InvalidInput(
                            "Input amount exceed u64::MAX".to_string(),
                        ));
                    }
                    amounts.push(amount as u64)
                }
            }
        }
        let sender = sender.ok_or_else(|| Error::MissingInput("Sender address".to_string()))?;
        Ok(InternalOperation::PayIota {
            sender,
            recipients,
            amounts,
        })
    }

    fn stake_ops_to_internal(self) -> Result<InternalOperation, Error> {
        let mut ops = self
            .0
            .into_iter()
            .filter(|op| op.type_ == OperationType::Stake)
            .collect::<Vec<_>>();
        if ops.len() != 1 {
            return Err(Error::MalformedOperation(
                "Delegation should only have one operation.".into(),
            ));
        }
        // Checked above, safe to unwrap.
        let op = ops.pop().unwrap();
        let sender = op
            .account
            .ok_or_else(|| Error::MissingInput("Sender address".to_string()))?
            .address;
        let metadata = op
            .metadata
            .ok_or_else(|| Error::MissingInput("Stake metadata".to_string()))?;

        // Total issued IOTA is less than u64, safe to cast.
        let amount = if let Some(amount) = op.amount {
            if amount.value.is_positive() {
                return Err(Error::MalformedOperation(
                    "Stake amount should be negative.".into(),
                ));
            }
            Some(amount.value.unsigned_abs() as u64)
        } else {
            None
        };

        let OperationMetadata::Stake { validator } = metadata else {
            return Err(Error::InvalidInput(
                "Cannot find delegation info from metadata.".into(),
            ));
        };

        Ok(InternalOperation::Stake {
            sender,
            validator,
            amount,
        })
    }

    fn withdraw_stake_ops_to_internal(self) -> Result<InternalOperation, Error> {
        let mut ops = self
            .0
            .into_iter()
            .filter(|op| op.type_ == OperationType::WithdrawStake)
            .collect::<Vec<_>>();
        if ops.len() != 1 {
            return Err(Error::MalformedOperation(
                "Delegation should only have one operation.".into(),
            ));
        }
        // Checked above, safe to unwrap.
        let op = ops.pop().unwrap();
        let sender = op
            .account
            .ok_or_else(|| Error::MissingInput("Sender address".to_string()))?
            .address;

        let stake_ids = if let Some(metadata) = op.metadata {
            let OperationMetadata::WithdrawStake { stake_ids } = metadata else {
                return Err(Error::InvalidInput(
                    "Cannot find withdraw stake info from metadata.".into(),
                ));
            };
            stake_ids
        } else {
            vec![]
        };

        Ok(InternalOperation::WithdrawStake { sender, stake_ids })
    }

    fn from_transaction(
        tx: IotaTransactionBlockKind,
        sender: IotaAddress,
        status: Option<OperationStatus>,
    ) -> Result<Vec<Operation>, Error> {
        Ok(match tx {
            IotaTransactionBlockKind::ProgrammableTransaction(pt) => {
                Self::parse_programmable_transaction(sender, status, pt)?
            }
            _ => vec![Operation::generic_op(status, sender, tx)],
        })
    }

    pub fn from_transaction_data(
        data: TransactionData,
        digest: impl Into<Option<TransactionDigest>>,
    ) -> Result<Self, Error> {
        struct NoOpsModuleResolver;
        impl ModuleResolver for NoOpsModuleResolver {
            type Error = Error;
            fn get_module(&self, _id: &ModuleId) -> Result<Option<Vec<u8>>, Self::Error> {
                Ok(None)
            }
        }

        let digest = digest.into().unwrap_or_default();

        // Rosetta don't need the call args to be parsed into readable format
        IotaTransactionBlockData::try_from(data, &&mut NoOpsModuleResolver, digest)?.try_into()
    }

    fn parse_programmable_transaction(
        sender: IotaAddress,
        status: Option<OperationStatus>,
        pt: IotaProgrammableTransactionBlock,
    ) -> Result<Vec<Operation>, Error> {
        #[derive(Debug)]
        enum KnownValue {
            GasCoin(u64),
        }
        fn resolve_result(
            known_results: &[Vec<KnownValue>],
            i: u16,
            j: u16,
        ) -> Option<&KnownValue> {
            known_results
                .get(i as usize)
                .and_then(|inner| inner.get(j as usize))
        }
        fn split_coins(
            inputs: &[IotaCallArg],
            known_results: &[Vec<KnownValue>],
            coin: IotaArgument,
            amounts: &[IotaArgument],
        ) -> Option<Vec<KnownValue>> {
            match coin {
                IotaArgument::Result(i) => {
                    let KnownValue::GasCoin(_) = resolve_result(known_results, i, 0)?;
                }
                IotaArgument::NestedResult(i, j) => {
                    let KnownValue::GasCoin(_) = resolve_result(known_results, i, j)?;
                }
                IotaArgument::GasCoin => (),
                // Might not be an IOTA coin
                IotaArgument::Input(_) => return None,
            };
            let amounts = amounts
                .iter()
                .map(|amount| {
                    let value: u64 = match *amount {
                        IotaArgument::Input(i) => {
                            u64::from_str(inputs[i as usize].pure()?.to_json_value().as_str()?)
                                .ok()?
                        }
                        IotaArgument::GasCoin
                        | IotaArgument::Result(_)
                        | IotaArgument::NestedResult(_, _) => return None,
                    };
                    Some(KnownValue::GasCoin(value))
                })
                .collect::<Option<_>>()?;
            Some(amounts)
        }
        fn transfer_object(
            aggregated_recipients: &mut HashMap<IotaAddress, u64>,
            inputs: &[IotaCallArg],
            known_results: &[Vec<KnownValue>],
            objs: &[IotaArgument],
            recipient: IotaArgument,
        ) -> Option<Vec<KnownValue>> {
            let addr = match recipient {
                IotaArgument::Input(i) => inputs[i as usize].pure()?.to_iota_address().ok()?,
                IotaArgument::GasCoin
                | IotaArgument::Result(_)
                | IotaArgument::NestedResult(_, _) => return None,
            };
            for obj in objs {
                let value = match *obj {
                    IotaArgument::Result(i) => {
                        let KnownValue::GasCoin(value) = resolve_result(known_results, i, 0)?;
                        value
                    }
                    IotaArgument::NestedResult(i, j) => {
                        let KnownValue::GasCoin(value) = resolve_result(known_results, i, j)?;
                        value
                    }
                    IotaArgument::GasCoin | IotaArgument::Input(_) => return None,
                };
                let aggregate = aggregated_recipients.entry(addr).or_default();
                *aggregate += value;
            }
            Some(vec![])
        }
        fn stake_call(
            inputs: &[IotaCallArg],
            known_results: &[Vec<KnownValue>],
            call: &IotaProgrammableMoveCall,
        ) -> Result<Option<(Option<u64>, IotaAddress)>, Error> {
            let IotaProgrammableMoveCall { arguments, .. } = call;
            let (amount, validator) = match &arguments[..] {
                [_, coin, validator] => {
                    let amount = match coin {
                        IotaArgument::Result(i) => {
                            let KnownValue::GasCoin(value) = resolve_result(known_results, *i, 0)
                                .ok_or_else(|| {
                                anyhow!("Cannot resolve Gas coin value at Result({i})")
                            })?;
                            value
                        }
                        _ => return Ok(None),
                    };
                    let (some_amount, validator) = match validator {
                        // [WORKAROUND] - this is a hack to work out if the staking ops is for a
                        // selected amount or None amount (whole wallet). We
                        // use the position of the validator arg as a indicator of if the rosetta
                        // stake transaction is staking the whole wallet or
                        // not, if staking whole wallet, we have to omit the
                        // amount value in the final operation output.
                        IotaArgument::Input(i) => (
                            *i == 1,
                            inputs[*i as usize]
                                .pure()
                                .map(|v| v.to_iota_address())
                                .transpose(),
                        ),
                        _ => return Ok(None),
                    };
                    (some_amount.then_some(*amount), validator)
                }
                _ => Err(anyhow!(
                    "Error encountered when extracting arguments from move call, expecting 3 elements, got {}",
                    arguments.len()
                ))?,
            };
            Ok(validator.map(|v| v.map(|v| (amount, v)))?)
        }

        fn unstake_call(
            inputs: &[IotaCallArg],
            call: &IotaProgrammableMoveCall,
        ) -> Result<Option<ObjectID>, Error> {
            let IotaProgrammableMoveCall { arguments, .. } = call;
            let id = match &arguments[..] {
                [_, stake_id] => {
                    match stake_id {
                        IotaArgument::Input(i) => {
                            let id = inputs[*i as usize]
                                .object()
                                .ok_or_else(|| anyhow!("Cannot find stake id from input args."))?;
                            // [WORKAROUND] - this is a hack to work out if the withdraw stake ops
                            // is for a selected stake or None (all stakes).
                            // this hack is similar to the one in stake_call.
                            let some_id = i % 2 == 1;
                            some_id.then_some(id)
                        }
                        _ => return Ok(None),
                    }
                }
                _ => Err(anyhow!(
                    "Error encountered when extracting arguments from move call, expecting 3 elements, got {}",
                    arguments.len()
                ))?,
            };
            Ok(id.cloned())
        }
        let IotaProgrammableTransactionBlock { inputs, commands } = &pt;
        let mut known_results: Vec<Vec<KnownValue>> = vec![];
        let mut aggregated_recipients: HashMap<IotaAddress, u64> = HashMap::new();
        let mut needs_generic = false;
        let mut operations = vec![];
        let mut stake_ids = vec![];
        for command in commands {
            let result = match command {
                IotaCommand::SplitCoins(coin, amounts) => {
                    split_coins(inputs, &known_results, *coin, amounts)
                }
                IotaCommand::TransferObjects(objs, addr) => transfer_object(
                    &mut aggregated_recipients,
                    inputs,
                    &known_results,
                    objs,
                    *addr,
                ),
                IotaCommand::MoveCall(m) if Self::is_stake_call(m) => {
                    stake_call(inputs, &known_results, m)?.map(|(amount, validator)| {
                        let amount = amount.map(|amount| Amount::new(-(amount as i128)));
                        operations.push(Operation {
                            operation_identifier: Default::default(),
                            type_: OperationType::Stake,
                            status,
                            account: Some(sender.into()),
                            amount,
                            coin_change: None,
                            metadata: Some(OperationMetadata::Stake { validator }),
                        });
                        vec![]
                    })
                }
                IotaCommand::MoveCall(m) if Self::is_unstake_call(m) => {
                    let stake_id = unstake_call(inputs, m)?;
                    stake_ids.push(stake_id);
                    Some(vec![])
                }
                _ => None,
            };
            if let Some(result) = result {
                known_results.push(result)
            } else {
                needs_generic = true;
                break;
            }
        }

        if !needs_generic && !aggregated_recipients.is_empty() {
            let total_paid: u64 = aggregated_recipients.values().copied().sum();
            operations.extend(
                aggregated_recipients
                    .into_iter()
                    .map(|(recipient, amount)| {
                        Operation::pay_iota(status, recipient, amount.into())
                    }),
            );
            operations.push(Operation::pay_iota(status, sender, -(total_paid as i128)));
        } else if !stake_ids.is_empty() {
            let stake_ids = stake_ids.into_iter().flatten().collect::<Vec<_>>();
            let metadata = stake_ids
                .is_empty()
                .not()
                .then_some(OperationMetadata::WithdrawStake { stake_ids });
            operations.push(Operation {
                operation_identifier: Default::default(),
                type_: OperationType::WithdrawStake,
                status,
                account: Some(sender.into()),
                amount: None,
                coin_change: None,
                metadata,
            });
        } else if operations.is_empty() {
            operations.push(Operation::generic_op(
                status,
                sender,
                IotaTransactionBlockKind::ProgrammableTransaction(pt),
            ))
        }
        Ok(operations)
    }

    fn is_stake_call(tx: &IotaProgrammableMoveCall) -> bool {
        tx.package == IOTA_SYSTEM_PACKAGE_ID
            && tx.module == IOTA_SYSTEM_MODULE_NAME.as_str()
            && tx.function == ADD_STAKE_FUN_NAME.as_str()
    }

    fn is_unstake_call(tx: &IotaProgrammableMoveCall) -> bool {
        tx.package == IOTA_SYSTEM_PACKAGE_ID
            && tx.module == IOTA_SYSTEM_MODULE_NAME.as_str()
            && tx.function == WITHDRAW_STAKE_FUN_NAME.as_str()
    }

    fn process_balance_change(
        gas_owner: IotaAddress,
        gas_used: i128,
        balance_changes: &[BalanceChange],
        status: Option<OperationStatus>,
        balances: HashMap<IotaAddress, i128>,
    ) -> impl Iterator<Item = Operation> {
        let mut balances = balance_changes
            .iter()
            .fold(balances, |mut balances, balance_change| {
                // Rosetta only care about address owner
                if let Owner::AddressOwner(owner) = balance_change.owner {
                    if balance_change.coin_type == GAS::type_tag() {
                        *balances.entry(owner).or_default() += balance_change.amount;
                    }
                }
                balances
            });
        // separate gas from balances
        *balances.entry(gas_owner).or_default() -= gas_used;

        let balance_change = balances
            .into_iter()
            .filter(|(_, amount)| *amount != 0)
            .map(move |(addr, amount)| Operation::balance_change(status, addr, amount));

        let gas = if gas_used != 0 {
            vec![Operation::gas(gas_owner, gas_used)]
        } else {
            // Gas can be 0 for system tx
            vec![]
        };
        balance_change.chain(gas)
    }
}

impl TryFrom<IotaTransactionBlockData> for Operations {
    type Error = Error;
    fn try_from(data: IotaTransactionBlockData) -> Result<Self, Self::Error> {
        let sender = *data.sender();
        Ok(Self::new(Self::from_transaction(
            data.transaction().clone(),
            sender,
            None,
        )?))
    }
}

impl TryFrom<IotaTransactionBlockResponse> for Operations {
    type Error = Error;
    fn try_from(response: IotaTransactionBlockResponse) -> Result<Self, Self::Error> {
        let tx = response
            .transaction
            .ok_or_else(|| anyhow!("Response input should not be empty"))?;
        let sender = *tx.data.sender();
        let effect = response
            .effects
            .ok_or_else(|| anyhow!("Response effects should not be empty"))?;
        let gas_owner = effect.gas_object().owner.get_owner_address()?;
        let gas_summary = effect.gas_cost_summary();
        let gas_used = gas_summary.storage_rebate as i128
            - gas_summary.storage_cost as i128
            - gas_summary.computation_cost as i128;

        let status = Some(effect.into_status().into());
        let ops: Operations = tx.data.try_into()?;
        let ops = ops.set_status(status).into_iter();

        // We will need to subtract the operation amounts from the actual balance
        // change amount extracted from event to prevent double counting.
        let mut accounted_balances =
            ops.as_ref()
                .iter()
                .fold(HashMap::new(), |mut balances, op| {
                    if let (Some(acc), Some(amount), Some(OperationStatus::Success)) =
                        (&op.account, &op.amount, &op.status)
                    {
                        *balances.entry(acc.address).or_default() -= amount.value;
                    }
                    balances
                });

        let mut principal_amounts = 0;
        let mut reward_amounts = 0;
        // Extract balance change from unstake events

        if let Some(events) = response.events {
            for event in events.data {
                if is_unstake_event(&event.type_) {
                    let principal_amount = event
                        .parsed_json
                        .pointer("/principal_amount")
                        .and_then(|v| v.as_str())
                        .and_then(|v| i128::from_str(v).ok());
                    let reward_amount = event
                        .parsed_json
                        .pointer("/reward_amount")
                        .and_then(|v| v.as_str())
                        .and_then(|v| i128::from_str(v).ok());
                    if let (Some(principal_amount), Some(reward_amount)) =
                        (principal_amount, reward_amount)
                    {
                        principal_amounts += principal_amount;
                        reward_amounts += reward_amount;
                    }
                }
            }
        }
        let staking_balance = if principal_amounts != 0 {
            *accounted_balances.entry(sender).or_default() -= principal_amounts;
            *accounted_balances.entry(sender).or_default() -= reward_amounts;
            vec![
                Operation::stake_principle(status, sender, principal_amounts),
                Operation::stake_reward(status, sender, reward_amounts),
            ]
        } else {
            vec![]
        };

        // Extract coin change operations from balance changes
        let coin_change_operations = Self::process_balance_change(
            gas_owner,
            gas_used,
            &response
                .balance_changes
                .ok_or_else(|| anyhow!("Response balance changes should not be empty."))?,
            status,
            accounted_balances,
        );

        Ok(ops
            .into_iter()
            .chain(coin_change_operations)
            .chain(staking_balance)
            .collect())
    }
}

fn is_unstake_event(tag: &StructTag) -> bool {
    tag.address == IOTA_SYSTEM_ADDRESS
        && tag.module.as_ident_str() == ident_str!("validator")
        && tag.name.as_ident_str() == ident_str!("UnstakingRequestEvent")
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Operation {
    operation_identifier: OperationIdentifier,
    #[serde(rename = "type")]
    pub type_: OperationType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<OperationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountIdentifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<Amount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coin_change: Option<CoinChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

impl PartialEq for Operation {
    fn eq(&self, other: &Self) -> bool {
        self.operation_identifier == other.operation_identifier
            && self.type_ == other.type_
            && self.account == other.account
            && self.amount == other.amount
            && self.coin_change == other.coin_change
            && self.metadata == other.metadata
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum OperationMetadata {
    GenericTransaction(IotaTransactionBlockKind),
    Stake { validator: IotaAddress },
    WithdrawStake { stake_ids: Vec<ObjectID> },
}

impl Operation {
    fn generic_op(
        status: Option<OperationStatus>,
        sender: IotaAddress,
        tx: IotaTransactionBlockKind,
    ) -> Self {
        Operation {
            operation_identifier: Default::default(),
            type_: (&tx).into(),
            status,
            account: Some(sender.into()),
            amount: None,
            coin_change: None,
            metadata: Some(OperationMetadata::GenericTransaction(tx)),
        }
    }

    pub fn genesis(index: u64, sender: IotaAddress, coin: GasCoin) -> Self {
        Operation {
            operation_identifier: index.into(),
            type_: OperationType::Genesis,
            status: Some(OperationStatus::Success),
            account: Some(sender.into()),
            amount: Some(Amount::new(coin.value().into())),
            coin_change: Some(CoinChange {
                coin_identifier: CoinIdentifier {
                    identifier: CoinID {
                        id: *coin.id(),
                        version: SequenceNumber::new(),
                    },
                },
                coin_action: CoinAction::CoinCreated,
            }),
            metadata: None,
        }
    }

    fn pay_iota(status: Option<OperationStatus>, address: IotaAddress, amount: i128) -> Self {
        Operation {
            operation_identifier: Default::default(),
            type_: OperationType::PayIota,
            status,
            account: Some(address.into()),
            amount: Some(Amount::new(amount)),
            coin_change: None,
            metadata: None,
        }
    }

    fn balance_change(status: Option<OperationStatus>, addr: IotaAddress, amount: i128) -> Self {
        Self {
            operation_identifier: Default::default(),
            type_: OperationType::IotaBalanceChange,
            status,
            account: Some(addr.into()),
            amount: Some(Amount::new(amount)),
            coin_change: None,
            metadata: None,
        }
    }
    fn gas(addr: IotaAddress, amount: i128) -> Self {
        Self {
            operation_identifier: Default::default(),
            type_: OperationType::Gas,
            status: Some(OperationStatus::Success),
            account: Some(addr.into()),
            amount: Some(Amount::new(amount)),
            coin_change: None,
            metadata: None,
        }
    }
    fn stake_reward(status: Option<OperationStatus>, addr: IotaAddress, amount: i128) -> Self {
        Self {
            operation_identifier: Default::default(),
            type_: OperationType::StakeReward,
            status,
            account: Some(addr.into()),
            amount: Some(Amount::new(amount)),
            coin_change: None,
            metadata: None,
        }
    }
    fn stake_principle(status: Option<OperationStatus>, addr: IotaAddress, amount: i128) -> Self {
        Self {
            operation_identifier: Default::default(),
            type_: OperationType::StakePrinciple,
            status,
            account: Some(addr.into()),
            amount: Some(Amount::new(amount)),
            coin_change: None,
            metadata: None,
        }
    }
}
