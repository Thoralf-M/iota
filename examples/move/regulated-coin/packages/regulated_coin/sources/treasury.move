module regulated_coin::treasury;

use iota::{
    coin::{
        Self, Coin, CoinMetadata, DenyCapV1, TreasuryCap, 
        // returns if address is on the deny list based on the most recent update
        deny_list_v1_contains_next_epoch as is_blocklisted,
        // returns if the global pause is effective based on the most recent update
        deny_list_v1_is_global_pause_enabled_next_epoch as is_paused,
    },
    deny_list::{DenyList},
    dynamic_field as df,
    dynamic_object_field as dof,
};

#[error]
const EMissingDenyCapV1: vector<u8> = b"Dynamic object field for DenyCapV1 not found.";
#[error]
const EMissingTreasuryCap: vector<u8> = b"Dynamic object field for TreasuryCap not found.";
#[error]
const EZeroAmount: vector<u8> = b"Amount must be greater than zero.";
#[error]
const EWouldExceedAllowance: vector<u8> = b"Mint amount would exceed allowance.";
#[error]
const EPaused: vector<u8> = b"Transfers are paused.";
#[error]
const EDeniedAddress: vector<u8> = b"Address is on the deny list.";
#[error]
const ESupplyManagerNotAuthorized: vector<u8> = b"Supply manager is not authorized.";

/// Admin capability. The admin has full control over the treasury.
/// This object must be issued only once during module initialization.
public struct AdminCap has key, store { id: UID }

/// Shared treasury object for the regulated coin.
/// Contains 
public struct Treasury<phantom T> has key, store {
    id: UID,
}

/// Keys for the dynamic fields
public struct TreasuryCapKey has copy, store, drop {}
public struct CoinMetadataKey has copy, store, drop {}
public struct DenyCapV1Key has copy, store, drop {}
public struct SupplyManagerKey has copy, store, drop { id: ID } 

/// Create a Treasury with TreasuryCap and DenyCapV1.
#[allow(lint(self_transfer))]
public (package) fun new<T>(
    treasury_cap: TreasuryCap<T>, 
    deny_cap: DenyCapV1<T>,
    metadata: CoinMetadata<T>,
    ctx: &mut TxContext
) {
    let mut treasury = Treasury<T> {
        id: object::new(ctx),
    };
    dof::add(&mut treasury.id, CoinMetadataKey {}, metadata);
    dof::add(&mut treasury.id, DenyCapV1Key {}, deny_cap);
    dof::add(&mut treasury.id, TreasuryCapKey {}, treasury_cap);

    transfer::public_transfer(
        AdminCap { id: object::new(ctx) },
        ctx.sender(),
    );

    transfer::public_share_object(treasury);
}

/// Supply manager capability that allows minting and burning.
public struct SupplyManagerCap<phantom T> has key, store {
    id: UID,
}

/// Create a new SupplyManagerCap with a mint allowance.
public fun new_supply_manager<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    mint_allowance: u64,
    ctx: &mut TxContext,
): SupplyManagerCap<T> {

    let id = object::new(ctx);
    let supply_manager_cap = SupplyManagerCap { id };

    df::add(&mut treasury.id, SupplyManagerKey{ id: object::id(&supply_manager_cap) }, mint_allowance);

    supply_manager_cap
}

/// Unauthorize a supply manager by removing its entry from the Treasury.
public fun unauthorize_supply_manager<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    supply_manager_cap_id: ID,
) {
    df::remove_if_exists<SupplyManagerKey, u64>(&mut treasury.id, SupplyManagerKey{ id: supply_manager_cap_id });
}

/// Adds the given address to the deny list, preventing it from interacting with the specified
/// coin type as an input to a transaction. Additionally at the start of the next epoch, the
/// address will be unable to receive objects of this coin type.
public fun block_address<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    denylist: &mut DenyList,
    address: address,
    ctx: &mut TxContext,
) {
    coin::deny_list_v1_add(denylist, treasury.deny_cap_mut(), address, ctx);
}

