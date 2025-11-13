/// Unit tests for OFT functions that don't rely on external packages.
///
/// This test module covers:
/// - OFT creation and initialization
/// - Decimal conversion functions (to_ld, remove_dust)
/// - Debit view functionality
/// - Version information
/// - Accessor functions for internal components
/// - Edge cases and error conditions
#[test_only]
module oft::oft_tests;

use call::call_cap;
use endpoint_v2::endpoint_v2::{Self, AdminCap as EndpointAdminCap, EndpointV2};
use oapp::oapp::{Self, AdminCap, OApp};
use oft::{oft::{Self, OFT}};
use std::u64;
use iota::{coin::Coin, test_scenario::{Self, Scenario}, test_utils, deny_list::{Self, DenyList}};
use regulated_coin::regulated_coin::{Self, REGULATED_COIN, Treasury, AdminCap as RegulatedAdminCap};

// === Test Constants ===

const ALICE: address = @0xa11ce;
const EID: u32 = 101;

// === Error Codes ===

const E_INVALID_VERSION: u64 = 0;
const E_INVALID_CONVERSION_RATE: u64 = 1;
const E_INVALID_DUST_REMOVAL: u64 = 2;
const E_INVALID_DEBIT_VIEW_SENT: u64 = 3;
const E_INVALID_DEBIT_VIEW_RECEIVED: u64 = 4;

// === OFT Creation Tests ===

#[test]
fun test_create_oft_edge_local_decimals() {
    let mut scenario = test_scenario::begin(ALICE);

    // Test with shared decimals equal to local decimals - should pass
    let local_decimals = 6u8;
    let shared_decimals = 6u8;

    let endpoint = setup_endpoint(&mut scenario, EID);

    // Create regulated coin for testing
    let (treasury, admin_cap_rc) = regulated_coin::init_for_testing_with_decimals(local_decimals, scenario.ctx());
    transfer::public_share_object(treasury);
    transfer::public_transfer(admin_cap_rc, ALICE);

    scenario.next_tx(ALICE);
    let mut treasury = scenario.take_shared<Treasury>();
    let admin_cap_rc = scenario.take_from_sender<RegulatedAdminCap>();


    // Create SupplyManagerCap
    let supply_manager_cap = regulated_coin::new_supply_manager(&mut treasury, &admin_cap_rc, scenario.ctx());

    // Get coin metadata
    let coin_metadata = regulated_coin::borrow_metadata(&treasury);

    // Create OApp and CallCap for the OFT
    let oft_cap = call_cap::new_package_cap_for_test(scenario.ctx());
    let admin_cap_oft = oapp::create_admin_cap_for_test(scenario.ctx());
    let oapp = oapp::create_oapp_for_test(&oft_cap, &admin_cap_oft, scenario.ctx());

    // This should pass - shared decimals equal to local decimals
    let (oft, migration_cap) = oft::init_oft_for_test(
        &oapp,
        oft_cap,
        supply_manager_cap,
        coin_metadata,
        shared_decimals,
        scenario.ctx(),
    );

    // Clean up
    transfer::public_transfer(migration_cap, ALICE);
    transfer::public_transfer(admin_cap_rc, ALICE);
    test_scenario::return_shared<Treasury>(treasury);
    test_utils::destroy(oapp);
    test_utils::destroy(oft);
    test_utils::destroy(admin_cap_oft);
    test_utils::destroy(endpoint);
    scenario.end();
}

#[test]
#[expected_failure(abort_code = oft::EInvalidLocalDecimals)]
fun test_create_oft_invalid_local_decimals() {
    let mut scenario = test_scenario::begin(ALICE);

    // Test with shared decimals greater than local decimals - should fail
    let local_decimals = 6u8;
    let shared_decimals = 7u8;

    let endpoint = setup_endpoint(&mut scenario, EID);

    // Create regulated coin for testing
    let (treasury, admin_cap_rc) = regulated_coin::init_for_testing_with_decimals(local_decimals, scenario.ctx());
    transfer::public_share_object(treasury);
    transfer::public_transfer(admin_cap_rc, ALICE);

    scenario.next_tx(ALICE);
    let mut treasury = scenario.take_shared<Treasury>();
    let admin_cap_rc = scenario.take_from_sender<RegulatedAdminCap>();

    // Create SupplyManagerCap
    let supply_manager_cap = regulated_coin::new_supply_manager(&mut treasury, &admin_cap_rc, scenario.ctx());

    // Transfer the regulated coin AdminCap away since we don't need it anymore
    transfer::public_transfer(admin_cap_rc, ALICE);

    // Get coin metadata
    let coin_metadata = regulated_coin::borrow_metadata(&treasury);

    // Create OApp and CallCap for the OFT
    let oft_cap = call_cap::new_package_cap_for_test(scenario.ctx());
    let admin_cap_oft = oapp::create_admin_cap_for_test(scenario.ctx());
    let oapp = oapp::create_oapp_for_test(&oft_cap, &admin_cap_oft, scenario.ctx());

    // This should fail due to invalid decimals
    let (oft, migration_cap) = oft::init_oft_for_test(
        &oapp,
        oft_cap,
        supply_manager_cap,
        coin_metadata,
        shared_decimals,
        scenario.ctx(),
    );

    // This line should never be reached due to the expected failure
    oapp::share_oapp_for_test(oapp);
    oft::share_oft_for_test(oft);
    transfer::public_transfer(admin_cap_oft, ALICE);
    transfer::public_transfer(migration_cap, ALICE);
    test_scenario::return_shared<Treasury>(treasury);

    // Note: oft_cap was consumed by create_oft
    test_utils::destroy(endpoint);
    scenario.end();
}

