#[test_only]
module regulated_coin::regulated_coin_tests;

use regulated_coin::{
    regulated_coin::{Self, REGULATED_COIN},
    treasury::{Self, Treasury, AdminCap, SupplyManagerCap}
};
use iota::{
    coin::{Coin, update_description, get_description},
    deny_list::{Self, DenyList},
    test_scenario,
    event,
};

#[test]
fun test_supply_manager_mint_and_burn() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let recipient_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    
    // Initialize the coin
    {
        regulated_coin::test_init(scenario.ctx());
    };
    
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Create SupplyManager
        let supply_manager_cap = treasury::new_supply_manager(
            &mut treasury,
            &admin_cap,
            scenario.ctx()
        );
        
        // Transfer SupplyManager cap to SupplyManager
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    
    // SupplyManager mints tokens
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        
        // Mint 100 tokens
        treasury::mint(
            &mut treasury,
            &supply_manager_cap,
            &deny_list,
            100,
            recipient_address,
            scenario.ctx()
        );
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    
    // Verify tokens were minted
    scenario.next_tx(recipient_address);
    {
        let coin = scenario.take_from_sender<Coin<REGULATED_COIN>>();
        assert!(coin.value() == 100, 0);
        scenario.return_to_sender(coin);
    };
    
    // Burn tokens
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let coin = scenario.take_from_address<Coin<REGULATED_COIN>>(recipient_address);
        let deny_list = scenario.take_shared<DenyList>();
        
        // Burn the coin
        treasury::burn(
            &mut treasury,
            &supply_manager_cap,
            &deny_list,
            coin,
            scenario.ctx()
        );
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    
    scenario.end();
}

#[test]
fun test_authorize_supply_manager_after_unauthorize() {
    let admin_address = @0xAA;
    let supply_manager_address = @0xBB;
    let recipient_address = @0xCC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    // Init coin
    scenario.next_tx(admin_address);
    { regulated_coin::test_init(scenario.ctx()); };

    // Create initial supply manager and transfer cap
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let sm_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(sm_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Unauthorize supply manager
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Re-authorize using authorize_supply_manager with the cap ID
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        // Temporarily take the cap from supply manager to read its ID
        let sm_cap = scenario.take_from_address<SupplyManagerCap<REGULATED_COIN>>(supply_manager_address);
    let sm_id = object::id(&sm_cap);
    treasury::authorize_supply_manager(&mut treasury, &admin_cap, sm_id);
        // Return objects
        transfer::public_transfer(sm_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Mint should now succeed again
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let sm_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        treasury::mint(&mut treasury, &sm_cap, &deny_list, 10, recipient_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(sm_cap);
        test_scenario::return_shared(deny_list);
    };

    // Verify minted amount
    scenario.next_tx(recipient_address);
    {
        let coin = scenario.take_from_sender<Coin<REGULATED_COIN>>();
        assert!(coin.value() == 10, 9000);
        scenario.return_to_sender(coin);
    };

    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ESupplyManagerEntryAlreadyExists)]
fun test_authorize_supply_manager_duplicate_fails() {
    let admin_address = @0xAD;
    let supply_manager_address = @0xBD;
    let sm_id: object::ID; // will capture SupplyManagerCap ID during creation

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    // Init coin
    scenario.next_tx(admin_address);
    { regulated_coin::test_init(scenario.ctx()); };

    // Create initial supply manager
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let sm_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        sm_id = object::id(&sm_cap);
        transfer::public_transfer(sm_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        // Attempt to authorize again while entry exists (should abort)
    };

    // Failing tx: try duplicate authorize
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        // Use previously recorded sm_id without moving the cap again (should abort)
        treasury::authorize_supply_manager(&mut treasury, &admin_cap, sm_id);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ESupplyManagerEntryAlreadyExists)]
