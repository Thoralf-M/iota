// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use fastcrypto::encoding::Base64;
use iota_json_rpc_types::{
    DevInspectArgs, DevInspectResults, DryRunTransactionBlockResponse,
    IotaTransactionBlockResponse, IotaTransactionBlockResponseOptions,
};
use iota_open_rpc_macros::open_rpc;
use iota_types::{
    base_types::IotaAddress, iota_serde::BigInt, quorum_driver_types::ExecuteTransactionRequestType,
};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// Provides methods for executing and testing transactions.
#[open_rpc(namespace = "iota", tag = "Write API")]
#[rpc(server, client, namespace = "iota")]
pub trait WriteApi {
    /// Execute the transaction and wait for results if desired.
    /// Request types:
    /// 1. WaitForEffectsCert: waits for TransactionEffectsCert and then return to
    ///    client. This mode is a proxy for transaction finality.
    /// 2. WaitForLocalExecution: waits for TransactionEffectsCert and make sure the
    ///    node executed the transaction locally before returning the client. The
    ///    local execution makes sure this node is aware of this transaction when
    ///    client fires subsequent queries. However if the node fails to execute the
    ///    transaction locally in a timely manner, a bool type in the response is
    ///    set to false to indicated the case.
    /// request_type is default to be `WaitForEffectsCert` unless
    /// options.show_events or options.show_effects is true
    #[rustfmt::skip]
    #[method(name = "executeTransactionBlock")]
    async fn execute_transaction_block(
        &self,
        /// BCS serialized transaction data bytes without its type tag, as base-64 encoded string.
        tx_bytes: Base64,
        /// A list of signatures (`flag || signature || pubkey` bytes, as base-64 encoded string). Signature is committed to the intent message of the transaction data, as base-64 encoded string.
        signatures: Vec<Base64>,
        /// options for specifying the content to be returned
        options: Option<IotaTransactionBlockResponseOptions>,
        /// The request type, derived from `IotaTransactionBlockResponseOptions` if None
        request_type: Option<ExecuteTransactionRequestType>,
    ) -> RpcResult<IotaTransactionBlockResponse>;

    /// Runs the transaction in dev-inspect mode. Which allows for nearly any
    /// transaction (or Move call) with any arguments. Detailed results are
    /// provided, including both the transaction effects and any return values.
    #[rustfmt::skip]
    #[method(name = "devInspectTransactionBlock")]
    async fn dev_inspect_transaction_block(
        &self,
        sender_address: IotaAddress,
        /// BCS encoded TransactionKind(as opposed to TransactionData, which include gasBudget and gasPrice)
        tx_bytes: Base64,
        /// Gas is not charged, but gas usage is still calculated. Default to use reference gas price
        gas_price: Option<BigInt<u64>>,
        /// The epoch to perform the call. Will be set from the system state object if not provided
        epoch: Option<BigInt<u64>>,
        /// Additional arguments including gas_budget, gas_objects, gas_sponsor and skip_checks.
        additional_args: Option<DevInspectArgs>,
    ) -> RpcResult<DevInspectResults>;

    /// Return transaction execution effects including the gas cost summary,
    /// while the effects are not committed to the chain.
    #[method(name = "dryRunTransactionBlock")]
    async fn dry_run_transaction_block(
        &self,
        tx_bytes: Base64,
    ) -> RpcResult<DryRunTransactionBlockResponse>;
}
