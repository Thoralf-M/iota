// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

// This file contains tests testing functionalities in `iota_system` that are not
// already tested by the other more themed tests such as `stake_tests` or
// `rewards_distribution_tests`.

#[test_only, allow(deprecated_usage)]
module iota_system::iota_system_tests;

use iota::balance;
use iota::iota::IOTA;
use iota::test_scenario::{Self, Scenario};
use iota::test_utils::assert_eq;
use iota::url;
use iota_system::governance_test_utils::{
    add_validator_full_flow,
    advance_epoch,
    remove_validator,
    set_up_iota_system_state,
    create_iota_system_state_for_testing,
    stake_with,
    unstake
};
use iota_system::iota_system::IotaSystemState;
use iota_system::iota_system_state_inner;
use iota_system::validator::{Self, ValidatorV1};
use iota_system::validator_cap::UnverifiedValidatorOperationCap;
use iota_system::validator_set;

#[test]
fun test_report_validator() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3]);

    report_helper(@0x1, @0x2, false, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1]);
    report_helper(@0x3, @0x2, false, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1, @0x3]);

    // Report again and result should stay the same.
    report_helper(@0x1, @0x2, false, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1, @0x3]);

    // Undo the report.
    report_helper(@0x3, @0x2, true, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1]);

    advance_epoch(scenario);

    // After an epoch ends, report records are still present.
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1]);

    report_helper(@0x2, @0x1, false, scenario);
    assert!(get_reporters_of(@0x1, scenario) == vector[@0x2]);

    report_helper(@0x3, @0x2, false, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1, @0x3]);

    // After 0x3 leaves, its reports are gone
    remove_validator(@0x3, scenario);
    advance_epoch(scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1]);

    // After 0x1 leaves, both its reports and the reports on its name are gone
    remove_validator(@0x1, scenario);
    advance_epoch(scenario);
    assert!(get_reporters_of(@0x1, scenario).is_empty());
    assert!(get_reporters_of(@0x2, scenario).is_empty());
    scenario_val.end();
}

#[test]
fun test_validator_ops_by_stakee_ok() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;
    set_up_iota_system_state(vector[@0x1, @0x2]);

    // @0x1 transfers the cap object to stakee.
    let stakee_address = @0xbeef;
    scenario.next_tx(@0x1);
    let cap = scenario.take_from_sender<UnverifiedValidatorOperationCap>();
    transfer::public_transfer(cap, stakee_address);

    // With the cap object in hand, stakee could report validators on behalf of @0x1.
    report_helper(stakee_address, @0x2, false, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1]);

    // stakee could also undo report.
    report_helper(stakee_address, @0x2, true, scenario);
    assert!(get_reporters_of(@0x2, scenario).is_empty());

    scenario.next_tx(stakee_address);
    let cap = scenario.take_from_sender<UnverifiedValidatorOperationCap>();
    let new_stakee_address = @0xcafe;
    transfer::public_transfer(cap, new_stakee_address);

    // New stakee could report validators on behalf of @0x1.
    report_helper(new_stakee_address, @0x2, false, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1]);

    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = validator_set::EInvalidCap)]
fun test_report_validator_by_stakee_revoked() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;
    set_up_iota_system_state(vector[@0x1, @0x2]);

    // @0x1 transfers the cap object to stakee.
    let stakee_address = @0xbeef;
    scenario.next_tx(@0x1);
    let cap = scenario.take_from_sender<UnverifiedValidatorOperationCap>();
    transfer::public_transfer(cap, stakee_address);

    report_helper(stakee_address, @0x2, false, scenario);
    assert!(get_reporters_of(@0x2, scenario) == vector[@0x1]);

    // @0x1 revokes stakee's permission by creating a new
    // operation cap object.
    rotate_operation_cap(@0x1, scenario);

    // stakee no longer has permission to report validators, here it aborts.
    report_helper(stakee_address, @0x2, true, scenario);

    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = validator::ECommissionRateTooHigh)]
fun test_set_commission_rate_failure() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;
    set_up_iota_system_state(vector[@0x1, @0x2]);

    scenario.next_tx(@0x2);
    let mut system_state = scenario.take_shared<IotaSystemState>();

    // Fails here since the commission rate is too high.
    system_state.request_set_commission_rate(2001, scenario.ctx());
    test_scenario::return_shared(system_state);

    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = iota_system_state_inner::ENotCommitteeValidator)]
fun test_report_non_validator_failure() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3]);
    report_helper(@0x1, @0x42, false, scenario);
    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = iota_system_state_inner::ECannotReportOneself)]
fun test_report_self_failure() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3]);
    report_helper(@0x1, @0x1, false, scenario);
    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = iota_system_state_inner::EReportRecordNotFound)]
fun test_undo_report_failure() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3]);
    report_helper(@0x2, @0x1, true, scenario);
    scenario_val.end();
}

#[test]
fun test_validator_address_by_pool_id() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3, @0x4]);
    scenario.next_tx(@0x1);

    let mut system_state = scenario.take_shared<IotaSystemState>();
    let pool_id_1 = system_state.validator_staking_pool_id(@0x1);
    let validator_address = system_state.validator_address_by_pool_id(&pool_id_1);

    assert_eq(validator_address, @0x1);
    test_scenario::return_shared(system_state);

    scenario_val.end();
}

