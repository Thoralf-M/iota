// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use async_graphql::*;
use fastcrypto::{
    encoding::{Base64, Encoding},
    traits::ToFromBytes,
};
use iota_json_rpc_types::IotaTransactionBlockResponseOptions;
use iota_sdk::IotaClient;
use iota_types::{
    effects::TransactionEffects as NativeTransactionEffects,
    event::Event as NativeEvent,
    quorum_driver_types::ExecuteTransactionRequestType,
    signature::GenericSignature,
    transaction::{SenderSignedData, Transaction},
};

use crate::{
    error::Error,
    types::{
        execution_result::ExecutionResult,
        transaction_block_effects::{TransactionBlockEffects, TransactionBlockEffectsKind},
    },
};
pub struct Mutation;

/// Mutations are used to write to the IOTA network.
#[Object]
impl Mutation {
    /// Execute a transaction, committing its effects on chain.
    ///
    /// - `txBytes` is a `TransactionData` struct that has been BCS-encoded and
    ///   then Base64-encoded.
    /// - `signatures` are a list of `flag || signature || pubkey` bytes,
    ///   Base64-encoded.
    ///
    /// Waits until the transaction has reached finality on chain to return its
    /// transaction digest, or returns the error that prevented finality if
    /// that was not possible. A transaction is final when its effects are
    /// guaranteed on chain (it cannot be revoked).
    ///
    /// There may be a delay between transaction finality and when GraphQL
    /// requests (including the request that issued the transaction) reflect
    /// its effects. As a result, queries that depend on indexing the state
    /// of the chain (e.g. contents of output objects, address-level balance
    /// information at the time of the transaction), must wait for indexing to
    /// catch up by polling for the transaction digest using
    /// `Query.transactionBlock`.
    async fn execute_transaction_block(
        &self,
        ctx: &Context<'_>,
        tx_bytes: String,
        signatures: Vec<String>,
    ) -> Result<ExecutionResult> {
        let iota_sdk_client: &Option<IotaClient> = ctx
            .data()
            .map_err(|_| Error::Internal("Unable to fetch IOTA SDK client".to_string()))
            .extend()?;
        let iota_sdk_client = iota_sdk_client
            .as_ref()
            .ok_or_else(|| Error::Internal("IOTA SDK client not initialized".to_string()))
            .extend()?;
        let tx_data = bcs::from_bytes(
            &Base64::decode(&tx_bytes)
                .map_err(|e| {
                    Error::Client(format!(
                        "Unable to deserialize transaction bytes from Base64: {e}"
                    ))
                })
                .extend()?,
        )
        .map_err(|e| {
            Error::Client(format!(
                "Unable to deserialize transaction bytes as BCS: {e}"
            ))
        })
        .extend()?;

        let mut sigs = Vec::new();
        for sig in signatures {
            sigs.push(
                GenericSignature::from_bytes(
                    &Base64::decode(&sig)
                        .map_err(|e| {
                            Error::Client(format!(
                                "Unable to deserialize signature bytes {sig} from Base64: {e}"
                            ))
                        })
                        .extend()?,
                )
                .map_err(|e| Error::Client(format!("Unable to create signature from bytes: {e}")))
                .extend()?,
            );
        }
        let transaction = Transaction::from_generic_sig_data(tx_data, sigs);
        let options = IotaTransactionBlockResponseOptions::new()
            .with_events()
            .with_raw_input()
            .with_raw_effects();

        let result = iota_sdk_client
            .quorum_driver_api()
            .execute_transaction_block(
                transaction,
                options,
                Some(ExecuteTransactionRequestType::WaitForEffectsCert),
            )
            .await
            // TODO: use proper error type as this could be a client error or internal error
            // depending on the specific error returned
            .map_err(|e| Error::Internal(format!("Unable to execute transaction: {e}")))
            .extend()?;

        let native: NativeTransactionEffects = bcs::from_bytes(&result.raw_effects)
            .map_err(|e| Error::Internal(format!("Unable to deserialize transaction effects: {e}")))
            .extend()?;
        let tx_data: SenderSignedData = bcs::from_bytes(&result.raw_transaction)
            .map_err(|e| Error::Internal(format!("Unable to deserialize transaction data: {e}")))
            .extend()?;

        let events = result
            .events
            .ok_or_else(|| {
                Error::Internal("No events are returned from transaction execution".to_string())
            })?
            .data
            .into_iter()
            .map(|e| NativeEvent {
                package_id: e.package_id,
                transaction_module: e.transaction_module,
                sender: e.sender,
                type_: e.type_,
                contents: e.bcs.into_bytes(),
            })
            .collect();

        Ok(ExecutionResult {
            errors: if result.errors.is_empty() {
                None
            } else {
                Some(result.errors)
            },
            effects: TransactionBlockEffects {
                kind: TransactionBlockEffectsKind::Executed {
                    tx_data,
                    native,
                    events,
                },
                // set to u64::MAX, as the executed transaction has not been indexed yet
                checkpoint_viewed_at: u64::MAX,
            },
        })
    }
}