fun test_reauthorize_old_after_new_fails() {
    let admin_address = @0xDE;
    let old_sm_address = @0xD1;
    let new_sm_address = @0xD2;
    let old_sm_id: object::ID; // capture first cap id

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    // Init coin
    scenario.next_tx(admin_address);
    { regulated_coin::test_init(scenario.ctx()); };

    // Create first supply manager (old)
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let sm_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        old_sm_id = object::id(&sm_cap);
        transfer::public_transfer(sm_cap, old_sm_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Unauthorize first supply manager
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Create second (new) supply manager; automatically authorized
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let sm_cap2 = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(sm_cap2, new_sm_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Attempt to re-authorize OLD supply manager (should fail because entry already exists for new one)
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        treasury::authorize_supply_manager(&mut treasury, &admin_cap, old_sm_id);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    scenario.end();
}

#[test]
fun test_events_emitted() {
    use regulated_coin::treasury::{MintEvent, BurnEvent, PauseEvent, DenyListChangeEvent};
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let recipient_address = @0xC;
    let deny_address = @0xD;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    // Init
    scenario.next_tx(admin_address);
    { 
        regulated_coin::test_init(scenario.ctx());
    };

    // Create supply manager
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let sm_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(sm_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Mint (expect MintEvent)
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let sm_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        treasury::mint(&mut treasury, &sm_cap, &deny_list, 100, recipient_address, scenario.ctx());
        assert!(event::num_events() == 1, 1000);
        let mints = event::events_by_type<MintEvent>();
        assert!(vector::length(&mints) == 1, 1001);
        assert!(treasury::mint_event_amount(&mints[0]) == 100, 1002);
        assert!(treasury::mint_event_recipient(&mints[0]) == recipient_address, 1003);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(sm_cap);
        test_scenario::return_shared(deny_list);
    };

    // Pause (PauseEvent enabled true)
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        treasury::pause_transfers(&mut treasury, &admin_cap, &mut deny_list, scenario.ctx());
        let pauses = event::events_by_type<PauseEvent>();
        assert!(vector::length(&pauses) == 1, 1100);
        assert!(treasury::pause_event_enabled(&pauses[0]), 1101);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };

    // Unpause (PauseEvent enabled false)
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        treasury::unpause_transfers(&mut treasury, &admin_cap, &mut deny_list, scenario.ctx());
        let pauses = event::events_by_type<PauseEvent>();
        assert!(vector::length(&pauses) == 1, 1200);
        assert!(!treasury::pause_event_enabled(&pauses[0]), 1201);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };

    // Block address (DenyListChangeEvent added true)
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        treasury::block_address(&mut treasury, &admin_cap, &mut deny_list, deny_address, scenario.ctx());
        let changes = event::events_by_type<DenyListChangeEvent>();
        assert!(vector::length(&changes) == 1, 1300);
        assert!(treasury::deny_list_change_event_address(&changes[0]) == deny_address, 1301);
        assert!(treasury::deny_list_change_event_added(&changes[0]), 1302);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };

    // Unblock address (DenyListChangeEvent added false)
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        treasury::unblock_address(&mut treasury, &admin_cap, &mut deny_list, deny_address, scenario.ctx());
        let changes = event::events_by_type<DenyListChangeEvent>();
        assert!(vector::length(&changes) == 1, 1400);
        assert!(treasury::deny_list_change_event_address(&changes[0]) == deny_address, 1401);
        assert!(!treasury::deny_list_change_event_added(&changes[0]), 1402);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };

    // Burn (BurnEvent)
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let sm_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let coin = scenario.take_from_address<Coin<REGULATED_COIN>>(recipient_address);
        let deny_list = scenario.take_shared<DenyList>();
        let amount = coin.value();
        treasury::burn(&mut treasury, &sm_cap, &deny_list, coin, scenario.ctx());
        let burns = event::events_by_type<BurnEvent>();
        assert!(vector::length(&burns) == 1, 1500);
        assert!(treasury::burn_event_amount(&burns[0]) == amount, 1501);
        assert!(treasury::burn_event_actor(&burns[0]) == supply_manager_address, 1502);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(sm_cap);
        test_scenario::return_shared(deny_list);
    };

    scenario.end();
}

