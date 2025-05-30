// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, BTreeSet, HashMap};

use async_graphql::{connection::Connection, dataloader::Loader, *};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use fastcrypto::encoding::{Base58, Encoding};
use iota_indexer::{models::epoch::QueryableEpochInfo, schema::epochs};
use iota_types::messages_checkpoint::CheckpointCommitment as EpochCommitment;

use crate::{
    connection::ScanConnection,
    context_data::db_data_provider::PgManager,
    data::{DataLoader, Db, DbConnection, QueryExecutor},
    error::Error,
    server::watermark_task::Watermark,
    types::{
        big_int::BigInt,
        checkpoint::{self, Checkpoint},
        cursor::Page,
        date_time::DateTime,
        protocol_config::ProtocolConfigs,
        system_state_summary::{NativeStateValidatorInfo, SystemStateSummary},
        transaction_block::{self, TransactionBlock, TransactionBlockFilter},
        uint53::UInt53,
        validator_set::ValidatorSet,
    },
};

#[derive(Clone)]
pub(crate) struct Epoch {
    pub stored: QueryableEpochInfo,
    pub checkpoint_viewed_at: u64,
}

/// `DataLoader` key for fetching an `Epoch` by its ID, optionally constrained
/// by a consistency cursor.
#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
struct EpochKey {
    pub epoch_id: u64,
    pub checkpoint_viewed_at: u64,
}

/// Operation of the IOTA network is temporally partitioned into non-overlapping
/// epochs, and the network aims to keep epochs roughly the same duration as
/// each other. During a particular epoch the following data is fixed:
///
/// - the protocol version
/// - the reference gas price
/// - the set of participating validators
#[Object]
impl Epoch {
    /// The epoch's id as a sequence number that starts at 0 and is incremented
    /// by one at every epoch change.
    async fn epoch_id(&self) -> UInt53 {
        UInt53::from(self.stored.epoch as u64)
    }

    /// The minimum gas price that a quorum of validators are guaranteed to sign
    /// a transaction for.
    async fn reference_gas_price(&self) -> Option<BigInt> {
        Some(BigInt::from(self.stored.reference_gas_price as u64))
    }

    /// Validator related properties, including the active validators.
    async fn validator_set(&self, ctx: &Context<'_>) -> Result<Option<ValidatorSet>> {
        let system_state = ctx
            .data_unchecked::<PgManager>()
            .fetch_iota_system_state(Some(self.stored.epoch as u64))
            .await?;

        let validator_set = NativeStateValidatorInfo::from(system_state).into_validator_set(
            self.stored.total_stake as u64,
            self.checkpoint_viewed_at,
            self.stored.epoch as u64,
        );
        Ok(Some(validator_set))
    }

    /// The epoch's starting timestamp.
    async fn start_timestamp(&self) -> Result<DateTime, Error> {
        DateTime::from_ms(self.stored.epoch_start_timestamp)
    }

    /// The epoch's ending timestamp.
    async fn end_timestamp(&self) -> Result<Option<DateTime>, Error> {
        self.stored
            .epoch_end_timestamp
            .map(DateTime::from_ms)
            .transpose()
    }

    /// The total number of checkpoints in this epoch.
    async fn total_checkpoints(&self, ctx: &Context<'_>) -> Result<Option<UInt53>> {
        let last = match self.stored.last_checkpoint_id {
            Some(last) => last as u64,
            None => {
                let Watermark { checkpoint, .. } = *ctx.data_unchecked();
                checkpoint
            }
        };

        Ok(Some(UInt53::from(
            last - self.stored.first_checkpoint_id as u64,
        )))
    }

    /// The total number of transaction blocks in this epoch.
    async fn total_transactions(&self) -> Result<Option<UInt53>> {
        // TODO: this currently returns None for the current epoch. Fix this.
        Ok(self
            .stored
            .epoch_total_transactions
            .map(|v| UInt53::from(v as u64)))
    }

    /// The total amount of gas fees (in NANOS) that were paid in this epoch.
    async fn total_gas_fees(&self) -> Option<BigInt> {
        self.stored.total_gas_fees.map(BigInt::from)
    }

    /// The total NANOS rewarded as stake.
    async fn total_stake_rewards(&self) -> Option<BigInt> {
        self.stored
            .total_stake_rewards_distributed
            .map(BigInt::from)
    }

