// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

module iota_system::validator_set;

use iota::bag::{Self, Bag};
use iota::balance::Balance;
use iota::event;
use iota::iota::IOTA;
use iota::priority_queue as pq;
use iota::table::{Self, Table};
use iota::table_vec::{Self, TableVec};
use iota::vec_map::{Self, VecMap};
use iota::vec_set::{Self, VecSet};
use iota_system::staking_pool::{PoolTokenExchangeRate, StakedIota, pool_id};
use iota_system::validator::{ValidatorV1, staking_pool_id, iota_address};
use iota_system::validator_cap::{Self, UnverifiedValidatorOperationCap, ValidatorOperationCap};
use iota_system::validator_wrapper::{Self, Validator};
use iota_system::voting_power;

public struct ValidatorSetV1 has store {
    /// Total amount of stake from all active validators at the beginning of the epoch.
    total_stake: u64,
    /// The current list of active validators.
    active_validators: vector<ValidatorV1>,
    /// List of new validator candidates added during the current epoch.
    /// They will be processed at the end of the epoch.
    pending_active_validators: TableVec<ValidatorV1>,
    /// Removal requests from the validators. Each element is an index
    /// pointing to `active_validators`.
    pending_removals: vector<u64>,
    /// Mappings from staking pool's ID to the iota address of a validator.
    staking_pool_mappings: Table<ID, address>,
    /// Mapping from a staking pool ID to the inactive validator that has that pool as its staking pool.
    /// When a validator is deactivated the validator is removed from `active_validators` it
    /// is added to this table so that stakers can continue to withdraw their stake from it.
    inactive_validators: Table<ID, Validator>,
    /// Table storing preactive/candidate validators, mapping their addresses to their `ValidatorV1 ` structs.
    /// When an address calls `request_add_validator_candidate`, they get added to this table and become a preactive
    /// validator.
    /// When the candidate has met the min stake requirement, they can call `request_add_validator` to
    /// officially add them to the active validator set `active_validators` next epoch.
    validator_candidates: Table<address, Validator>,
    /// Table storing the number of epochs during which a validator's stake has been below the low stake threshold.
    at_risk_validators: VecMap<address, u64>,
    /// Any extra fields that's not defined statically.
    extra_fields: Bag,
}

/// The second version of the struct storing information about validator set.
/// This version is an extension on the first one, that supports a new approach to committee selection,
/// where committee members taking part in consensus are selected from a set of `active_validators`
/// before an epoch begins. `committee_members` is a vector of indices of validators stored in `active_validators`,
/// that have been selected to take part in consensus during the current epoch.
public struct ValidatorSetV2 has store {
    /// Total amount of stake from all committee validators at the beginning of the epoch.
    total_stake: u64,
    /// The current list of active validators.
    active_validators: vector<ValidatorV1>,
    /// Subset of validators responsible for consensus. Each element is an index
    /// pointing to `active_validators`.
    committee_members: vector<u64>,
    /// List of new validator candidates added during the current epoch.
    /// They will be processed at the end of the epoch.
    pending_active_validators: TableVec<ValidatorV1>,
    /// Removal requests from the validators. Each element is an index
    /// pointing to `active_validators`.
    pending_removals: vector<u64>,
    /// Mappings from staking pool's ID to the iota address of a validator.
    staking_pool_mappings: Table<ID, address>,
    /// Mapping from a staking pool ID to the inactive validator that has that pool as its staking pool.
    /// When a validator is deactivated the validator is removed from `active_validators` it
    /// is added to this table so that stakers can continue to withdraw their stake from it.
    inactive_validators: Table<ID, Validator>,
    /// Table storing preactive/candidate validators, mapping their addresses to their `ValidatorV1 ` structs.
    /// When an address calls `request_add_validator_candidate`, they get added to this table and become a preactive
    /// validator.
    /// When the candidate has met the min stake requirement, they can call `request_add_validator` to
    /// officially add them to the active validator set `active_validators` next epoch.
    validator_candidates: Table<address, Validator>,
    /// Table storing the number of epochs during which a validator's stake has been below the low stake threshold.
    at_risk_validators: VecMap<address, u64>,
    /// Any extra fields that's not defined statically.
    extra_fields: Bag,
}

#[allow(unused_field)]
/// Event containing staking and rewards related information of
/// each validator, emitted during epoch advancement.
public struct ValidatorEpochInfoEventV1 has copy, drop {
    epoch: u64,
    validator_address: address,
    reference_gas_survey_quote: u64,
    stake: u64,
    voting_power: u64,
    commission_rate: u64,
    pool_staking_reward: u64,
    pool_token_exchange_rate: PoolTokenExchangeRate,
    tallying_rule_reporters: vector<address>,
    tallying_rule_global_score: u64,
}

/// Event emitted every time a new validator becomes active.
/// The epoch value corresponds to the first epoch this change takes place.
public struct ValidatorJoinEvent has copy, drop {
    epoch: u64,
    validator_address: address,
    staking_pool_id: ID,
}

/// Event emitted every time a validator leaves the active validator set.
/// The epoch value corresponds to the first epoch this change takes place.
public struct ValidatorLeaveEvent has copy, drop {
    epoch: u64,
    validator_address: address,
    staking_pool_id: ID,
    is_voluntary: bool,
}

/// Event emitted every time a new validator becomes part of the committee.
/// The epoch value corresponds to the first epoch this change takes place.
public struct CommitteeValidatorJoinEvent has copy, drop {
    epoch: u64,
    validator_address: address,
    staking_pool_id: ID,
}

/// Event emitted every time a validator leaves the committee at the end of the epoch.
/// The epoch value corresponds to the first epoch this change takes place.
public struct CommitteeValidatorLeaveEvent has copy, drop {
    epoch: u64,
    validator_address: address,
    staking_pool_id: ID,
}

// same as in iota_system
const COMMITTEE_VALIDATOR_ONLY: u8 = 1;
const ACTIVE_OR_PENDING_VALIDATOR: u8 = 2;
const ANY_VALIDATOR: u8 = 3;

const BASIS_POINT_DENOMINATOR: u128 = 10000;
const MIN_STAKING_THRESHOLD: u64 = 1_000_000_000; // 1 IOTA

// Errors
const ENonValidatorInReportRecords: u64 = 0;
#[allow(unused_const)]
const EInvalidStakeAdjustmentAmount: u64 = 1;
const EDuplicateValidator: u64 = 2;
const ENoPoolFound: u64 = 3;
const ENotAValidator: u64 = 4;
const EMinJoiningStakeNotReached: u64 = 5;
const EAlreadyValidatorCandidate: u64 = 6;
const EValidatorNotCandidate: u64 = 7;
const ENotValidatorCandidate: u64 = 8;
const ENotActiveOrPendingValidator: u64 = 9;
const EStakingBelowThreshold: u64 = 10;
const EValidatorAlreadyRemoved: u64 = 11;
const ENotAPendingValidator: u64 = 12;
const EValidatorSetEmpty: u64 = 13;
const ENotACommitteeValidator: u64 = 14;