#[test]
fun test_total_supply() {
    let admin_address = @0x1;
    let supply_manager_address = @0x2;
    let recipient_address = @0x3;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    // Init coin
    scenario.next_tx(admin_address);
    { regulated_coin::test_init(scenario.ctx()); };

    // Authorize supply manager
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let sm_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(sm_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    // Record supply before
    let supply_before;
    scenario.next_tx(admin_address);
    {
        let treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let cap_ref = treasury::borrow_treasury_cap(&treasury);
        supply_before = iota::coin::total_supply(cap_ref);
        test_scenario::return_shared(treasury);
    };

    // Mint in supply manager tx
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let sm_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        treasury::mint(&mut treasury, &sm_cap, &deny_list, 200, recipient_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(sm_cap);
        test_scenario::return_shared(deny_list);
    };

    // Check supply after mint
    scenario.next_tx(admin_address);
    {
        let treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let cap_ref = treasury::borrow_treasury_cap(&treasury);
        let supply_after = iota::coin::total_supply(cap_ref);
        // minted 200
        assert!(supply_after == supply_before + 200, 42);
        test_scenario::return_shared(treasury);
    };

    // Burn part of minted coins
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let sm_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let coin = scenario.take_from_address<Coin<REGULATED_COIN>>(recipient_address);
        let deny_list = scenario.take_shared<DenyList>();
        // Split coin to burn 50, leave remainder with recipient
        let mut remaining = coin;
        let burn_part = iota::coin::split(&mut remaining, 50, scenario.ctx());
        // Return remainder to recipient
        transfer::public_transfer(remaining, recipient_address);
        treasury::burn(&mut treasury, &sm_cap, &deny_list, burn_part, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(sm_cap);
        test_scenario::return_shared(deny_list);
    };

    // Check supply after mint and burn
    scenario.next_tx(admin_address);
    {
        let treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let cap_ref = treasury::borrow_treasury_cap(&treasury);
        let supply_after = iota::coin::total_supply(cap_ref);
        // minted 200, burned 50 => net +150
        assert!(supply_after == supply_before + 150, 42);
        test_scenario::return_shared(treasury);
    };

    scenario.end();
}

#[test]
fun test_borrow_treasury_cap_total_supply() {
    let admin_address = @0xA;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };

    scenario.next_tx(admin_address);
    {
        let treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let cap_ref = treasury::borrow_treasury_cap(&treasury);
        let supply = iota::coin::total_supply(cap_ref);
        // For a freshly initialized coin, supply should be 0
        assert!(supply == 0, 0);
        test_scenario::return_shared(treasury);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::EZeroAmount)]
fun test_mint_zero_amount() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let recipient_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    
    {
        regulated_coin::test_init(scenario.ctx());
    };
    
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        let supply_manager_cap = treasury::new_supply_manager(
            &mut treasury,
            &admin_cap,
            scenario.ctx()
        );
        
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        
        // Try to mint 0 tokens (should fail)
        treasury::mint(
            &mut treasury,
            &supply_manager_cap,
            &deny_list,
            0,
            recipient_address,
            scenario.ctx()
        );
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    
    scenario.end();
}

#[test]
fun test_block_unblock_address() {
    let admin_address = @0xA;
    let blocked_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };

    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        // Block address
        treasury::block_address(&mut treasury, &admin_cap, &mut deny_list, blocked_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    // Advance epoch to apply block for receiving
    scenario.next_epoch(@0);
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        // Unblock address
        treasury::unblock_address(&mut treasury, &admin_cap, &mut deny_list, blocked_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::EDeniedAddress)]