    /// The storage fund available in this epoch.
    /// This fund is used to redistribute storage fees from past transactions
    /// to future validators.
    async fn fund_size(&self) -> Option<BigInt> {
        Some(BigInt::from(self.stored.storage_fund_balance))
    }

    /// The difference between the fund inflow and outflow, representing
    /// the net amount of storage fees accumulated in this epoch.
    async fn net_inflow(&self) -> Option<BigInt> {
        if let (Some(fund_inflow), Some(fund_outflow)) =
            (self.stored.storage_charge, self.stored.storage_rebate)
        {
            Some(BigInt::from(fund_inflow - fund_outflow))
        } else {
            None
        }
    }

    /// The storage fees paid for transactions executed during the epoch.
    async fn fund_inflow(&self) -> Option<BigInt> {
        self.stored.storage_charge.map(BigInt::from)
    }

    /// The storage fee rebates paid to users who deleted the data associated
    /// with past transactions.
    async fn fund_outflow(&self) -> Option<BigInt> {
        self.stored.storage_rebate.map(BigInt::from)
    }

    /// The epoch's corresponding protocol configuration, including the feature
    /// flags and the configuration options.
    async fn protocol_configs(&self, ctx: &Context<'_>) -> Result<ProtocolConfigs> {
        ProtocolConfigs::query(ctx.data_unchecked(), Some(self.protocol_version()))
            .await
            .extend()
    }

    #[graphql(flatten)]
    async fn system_state_summary(&self, ctx: &Context<'_>) -> Result<SystemStateSummary> {
        let state = ctx
            .data_unchecked::<PgManager>()
            .fetch_iota_system_state(Some(self.stored.epoch as u64))
            .await?;
        Ok(SystemStateSummary { native: state })
    }

    /// A commitment by the committee at the end of epoch on the contents of the
    /// live object set at that time. This can be used to verify state
    /// snapshots.
    async fn live_object_set_digest(&self) -> Result<Option<String>> {
        let Some(commitments) = self.stored.epoch_commitments.as_ref() else {
            return Ok(None);
        };
        let commitments: Vec<EpochCommitment> = bcs::from_bytes(commitments).map_err(|e| {
            Error::Internal(format!("Error deserializing commitments: {e}")).extend()
        })?;

        let digest = commitments.into_iter().next().map(|commitment| {
            let EpochCommitment::ECMHLiveObjectSetDigest(digest) = commitment;
            Base58::encode(digest.digest.into_inner())
        });

        Ok(digest)
    }

    /// The epoch's corresponding checkpoints.
    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<checkpoint::Cursor>,
        last: Option<u64>,
        before: Option<checkpoint::Cursor>,
    ) -> Result<Connection<String, Checkpoint>> {
        let page = Page::from_params(ctx.data_unchecked(), first, after, last, before)?;
        let epoch = self.stored.epoch as u64;
        Checkpoint::paginate(
            ctx.data_unchecked(),
            page,
            Some(epoch),
            self.checkpoint_viewed_at,
        )
        .await
        .extend()
    }

    /// The epoch's corresponding transaction blocks.
    ///
    /// `scanLimit` restricts the number of candidate transactions scanned when
    /// gathering a page of results. It is required for queries that apply
    /// more than two complex filters (on function, kind, sender, recipient,
    /// input object, changed object, or ids), and can be at most
    /// `serviceConfig.maxScanLimit`.
    ///
    /// When the scan limit is reached the page will be returned even if it has
    /// fewer than `first` results when paginating forward (`last` when
    /// paginating backwards). If there are more transactions to scan,
    /// `pageInfo.hasNextPage` (or `pageInfo.hasPreviousPage`) will be set to
    /// `true`, and `PageInfo.endCursor` (or `PageInfo.startCursor`) will be set
    /// to the last transaction that was scanned as opposed to the last (or
    /// first) transaction in the page.
    ///
    /// Requesting the next (or previous) page after this cursor will resume the
    /// search, scanning the next `scanLimit` many transactions in the
    /// direction of pagination, and so on until all transactions in the
    /// scanning range have been visited.
    ///
    /// By default, the scanning range consists of all transactions in this
    /// epoch.
    async fn transaction_blocks(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<transaction_block::Cursor>,
        last: Option<u64>,
        before: Option<transaction_block::Cursor>,
        filter: Option<TransactionBlockFilter>,
        scan_limit: Option<u64>,
    ) -> Result<ScanConnection<String, TransactionBlock>> {
        let page = Page::from_params(ctx.data_unchecked(), first, after, last, before)?;

        let Some(filter) = filter
            .unwrap_or_default()
            .intersect(TransactionBlockFilter {
                // If `first_checkpoint_id` is 0, we include the 0th checkpoint by leaving it None
                after_checkpoint: (self.stored.first_checkpoint_id > 0)
                    .then(|| UInt53::from(self.stored.first_checkpoint_id as u64 - 1)),
                before_checkpoint: self
                    .stored
                    .last_checkpoint_id
                    .map(|id| UInt53::from(id as u64 + 1)),
                ..Default::default()
            })
        else {
            return Ok(ScanConnection::new(false, false));
        };

        TransactionBlock::paginate(ctx, page, filter, self.checkpoint_viewed_at, scan_limit)
            .await
            .extend()
    }
}

