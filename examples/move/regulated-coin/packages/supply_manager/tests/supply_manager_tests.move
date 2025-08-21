#[test_only]
module supply_manager::supply_manager_tests;

use supply_manager::supply_manager::{Self, SupplyManager, SupplyManagerAdminCap, add_supply_manager_cap, remove_supply_manager_cap, mint, burn};
use regulated_coin::treasury::{Self as TreasuryMod, Treasury, SupplyManagerCap, AdminCap};
use regulated_coin::regulated_coin;
use iota::{coin::Coin, deny_list::{Self, DenyList}, test_scenario};

#[test]
fun test_supply_manager_mint_and_burn() {
    let admin_address = @0xA;
    let recipient_address = @0xB;

    let mut scenario = test_scenario::begin(@0);
    deny_list::create_for_test(scenario.ctx());

    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
        supply_manager::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut supply_manager = scenario.take_shared<SupplyManager>();
        let admin_cap = scenario.take_from_sender<SupplyManagerAdminCap>();
        let mut treasury = scenario.take_shared<Treasury<regulated_coin::REGULATED_COIN>>();
        let regulated_admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_cap = TreasuryMod::new_supply_manager(
            &mut treasury,
            &regulated_admin_cap,
            scenario.ctx()
        );
        add_supply_manager_cap(&mut supply_manager, &admin_cap, supply_manager_cap);
        test_scenario::return_shared(supply_manager);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        scenario.return_to_sender(regulated_admin_cap);
    };
    scenario.next_tx(admin_address);
    {
        let mut supply_manager = scenario.take_shared<SupplyManager>();
        let admin_cap = scenario.take_from_sender<SupplyManagerAdminCap>();
        let mut treasury = scenario.take_shared<Treasury<regulated_coin::REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        // Mint tokens
        mint(&mut supply_manager, &admin_cap, &mut treasury, &deny_list, 100, recipient_address, scenario.ctx());
        test_scenario::return_shared(supply_manager);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.next_tx(admin_address);
    {
        let mut supply_manager = scenario.take_shared<SupplyManager>();
        let admin_cap = scenario.take_from_sender<SupplyManagerAdminCap>();
        let mut treasury = scenario.take_shared<Treasury<regulated_coin::REGULATED_COIN>>();
        let deny_list = scenario.take_shared<DenyList>();
        let coin = scenario.take_from_address<Coin<regulated_coin::REGULATED_COIN>>(recipient_address);
        // Burn tokens
        burn(&mut supply_manager, &admin_cap, &mut treasury, &deny_list, coin, scenario.ctx());
        test_scenario::return_shared(supply_manager);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        test_scenario::return_shared(deny_list);
    };
    scenario.end();
}

#[test]
fun test_supply_manager_remove_cap() {
    let admin_address = @0xA;
    let mut scenario = test_scenario::begin(@0);
    scenario.next_tx(admin_address);
    {
        regulated_coin::test_init(scenario.ctx());
        supply_manager::test_init(scenario.ctx());
    };
    scenario.next_tx(admin_address);
    {
        let mut supply_manager = scenario.take_shared<SupplyManager>();
        let admin_cap = scenario.take_from_sender<SupplyManagerAdminCap>();
        let mut treasury = scenario.take_shared<Treasury<regulated_coin::REGULATED_COIN>>();
        let regulated_admin_cap = scenario.take_from_sender<AdminCap>();
        let supply_manager_cap = TreasuryMod::new_supply_manager(
            &mut treasury,
            &regulated_admin_cap,
            scenario.ctx()
        );
        add_supply_manager_cap(&mut supply_manager, &admin_cap, supply_manager_cap);
        let supply_manager_cap: SupplyManagerCap<regulated_coin::REGULATED_COIN> = remove_supply_manager_cap(&mut supply_manager, &admin_cap);
        transfer::public_transfer(supply_manager_cap, admin_address);

        test_scenario::return_shared(supply_manager);
        test_scenario::return_shared(treasury);
        scenario.return_to_sender(admin_cap);
        scenario.return_to_sender(regulated_admin_cap);
    };
    scenario.end();
}
