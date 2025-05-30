// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use async_graphql::{
    connection::{Connection, CursorType, Edge},
    *,
};

use crate::{
    consistency::ConsistentIndexCursor,
    types::{
        big_int::BigInt,
        cursor::{JsonCursor, Page},
        iota_address::IotaAddress,
        validator::Validator,
    },
};

/// Representation of `0x3::validator_set::ValidatorSet`.
#[derive(Clone, Debug, SimpleObject, Default)]
#[graphql(complex)]
pub(crate) struct ValidatorSet {
    /// Total amount of stake for all active validators at the beginning of the
    /// epoch.
    pub total_stake: Option<BigInt>,

    #[graphql(skip)]
    /// The current list of active validators.
    pub active_validators: Option<Vec<Validator>>,

    /// Validators that are pending removal from the active validator set,
    /// expressed as indices in to `activeValidators`.
    pub pending_removals: Option<Vec<u64>>,

    // TODO: instead of returning the id and size of the table, potentially return the table
    // itself, paginated.
    /// Object ID of the wrapped object `TableVec` storing the pending active
    /// validators.
    pub pending_active_validators_id: Option<IotaAddress>,

    /// Size of the pending active validators table.
    pub pending_active_validators_size: Option<u64>,

    /// Object ID of the `Table` storing the mapping from staking pool ids to
    /// the addresses of the corresponding validators. This is needed
    /// because a validator's address can potentially change but the object
    /// ID of its pool will not.
    pub staking_pool_mappings_id: Option<IotaAddress>,

    /// Size of the stake pool mappings `Table`.
    pub staking_pool_mappings_size: Option<u64>,

    /// Object ID of the `Table` storing the inactive staking pools.
    pub inactive_pools_id: Option<IotaAddress>,

    /// Size of the inactive pools `Table`.
    pub inactive_pools_size: Option<u64>,

    /// Object ID of the `Table` storing the validator candidates.
    pub validator_candidates_id: Option<IotaAddress>,

    /// Size of the validator candidates `Table`.
    pub validator_candidates_size: Option<u64>,

    #[graphql(skip)]
    pub checkpoint_viewed_at: u64,

    #[graphql(skip)]
    /// The current list of committee_members.
    pub committee_members: Option<Vec<Validator>>,
}

type CValidator = JsonCursor<ConsistentIndexCursor>;

#[ComplexObject]
impl ValidatorSet {
    /// The current set of active validators.
    async fn active_validators(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        before: Option<CValidator>,
        last: Option<u64>,
        after: Option<CValidator>,
    ) -> Result<Connection<String, Validator>> {
        let page = Page::from_params(ctx.data_unchecked(), first, after, last, before)?;

        let mut connection = Connection::new(false, false);
        let Some(validators) = &self.active_validators else {
            return Ok(connection);
        };

        let Some((prev, next, _, cs)) =
            page.paginate_consistent_indices(validators.len(), self.checkpoint_viewed_at)?
        else {
            return Ok(connection);
        };

        connection.has_previous_page = prev;
        connection.has_next_page = next;

        for c in cs {
            let mut validator = validators[c.ix].clone();
            validator.checkpoint_viewed_at = c.c;
            connection
                .edges
                .push(Edge::new(c.encode_cursor(), validator));
        }

        Ok(connection)
    }

    /// The current set of committee members.
    async fn committee_members(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        before: Option<CValidator>,
        last: Option<u64>,
        after: Option<CValidator>,
    ) -> Result<Connection<String, Validator>> {
        let page = Page::from_params(ctx.data_unchecked(), first, after, last, before)?;

        let mut connection = Connection::new(false, false);
        let Some(validators) = &self.committee_members else {
            return Ok(connection);
        };

        let Some((prev, next, _, cs)) =
            page.paginate_consistent_indices(validators.len(), self.checkpoint_viewed_at)?
        else {
            return Ok(connection);
        };

        connection.has_previous_page = prev;
        connection.has_next_page = next;

        for c in cs {
            let mut validator = validators[c.ix].clone();
            validator.checkpoint_viewed_at = c.c;
            connection
                .edges
                .push(Edge::new(c.encode_cursor(), validator));
        }

        Ok(connection)
    }
}