const EInvalidCap: u64 = 101;

// ==== initialization at genesis ====

public(package) fun new_v1(
    init_active_validators: vector<ValidatorV1>,
    ctx: &mut TxContext,
): ValidatorSetV1 {
    let total_stake = calculate_total_active_stakes(&init_active_validators);
    let mut staking_pool_mappings = table::new(ctx);
    let num_validators = init_active_validators.length();
    let mut i = 0;
    while (i < num_validators) {
        let validator = &init_active_validators[i];
        staking_pool_mappings.add(staking_pool_id(validator), iota_address(validator));
        i = i + 1;
    };
    let mut validators = ValidatorSetV1 {
        total_stake,
        active_validators: init_active_validators,
        pending_active_validators: table_vec::empty(ctx),
        pending_removals: vector[],
        staking_pool_mappings,
        inactive_validators: table::new(ctx),
        validator_candidates: table::new(ctx),
        at_risk_validators: vec_map::empty(),
        extra_fields: bag::new(ctx),
    };
    let validators_num = validators.active_validators.length();
    let committee_of_all_validators = vector::tabulate!(validators_num, |i| i);
    voting_power::set_voting_power(&committee_of_all_validators, &mut validators.active_validators);
    validators
}

#[test_only]
public(package) fun new_v2(
    init_active_validators: vector<ValidatorV1>,
    committee_size: u64,
    ctx: &mut TxContext,
): ValidatorSetV2 {
    let mut staking_pool_mappings = table::new(ctx);
    let num_validators = init_active_validators.length();
    let mut i = 0;
    while (i < num_validators) {
        let validator = &init_active_validators[i];
        staking_pool_mappings.add(staking_pool_id(validator), iota_address(validator));
        i = i + 1;
    };
    let mut validators = ValidatorSetV2 {
        total_stake: 0,
        active_validators: init_active_validators,
        committee_members: vector[],
        pending_active_validators: table_vec::empty(ctx),
        pending_removals: vector[],
        staking_pool_mappings,
        inactive_validators: table::new(ctx),
        validator_candidates: table::new(ctx),
        at_risk_validators: vec_map::empty(),
        extra_fields: bag::new(ctx),
    };

    // Only assign new committee, no need to call `process_new_committee` which also emits events.
    validators.committee_members =
        validators.select_committee_members_top_n_stakers(committee_size);

    validators.total_stake =
        calculate_total_committee_stakes(
            &validators.active_validators,
            &validators.committee_members,
        );
    voting_power::set_voting_power(
        &validators.committee_members,
        &mut validators.active_validators,
    );

    validators
}

public(package) fun v1_to_v2(self: ValidatorSetV1): ValidatorSetV2 {
    let ValidatorSetV1 {
        total_stake,
        active_validators,
        pending_active_validators,
        pending_removals,
        staking_pool_mappings,
        inactive_validators,
        validator_candidates,
        at_risk_validators,
        extra_fields,
    } = self;
    let committee_members = vector::tabulate!(active_validators.length(), |i| i);

    let validators = ValidatorSetV2 {
        total_stake,
        active_validators,
        committee_members,
        pending_active_validators,
        pending_removals,
        staking_pool_mappings,
        inactive_validators,
        validator_candidates,
        at_risk_validators,
        extra_fields,
    };

    validators
}

// ==== functions to add or remove validators ====

/// Called by `iota_system` to add a new validator candidate.
public(package) fun request_add_validator_candidate(
    self: &mut ValidatorSetV2,
    validator: ValidatorV1,
    ctx: &mut TxContext,
) {
    // The next assertions are not critical for the protocol, but they are here to catch problematic configs earlier.
    assert!(
        !is_duplicate_with_active_validator(self, &validator)
                && !is_duplicate_with_pending_validator(self, &validator),
        EDuplicateValidator,
    );
    let validator_address = iota_address(&validator);
    assert!(!self.validator_candidates.contains(validator_address), EAlreadyValidatorCandidate);

    assert!(validator.is_preactive(), EValidatorNotCandidate);
    // Add validator to the candidates mapping and the pool id mappings so that users can start
    // staking with this candidate.
    self.staking_pool_mappings.add(staking_pool_id(&validator), validator_address);
    self
        .validator_candidates
        .add(
            iota_address(&validator),
            validator_wrapper::create_v1(validator, ctx),
        );
}

/// Called by `iota_system` to remove a validator candidate, and move them to `inactive_validators`.
public(package) fun request_remove_validator_candidate(
    self: &mut ValidatorSetV2,
    ctx: &mut TxContext,
) {
    let validator_address = ctx.sender();
    assert!(self.validator_candidates.contains(validator_address), ENotValidatorCandidate);
    let wrapper = self.validator_candidates.remove(validator_address);
    let mut validator = wrapper.destroy();
    assert!(validator.is_preactive(), EValidatorNotCandidate);

    let staking_pool_id = staking_pool_id(&validator);

    // Remove the validator's staking pool from mappings.
    self.staking_pool_mappings.remove(staking_pool_id);

    // Deactivate the staking pool.
    validator.deactivate(ctx.epoch());

    // Add to the inactive tables.
    self
        .inactive_validators
        .add(
            staking_pool_id,
            validator_wrapper::create_v1(validator, ctx),
        );
}

/// Called by `iota_system` to add a new validator to `pending_active_validators`, which will be
/// processed at the end of epoch.
public(package) fun request_add_validator(
    self: &mut ValidatorSetV2,
    min_joining_stake_amount: u64,
    ctx: &TxContext,
) {
    let validator_address = ctx.sender();
    assert!(self.validator_candidates.contains(validator_address), ENotValidatorCandidate);
    let wrapper = self.validator_candidates.remove(validator_address);
    let validator = wrapper.destroy();
    assert!(
        !is_duplicate_with_active_validator(self, &validator)
                && !is_duplicate_with_pending_validator(self, &validator),
        EDuplicateValidator,
    );
    assert!(validator.is_preactive(), EValidatorNotCandidate);
    assert!(validator.total_stake_amount() >= min_joining_stake_amount, EMinJoiningStakeNotReached);

    self.pending_active_validators.push_back(validator);
}

public(package) fun assert_no_pending_or_active_duplicates(
    self: &ValidatorSetV2,
    validator: &ValidatorV1,
) {
    // Validator here must be active or pending, and thus must be identified as duplicate exactly once.
    assert!(
        count_duplicates_vec(&self.active_validators, validator) +
                count_duplicates_tablevec(&self.pending_active_validators, validator) == 1,
        EDuplicateValidator,
    );
}