// === Version Tests ===

#[test]
fun test_oft_version() {
    let mut scenario = test_scenario::begin(ALICE);
    let ctx = setup_oft_with_defaults(&mut scenario, 6);

    let (version_hash, version_number) = oft::oft_version(&ctx.oft);

    // Verify expected version values
    assert!(version_hash == 1, E_INVALID_VERSION);
    assert!(version_number == 1, E_INVALID_VERSION);

    cleanup_oft_context(ctx);
    scenario.end();
}

// === Decimal Conversion Tests ===

#[test]
fun test_to_ld_conversion() {
    let mut scenario = test_scenario::begin(ALICE);
    // Create OFT with 18 local decimals and 6 shared decimals
    // Conversion rate should be 10^(18-6) = 10^12
    let ctx = setup_oft_with_defaults(&mut scenario, 6);

    // Test conversion from shared decimals to local decimals
    let amount_sd = 1000000u64; // 1 token in shared decimals (6 decimals)
    let expected_ld = 1000000000000000000u64; // 1 token in local decimals (18 decimals)

    let actual_ld = oft::to_ld_for_test(&ctx.oft, amount_sd);
    assert!(actual_ld == expected_ld, E_INVALID_CONVERSION_RATE);

    // Test with zero
    let zero_ld = oft::to_ld_for_test(&ctx.oft, 0);
    assert!(zero_ld == 0, E_INVALID_CONVERSION_RATE);

    // Test with larger amounts
    let expected_large_ld = oft::remove_dust_for_test(&ctx.oft, u64::max_value!());
    let large_amount_sd = oft::to_sd_for_test(&ctx.oft, expected_large_ld);
    let actual_large_ld = oft::to_ld_for_test(&ctx.oft, large_amount_sd);
    assert!(actual_large_ld == expected_large_ld, E_INVALID_CONVERSION_RATE);

    cleanup_oft_context(ctx);
    scenario.end();
}

#[test]
fun test_remove_dust() {
    let mut scenario = test_scenario::begin(ALICE);
    // Create OFT with 18 local decimals and 6 shared decimals
    // Conversion rate = 10^12
    let ctx = setup_oft_with_defaults(&mut scenario, 6);

    // Test dust removal - should round down to nearest convertible unit
    let dusty_amount = 1000000000000000123u64; // Has dust in last 12 digits
    let expected_clean = 1000000000000000000u64; // Rounded down

    let actual_clean = oft::remove_dust_for_test(&ctx.oft, dusty_amount);
    assert!(actual_clean == expected_clean, E_INVALID_DUST_REMOVAL);

    // Test with exact amount (no dust)
    let exact_amount = 1000000000000000000u64;
    let clean_exact = oft::remove_dust_for_test(&ctx.oft, exact_amount);
    assert!(clean_exact == exact_amount, E_INVALID_DUST_REMOVAL);

    // Test with zero
    let zero_clean = oft::remove_dust_for_test(&ctx.oft, 0);
    assert!(zero_clean == 0, E_INVALID_DUST_REMOVAL);

    // Test with amount less than conversion rate
    let small_amount = 500000000000u64; // Less than 10^12
    let small_clean = oft::remove_dust_for_test(&ctx.oft, small_amount);
    assert!(small_clean == 0, E_INVALID_DUST_REMOVAL);

    cleanup_oft_context(ctx);
    scenario.end();
}

// === Debit View Tests ===