impl Epoch {
    /// The epoch's protocol version.
    pub(crate) fn protocol_version(&self) -> u64 {
        self.stored.protocol_version as u64
    }

    /// Look up an `Epoch` in the database, optionally filtered by its Epoch ID.
    /// If no ID is supplied, defaults to fetching the latest epoch.
    pub(crate) async fn query(
        ctx: &Context<'_>,
        filter: Option<u64>,
        checkpoint_viewed_at: u64,
    ) -> Result<Option<Self>, Error> {
        if let Some(epoch_id) = filter {
            let DataLoader(dl) = ctx.data_unchecked();
            dl.load_one(EpochKey {
                epoch_id,
                checkpoint_viewed_at,
            })
            .await
        } else {
            Self::query_latest_at(ctx.data_unchecked(), checkpoint_viewed_at).await
        }
    }

    /// Look up the latest `Epoch` from the database, optionally filtered by a
    /// consistency cursor (querying for a consistency cursor in the past
    /// looks for the latest epoch as of that cursor).
    pub(crate) async fn query_latest_at(
        db: &Db,
        checkpoint_viewed_at: u64,
    ) -> Result<Option<Self>, Error> {
        use epochs::dsl;

        let stored: Option<QueryableEpochInfo> = db
            .execute(move |conn| {
                conn.first(move || {
                    // Bound the query on `checkpoint_viewed_at` by filtering for the epoch
                    // whose `first_checkpoint_id <= checkpoint_viewed_at`, selecting the epoch
                    // with the largest `first_checkpoint_id` among the filtered set.
                    dsl::epochs
                        .select(QueryableEpochInfo::as_select())
                        .filter(dsl::first_checkpoint_id.le(checkpoint_viewed_at as i64))
                        .order_by(dsl::first_checkpoint_id.desc())
                })
                .optional()
            })
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch epoch: {e}")))?;

        Ok(stored.map(|stored| Epoch {
            stored,
            checkpoint_viewed_at,
        }))
    }
}

impl Loader<EpochKey> for Db {
    type Value = Epoch;
    type Error = Error;

    async fn load(&self, keys: &[EpochKey]) -> Result<HashMap<EpochKey, Epoch>, Error> {
        use epochs::dsl;

        let epoch_ids: BTreeSet<_> = keys.iter().map(|key| key.epoch_id as i64).collect();
        let epochs: Vec<QueryableEpochInfo> = self
            .execute_repeatable(move |conn| {
                conn.results(move || {
                    dsl::epochs
                        .select(QueryableEpochInfo::as_select())
                        .filter(dsl::epoch.eq_any(epoch_ids.iter().cloned()))
                })
            })
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch epochs: {e}")))?;

        let epoch_id_to_stored: BTreeMap<_, _> = epochs
            .into_iter()
            .map(|stored| (stored.epoch as u64, stored))
            .collect();

        Ok(keys
            .iter()
            .filter_map(|key| {
                let stored = epoch_id_to_stored.get(&key.epoch_id).cloned()?;
                let epoch = Epoch {
                    stored,
                    checkpoint_viewed_at: key.checkpoint_viewed_at,
                };

                // We filter by checkpoint viewed at in memory because it should be quite rare
                // that this query actually filters something (only in edge
                // cases), and not trying to encode it in the SQL query makes
                // the query much simpler and therefore easier for
                // the DB to plan.
                let start = epoch.stored.first_checkpoint_id as u64;
                (key.checkpoint_viewed_at >= start).then_some((*key, epoch))
            })
            .collect())
    }
}