/// Called by `iota_system`, to remove a validator.
/// The index of the validator is added to `pending_removals` and
/// will be processed at the end of epoch.
/// Only an active validator can request to be removed.
public(package) fun request_remove_validator(self: &mut ValidatorSetV2, ctx: &TxContext) {
    let validator_address = ctx.sender();
    let mut validator_index_opt = find_validator(&self.active_validators, validator_address);
    assert!(validator_index_opt.is_some(), ENotAValidator);
    let validator_index = validator_index_opt.extract();
    assert!(!self.pending_removals.contains(&validator_index), EValidatorAlreadyRemoved);
    self.pending_removals.push_back(validator_index);
}

// ==== staking related functions ====

/// Called by `iota_system`, to add a new stake to the validator.
/// This request is added to the validator's staking pool's pending stake entries, processed at the end
/// of the epoch.
/// Aborts in case the staking amount is smaller than MIN_STAKING_THRESHOLD
public(package) fun request_add_stake(
    self: &mut ValidatorSetV2,
    validator_address: address,
    stake: Balance<IOTA>,
    ctx: &mut TxContext,
): StakedIota {
    let iota_amount = stake.value();
    assert!(iota_amount >= MIN_STAKING_THRESHOLD, EStakingBelowThreshold);
    let validator = get_candidate_or_active_validator_mut(self, validator_address);
    validator.request_add_stake(stake, ctx.sender(), ctx)
}

/// Called by `iota_system`, to withdraw some share of a stake from the validator. The share to withdraw
/// is denoted by `principal_withdraw_amount`. One of two things occurs in this function:
/// 1. If the `staked_iota` is staked with an active validator, the request is added to the validator's
///    staking pool's pending stake withdraw entries, processed at the end of the epoch.
/// 2. If the `staked_iota` was staked with a validator that is no longer active,
///    the stake and any rewards corresponding to it will be immediately processed.
public(package) fun request_withdraw_stake(
    self: &mut ValidatorSetV2,
    staked_iota: StakedIota,
    ctx: &TxContext,
): Balance<IOTA> {
    let staking_pool_id = pool_id(&staked_iota);
    let validator = if (self.staking_pool_mappings.contains(staking_pool_id)) {
        // This is an active validator.
        let validator_address = self.staking_pool_mappings[pool_id(&staked_iota)];
        get_candidate_or_active_validator_mut(self, validator_address)
    } else {
        // This is an inactive pool.
        assert!(self.inactive_validators.contains(staking_pool_id), ENoPoolFound);
        let wrapper = &mut self.inactive_validators[staking_pool_id];
        wrapper.load_validator_maybe_upgrade()
    };
    validator.request_withdraw_stake(staked_iota, ctx)
}

// ==== validator config setting functions ====

public(package) fun request_set_commission_rate(
    self: &mut ValidatorSetV2,
    new_commission_rate: u64,
    ctx: &TxContext,
) {
    let validator_address = ctx.sender();
    let validator = get_validator_mut(&mut self.active_validators, validator_address);
    validator.request_set_commission_rate(new_commission_rate);
}

// ==== epoch change functions ====

/// Update the validator set at the end of epoch.
/// It does the following things:
///   1. Distribute stake award.
///   2. Process pending stake deposits and withdraws for each validator (`adjust_stake`).
///   3. Process pending stake deposits, and withdraws.
///   4. Process pending validator application and withdraws.
///   5. At the end, we calculate the total stake for the new epoch.
public(package) fun advance_epoch(
    self: &mut ValidatorSetV2,
    total_validator_rewards: &mut Balance<IOTA>,
    validator_report_records: &mut VecMap<address, VecSet<address>>,
    reward_slashing_rate: u64,
    low_stake_threshold: u64,
    very_low_stake_threshold: u64,
    low_stake_grace_period: u64,
    committee_size: u64,
    ctx: &mut TxContext,
) {
    let new_epoch = ctx.epoch() + 1;
    let total_voting_power = voting_power::total_voting_power();

    // Compute the reward distribution without taking into account the tallying rule slashing.
    let unadjusted_staking_reward_amounts = compute_unadjusted_reward_distribution(
        &self.active_validators,
        &self.committee_members,
        total_voting_power,
        total_validator_rewards.value(),
    );

    // Use the tallying rule report records for the epoch to compute validators that will be
    // punished.
    let slashed_validators = compute_slashed_validators(self, *validator_report_records);

    // Compute the adjusted amounts of stake each committee validator should get according to the tallying rule.
    // `compute_adjusted_reward_distribution` must be called before `distribute_reward` and `adjust_next_epoch_commission_rate` to
    // make sure we are using the current epoch's stake information to compute reward distribution.
    let adjusted_staking_reward_amounts = compute_adjusted_reward_distribution(
        &self.committee_members,
        unadjusted_staking_reward_amounts,
        get_validator_indices_set(&self.active_validators, &slashed_validators),
        reward_slashing_rate,
    );

    // Distribute the rewards before adjusting stake so that we immediately start compounding
    // the rewards for validators and stakers.
    distribute_reward(
        &mut self.active_validators,
        &self.committee_members,
        &adjusted_staking_reward_amounts,
        total_validator_rewards,
        ctx,
    );

    adjust_next_epoch_commission_rate(&mut self.active_validators);

    process_pending_stakes_and_withdraws(&mut self.active_validators, ctx);

    // Emit events after we have processed all the rewards distribution and pending stakes.
    emit_validator_epoch_events(
        new_epoch,
        &self.active_validators,
        &self.committee_members,
        &adjusted_staking_reward_amounts,
        validator_report_records,
        &slashed_validators,
    );

    // Collect committee validator addresses before modifying the `active_validators`.
    // Getting this later would result in incorrect addresses, because `committee_members` values
    // would be pointing to incorrect validators in `active_validators`.
    let prev_committee_validator_addresses = self.committee_validator_addresses();

    // Note that all their staged next epoch metadata will be effectuated below.
    process_pending_validators(self, new_epoch);

    process_pending_removals(
        self,
        prev_committee_validator_addresses,
        validator_report_records,
        ctx,
    );

    // kick low stake validators out.
    update_and_process_low_stake_departures(
        self,
        low_stake_threshold,
        very_low_stake_threshold,
        low_stake_grace_period,
        validator_report_records,
        prev_committee_validator_addresses,
        ctx,
    );

    self.process_new_committee(committee_size, prev_committee_validator_addresses, ctx);

    self.total_stake =
        calculate_total_committee_stakes(&self.active_validators, &self.committee_members);

    voting_power::set_voting_power(&self.committee_members, &mut self.active_validators);

    // At this point, self.active_validators and the self.committee_members are updated for next epoch.
    // Now we process the staged validator metadata.
    effectuate_staged_metadata(self);
}