#[test]
fun test_staking_pool_mappings() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3, @0x4]);
    scenario.next_tx(@0x1);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    let pool_id_1 = system_state.validator_staking_pool_id(@0x1);
    let pool_id_2 = system_state.validator_staking_pool_id(@0x2);
    let pool_id_3 = system_state.validator_staking_pool_id(@0x3);
    let pool_id_4 = system_state.validator_staking_pool_id(@0x4);
    let mut pool_mappings = system_state.validator_staking_pool_mappings();
    assert_eq(pool_mappings.length(), 4);
    assert_eq(pool_mappings[pool_id_1], @0x1);
    assert_eq(pool_mappings[pool_id_2], @0x2);
    assert_eq(pool_mappings[pool_id_3], @0x3);
    assert_eq(pool_mappings[pool_id_4], @0x4);
    test_scenario::return_shared(system_state);

    let new_validator_addr = @0xaf76afe6f866d8426d2be85d6ef0b11f871a251d043b2f11e15563bf418f5a5a;
    scenario.next_tx(new_validator_addr);
    // Seed [0; 32]
    let pubkey =
        x"99f25ef61f8032b914636460982c5cc6f134ef1ddae76657f2cbfec1ebfc8d097374080df6fcf0dcb8bc4b0d8e0af5d80ebbff2b4c599f54f42d6312dfc314276078c1cc347ebbbec5198be258513f386b930d02c2749a803e2330955ebd1a10";
    // Generated with [fn test_proof_of_possession]
    let pop =
        x"b01cc86f421beca7ab4cfca87c0799c4d038c199dd399fbec1924d4d4367866dba9e84d514710b91feb65316e4ceef43";

    // Add a validator
    add_validator_full_flow(
        new_validator_addr,
        b"name2",
        b"/ip4/127.0.0.1/udp/82",
        100,
        pubkey,
        pop,
        scenario,
    );
    advance_epoch(scenario);

    scenario.next_tx(@0x1);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    let pool_id_5 = system_state.validator_staking_pool_id(new_validator_addr);
    pool_mappings = system_state.validator_staking_pool_mappings();
    // Check that the previous mappings didn't change as well.
    assert_eq(pool_mappings.length(), 5);
    assert_eq(pool_mappings[pool_id_1], @0x1);
    assert_eq(pool_mappings[pool_id_2], @0x2);
    assert_eq(pool_mappings[pool_id_3], @0x3);
    assert_eq(pool_mappings[pool_id_4], @0x4);
    assert_eq(pool_mappings[pool_id_5], new_validator_addr);
    test_scenario::return_shared(system_state);

    // Remove one of the original validators.
    remove_validator(@0x1, scenario);
    advance_epoch(scenario);

    scenario.next_tx(@0x1);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    pool_mappings = system_state.validator_staking_pool_mappings();
    // Check that the previous mappings didn't change as well.
    assert_eq(pool_mappings.length(), 4);
    assert_eq(pool_mappings.contains(pool_id_1), false);
    assert_eq(pool_mappings[pool_id_2], @0x2);
    assert_eq(pool_mappings[pool_id_3], @0x3);
    assert_eq(pool_mappings[pool_id_4], @0x4);
    assert_eq(pool_mappings[pool_id_5], new_validator_addr);
    test_scenario::return_shared(system_state);

    scenario_val.end();
}

fun report_helper(sender: address, reported: address, is_undo: bool, scenario: &mut Scenario) {
    scenario.next_tx(sender);

    let mut system_state = scenario.take_shared<IotaSystemState>();
    let cap = scenario.take_from_sender<UnverifiedValidatorOperationCap>();
    if (is_undo) {
        system_state.undo_report_validator(&cap, reported);
    } else {
        system_state.report_validator(&cap, reported);
    };
    scenario.return_to_sender(cap);
    test_scenario::return_shared(system_state);
}

fun rotate_operation_cap(sender: address, scenario: &mut Scenario) {
    scenario.next_tx(sender);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    let ctx = scenario.ctx();
    system_state.rotate_operation_cap(ctx);
    test_scenario::return_shared(system_state);
}

fun get_reporters_of(addr: address, scenario: &mut Scenario): vector<address> {
    scenario.next_tx(addr);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    let res = system_state.get_reporters_of(addr).into_keys();
    test_scenario::return_shared(system_state);
    res
}