#[test]
fun test_debit_view_zero_amount() {
    let mut scenario = test_scenario::begin(ALICE);
    let ctx = setup_oft_with_defaults(&mut scenario, 6);

    let (amount_sent_ld, amount_received_ld) = oft::debit_view_for_test(
        &ctx.oft,
        EID,
        0,
        0,
    );

    assert!(amount_sent_ld == 0, E_INVALID_DEBIT_VIEW_SENT);
    assert!(amount_received_ld == 0, E_INVALID_DEBIT_VIEW_RECEIVED);

    cleanup_oft_context(ctx);
    scenario.end();
}

#[test]
fun test_debit_view_exact_amount() {
    let mut scenario = test_scenario::begin(ALICE);
    let ctx = setup_oft_with_defaults(&mut scenario, 6);

    // Test with exact amount (no dust)
    let exact_amount = 2000000000000000000u64; // 2 tokens exactly

    let (amount_sent_ld, amount_received_ld) = oft::debit_view_for_test(
        &ctx.oft,
        EID,
        exact_amount,
        exact_amount,
    );

    assert!(amount_sent_ld == exact_amount, E_INVALID_DEBIT_VIEW_SENT);
    assert!(amount_received_ld == exact_amount, E_INVALID_DEBIT_VIEW_RECEIVED);

    cleanup_oft_context(ctx);
    scenario.end();
}

// === Fee Tests ===

#[test]
fun test_fee_recipient_receives_fee() {
    let mut scenario = test_scenario::begin(ALICE);
    let mut ctx = setup_oft_with_defaults(&mut scenario, 6);

    // Set up fee recipient
    let fee_recipient = @0xfee;
    let fee_bps = 500u64; // 5% fee

    // Configure the OFT with fee settings
    {
        let admin_cap = &ctx.admin_cap;
        let oft = &mut ctx.oft;
        oft::set_fee_bps(oft, admin_cap, EID, fee_bps);
        oft::set_fee_deposit_address(oft, admin_cap, fee_recipient);
    };

    // Create initial coin balance
    let initial_amount = 1000000000000000000u64; // 1 token in 18 decimals
    let OFTTestContext { oft: oft_ref, treasury: treasury_ref, deny_list: deny_list_ref, .. } = &mut ctx;
    let mut coin = oft::mint_for_testing(oft_ref, initial_amount,  treasury_ref, deny_list_ref, scenario.ctx());

    // Calculate expected amounts
    let amount_to_send = 500000000000000000u64; // 0.5 tokens
    let expected_fee = (((amount_to_send as u128) * (fee_bps as u128)) / 10000u128) as u64; // 5% fee
    let expected_received = amount_to_send - expected_fee;
    let expected_dust_removed = oft::remove_dust_for_test(oft_ref, expected_received);

    // Record initial balances
    let initial_coin_balance = coin.value();

    // Perform debit operation
    let (amount_sent_ld, amount_received_ld) = oft::debit_for_test(
        oft_ref,
        &mut coin,
        EID,
        amount_to_send,
        0, // min_amount_ld = 0 for this test
        treasury_ref,
        deny_list_ref,
        scenario.ctx(),
    );

    // Verify the returned amounts
    assert!(amount_sent_ld == amount_to_send, 100);
    assert!(amount_received_ld == expected_dust_removed, 101);

    // Verify coin balance was reduced by the sent amount
    assert!(coin.value() == initial_coin_balance - amount_to_send, 102);

    scenario.next_tx(fee_recipient);

    // Check if fee recipient received the fee
    let fee_recipient_balance = test_scenario::take_from_address<Coin<REGULATED_COIN>>(&scenario, fee_recipient);
    assert!(fee_recipient_balance.value() == amount_sent_ld - amount_received_ld, 103);

    test_scenario::return_to_address(fee_recipient, fee_recipient_balance);
    cleanup_oft_context(ctx);
    test_utils::destroy(coin);
    scenario.end();
}

// === Edge Case Tests ===

#[test]
fun test_different_decimal_configurations() {
    let mut scenario = test_scenario::begin(ALICE);

    // Test various valid decimal configurations

    // Configuration 1: 9 local, 6 shared (rate = 1000)
    let ctx1 = setup_oft_with_decimals(&mut scenario, 9, 6);
    let amount_1_ld = oft::to_ld_for_test(&ctx1.oft, 1000000u64); // 1 token in shared decimals
    assert!(amount_1_ld == 1000000000u64, E_INVALID_CONVERSION_RATE); // 1 token in 9 decimals
    cleanup_oft_context(ctx1);

    // Configuration 2: 12 local, 6 shared (rate = 1,000,000)
    let ctx2 = setup_oft_with_decimals(&mut scenario, 12, 6);
    let amount_2_ld = oft::to_ld_for_test(&ctx2.oft, 1000000u64);
    assert!(amount_2_ld == 1000000000000u64, E_INVALID_CONVERSION_RATE); // 1 token in 12 decimals
    cleanup_oft_context(ctx2);

    // Configuration 3: 8 local, 6 shared (rate = 100)
    let ctx3 = setup_oft_with_decimals(&mut scenario, 8, 6);
    let amount_3_ld = oft::to_ld_for_test(&ctx3.oft, 1000000u64);
    assert!(amount_3_ld == 100000000u64, E_INVALID_CONVERSION_RATE); // 1 token in 8 decimals
    cleanup_oft_context(ctx3);

    scenario.end();
}