fun update_and_process_low_stake_departures(
    self: &mut ValidatorSetV2,
    low_stake_threshold: u64,
    very_low_stake_threshold: u64,
    low_stake_grace_period: u64,
    validator_report_records: &mut VecMap<address, VecSet<address>>,
    committee_addresses: vector<address>,
    ctx: &mut TxContext,
) {
    // Iterate through all the active validators, record their low stake status, and kick them out if the condition is met.
    let mut i = self.active_validators.length();
    while (i > 0) {
        i = i - 1;
        let validator_ref = &self.active_validators[i];
        let validator_address = validator_ref.iota_address();
        let stake = validator_ref.total_stake_amount();
        if (stake >= low_stake_threshold) {
            // The validator is safe. We remove their entry from the at_risk map if there exists one.
            if (self.at_risk_validators.contains(&validator_address)) {
                self.at_risk_validators.remove(&validator_address);
            }
        } else if (stake >= very_low_stake_threshold) {
            // The stake is a bit below the threshold so we increment the entry of the validator in the map.
            let new_low_stake_period = if (self.at_risk_validators.contains(&validator_address)) {
                let num_epochs = &mut self.at_risk_validators[&validator_address];
                *num_epochs = *num_epochs + 1;
                *num_epochs
            } else {
                self.at_risk_validators.insert(validator_address, 1);
                1
            };

            // If the grace period has passed, the validator has to leave us.
            if (new_low_stake_period > low_stake_grace_period) {
                let validator = self.active_validators.remove(i);
                let is_committee = committee_addresses.contains(&validator.iota_address());
                process_validator_departure(
                    self,
                    validator,
                    validator_report_records,
                    false,
                    /* the validator is kicked out involuntarily */
                    is_committee,
                    ctx,
                );
            }
        } else {
            // The validator's stake is lower than the very low threshold so we kick them out immediately.
            let validator = self.active_validators.remove(i);
            let is_committee = committee_addresses.contains(&validator.iota_address());
            process_validator_departure(
                self,
                validator,
                validator_report_records,
                false,
                /* the validator is kicked out involuntarily */
                is_committee,
                ctx,
            );
        }
    }
}

/// Effectutate pending next epoch metadata if they are staged.
fun effectuate_staged_metadata(self: &mut ValidatorSetV2) {
    self.active_validators.do_mut!(|v| v.effectuate_staged_metadata());
}

/// Called by `iota_system` to derive reference gas price for the new epoch for ValidatorSetV1.
/// Derive the reference gas price based on the gas price quote submitted by each validator.
/// The returned gas price should be greater than or equal to 2/3 of the validators submitted
/// gas price, weighted by stake.
public fun derive_reference_gas_price(self: &ValidatorSetV1): u64 {
    let vs = &self.active_validators;
    let num_validators = vs.length();
    let mut entries = vector[];
    let mut i = 0;
    while (i < num_validators) {
        let v = &vs[i];
        entries.push_back(
            pq::new_entry(v.gas_price(), v.voting_power()),
        );
        i = i + 1;
    };
    // Build a priority queue that will pop entries with gas price from the highest to the lowest.
    let mut pq = pq::new(entries);
    let mut sum = 0;
    let threshold = voting_power::total_voting_power() - voting_power::quorum_threshold();
    let mut result = 0;
    while (sum < threshold) {
        let (gas_price, voting_power) = pq.pop_max();
        result = gas_price;
        sum = sum + voting_power;
    };
    result
}

// ==== getter functions for ValidatorSetV1 ====

#[deprecated]
public fun total_stake(self: &ValidatorSetV1): u64 {
    self.total_stake
}

#[deprecated]
public fun validator_total_stake_amount(self: &ValidatorSetV1, validator_address: address): u64 {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.total_stake_amount()
}

#[deprecated]
public fun validator_stake_amount(self: &ValidatorSetV1, validator_address: address): u64 {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.stake_amount()
}

#[deprecated]
public fun validator_voting_power(self: &ValidatorSetV1, validator_address: address): u64 {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.voting_power()
}

#[deprecated]
public fun validator_staking_pool_id(self: &ValidatorSetV1, validator_address: address): ID {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.staking_pool_id()
}

#[deprecated]
public fun staking_pool_mappings(self: &ValidatorSetV1): &Table<ID, address> {
    &self.staking_pool_mappings
}

// ==== upgradeable getter functions for ValidatorSetV2 ====
public(package) fun total_stake_inner(self: &ValidatorSetV2): u64 {
    self.total_stake
}

public(package) fun validator_total_stake_amount_inner(
    self: &ValidatorSetV2,
    validator_address: address,
): u64 {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.total_stake_amount()
}

public(package) fun validator_stake_amount_inner(
    self: &ValidatorSetV2,
    validator_address: address,
): u64 {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.stake_amount()
}

public(package) fun validator_voting_power_inner(
    self: &ValidatorSetV2,
    validator_address: address,
): u64 {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.voting_power()
}

public(package) fun validator_staking_pool_id_inner(
    self: &ValidatorSetV2,
    validator_address: address,
): ID {
    let validator = get_validator_ref(&self.active_validators, validator_address);
    validator.staking_pool_id()
}

public(package) fun staking_pool_mappings_inner(self: &ValidatorSetV2): &Table<ID, address> {
    &self.staking_pool_mappings
}

public(package) fun validator_address_by_pool_id_inner(
    self: &mut ValidatorSetV2,
    pool_id: &ID,
): address {
    // If the pool id is recorded in the mapping, then it must be either candidate or active.
    if (self.staking_pool_mappings.contains(*pool_id)) {
        self.staking_pool_mappings[*pool_id]
    } else {
        // otherwise it's inactive
        let wrapper = &mut self.inactive_validators[*pool_id];
        let validator = wrapper.load_validator_maybe_upgrade();
        validator.iota_address()
    }
}

public(package) fun pool_exchange_rates(
    self: &mut ValidatorSetV2,
    pool_id: &ID,
): &Table<u64, PoolTokenExchangeRate> {
    let validator // If the pool id is recorded in the mapping, then it must be either candidate or active.
     = if (self.staking_pool_mappings.contains(*pool_id)) {
        let validator_address = self.staking_pool_mappings[*pool_id];
        get_active_or_pending_or_candidate_validator_ref(self, validator_address, ANY_VALIDATOR)
    } else {
        // otherwise it's inactive
        let wrapper = &mut self.inactive_validators[*pool_id];
        wrapper.load_validator_maybe_upgrade()
    };
    validator.get_staking_pool_ref().exchange_rates()
}

/// Get the total number of validators in the next epoch.
public(package) fun next_epoch_validator_count(self: &ValidatorSetV2): u64 {
    self.active_validators.length() - self.pending_removals.length() + self.pending_active_validators.length()
}