fun update_candidate(
    scenario: &mut Scenario,
    system_state: &mut IotaSystemState,
    name: vector<u8>,
    authority_pub_key: vector<u8>,
    pop: vector<u8>,
    network_address: vector<u8>,
    p2p_address: vector<u8>,
    commission_rate: u64,
) {
    let ctx = scenario.ctx();
    system_state.update_validator_name(name, ctx);
    system_state.update_validator_description(b"new_desc", ctx);
    system_state.update_validator_image_url(b"new_image_url", ctx);
    system_state.update_validator_project_url(b"new_project_url", ctx);
    system_state.update_candidate_validator_network_address(network_address, ctx);
    system_state.update_candidate_validator_p2p_address(p2p_address, ctx);
    system_state.update_candidate_validator_primary_address(b"/ip4/127.0.0.1/udp/80", ctx);
    system_state.update_candidate_validator_authority_pubkey(
        authority_pub_key,
        pop,
        ctx,
    );
    system_state.update_candidate_validator_protocol_pubkey(
        vector[
            68,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
        ctx,
    );
    system_state.update_candidate_validator_network_pubkey(
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        ctx,
    );

    system_state.set_candidate_validator_commission_rate(commission_rate, ctx);
    let cap = scenario.take_from_sender<UnverifiedValidatorOperationCap>();
    scenario.return_to_sender(cap);
}

fun verify_candidate(
    validator: &ValidatorV1,
    name: vector<u8>,
    authority_pub_key: vector<u8>,
    pop: vector<u8>,
    network_address: vector<u8>,
    p2p_address: vector<u8>,
    commission_rate: u64,
) {
    verify_current_epoch_metadata(
        validator,
        name,
        authority_pub_key,
        pop,
        b"/ip4/127.0.0.1/udp/80",
        network_address,
        p2p_address,
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[
            68,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
    );
    assert!(validator.commission_rate() == commission_rate);
}

// Note: `pop` MUST be a valid signature using iota_address and authority_pubkey_bytes.
// To produce a valid PoP, run [fn test_proof_of_possession].
fun update_metadata(
    scenario: &mut Scenario,
    system_state: &mut IotaSystemState,
    name: vector<u8>,
    authority_pub_key: vector<u8>,
    pop: vector<u8>,
    network_address: vector<u8>,
    p2p_address: vector<u8>,
    network_pubkey: vector<u8>,
    protocol_pubkey: vector<u8>,
) {
    let ctx = scenario.ctx();
    system_state.update_validator_name(name, ctx);
    system_state.update_validator_description(b"new_desc", ctx);
    system_state.update_validator_image_url(b"new_image_url", ctx);
    system_state.update_validator_project_url(b"new_project_url", ctx);
    system_state.update_validator_next_epoch_network_address(network_address, ctx);
    system_state.update_validator_next_epoch_p2p_address(p2p_address, ctx);
    system_state.update_validator_next_epoch_primary_address(b"/ip4/168.168.168.168/udp/80", ctx);
    system_state.update_validator_next_epoch_authority_pubkey(
        authority_pub_key,
        pop,
        ctx,
    );
    system_state.update_validator_next_epoch_network_pubkey(network_pubkey, ctx);
    system_state.update_validator_next_epoch_protocol_pubkey(protocol_pubkey, ctx);
}

fun verify_metadata(
    validator: &ValidatorV1,
    name: vector<u8>,
    authority_pub_key: vector<u8>,
    pop: vector<u8>,
    network_address: vector<u8>,
    p2p_address: vector<u8>,
    network_pubkey: vector<u8>,
    protocol_pubkey: vector<u8>,
    new_authority_pub_key: vector<u8>,
    new_pop: vector<u8>,
    new_network_address: vector<u8>,
    new_p2p_address: vector<u8>,
    new_network_pubkey: vector<u8>,
    new_protocol_pubkey: vector<u8>,
) {
    // Current epoch
    verify_current_epoch_metadata(
        validator,
        name,
        authority_pub_key,
        pop,
        b"/ip4/127.0.0.1/udp/80",
        network_address,
        p2p_address,
        network_pubkey,
        protocol_pubkey,
    );

    // Next epoch
    assert!(
        validator.next_epoch_network_address() == &option::some(new_network_address.to_string()),
    );
    assert!(validator.next_epoch_p2p_address() == &option::some(new_p2p_address.to_string()));
    assert!(
        validator.next_epoch_primary_address() == &option::some(b"/ip4/168.168.168.168/udp/80".to_string()),
    );
    assert!(
        validator.next_epoch_authority_pubkey_bytes() == &option::some(new_authority_pub_key),
        0,
    );
    assert!(validator.next_epoch_proof_of_possession() == &option::some(new_pop), 0);
    assert!(validator.next_epoch_protocol_pubkey_bytes() == &option::some(new_protocol_pubkey), 0);
    assert!(validator.next_epoch_network_pubkey_bytes() == &option::some(new_network_pubkey), 0);
}

fun verify_current_epoch_metadata(
    validator: &ValidatorV1,
    name: vector<u8>,
    authority_pub_key: vector<u8>,
    pop: vector<u8>,
    primary_address: vector<u8>,
    network_address: vector<u8>,
    p2p_address: vector<u8>,
    network_pubkey_bytes: vector<u8>,
    protocol_pubkey_bytes: vector<u8>,
) {
    // Current epoch
    assert!(validator.name() == &name.to_string());
    assert!(validator.description() == &b"new_desc".to_string());
    assert!(validator.image_url() == &url::new_unsafe_from_bytes(b"new_image_url"));
    assert!(validator.project_url() == &url::new_unsafe_from_bytes(b"new_project_url"));
    assert!(validator.network_address() == &network_address.to_string());
    assert!(validator.p2p_address() == &p2p_address.to_string());
    assert!(validator.primary_address() == &primary_address.to_string());
    assert!(validator.authority_pubkey_bytes() == &authority_pub_key);
    assert!(validator.proof_of_possession() == &pop);
    assert!(validator.protocol_pubkey_bytes() == &protocol_pubkey_bytes);
    assert!(validator.network_pubkey_bytes() == &network_pubkey_bytes);
}

fun verify_metadata_after_advancing_epoch(
    validator: &ValidatorV1,
    name: vector<u8>,
    authority_pub_key: vector<u8>,
    pop: vector<u8>,
    network_address: vector<u8>,
    p2p_address: vector<u8>,
    network_pubkey: vector<u8>,
    protocol_pubkey: vector<u8>,
) {
    // Current epoch
    verify_current_epoch_metadata(
        validator,
        name,
        authority_pub_key,
        pop,
        b"/ip4/168.168.168.168/udp/80",
        network_address,
        p2p_address,
        network_pubkey,
        protocol_pubkey,
    );

    // Next epoch
    assert!(validator.next_epoch_network_address().is_none());
    assert!(validator.next_epoch_p2p_address().is_none());
    assert!(validator.next_epoch_primary_address().is_none());
    assert!(validator.next_epoch_authority_pubkey_bytes().is_none());
    assert!(validator.next_epoch_proof_of_possession().is_none());
    assert!(validator.next_epoch_protocol_pubkey_bytes().is_none());
    assert!(validator.next_epoch_network_pubkey_bytes().is_none());
}

#[test]
fun test_active_validator_update_metadata() {
    let validator_addr = @0xaf76afe6f866d8426d2be85d6ef0b11f871a251d043b2f11e15563bf418f5a5a;
    // pubkey generated with protocol key on seed [0; 32]
    let pubkey =
        x"99f25ef61f8032b914636460982c5cc6f134ef1ddae76657f2cbfec1ebfc8d097374080df6fcf0dcb8bc4b0d8e0af5d80ebbff2b4c599f54f42d6312dfc314276078c1cc347ebbbec5198be258513f386b930d02c2749a803e2330955ebd1a10";
    // pop generated using the authority key and address with [fn test_proof_of_possession]
    let pop =
        x"b01cc86f421beca7ab4cfca87c0799c4d038c199dd399fbec1924d4d4367866dba9e84d514710b91feb65316e4ceef43";

    // pubkey generated with authority key on seed [1; 32]
    let pubkey1 =
        x"96d19c53f1bee2158c3fcfb5bb2f06d3a8237667529d2d8f0fbb22fe5c3b3e64748420b4103674490476d98530d063271222d2a59b0f7932909cc455a30f00c69380e6885375e94243f7468e9563aad29330aca7ab431927540e9508888f0e1c";
    let pop1 =
        x"a8a0bcaf04e13565914eb22fa9f27a76f297db04446860ee2b923d10224cedb130b30783fb60b12556e7fc50e5b57a86";

    let new_validator_addr = @0x8e3446145b0c7768839d71840df389ffa3b9742d0baaff326a3d453b595f87d7;
    // pubkey generated with authority key on seed [2; 32]
    let new_pubkey =
        x"adf2e2350fe9a58f3fa50777499f20331c4550ab70f6a4fb25a58c61b50b5366107b5c06332e71bb47aa99ce2d5c07fe0dab04b8af71589f0f292c50382eba6ad4c90acb010ab9db7412988b2aba1018aaf840b1390a8b2bee3fde35b4ab7fdf";
    let new_pop =
        x"926fdb08b2b46d802e3642044f215dcb049e6c17a376a272ffd7dba32739bb995370966698ab235ee172fbd974985cfe";

    // pubkey generated with authority key on seed [3; 32]
    let new_pubkey1 =
        x"91b8de031e0b60861c655c8168596d98b065d57f26f287f8c810590b06a636eff13c4055983e95b2f60a4d6ba5484fa4176923d1f7807cc0b222ddf6179c1db099dba0433f098aae82542b3fd27b411d64a0a35aad01b2c07ac67f7d0a1d2c11";
    let new_pop1 =
        x"b61913eb4dc7ea1d92f174e1a3c6cad3f49ae8de40b13b69046ce072d8d778bfe87e734349c7394fd1543fff0cb6e2d0";

    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    // Set up IotaSystemState with an active validator
    let mut validators = vector[];
    let ctx = scenario.ctx();
    let validator = validator::new_for_testing(
        validator_addr,
        pubkey,
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[
            68,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
        pop,
        b"ValidatorName",
        b"description",
        b"image_url",
        b"project_url",
        b"/ip4/127.0.0.1/tcp/80",
        b"/ip4/127.0.0.1/udp/80",
        b"/ip4/127.0.0.1/udp/80",
        option::some(balance::create_for_testing<IOTA>(100_000_000_000)),
        1,
        0,
        true,
        ctx,
    );
    validators.push_back(validator);
    create_iota_system_state_for_testing(validators, 1000, 0, ctx);

    scenario.next_tx(validator_addr);

    let mut system_state = scenario.take_shared<IotaSystemState>();

    // Test active validator metadata changes
    scenario.next_tx(validator_addr);
    {
        update_metadata(
            scenario,
            &mut system_state,
            b"validator_new_name",
            pubkey1,
            pop1,
            b"/ip4/42.42.42.42/tcp/80",
            b"/ip4/43.43.43.43/udp/80",
            vector[
                148,
                117,
                212,
                171,
                44,
                104,
                167,
                11,
                177,
                100,
                4,
                55,
                17,
                235,
                117,
                45,
                117,
                84,
                159,
                49,
                14,
                159,
                239,
                246,
                237,
                21,
                83,
                166,
                112,
                53,
                62,
                199,
            ],
            vector[
                215,
                64,
                85,
                185,
                231,
                116,
                69,
                151,
                97,
                79,
                4,
                183,
                20,
                70,
                84,
                51,
                211,
                162,
                115,
                221,
                73,
                241,
                240,
                171,
                192,
                25,
                232,
                106,
                175,
                162,
                176,
                43,
            ],
        );
    };

    scenario.next_tx(validator_addr);
    let validator = system_state.active_validator_by_address(validator_addr);
    verify_metadata(
        validator,
        b"validator_new_name",
        pubkey,
        pop,
        b"/ip4/127.0.0.1/tcp/80",
        b"/ip4/127.0.0.1/udp/80",
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[
            68,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
        pubkey1,
        pop1,
        b"/ip4/42.42.42.42/tcp/80",
        b"/ip4/43.43.43.43/udp/80",
        vector[
            148,
            117,
            212,
            171,
            44,
            104,
            167,
            11,
            177,
            100,
            4,
            55,
            17,
            235,
            117,
            45,
            117,
            84,
            159,
            49,
            14,
            159,
            239,
            246,
            237,
            21,
            83,
            166,
            112,
            53,
            62,
            199,
        ],
        vector[
            215,
            64,
            85,
            185,
            231,
            116,
            69,
            151,
            97,
            79,
            4,
            183,
            20,
            70,
            84,
            51,
            211,
            162,
            115,
            221,
            73,
            241,
            240,
            171,
            192,
            25,
            232,
            106,
            175,
            162,
            176,
            43,
        ],
    );

    test_scenario::return_shared(system_state);
    scenario_val.end();

    // Test pending validator metadata changes
    let mut scenario_val = test_scenario::begin(new_validator_addr);
    let scenario = &mut scenario_val;
    let mut system_state = scenario.take_shared<IotaSystemState>();
    scenario.next_tx(new_validator_addr);
    {
        let ctx = scenario.ctx();
        system_state.request_add_validator_candidate(
            new_pubkey,
            vector[
                33,
                219,
                38,
                23,
                242,
                109,
                116,
                235,
                225,
                192,
                219,
                45,
                40,
                124,
                162,
                25,
                33,
                68,
                52,
                41,
                123,
                9,
                98,
                11,
                184,
                150,
                214,
                62,
                60,
                210,
                121,
                62,
            ],
            vector[
                69,
                55,
                206,
                25,
                199,
                14,
                169,
                53,
                68,
                92,
                142,
                136,
                174,
                149,
                54,
                215,
                101,
                63,
                249,
                206,
                197,
                98,
                233,
                80,
                60,
                12,
                183,
                32,
                216,
                88,
                103,
                25,
            ],
            new_pop,
            b"ValidatorName2",
            b"description2",
            b"image_url2",
            b"project_url2",
            b"/ip4/127.0.0.2/tcp/80",
            b"/ip4/127.0.0.2/udp/80",
            b"/ip4/127.0.0.1/udp/80",
            1,
            0,
            ctx,
        );
        system_state.request_add_validator_for_testing(0, ctx);
    };

    scenario.next_tx(new_validator_addr);
    {
        update_metadata(
            scenario,
            &mut system_state,
            b"new_validator_new_name",
            new_pubkey1,
            new_pop1,
            b"/ip4/66.66.66.66/tcp/80",
            b"/ip4/77.77.77.77/udp/80",
            vector[
                215,
                65,
                85,
                185,
                231,
                116,
                69,
                151,
                97,
                79,
                4,
                183,
                20,
                70,
                84,
                51,
                211,
                162,
                115,
                221,
                73,
                241,
                240,
                171,
                192,
                25,
                232,
                106,
                175,
                162,
                176,
                43,
            ],
            vector[
                149,
                117,
                212,
                171,
                44,
                104,
                167,
                11,
                177,
                100,
                4,
                55,
                17,
                235,
                117,
                45,
                117,
                84,
                159,
                49,
                14,
                159,
                239,
                246,
                237,
                21,
                83,
                166,
                112,
                53,
                62,
                199,
            ],
        );
    };

    scenario.next_tx(new_validator_addr);
    let validator = system_state.pending_validator_by_address(new_validator_addr);
    verify_metadata(
        validator,
        b"new_validator_new_name",
        new_pubkey,
        new_pop,
        b"/ip4/127.0.0.2/tcp/80",
        b"/ip4/127.0.0.2/udp/80",
        vector[
            33,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[
            69,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
        new_pubkey1,
        new_pop1,
        b"/ip4/66.66.66.66/tcp/80",
        b"/ip4/77.77.77.77/udp/80",
        vector[
            215,
            65,
            85,
            185,
            231,
            116,
            69,
            151,
            97,
            79,
            4,
            183,
            20,
            70,
            84,
            51,
            211,
            162,
            115,
            221,
            73,
            241,
            240,
            171,
            192,
            25,
            232,
            106,
            175,
            162,
            176,
            43,
        ],
        vector[
            149,
            117,
            212,
            171,
            44,
            104,
            167,
            11,
            177,
            100,
            4,
            55,
            17,
            235,
            117,
            45,
            117,
            84,
            159,
            49,
            14,
            159,
            239,
            246,
            237,
            21,
            83,
            166,
            112,
            53,
            62,
            199,
        ],
    );

    test_scenario::return_shared(system_state);

    // Advance epoch to effectuate the metadata changes.
    scenario.next_tx(new_validator_addr);
    advance_epoch(scenario);

    // Now both validators are active, verify their metadata.
    scenario.next_tx(new_validator_addr);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    let validator = system_state.active_validator_by_address(validator_addr);
    verify_metadata_after_advancing_epoch(
        validator,
        b"validator_new_name",
        pubkey1,
        pop1,
        b"/ip4/42.42.42.42/tcp/80",
        b"/ip4/43.43.43.43/udp/80",
        vector[
            148,
            117,
            212,
            171,
            44,
            104,
            167,
            11,
            177,
            100,
            4,
            55,
            17,
            235,
            117,
            45,
            117,
            84,
            159,
            49,
            14,
            159,
            239,
            246,
            237,
            21,
            83,
            166,
            112,
            53,
            62,
            199,
        ],
        vector[
            215,
            64,
            85,
            185,
            231,
            116,
            69,
            151,
            97,
            79,
            4,
            183,
            20,
            70,
            84,
            51,
            211,
            162,
            115,
            221,
            73,
            241,
            240,
            171,
            192,
            25,
            232,
            106,
            175,
            162,
            176,
            43,
        ],
    );

    let validator = system_state.active_validator_by_address(new_validator_addr);
    verify_metadata_after_advancing_epoch(
        validator,
        b"new_validator_new_name",
        new_pubkey1,
        new_pop1,
        b"/ip4/66.66.66.66/tcp/80",
        b"/ip4/77.77.77.77/udp/80",
        vector[
            215,
            65,
            85,
            185,
            231,
            116,
            69,
            151,
            97,
            79,
            4,
            183,
            20,
            70,
            84,
            51,
            211,
            162,
            115,
            221,
            73,
            241,
            240,
            171,
            192,
            25,
            232,
            106,
            175,
            162,
            176,
            43,
        ],
        vector[
            149,
            117,
            212,
            171,
            44,
            104,
            167,
            11,
            177,
            100,
            4,
            55,
            17,
            235,
            117,
            45,
            117,
            84,
            159,
            49,
            14,
            159,
            239,
            246,
            237,
            21,
            83,
            166,
            112,
            53,
            62,
            199,
        ],
    );

    test_scenario::return_shared(system_state);
    scenario_val.end();
}

#[test]
fun test_validator_candidate_update() {
    let validator_addr = @0xaf76afe6f866d8426d2be85d6ef0b11f871a251d043b2f11e15563bf418f5a5a;
    // pubkey generated with authority key on seed [0; 32]
    let pubkey =
        x"99f25ef61f8032b914636460982c5cc6f134ef1ddae76657f2cbfec1ebfc8d097374080df6fcf0dcb8bc4b0d8e0af5d80ebbff2b4c599f54f42d6312dfc314276078c1cc347ebbbec5198be258513f386b930d02c2749a803e2330955ebd1a10";
    // pop generated using the authority key and address with [fn test_proof_of_possession]
    let pop =
        x"b01cc86f421beca7ab4cfca87c0799c4d038c199dd399fbec1924d4d4367866dba9e84d514710b91feb65316e4ceef43";

    // pubkey generated with authority key on seed [1; 32]
    let pubkey1 =
        x"96d19c53f1bee2158c3fcfb5bb2f06d3a8237667529d2d8f0fbb22fe5c3b3e64748420b4103674490476d98530d063271222d2a59b0f7932909cc455a30f00c69380e6885375e94243f7468e9563aad29330aca7ab431927540e9508888f0e1c";
    let pop1 =
        x"a8a0bcaf04e13565914eb22fa9f27a76f297db04446860ee2b923d10224cedb130b30783fb60b12556e7fc50e5b57a86";

    let mut scenario_val = test_scenario::begin(validator_addr);
    let scenario = &mut scenario_val;

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3]);
    scenario.next_tx(validator_addr);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    scenario.next_tx(validator_addr);
    {
        system_state.request_add_validator_candidate_for_testing(
            pubkey,
            vector[
                215,
                64,
                85,
                185,
                231,
                116,
                69,
                151,
                97,
                79,
                4,
                183,
                20,
                70,
                84,
                51,
                211,
                162,
                115,
                221,
                73,
                241,
                240,
                171,
                192,
                25,
                232,
                106,
                175,
                162,
                176,
                43,
            ],
            vector[
                148,
                117,
                212,
                171,
                44,
                104,
                167,
                11,
                177,
                100,
                4,
                55,
                17,
                235,
                117,
                45,
                117,
                84,
                159,
                49,
                14,
                159,
                239,
                246,
                237,
                21,
                83,
                166,
                112,
                53,
                62,
                199,
            ],
            pop,
            b"ValidatorName2",
            b"description2",
            b"image_url2",
            b"project_url2",
            b"/ip4/127.0.0.2/tcp/80",
            b"/ip4/127.0.0.2/udp/80",
            b"/ip4/168.168.168.168/udp/80",
            1,
            0,
            scenario.ctx(),
        );
    };

    scenario.next_tx(validator_addr);
    update_candidate(
        scenario,
        &mut system_state,
        b"validator_new_name",
        pubkey1,
        pop1,
        b"/ip4/42.42.42.42/tcp/80",
        b"/ip4/43.43.43.43/udp/80",
        7,
    );

    scenario.next_tx(validator_addr);

    let validator = system_state.candidate_validator_by_address(validator_addr);
    verify_candidate(
        validator,
        b"validator_new_name",
        pubkey1,
        pop1,
        b"/ip4/42.42.42.42/tcp/80",
        b"/ip4/43.43.43.43/udp/80",
        7,
    );

    test_scenario::return_shared(system_state);
    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = validator::EMetadataInvalidProtocolPubkey)]
fun test_add_validator_candidate_failure_invalid_metadata() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    // Generated using [fn test_proof_of_possession]
    let new_validator_addr = @0x8e3446145b0c7768839d71840df389ffa3b9742d0baaff326a3d453b595f87d7;
    let pubkey =
        x"99f25ef61f8032b914636460982c5cc6f134ef1ddae76657f2cbfec1ebfc8d097374080df6fcf0dcb8bc4b0d8e0af5d80ebbff2b4c599f54f42d6312dfc314276078c1cc347ebbbec5198be258513f386b930d02c2749a803e2330955ebd1a10";
    let pop =
        x"83809369ce6572be211512d85621a075ee6a8da57fbb2d867d05e6a395e71f10e4e957796944d68a051381eb91720fba";

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3]);
    scenario.next_tx(new_validator_addr);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    system_state.request_add_validator_candidate(
        pubkey,
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[42], // invalid
        pop,
        b"ValidatorName2",
        b"description2",
        b"image_url2",
        b"project_url2",
        b"/ip4/127.0.0.2/tcp/80",
        b"/ip4/127.0.0.2/udp/80",
        b"/ip4/127.0.0.1/udp/80",
        1,
        0,
        scenario.ctx(),
    );
    test_scenario::return_shared(system_state);
    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = validator_set::EAlreadyValidatorCandidate)]