// === Helper functions ===

public fun setup_endpoint(scenario: &mut Scenario, eid: u32): EndpointV2 {
    endpoint_v2::init_for_test(scenario.ctx());
    scenario.next_tx(ALICE);
    let endpoint_admin_cap = scenario.take_from_sender<EndpointAdminCap>();
    let mut endpoint = scenario.take_shared<EndpointV2>();
    endpoint.init_eid(&endpoint_admin_cap, eid);

    scenario.return_to_sender<EndpointAdminCap>(endpoint_admin_cap);

    endpoint
}

/// Helper structure to hold OFT test context
public struct OFTTestContext {
    oapp: OApp,
    oft: OFT,
    admin_cap: AdminCap,
    endpoint: EndpointV2,
    treasury: Treasury,
    deny_list: DenyList,
}

/// Setup OFT with default 18 local decimals and specified shared decimals
public fun setup_oft_with_defaults(scenario: &mut Scenario, shared_decimals: u8): OFTTestContext {
    setup_oft_with_decimals(scenario, 18, shared_decimals)
}

/// Setup OFT with custom local and shared decimals
public fun setup_oft_with_decimals(
    scenario: &mut Scenario,
    local_decimals: u8,
    shared_decimals: u8,
): OFTTestContext {
    let endpoint = setup_endpoint(scenario, EID);
    // Create DenyList with system address (only once)
    scenario.next_tx(@0x0);
    deny_list::create_for_test(scenario.ctx());

    // Create regulated coin for testing
    let (treasury, admin_cap_rc) = regulated_coin::init_for_testing_with_decimals(local_decimals, scenario.ctx());

    transfer::public_share_object(treasury);
    // Transfer AdminCap to ALICE
    scenario.next_tx(@0x0);
    transfer::public_transfer(admin_cap_rc, ALICE);

    scenario.next_tx(ALICE);


    let mut treasury = scenario.take_shared<Treasury>();
    let admin_cap_rc = scenario.take_from_sender<RegulatedAdminCap>();
    let deny_list = scenario.take_shared<DenyList>();

    // Create SupplyManagerCap
    let supply_manager_cap = regulated_coin::new_supply_manager(&mut treasury, &admin_cap_rc, scenario.ctx());

    // Transfer the regulated coin AdminCap away since we don't need it anymore
    transfer::public_transfer(admin_cap_rc, ALICE);

    // Get coin metadata
    let coin_metadata = regulated_coin::borrow_metadata(&treasury);

    // Create OApp and CallCap for the OFT
    let oft_cap = call_cap::new_package_cap_for_test(scenario.ctx());
    let admin_cap = oapp::create_admin_cap_for_test(scenario.ctx());
    let oapp = oapp::create_oapp_for_test(&oft_cap, &admin_cap, scenario.ctx());

    let (oft, migration_cap) = oft::init_oft_for_test(
        &oapp,
        oft_cap,
        supply_manager_cap,
        coin_metadata,
        shared_decimals,
        scenario.ctx(),
    );
    transfer::public_transfer(migration_cap, ALICE);
    test_scenario::return_shared<DenyList>(deny_list);
    test_scenario::return_shared<Treasury>(treasury);

    scenario.next_tx(ALICE);
    let treasury = scenario.take_shared<Treasury>();
    let deny_list = scenario.take_shared<DenyList>();

    OFTTestContext {
        oapp,
        oft,
        admin_cap,
        endpoint,
        treasury,
        deny_list,
    }
}

/// Cleanup OFT test context
public fun cleanup_oft_context(ctx: OFTTestContext) {
    let OFTTestContext { oapp, oft, admin_cap, endpoint, treasury, deny_list } = ctx;
    test_utils::destroy(oapp);
    test_utils::destroy(oft);
    test_utils::destroy(admin_cap);
    test_utils::destroy(endpoint);
    // Return shared objects to their shared state
    transfer::public_share_object(treasury);
    test_utils::destroy(deny_list);
}