/// Returns true iff the address exists in active validators.
public(package) fun is_active_validator_by_iota_address(
    self: &ValidatorSetV2,
    validator_address: address,
): bool {
    find_validator(&self.active_validators, validator_address).is_some()
}

/// Returns true iff the address exists in committee validators.
public(package) fun is_committee_validator_by_iota_address(
    self: &ValidatorSetV2,
    validator_address: address,
): bool {
    let validator_index_opt = find_validator(&self.active_validators, validator_address);
    // Validator is part of the committee if it belongs to the set of active validators
    // and it's index is part of the committee members set.
    validator_index_opt.is_some() && self.committee_members.contains(validator_index_opt.borrow())
}

// ==== private helpers ====

/// Checks whether `new_validator` is duplicate with any currently active validators.
/// It differs from `is_active_validator_by_iota_address` in that the former checks
/// only the iota address but this function looks at more metadata.
fun is_duplicate_with_active_validator(self: &ValidatorSetV2, new_validator: &ValidatorV1): bool {
    is_duplicate_validator(&self.active_validators, new_validator)
}

public(package) fun is_duplicate_validator(
    validators: &vector<ValidatorV1>,
    new_validator: &ValidatorV1,
): bool {
    count_duplicates_vec(validators, new_validator) > 0
}

fun count_duplicates_vec(validators: &vector<ValidatorV1>, validator: &ValidatorV1): u64 {
    let mut result = 0;
    validators.do_ref!(|v| {
        if (v.is_duplicate(validator)) {
            result = result + 1;
        };
    });
    result
}

/// Checks whether `new_validator` is duplicate with any currently pending validators.
fun is_duplicate_with_pending_validator(self: &ValidatorSetV2, new_validator: &ValidatorV1): bool {
    count_duplicates_tablevec(&self.pending_active_validators, new_validator) > 0
}

fun count_duplicates_tablevec(validators: &TableVec<ValidatorV1>, validator: &ValidatorV1): u64 {
    let len = validators.length();
    let mut i = 0;
    let mut result = 0;
    while (i < len) {
        let v = &validators[i];
        if (v.is_duplicate(validator)) {
            result = result + 1;
        };
        i = i + 1;
    };
    result
}

/// Get mutable reference to either a candidate or an active validator by address.
fun get_candidate_or_active_validator_mut(
    self: &mut ValidatorSetV2,
    validator_address: address,
): &mut ValidatorV1 {
    if (self.validator_candidates.contains(validator_address)) {
        let wrapper = &mut self.validator_candidates[validator_address];
        return wrapper.load_validator_maybe_upgrade()
    };
    get_validator_mut(&mut self.active_validators, validator_address)
}

/// Find validator by `validator_address`, in `validators`.
/// Returns (true, index) if the validator is found, and the index is its index in the list.
/// If not found, returns (false, 0).
fun find_validator(validators: &vector<ValidatorV1>, validator_address: address): Option<u64> {
    validators.find_index!(|v| v.iota_address() == validator_address)
}

/// Find validator by `validator_address`, in `validators`.
/// Returns (true, index) if the validator is found, and the index is its index in the list.
/// If not found, returns (false, 0).
fun find_validator_from_table_vec(
    validators: &TableVec<ValidatorV1>,
    validator_address: address,
): Option<u64> {
    let length = validators.length();
    let mut i = 0;
    while (i < length) {
        let v = &validators[i];
        if (v.iota_address() == validator_address) {
            return option::some(i)
        };
        i = i + 1;
    };
    option::none()
}

/// Given a vector of validator addresses, return a set of all indices of the validators.
/// Aborts if any address isn't in the given validator set.
fun get_validator_indices_set(
    validators: &vector<ValidatorV1>,
    validator_addresses: &vector<address>,
): VecSet<u64> {
    let mut res = vec_set::empty();
    validator_addresses.do_ref!(|addr| {
        let index_opt = find_validator(validators, *addr);
        assert!(index_opt.is_some(), ENotAValidator);
        res.insert(index_opt.destroy_some());
    });
    res
}

public(package) fun get_validator_mut(
    validators: &mut vector<ValidatorV1>,
    validator_address: address,
): &mut ValidatorV1 {
    let mut validator_index_opt = find_validator(validators, validator_address);
    assert!(validator_index_opt.is_some(), ENotAValidator);
    let validator_index = validator_index_opt.extract();
    &mut validators[validator_index]
}

/// Get mutable reference to an active or (if active does not exist) pending or (if pending and
/// active do not exist) or candidate validator by address.
/// Note: this function should be called carefully, only after verifying the transaction
/// sender has the ability to modify the `ValidatorV1`.
fun get_active_or_pending_or_candidate_validator_mut(
    self: &mut ValidatorSetV2,
    validator_address: address,
    include_candidate: bool,
): &mut ValidatorV1 {
    let mut validator_index_opt = find_validator(&self.active_validators, validator_address);
    if (validator_index_opt.is_some()) {
        let validator_index = validator_index_opt.extract();
        return &mut self.active_validators[validator_index]
    };
    let mut validator_index_opt = find_validator_from_table_vec(
        &self.pending_active_validators,
        validator_address,
    );
    // consider both pending validators and the candidate ones
    if (validator_index_opt.is_some()) {
        let validator_index = validator_index_opt.extract();
        return &mut self.pending_active_validators[validator_index]
    };
    assert!(include_candidate, ENotActiveOrPendingValidator);
    let wrapper = &mut self.validator_candidates[validator_address];
    wrapper.load_validator_maybe_upgrade()
}

public(package) fun get_validator_mut_with_verified_cap(
    self: &mut ValidatorSetV2,
    verified_cap: &ValidatorOperationCap,
    include_candidate: bool,
): &mut ValidatorV1 {
    get_active_or_pending_or_candidate_validator_mut(
        self,
        *verified_cap.verified_operation_cap_address(),
        include_candidate,
    )
}

public(package) fun get_validator_mut_with_ctx(
    self: &mut ValidatorSetV2,
    ctx: &TxContext,
): &mut ValidatorV1 {
    let validator_address = ctx.sender();
    get_active_or_pending_or_candidate_validator_mut(self, validator_address, false)
}

public(package) fun get_validator_mut_with_ctx_including_candidates(
    self: &mut ValidatorSetV2,
    ctx: &TxContext,
): &mut ValidatorV1 {
    let validator_address = ctx.sender();
    get_active_or_pending_or_candidate_validator_mut(self, validator_address, true)
}

fun get_validator_ref(validators: &vector<ValidatorV1>, validator_address: address): &ValidatorV1 {
    let mut validator_index_opt = find_validator(validators, validator_address);
    assert!(validator_index_opt.is_some(), ENotAValidator);
    let validator_index = validator_index_opt.extract();
    &validators[validator_index]
}