fun test_add_validator_candidate_failure_double_register() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;
    let new_validator_addr = @0x8e3446145b0c7768839d71840df389ffa3b9742d0baaff326a3d453b595f87d7;
    let pubkey =
        x"99f25ef61f8032b914636460982c5cc6f134ef1ddae76657f2cbfec1ebfc8d097374080df6fcf0dcb8bc4b0d8e0af5d80ebbff2b4c599f54f42d6312dfc314276078c1cc347ebbbec5198be258513f386b930d02c2749a803e2330955ebd1a10";
    let pop =
        x"83809369ce6572be211512d85621a075ee6a8da57fbb2d867d05e6a395e71f10e4e957796944d68a051381eb91720fba";

    set_up_iota_system_state(vector[@0x1, @0x2, @0x3]);
    scenario.next_tx(new_validator_addr);
    let mut system_state = scenario.take_shared<IotaSystemState>();
    system_state.request_add_validator_candidate(
        pubkey,
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[
            68,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
        pop,
        b"ValidatorName2",
        b"description2",
        b"image_url2",
        b"project_url2",
        b"/ip4/127.0.0.2/tcp/80",
        b"/ip4/127.0.0.2/udp/80",
        b"/ip4/127.0.0.1/udp/80",
        1,
        0,
        scenario.ctx(),
    );

    // Add the same address as candidate again, should fail this time.
    system_state.request_add_validator_candidate(
        pubkey,
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[
            68,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
        pop,
        b"ValidatorName2",
        b"description2",
        b"image_url2",
        b"project_url2",
        b"/ip4/127.0.0.2/tcp/80",
        b"/ip4/127.0.0.2/udp/80",
        b"/ip4/127.0.0.1/udp/80",
        1,
        0,
        scenario.ctx(),
    );
    test_scenario::return_shared(system_state);
    scenario_val.end();
}

#[test]
#[expected_failure(abort_code = validator_set::EDuplicateValidator)]
fun test_add_validator_candidate_failure_duplicate_with_active() {
    let validator_addr = @0xaf76afe6f866d8426d2be85d6ef0b11f871a251d043b2f11e15563bf418f5a5a;
    // Seed [0; 32]
    let pubkey =
        x"99f25ef61f8032b914636460982c5cc6f134ef1ddae76657f2cbfec1ebfc8d097374080df6fcf0dcb8bc4b0d8e0af5d80ebbff2b4c599f54f42d6312dfc314276078c1cc347ebbbec5198be258513f386b930d02c2749a803e2330955ebd1a10";
    let pop =
        x"b01cc86f421beca7ab4cfca87c0799c4d038c199dd399fbec1924d4d4367866dba9e84d514710b91feb65316e4ceef43";

    let new_addr = @0x1a4623343cd42be47d67314fce0ad042f3c82685544bc91d8c11d24e74ba7357;
    // Seed [1; 32]
    let new_pubkey =
        x"96d19c53f1bee2158c3fcfb5bb2f06d3a8237667529d2d8f0fbb22fe5c3b3e64748420b4103674490476d98530d063271222d2a59b0f7932909cc455a30f00c69380e6885375e94243f7468e9563aad29330aca7ab431927540e9508888f0e1c";
    let new_pop =
        x"932336c35a8c393019c63eb0f7d385dd4e0bd131f04b54cf45aa9544f14dca4dab53bd70ffcb8e0b34656e4388309720";

    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;

    // Set up IotaSystemState with an active validator
    let ctx = scenario.ctx();
    let validator = validator::new_for_testing(
        validator_addr,
        pubkey,
        vector[
            32,
            219,
            38,
            23,
            242,
            109,
            116,
            235,
            225,
            192,
            219,
            45,
            40,
            124,
            162,
            25,
            33,
            68,
            52,
            41,
            123,
            9,
            98,
            11,
            184,
            150,
            214,
            62,
            60,
            210,
            121,
            62,
        ],
        vector[
            68,
            55,
            206,
            25,
            199,
            14,
            169,
            53,
            68,
            92,
            142,
            136,
            174,
            149,
            54,
            215,
            101,
            63,
            249,
            206,
            197,
            98,
            233,
            80,
            60,
            12,
            183,
            32,
            216,
            88,
            103,
            25,
        ],
        pop,
        b"ValidatorName",
        b"description",
        b"image_url",
        b"project_url",
        b"/ip4/127.0.0.1/tcp/80",
        b"/ip4/127.0.0.1/udp/80",
        b"/ip4/127.0.0.1/udp/80",
        option::some(balance::create_for_testing<IOTA>(100_000_000_000)),
        1,
        0,
        true,
        ctx,
    );
    create_iota_system_state_for_testing(vector[validator], 1000, 0, ctx);

    scenario.next_tx(new_addr);

    let mut system_state = scenario.take_shared<IotaSystemState>();

    // Add a candidate with the same name. Fails due to duplicating with an already active validator.
    system_state.request_add_validator_candidate(
        new_pubkey,
        vector[
            115,
            220,
            238,
            151,
            134,
            159,
            173,
            41,
            80,
            2,
            66,
            196,
            61,
            17,
            191,
            76,
            103,
            39,
            246,
            127,
            171,
            85,
            19,
            235,
            210,
            106,
            97,
            97,
            116,
            48,
            244,
            191,
        ],
        vector[
            149,
            128,
            161,
            13,
            11,
            183,
            96,
            45,
            89,
            20,
            188,
            205,
            26,
            127,
            147,
            254,
            184,
            229,
            184,
            102,
            64,
            170,
            104,
            29,
            191,
            171,
            91,
            99,
            58,
            178,
            41,
            156,
        ],
        new_pop,
        // same name
        b"ValidatorName",
        b"description2",
        b"image_url2",
        b"project_url2",
        b"/ip4/127.0.0.2/tcp/80",
        b"/ip4/127.0.0.2/udp/80",
        b"/ip4/127.0.0.1/udp/80",
        1,
        0,
        scenario.ctx(),
    );
    test_scenario::return_shared(system_state);
    scenario_val.end();
}

#[test]
fun test_withdraw_inactive_stake() {
    let mut scenario_val = test_scenario::begin(@0x0);
    let scenario = &mut scenario_val;
    // Epoch duration is set to be 42 here.
    set_up_iota_system_state(vector[@0x1, @0x2]);

    {
        scenario.next_tx(@0x0);
        let mut system_state = scenario.take_shared<IotaSystemState>();
        let staking_pool = system_state.active_validator_by_address(@0x1).get_staking_pool_ref();

        assert!(staking_pool.pending_stake_amount() == 0, 0);
        assert!(staking_pool.pending_stake_withdraw_amount() == 0, 0);
        assert!(staking_pool.iota_balance() == 100 * 1_000_000_000, 0);

        test_scenario::return_shared(system_state);
    };

    stake_with(@0x0, @0x1, 1, scenario);

    {
        scenario.next_tx(@0x0);
        let mut system_state = scenario.take_shared<IotaSystemState>();
        let staking_pool = system_state.active_validator_by_address(@0x1).get_staking_pool_ref();

        assert!(staking_pool.pending_stake_amount() == 1_000_000_000, 0);
        assert!(staking_pool.pending_stake_withdraw_amount() == 0, 0);
        assert!(staking_pool.iota_balance() == 100 * 1_000_000_000, 0);

        test_scenario::return_shared(system_state);
    };

    unstake(@0x0, 0, scenario);

    {
        scenario.next_tx(@0x0);
        let mut system_state = scenario.take_shared<IotaSystemState>();
        let staking_pool = system_state.active_validator_by_address(@0x1).get_staking_pool_ref();

        assert!(staking_pool.pending_stake_amount() == 0, 0);
        assert!(staking_pool.pending_stake_withdraw_amount() == 0, 0);
        assert!(staking_pool.iota_balance() == 100 * 1_000_000_000, 0);

        test_scenario::return_shared(system_state);
    };

    scenario_val.end();
}
