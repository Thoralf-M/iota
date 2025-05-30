// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use iota_json_rpc_types::{
    Checkpoint, CheckpointId, CheckpointPage, IotaEvent, IotaGetPastObjectRequest,
    IotaObjectDataOptions, IotaObjectResponse, IotaPastObjectResponse,
    IotaTransactionBlockResponse, IotaTransactionBlockResponseOptions, ProtocolConfigResponse,
};
use iota_open_rpc_macros::open_rpc;
use iota_types::{
    base_types::{ObjectID, SequenceNumber, TransactionDigest},
    iota_serde::BigInt,
};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// Provides methods for reading transaction related data such as transaction
/// blocks, checkpoints, and protocol configuration. The trait further provides
/// methods for reading the ledger (current objects) as well its history (past
/// objects).
#[open_rpc(namespace = "iota", tag = "Read API")]
#[rpc(server, client, namespace = "iota")]
pub trait ReadApi {
    /// Return the transaction response object.
    #[rustfmt::skip]
    #[method(name = "getTransactionBlock")]
    async fn get_transaction_block(
        &self,
        /// the digest of the queried transaction
        digest: TransactionDigest,
        /// options for specifying the content to be returned
        options: Option<IotaTransactionBlockResponseOptions>,
    ) -> RpcResult<IotaTransactionBlockResponse>;

    /// Returns an ordered list of transaction responses
    /// The method will throw an error if the input contains any duplicate or
    /// the input size exceeds QUERY_MAX_RESULT_LIMIT
    #[rustfmt::skip]
    #[method(name = "multiGetTransactionBlocks")]
    async fn multi_get_transaction_blocks(
        &self,
        /// A list of transaction digests.
        digests: Vec<TransactionDigest>,
        /// config options to control which fields to fetch
        options: Option<IotaTransactionBlockResponseOptions>,
    ) -> RpcResult<Vec<IotaTransactionBlockResponse>>;

    /// Return the object information for a specified object
    #[rustfmt::skip]
    #[method(name = "getObject")]
    async fn get_object(
        &self,
        /// the ID of the queried object
        object_id: ObjectID,
        /// options for specifying the content to be returned
        options: Option<IotaObjectDataOptions>,
    ) -> RpcResult<IotaObjectResponse>;

    /// Return the object data for a list of objects
    #[rustfmt::skip]
    #[method(name = "multiGetObjects")]
    async fn multi_get_objects(
        &self,
        /// the IDs of the queried objects
        object_ids: Vec<ObjectID>,
        /// options for specifying the content to be returned
        options: Option<IotaObjectDataOptions>,
    ) -> RpcResult<Vec<IotaObjectResponse>>;

    /// Note there is no software-level guarantee/SLA that objects with past
    /// versions can be retrieved by this API, even if the object and version
    /// exists/existed. The result may vary across nodes depending on their
    /// pruning policies. Return the object information for a specified version
    #[rustfmt::skip]
    #[method(name = "tryGetPastObject")]
    async fn try_get_past_object(
        &self,
        /// the ID of the queried object
        object_id: ObjectID,
        /// the version of the queried object. If None, default to the latest known version
        version: SequenceNumber,
        /// options for specifying the content to be returned
        options: Option<IotaObjectDataOptions>,
    ) -> RpcResult<IotaPastObjectResponse>;

    /// Note there is no software-level guarantee/SLA that objects with past
    /// versions can be retrieved by this API, even if the object and
    /// version exists/existed. The result may vary across nodes depending
    /// on their pruning policies. Returns the latest object information
    /// with a version less than or equal to the given version
    // Note that this endpoint is used by iota replay tool. Also the
    // implementation in `iota-json-rpc` uses internally the
    // `AuthorityState::find_object_lt_or_eq_version` method, which has
    // underlying utility, e.g., `RemoteFetcher::get_child_object` uses
    // `try_get_object_before_version` to get the object with the versions <=
    // the given version. We have the `deprecated` flag here to not expose it in
    // the generated spec file, and it should be only for internal usage.
    #[method(name = "tryGetObjectBeforeVersion", deprecated = "true")]
    async fn try_get_object_before_version(
        &self,
        /// the ID of the queried object
        object_id: ObjectID,
        /// the version of the queried object
        version: SequenceNumber,
    ) -> RpcResult<IotaPastObjectResponse>;

    /// Note there is no software-level guarantee/SLA that objects with past
    /// versions can be retrieved by this API, even if the object and version
    /// exists/existed. The result may vary across nodes depending on their
    /// pruning policies. Return the object information for a specified version
    #[rustfmt::skip]
    #[method(name = "tryMultiGetPastObjects")]
    async fn try_multi_get_past_objects(
        &self,
        /// a vector of object and versions to be queried
        past_objects: Vec<IotaGetPastObjectRequest>,
        /// options for specifying the content to be returned
        options: Option<IotaObjectDataOptions>,
    ) -> RpcResult<Vec<IotaPastObjectResponse>>;

    /// Return a checkpoint
    #[rustfmt::skip]
    #[method(name = "getCheckpoint")]
    async fn get_checkpoint(
        &self,
        /// Checkpoint identifier, can use either checkpoint digest, or checkpoint sequence number as input.
        id: CheckpointId,
    ) -> RpcResult<Checkpoint>;

    /// Return paginated list of checkpoints
    #[rustfmt::skip]
    #[method(name = "getCheckpoints")]
    async fn get_checkpoints(
        &self,
        /// An optional paging cursor. If provided, the query will start from the next item after the specified cursor. Default to start from the first item if not specified.
        cursor: Option<BigInt<u64>>,
        /// Maximum item returned per page, default to [QUERY_MAX_RESULT_LIMIT_CHECKPOINTS] if not specified.
        limit: Option<usize>,
        /// query result ordering, default to false (ascending order), oldest record first.
        descending_order: bool,
    ) -> RpcResult<CheckpointPage>;

    /// Return transaction events.
    #[method(name = "getEvents")]
    async fn get_events(
        &self,
        /// the event query criteria.
        transaction_digest: TransactionDigest,
    ) -> RpcResult<Vec<IotaEvent>>;

    /// Return the total number of transaction blocks known to the server.
    #[method(name = "getTotalTransactionBlocks")]
    async fn get_total_transaction_blocks(&self) -> RpcResult<BigInt<u64>>;

    /// Return the sequence number of the latest checkpoint that has been
    /// executed
    #[method(name = "getLatestCheckpointSequenceNumber")]
    async fn get_latest_checkpoint_sequence_number(&self) -> RpcResult<BigInt<u64>>;

    /// Return the protocol config table for the given version number.
    /// If the version number is not specified, If none is specified, the node uses
    /// the version of the latest epoch it has processed.
    #[rustfmt::skip]
    #[method(name = "getProtocolConfig")]
    async fn get_protocol_config(
        &self,
        /// An optional protocol version specifier. If omitted, the latest protocol config table for the node will be returned.
        version: Option<BigInt<u64>>,
    ) -> RpcResult<ProtocolConfigResponse>;

    /// Return the first four bytes of the chain's genesis checkpoint digest.
    #[method(name = "getChainIdentifier")]
    async fn get_chain_identifier(&self) -> RpcResult<String>;
}