public(package) fun get_active_or_pending_or_candidate_validator_ref(
    self: &mut ValidatorSetV2,
    validator_address: address,
    which_validator: u8,
): &ValidatorV1 {
    let mut validator_index_opt = find_validator(&self.active_validators, validator_address);
    if (validator_index_opt.is_some() || which_validator == COMMITTEE_VALIDATOR_ONLY) {
        let validator_index = validator_index_opt.extract();
        return &self.active_validators[validator_index]
    };
    let mut validator_index_opt = find_validator_from_table_vec(
        &self.pending_active_validators,
        validator_address,
    );
    if (validator_index_opt.is_some() || which_validator == ACTIVE_OR_PENDING_VALIDATOR) {
        let validator_index = validator_index_opt.extract();
        return &self.pending_active_validators[validator_index]
    };
    self.validator_candidates[validator_address].load_validator_maybe_upgrade()
}

public fun get_active_validator_ref(
    self: &ValidatorSetV1,
    validator_address: address,
): &ValidatorV1 {
    let mut validator_index_opt = find_validator(&self.active_validators, validator_address);
    assert!(validator_index_opt.is_some(), ENotAValidator);
    let validator_index = validator_index_opt.extract();
    &self.active_validators[validator_index]
}

public fun get_pending_validator_ref(
    self: &ValidatorSetV1,
    validator_address: address,
): &ValidatorV1 {
    let mut validator_index_opt = find_validator_from_table_vec(
        &self.pending_active_validators,
        validator_address,
    );
    assert!(validator_index_opt.is_some(), ENotAPendingValidator);
    let validator_index = validator_index_opt.extract();
    &self.pending_active_validators[validator_index]
}

public(package) fun get_active_validator_ref_inner(
    self: &ValidatorSetV2,
    validator_address: address,
): &ValidatorV1 {
    let mut validator_index_opt = find_validator(&self.active_validators, validator_address);
    assert!(validator_index_opt.is_some(), ENotAValidator);
    let validator_index = validator_index_opt.extract();
    &self.active_validators[validator_index]
}

public(package) fun get_committee_validator_ref_inner(
    self: &ValidatorSetV2,
    validator_address: address,
): &ValidatorV1 {
    let mut validator_index_opt = find_validator(&self.active_validators, validator_address);
    assert!(validator_index_opt.is_some(), ENotAValidator);
    assert!(self.committee_members.contains(validator_index_opt.borrow()), ENotACommitteeValidator);

    let validator_index = validator_index_opt.extract();
    &self.active_validators[validator_index]
}

public(package) fun get_pending_validator_ref_inner(
    self: &ValidatorSetV2,
    validator_address: address,
): &ValidatorV1 {
    let mut validator_index_opt = find_validator_from_table_vec(
        &self.pending_active_validators,
        validator_address,
    );
    assert!(validator_index_opt.is_some(), ENotAPendingValidator);
    let validator_index = validator_index_opt.extract();
    &self.pending_active_validators[validator_index]
}

#[test_only]
public fun get_candidate_validator_ref(
    self: &ValidatorSetV2,
    validator_address: address,
): &ValidatorV1 {
    self.validator_candidates[validator_address].get_inner_validator_ref()
}

/// Verify the capability is valid for a Validator.
/// If `which_validator == COMMITTEE_VALIDATOR_ONLY` is true, only verify the Cap for an committee validator.
/// Otherwise, verify the Cap for an either active or pending validator.
public(package) fun verify_cap(
    self: &mut ValidatorSetV2,
    cap: &UnverifiedValidatorOperationCap,
    which_validator: u8,
): ValidatorOperationCap {
    let cap_address = *cap.unverified_operation_cap_address();
    let validator = if (which_validator == COMMITTEE_VALIDATOR_ONLY)
        get_committee_validator_ref_inner(self, cap_address)
    else get_active_or_pending_or_candidate_validator_ref(self, cap_address, which_validator);
    assert!(validator.operation_cap_id() == &object::id(cap), EInvalidCap);
    validator_cap::new_from_unverified(cap)
}

/// Process the pending withdraw requests. For each pending request, the validator
/// is removed from `validators` and its staking pool is put into the `inactive_validators` table.
fun process_pending_removals(
    self: &mut ValidatorSetV2,
    committee_addresses: vector<address>,
    validator_report_records: &mut VecMap<address, VecSet<address>>,
    ctx: &mut TxContext,
) {
    sort_removal_list(&mut self.pending_removals);
    while (!self.pending_removals.is_empty()) {
        let index = self.pending_removals.pop_back();
        let validator = self.active_validators.remove(index);
        let is_committee = committee_addresses.contains(&validator.iota_address());

        process_validator_departure(
            self,
            validator,
            validator_report_records,
            true,
            /* the validator removes itself voluntarily */
            is_committee,
            ctx,
        );
    }
}

fun process_validator_departure(
    self: &mut ValidatorSetV2,
    mut validator: ValidatorV1,
    validator_report_records: &mut VecMap<address, VecSet<address>>,
    is_voluntary: bool,
    is_committee: bool,
    ctx: &mut TxContext,
) {
    let new_epoch = ctx.epoch() + 1;
    let validator_address = validator.iota_address();
    let validator_pool_id = staking_pool_id(&validator);

    // Remove the validator from our tables.
    self.staking_pool_mappings.remove(validator_pool_id);
    if (self.at_risk_validators.contains(&validator_address)) {
        self.at_risk_validators.remove(&validator_address);
    };

    if (is_committee) {
        self.total_stake = self.total_stake - validator.total_stake_amount();
        event::emit(CommitteeValidatorLeaveEvent {
            epoch: new_epoch,
            validator_address,
            staking_pool_id: staking_pool_id(&validator),
        });
    };

    clean_report_records_leaving_validator(validator_report_records, validator_address);

    event::emit(ValidatorLeaveEvent {
        epoch: new_epoch,
        validator_address,
        staking_pool_id: staking_pool_id(&validator),
        is_voluntary,
    });

    // Deactivate the validator and its staking pool
    validator.deactivate(new_epoch);
    self
        .inactive_validators
        .add(
            validator_pool_id,
            validator_wrapper::create_v1(validator, ctx),
        );
}

fun clean_report_records_leaving_validator(
    validator_report_records: &mut VecMap<address, VecSet<address>>,
    leaving_validator_addr: address,
) {
    // Remove the records about this validator
    if (validator_report_records.contains(&leaving_validator_addr)) {
        validator_report_records.remove(&leaving_validator_addr);
    };

    // Remove the reports submitted by this validator
    let reported_validators = validator_report_records.keys();
    let length = reported_validators.length();
    let mut i = 0;
    while (i < length) {
        let reported_validator_addr = &reported_validators[i];
        let reporters = &mut validator_report_records[reported_validator_addr];
        if (reporters.contains(&leaving_validator_addr)) {
            reporters.remove(&leaving_validator_addr);
            if (reporters.is_empty()) {
                validator_report_records.remove(reported_validator_addr);
            };
        };
        i = i + 1;
    }
}