fun test_blocked_address_cannot_mint() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let blocked_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        treasury::block_address(&mut treasury, &admin_cap, &mut deny_list, blocked_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.next_epoch(@0);
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap,  scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        // Try to mint to blocked address (should fail)
        treasury::mint(&mut treasury, &supply_manager_cap, &deny_list, 10, blocked_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test]
fun test_blocked_address_cannot_transfer() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let blocked_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap,  scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        // Mint to blocked_address (not blocked yet)
        treasury::mint(&mut treasury, &supply_manager_cap, &deny_list, 10, blocked_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    // Now block the address
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        treasury::block_address(&mut treasury, &admin_cap, &mut deny_list, blocked_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.next_epoch(@0);
    // Blocked address tries to transfer coins (should fail)
    scenario.next_tx(blocked_address);
    {
        let deny_list = scenario.take_shared<DenyList>();
        assert!(iota::coin::deny_list_v1_contains_current_epoch<REGULATED_COIN>(
            &deny_list,
            blocked_address,
            scenario.ctx()
        ), 1337);
        test_scenario::return_shared(deny_list);
        // The DenyList is enforced by the validators and doesn't work in the test scenario, but in a real network the following would fail:
        // let coin = scenario.take_from_sender<Coin<REGULATED_COIN>>();
        // // Try to transfer coin to some address
        // transfer::public_transfer(coin, admin_address);
    };
    scenario.end();
}

#[test]
fun test_pause_unpause_transfers() {
    let admin_address = @0xA;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        // Pause transfers
        treasury::pause_transfers(&mut treasury, &admin_cap, &mut deny_list, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        // Unpause transfers
        treasury::unpause_transfers(&mut treasury, &admin_cap, &mut deny_list, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::EPaused)]
fun test_paused_transfers_fail() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let recipient_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let mut deny_list = scenario.take_shared<DenyList>();
        treasury::pause_transfers(&mut treasury, &admin_cap, &mut deny_list, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        // Try to mint while paused (should fail)
        treasury::mint(&mut treasury, &supply_manager_cap, &deny_list, 10, recipient_address, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test]
fun test_unauthorize_supply_manager() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        // Create a new SupplyManager
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        // Unauthorize the current SupplyManager
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ENoSupplyManagerSet)]
fun test_unauthorized_supply_manager_mint_fails() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let recipient_address = @0xC;
    let mut supply_manager_id_opt = option::none<object::ID>();

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        supply_manager_id_opt.fill(object::id(&supply_manager_cap));
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        // Try to mint after unauthorization (should fail)
        treasury::mint(&mut treasury, &supply_manager_cap, &deny_list, 10, recipient_address, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test]
fun test_update_and_verify_description() {
    let admin_address = @0xA;
    let new_description = std::string::utf8(b"Updated regulated coin description");

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };

    scenario.next_tx(admin_address);
    {
        let treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        // Verify the initial description
        let metadata = treasury::borrow_metadata(&treasury);
        let desc = get_description(metadata);
        assert!(desc == std::string::utf8(b"Example Regulated Coin"), 42);
        test_scenario::return_shared(treasury);
    };

    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        // Update the description
        let mut metadata = treasury::take_metadata(&mut treasury, &admin_cap);
        let treasury_cap = treasury::borrow_treasury_cap_mut(&mut treasury, &admin_cap);
        update_description(treasury_cap, &mut metadata, new_description);
        treasury::set_metadata(&mut treasury, &admin_cap, metadata);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };

    scenario.next_tx(admin_address);
    {
        let treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        // Verify the description was updated
        let metadata = treasury::borrow_metadata(&treasury);
        let desc = get_description(metadata);
        assert!(desc == new_description, 42);
        test_scenario::return_shared(treasury);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::EZeroAmount)]
fun test_burn_zero_amount() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap,  scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        // Mint a coin with value 10
        treasury::mint(&mut treasury, &supply_manager_cap, &deny_list, 10, supply_manager_address, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    // Split the coin with split_amount 0 to get a zero-value coin
    scenario.next_tx(supply_manager_address);
    {
        let mut coin = scenario.take_from_sender<Coin<REGULATED_COIN>>();
        let zero_coin = iota::coin::split(&mut coin, 0, scenario.ctx());
        scenario.return_to_sender(coin);
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        treasury::burn(&mut treasury, &supply_manager_cap, &deny_list, zero_coin, scenario.ctx());
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ESupplyManagerEntryAlreadyExists)]
fun test_create_second_supply_manager_fails() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Create first SupplyManager
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        
        // Try to create second SupplyManager (should fail)
        let supply_manager_cap_2 = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(supply_manager_cap_2, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.end();
}

