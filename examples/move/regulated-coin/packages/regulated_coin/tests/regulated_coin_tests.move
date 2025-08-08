#[test_only]
module regulated_coin::regulated_coin_tests;

use regulated_coin::{
    regulated_coin::{Self, REGULATED_COIN},
    treasury::{Self, Treasury, AdminCap, SupplyManagerCap}
};
use iota::{
    coin::Coin,
    deny_list::{Self, DenyList},
    test_scenario,
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
        
        // Create supply manager with 1000 mint allowance
        let supply_manager_cap = treasury::new_supply_manager(
            &mut treasury,
            &admin_cap,
            1000,
            scenario.ctx()
        );
        
        // Transfer supply manager cap to supply manager
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    
    // Supply manager mints tokens
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

#[test, expected_failure(abort_code = treasury::EWouldExceedAllowance)]
fun test_mint_exceeds_allowance() {
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
        
        // Create supply manager with only 50 mint allowance
        let supply_manager_cap = treasury::new_supply_manager(
            &mut treasury,
            &admin_cap,
            50,
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
        
        // Try to mint 100 tokens (should fail - exceeds allowance of 50)
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
            1000,
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
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, 100, scenario.ctx());
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
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, 100, scenario.ctx());
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
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, 100, scenario.ctx());
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
        // Create supply manager
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, 100, scenario.ctx());
        supply_manager_id_opt.fill(object::id(&supply_manager_cap));
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_id = option::borrow(&supply_manager_id_opt);
        // Unauthorize supply manager using its object id
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap, *supply_manager_id);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.end();
}

#[test, expected_failure(abort_code = treasury::ESupplyManagerNotAuthorized)]
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
        let supply_manager_cap = treasury::new_supply_manager(&mut treasury, &admin_cap, 100, scenario.ctx());
        supply_manager_id_opt.fill(object::id(&supply_manager_cap));
        transfer::public_transfer(supply_manager_cap, supply_manager_address);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
    };
    scenario.next_tx(admin_address);
    {
        let mut treasury = scenario.take_shared<Treasury<REGULATED_COIN>>();
        let admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_id = option::borrow(&supply_manager_id_opt);
        treasury::unauthorize_supply_manager(&mut treasury, &admin_cap, *supply_manager_id);
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