/// Process the pending new validators. They are activated and inserted into `validators`.
fun process_pending_validators(self: &mut ValidatorSetV2, new_epoch: u64) {
    while (!self.pending_active_validators.is_empty()) {
        let mut validator = self.pending_active_validators.pop_back();
        validator.activate(new_epoch);
        event::emit(ValidatorJoinEvent {
            epoch: new_epoch,
            validator_address: validator.iota_address(),
            staking_pool_id: staking_pool_id(&validator),
        });
        self.active_validators.push_back(validator);
    }
}

/// Sort all the pending removal indexes.
fun sort_removal_list(withdraw_list: &mut vector<u64>) {
    let length = withdraw_list.length();
    let mut i = 1;
    while (i < length) {
        let cur = withdraw_list[i];
        let mut j = i;
        while (j > 0) {
            j = j - 1;
            if (withdraw_list[j] > cur) {
                withdraw_list.swap(j, j + 1);
            } else {
                break
            };
        };
        i = i + 1;
    };
}

/// Process all active validators' pending stake deposits and withdraws.
fun process_pending_stakes_and_withdraws(validators: &mut vector<ValidatorV1>, ctx: &TxContext) {
    let length = validators.length();
    let mut i = 0;
    while (i < length) {
        let validator = &mut validators[i];
        validator.process_pending_stakes_and_withdraws(ctx);
        i = i + 1;
    }
}

/// Calculate the total active validator stake.
fun calculate_total_active_stakes(validators: &vector<ValidatorV1>): u64 {
    let mut stake = 0;
    let length = validators.length();
    let mut i = 0;
    while (i < length) {
        let v = &validators[i];
        stake = stake + v.total_stake();
        i = i + 1;
    };
    stake
}

/// Calculate the total committee validator stake.
fun calculate_total_committee_stakes(
    validators: &vector<ValidatorV1>,
    committee_members: &vector<u64>,
): u64 {
    voting_power::total_committee_stake(validators, committee_members)
}

/// Process the pending stake changes for each validator.
fun adjust_next_epoch_commission_rate(validators: &mut vector<ValidatorV1>) {
    let length = validators.length();
    let mut i = 0;
    while (i < length) {
        let validator = &mut validators[i];
        validator.adjust_next_epoch_commission_rate();
        i = i + 1;
    }
}

/// Process the validator report records of the epoch and return the addresses of the
/// non-performant committee validators according to the input threshold.
fun compute_slashed_validators(
    self: &ValidatorSetV2,
    mut validator_report_records: VecMap<address, VecSet<address>>,
): vector<address> {
    let mut slashed_validators = vector[];
    while (!validator_report_records.is_empty()) {
        let (validator_address, reporters) = validator_report_records.pop();
        assert!(
            is_committee_validator_by_iota_address(self, validator_address),
            ENonValidatorInReportRecords,
        );
        // Sum up the voting power of validators that have reported this validator and check if it has
        // passed the slashing threshold.
        let reporter_votes = sum_committee_voting_power_by_addresses(self, &reporters.into_keys());
        if (reporter_votes >= voting_power::quorum_threshold()) {
            slashed_validators.push_back(validator_address);
        }
    };
    slashed_validators
}

/// Given the current list of committee validators, the total stake and total reward,
/// calculate the amount of reward each validator should get, without taking into
/// account the tallying rule results.
/// Returns the unadjusted amounts of staking reward for each validator.
fun compute_unadjusted_reward_distribution(
    active_validators: &vector<ValidatorV1>,
    committee_members: &vector<u64>,
    total_voting_power: u64,
    total_staking_reward: u64,
): vector<u64> {
    active_validators.take_map_ref!(committee_members, |validator| {
        // Integer divisions will truncate the results. Because of this, we expect that at the end
        // there will be some reward remaining in `total_staking_reward`.
        // Use u128 to avoid multiplication overflow.
        let voting_power: u128 = validator.voting_power() as u128;
        let reward_amount =
            voting_power * (total_staking_reward as u128) / (total_voting_power as u128);
        reward_amount as u64
    })
}

/// Use the reward adjustment info to compute the adjusted rewards each validator should get.
/// Returns the staking rewards each validator gets.
/// The staking rewards are shared with the stakers.
fun compute_adjusted_reward_distribution(
    committee_members: &vector<u64>,
    unadjusted_staking_reward_amounts: vector<u64>,
    slashed_validator_indices_set: VecSet<u64>,
    reward_slashing_rate: u64,
): vector<u64> {
    let mut adjusted_staking_reward_amounts = vector[];

    // Loop through each validator and adjust rewards as necessary
    let length = committee_members.length();
    let mut i = 0;
    while (i < length) {
        let unadjusted_staking_reward_amount = unadjusted_staking_reward_amounts[i];

        // Check if the validator is slashed
        let adjusted_staking_reward_amount = if (
            slashed_validator_indices_set.contains(&committee_members[i])
        ) {
            // Use the slashing rate to compute the amount of staking rewards slashed from this punished validator.
            // Use u128 to avoid multiplication overflow.
            let staking_reward_adjustment_u128 =
                ((unadjusted_staking_reward_amount as u128) * (reward_slashing_rate as u128)) / BASIS_POINT_DENOMINATOR;
            unadjusted_staking_reward_amount - (staking_reward_adjustment_u128 as u64)
        } else {
            // Otherwise, unadjusted staking reward amount is assigned to the unslashed validators
            unadjusted_staking_reward_amount
        };

        adjusted_staking_reward_amounts.push_back(adjusted_staking_reward_amount);

        // Move to the next validator
        i = i + 1;
    };

    // The sum of the adjusted staking rewards may not be equal to the total staking reward,
    // because of integer division truncation and the slashing of the rewards for the slashed validators.
    adjusted_staking_reward_amounts
}