/// Removes an address from the deny list. Similar to `block_address`, the effect for input
/// objects will be immediate, but the effect for receiving objects will be delayed until the
/// next epoch.
public fun unblock_address<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    denylist: &mut DenyList,
    address: address,
    ctx: &mut TxContext,
) {
    coin::deny_list_v1_remove(denylist, treasury.deny_cap_mut(), address, ctx);
}

fun deny_cap_mut<T>(treasury: &mut Treasury<T>): &mut DenyCapV1<T> {
    assert!(dof::exists_with_type<_, DenyCapV1<T>>(&treasury.id, DenyCapV1Key {}), EMissingDenyCapV1);
    dof::borrow_mut(&mut treasury.id, DenyCapV1Key {})
}

fun treasury_cap_mut<T>(treasury: &mut Treasury<T>): &mut TreasuryCap<T> {
    assert!(dof::exists_with_type<_, TreasuryCap<T>>(&treasury.id, TreasuryCapKey {}), EMissingTreasuryCap);
    dof::borrow_mut(&mut treasury.id, TreasuryCapKey {})
}

/// Mint tokens using a SupplyManagerCap. Reduces the mint allowance.
public fun mint<T>(
    treasury: &mut Treasury<T>,
    supply_manager_cap: &SupplyManagerCap<T>,
    deny_list: &DenyList,
    amount: u64,
    recipient: address,
    ctx: &mut TxContext,
) {
    assert!(amount > 0, EZeroAmount);
    assert!(!is_paused<T>(deny_list), EPaused);
    assert!(!is_blocklisted<T>(deny_list, ctx.sender()), EDeniedAddress);
    assert!(!is_blocklisted<T>(deny_list, recipient), EDeniedAddress);

    let supply_manager_key = SupplyManagerKey { id: object::id(supply_manager_cap) };
    assert!(df::exists_(&treasury.id, supply_manager_key), ESupplyManagerNotAuthorized);
    
    let mint_allowance = df::borrow_mut<SupplyManagerKey, u64>(&mut treasury.id, supply_manager_key);
    assert!(*mint_allowance >= amount, EWouldExceedAllowance);
    
    *mint_allowance = *mint_allowance - amount;
    
    treasury.treasury_cap_mut().mint_and_transfer(amount, recipient, ctx);
}

/// Burn tokens using a SupplyManagerCap.
public fun burn<T>(
    treasury: &mut Treasury<T>,
    supply_manager_cap: &SupplyManagerCap<T>,
    deny_list: &DenyList,
    coin: Coin<T>,
    ctx: &mut TxContext,
) {
    assert!(!is_paused<T>(deny_list), EPaused);
    assert!(!is_blocklisted<T>(deny_list, ctx.sender()), EDeniedAddress);

    let supply_manager_key = SupplyManagerKey { id: object::id(supply_manager_cap) };
    assert!(df::exists_(&treasury.id, supply_manager_key), ESupplyManagerNotAuthorized);

    let amount = coin.value();
    assert!(amount > 0, EZeroAmount);

    treasury.treasury_cap_mut().burn(coin);
}

/// Pause all transfers.
entry fun pause_transfers<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    deny_list: &mut DenyList,
    ctx: &mut TxContext
) {
    if (!is_paused<T>(deny_list)) {
        coin::deny_list_v1_enable_global_pause(deny_list,  treasury.deny_cap_mut(), ctx);
    };
}

/// Unpause all transfers.
entry fun unpause_transfers<T>(
    treasury: &mut Treasury<T>, 
    _: &AdminCap,
    deny_list: &mut DenyList,
    ctx: &mut TxContext
) {
    if (is_paused<T>(deny_list)) {
        coin::deny_list_v1_disable_global_pause(deny_list, treasury.deny_cap_mut(), ctx);
    };
}
