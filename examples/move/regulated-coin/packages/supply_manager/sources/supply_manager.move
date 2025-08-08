module supply_manager::supply_manager;

use regulated_coin::treasury::{Self as TreasuryMod, Treasury, SupplyManagerCap};
use iota::{
    coin::Coin,
    deny_list::DenyList,
    dynamic_object_field as dof,
};

#[error]
const ESupplyManagerCapMissing: vector<u8> = b"SupplyManagerCap is missing.";

/// Key for dynamic field storing the SupplyManagerCap
public struct SupplyManagerCapKey has copy, store, drop {}

/// Shared wrapper object for managing SupplyManagerCap-controlled mint/burn
public struct SupplyManager has key, store {
    id: UID,
}

/// Admin capability for the wrapper
public struct SupplyManagerAdminCap has key, store { id: UID }

/// Initialize the wrapper and issue an admin cap to the sender
fun init(ctx: &mut TxContext) {
    let admin_cap = SupplyManagerAdminCap { id: object::new(ctx) };
    let supply_manager = SupplyManager { id: object::new(ctx) };

    transfer::public_transfer(admin_cap, ctx.sender());
    transfer::public_share_object(supply_manager);
}

/// Attach a SupplyManagerCap to the SupplyManager object
public fun add_supply_manager_cap<T>(
    supply_manager: &mut SupplyManager,
    _admin_cap: &SupplyManagerAdminCap,
    supply_manager_cap: SupplyManagerCap<T>,
) {
    dof::add(&mut supply_manager.id, SupplyManagerCapKey {}, supply_manager_cap);
}

/// Remove the attached SupplyManagerCap from the SupplyManager object
public fun remove_supply_manager_cap<T>(
    supply_manager: &mut SupplyManager,
    _admin_cap: &SupplyManagerAdminCap,
): SupplyManagerCap<T> {
    assert!(dof::exists_with_type<_, SupplyManagerCap<T>>(&supply_manager.id, SupplyManagerCapKey {}), ESupplyManagerCapMissing);
    dof::remove<SupplyManagerCapKey, SupplyManagerCap<T>>(&mut supply_manager.id, SupplyManagerCapKey {})
}

/// Internal helper to get mutable reference to the attached SupplyManagerCap
fun supply_manager_cap_mut<T>(supply_manager: &mut SupplyManager): &mut SupplyManagerCap<T> {
    assert!(dof::exists_with_type<_, SupplyManagerCap<T>>(&supply_manager.id, SupplyManagerCapKey {}), ESupplyManagerCapMissing);
    dof::borrow_mut(&mut supply_manager.id, SupplyManagerCapKey {})
}

/// Mint tokens via regulated_coin::treasury::mint, protected by the SupplyManagerAdminCap
public fun mint<T>(
    supply_manager: &mut SupplyManager,
    _admin_cap: &SupplyManagerAdminCap,
    treasury: &mut Treasury<T>,
    deny_list: &DenyList,
    amount: u64,
    recipient: address,
    ctx: &mut TxContext,
) {
    let supply_manager_cap = supply_manager_cap_mut<T>(supply_manager);
    TreasuryMod::mint(treasury, supply_manager_cap, deny_list, amount, recipient, ctx);
}

/// Burn tokens via regulated_coin::treasury::burn, protected by the SupplyManagerAdminCap
public fun burn<T>(
    supply_manager: &mut SupplyManager,
    _admin_cap: &SupplyManagerAdminCap,
    treasury: &mut Treasury<T>,
    deny_list: &DenyList,
    coin: Coin<T>,
    ctx: &mut TxContext,
) {
    let supply_manager_cap = supply_manager_cap_mut<T>(supply_manager);
    TreasuryMod::burn(treasury, supply_manager_cap, deny_list, coin, ctx);
}

#[test_only]
public fun test_init(ctx: &mut TxContext) {
    init(ctx);
}