#[test]
fun test_unauthorize_and_create_new_supply_manager_works() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let recipient_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Create first SupplyManager
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Unauthorize current SupplyManager
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        
        // Create new SupplyManager (should work)
        let new_supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(new_supply_manager_cap, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    // Test that new SupplyManager can mint
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let new_supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        
        // New SupplyManager should be able to mint
        treasury::mint(&mut treasury, &new_supply_manager_cap, &deny_list, 100, recipient_address, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(new_supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ESupplyManagerNotAuthorized)]
fun test_old_supply_manager_fails_after_unauthorization() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;
    let recipient_address = @0xC;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Create first SupplyManager
        let old_supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(old_supply_manager_cap, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Unauthorize current SupplyManager
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        
        // Create new SupplyManager
        let new_supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(new_supply_manager_cap, admin_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    // Test that old SupplyManager can no longer mint
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let old_supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        
        // Old SupplyManager should fail to mint
        treasury::mint(&mut treasury, &old_supply_manager_cap, &deny_list, 100, recipient_address, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(old_supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ENoSupplyManagerSet)]
fun test_unauthorize_nonexistent_supply_manager_fails() {
    let admin_address = @0xA;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Try to unauthorize SupplyManager when none was created (should fail)
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ENoSupplyManagerSet)]
fun test_burn_with_unauthorized_supply_manager_fails() {
    let admin_address = @0xA;
    let supply_manager_address = @0xB;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Create and authorize a SupplyManager to mint some coins first
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        
        // Mint some coins
        treasury::mint(&mut treasury, &supply_manager_cap, &deny_list, 100, supply_manager_address, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Unauthorize the SupplyManager
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    // Try to burn with unauthorized SupplyManager
    scenario.next_tx(supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let unauthorized_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let coin = scenario.take_from_sender<Coin<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        
        // Should fail because SupplyManager is no longer authorized
        treasury::burn(&mut treasury, &unauthorized_cap, &deny_list, coin, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(unauthorized_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ESupplyManagerNotAuthorized)]
fun test_old_supply_manager_burn_fails_with_new_active() {
    let admin_address = @0xA;
    let old_supply_manager_address = @0xB;
    let new_supply_manager_address = @0xC;
    let recipient_address = @0xD;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
    };
    
    // Create first SupplyManager and mint some coins
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        let old_supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(old_supply_manager_cap, old_supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    
    // Old SupplyManager mints coins to recipient
    scenario.next_tx(old_supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let old_supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        
        treasury::mint(&mut treasury, &old_supply_manager_cap, &deny_list, 100, recipient_address, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(old_supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    
    // Admin unauthorizes old SupplyManager and creates new one
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        
        // Unauthorize the old SupplyManager
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap);
        
        // Create new SupplyManager (this one is now the active one)
        let new_supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, scenario.ctx());
        transfer::public_transfer(new_supply_manager_cap, new_supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    
    // Old SupplyManager tries to burn (should fail even though a new one is authorized)
    scenario.next_tx(old_supply_manager_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let old_supply_manager_cap = scenario.take_from_sender<SupplyManagerCap<REGULATED_COIN>>();
        let coin = scenario.take_from_address<Coin<REGULATED_COIN>>(recipient_address);
        let deny_list = scenario.take_shared<DenyList>();
        
        treasury::burn(&mut treasury, &old_supply_manager_cap, &deny_list, coin, scenario.ctx());
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(old_supply_manager_cap);
        test_scenario::return_shared(deny_list);
    };
    
    scenario.end();
}