fun distribute_reward(
    validators: &mut vector<ValidatorV1>,
    committee_members: &vector<u64>,
    adjusted_staking_reward_amounts: &vector<u64>,
    staking_rewards: &mut Balance<IOTA>,
    ctx: &mut TxContext,
) {
    // non-empty committee_members implies non-empty validators, but not vice versa
    assert!(!committee_members.is_empty(), EValidatorSetEmpty);

    validators.take_do_with_ix_mut!(committee_members, |i, _, validator| {
        let staking_reward_amount = adjusted_staking_reward_amounts[i];
        let mut staker_reward = staking_rewards.split(staking_reward_amount);

        // Validator takes a cut of the rewards as commission.
        let validator_commission_amount =
            (staking_reward_amount as u128) * (validator.commission_rate() as u128) / BASIS_POINT_DENOMINATOR;

        // The validator reward = commission.
        let validator_reward = staker_reward.split(validator_commission_amount as u64);

        // Add rewards to the validator. Don't try and distribute rewards though if the payout is zero.
        if (validator_reward.value() > 0) {
            let validator_address = validator.iota_address();
            let rewards_stake = validator.request_add_stake(
                validator_reward,
                validator_address,
                ctx,
            );
            transfer::public_transfer(rewards_stake, validator_address);
        } else {
            validator_reward.destroy_zero();
        };

        // Add rewards to stake staking pool to auto compound for stakers.
        validator.deposit_stake_rewards(staker_reward);
    });
}

/// Emit events containing information of each committee validator for the epoch,
/// including stakes, rewards, performance, etc.
fun emit_validator_epoch_events(
    new_epoch: u64,
    vs: &vector<ValidatorV1>,
    committee_members: &vector<u64>,
    pool_staking_reward_amounts: &vector<u64>,
    report_records: &VecMap<address, VecSet<address>>,
    slashed_validators: &vector<address>,
) {
    assert!(committee_members.length() == pool_staking_reward_amounts.length());
    let mut i = 0;
    vs.do_ref!(|v| {
        let validator_address = v.iota_address();
        let tallying_rule_reporters =
            if (report_records.contains(&validator_address)) {
                report_records[&validator_address].into_keys()
            } else {
                vector[]
            };
        let tallying_rule_global_score =
            if (slashed_validators.contains(&validator_address)) 0
            else 1;
        let mut committee_member_index = committee_members.find_index!(|c| c == i);
        let pool_staking_reward = if (committee_member_index.is_some()) {
            // prepare event for a committee validator
            pool_staking_reward_amounts[committee_member_index.extract()]
        } else {
            // prepare event for an active non-committee validator
            0
        };
        event::emit(
            ValidatorEpochInfoEventV1 {
                epoch: new_epoch,
                validator_address,
                reference_gas_survey_quote: v.gas_price(),
                stake: v.total_stake_amount(),
                voting_power: v.voting_power(),
                commission_rate: v.commission_rate(),
                pool_staking_reward,
                pool_token_exchange_rate: v.pool_token_exchange_rate_at_epoch(new_epoch),
                tallying_rule_reporters,
                tallying_rule_global_score,
            }
        );
        i = i + 1;
    });
}

/// Sum up the total stake of a given list of validator addresses.
public fun sum_voting_power_by_addresses(
    vs: &vector<ValidatorV1>,
    addresses: &vector<address>,
): u64 {
    let mut sum = 0;
    let mut i = 0;
    let length = addresses.length();
    while (i < length) {
        let validator = get_validator_ref(vs, addresses[i]);
        sum = sum + validator.voting_power();
        i = i + 1;
    };
    sum
}

/// Sum up the total stake of a given list of committee validator addresses.
public(package) fun sum_committee_voting_power_by_addresses(
    vs: &ValidatorSetV2,
    addresses: &vector<address>,
): u64 {
    let mut sum = 0;
    addresses.do_ref!(|addr| {
        let validator = get_committee_validator_ref_inner(vs, *addr);
        sum = sum + validator.voting_power();
    });
    sum
}

/// Return the active validators in `self`
#[deprecated]
public fun active_validators(self: &ValidatorSetV1): &vector<ValidatorV1> {
    &self.active_validators
}

/// Returns true if the `addr` is a validator candidate.
#[deprecated]
public fun is_validator_candidate(self: &ValidatorSetV1, addr: address): bool {
    self.validator_candidates.contains(addr)
}

/// Returns true if the staking pool identified by `staking_pool_id` is of an inactive validator.
#[deprecated]
public fun is_inactive_validator(self: &ValidatorSetV1, staking_pool_id: ID): bool {
    self.inactive_validators.contains(staking_pool_id)
}

/// Return the active validators in `self`
public(package) fun active_validators_inner(self: &ValidatorSetV2): &vector<ValidatorV1> {
    &self.active_validators
}

/// Returns true if the `addr` is a validator candidate.
public(package) fun is_validator_candidate_inner(self: &ValidatorSetV2, addr: address): bool {
    self.validator_candidates.contains(addr)
}

/// Returns true if the staking pool identified by `staking_pool_id` is of an inactive validator.
public(package) fun is_inactive_validator_inner(self: &ValidatorSetV2, staking_pool_id: ID): bool {
    self.inactive_validators.contains(staking_pool_id)
}

public(package) fun active_validator_addresses(self: &ValidatorSetV2): vector<address> {
    self.active_validators.map_ref!(|v| v.iota_address())
}

#[allow(dead_code)]
public(package) fun committee_validator_addresses(self: &ValidatorSetV2): vector<address> {
    self.active_validators.take_map_ref!(&self.committee_members, |v| v.iota_address())
}

// Selects top N stakers among all active validators to be part of the committee.
public(package) fun select_committee_members_top_n_stakers(
    self: &ValidatorSetV2,
    n: u64,
): vector<u64> {
    self.active_validators.take_top_n!(n, |v1, v2| v1.smaller_than(v2))
}

// Emits events for committee validators that were added or left the committee.
public(package) fun process_new_committee(
    self: &mut ValidatorSetV2,
    committee_size: u64,
    prev_committee_addresses: vector<address>,
    ctx: &TxContext,
) {
    self.committee_members = self.select_committee_members_top_n_stakers(committee_size);

    let new_epoch = ctx.epoch() + 1;

    self.active_validators.take_do_ref!(&self.committee_members, |validator| {
        let validator_address = validator.iota_address();

        // Emit join committee event only if the validator wasn't part of the old committee.
        if (!prev_committee_addresses.contains(&validator_address)) {
            event::emit(CommitteeValidatorJoinEvent {
                epoch: new_epoch,
                validator_address: validator_address,
                staking_pool_id: staking_pool_id(validator),
            });
        };
    });

    // Emit leave committee events.
    let new_committee_addresses = self.committee_validator_addresses();
    prev_committee_addresses.do_ref!(|validator_address| {
        // Emit leave committee event only if validator is not part of the new committee AND is still an active validator.
        if (!new_committee_addresses.contains(validator_address)) {
            find_validator(&self.active_validators, *validator_address).do!(|validator_index| {
                // SAFETY: find_validator returns a valid index within self.active_validators
                let validator = &self.active_validators[validator_index];

                // If it's not part of active validators anymore, it means that the leave committee event has been emitted before.
                event::emit(CommitteeValidatorLeaveEvent {
                    epoch: new_epoch,
                    validator_address: *validator_address,
                    staking_pool_id: staking_pool_id(validator),
                });
            });
        };
    });
}
