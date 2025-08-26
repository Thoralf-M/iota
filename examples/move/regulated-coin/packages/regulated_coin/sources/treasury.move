module regulated_coin::treasury;

use iota::{
    coin::{
        Self, Coin, CoinMetadata, DenyCapV1, TreasuryCap, 
        deny_list_v1_contains_next_epoch,
        deny_list_v1_is_global_pause_enabled_next_epoch,
    },
    deny_list::{DenyList},
    dynamic_field as df,
    dynamic_object_field as dof,
    event,
};

#[error]
const EMissingDenyCapV1: vector<u8> = b"Dynamic object field for DenyCapV1 not found.";
#[error]
const EMissingCoinMetadata: vector<u8> = b"Dynamic object field for CoinMetadata not found.";
#[error]
const EMissingTreasuryCap: vector<u8> = b"Dynamic object field for TreasuryCap not found.";
#[error]
const EZeroAmount: vector<u8> = b"Amount must be greater than zero.";
#[error]
const EPaused: vector<u8> = b"Transfers are paused.";
#[error]
const EDeniedAddress: vector<u8> = b"Address is on the deny list.";
#[error]
const ENoSupplyManagerSet: vector<u8> = b"No supply manager has been set as dynamic field.";
#[error]
const ESupplyManagerNotAuthorized: vector<u8> = b"Supply manager is not authorized.";
#[error]
const ESupplyManagerEntryAlreadyExists: vector<u8> = b"There is already an entry for a supply manager.";

/// Admin capability. The admin has full control over the treasury.
/// This object must be issued only once during module initialization.
public struct AdminCap has key, store { id: UID }

/// Shared treasury object for the regulated coin.
/// Can have the following things attached as dynamic fields:
/// - TreasuryCap
/// - DenyCapV1
/// - CoinMetadata
/// - SupplyManagerKey
public struct Treasury<phantom T> has key, store {
    id: UID,
}

// Keys for the dynamic fields
public struct TreasuryCapKey has copy, store, drop {}
public struct CoinMetadataKey has copy, store, drop {}
public struct DenyCapV1Key has copy, store, drop {}
public struct SupplyManagerKey has copy, store, drop { } 

// Events
public struct MintEvent has copy, drop, store { amount: u64, recipient: address }
public struct BurnEvent has copy, drop, store { amount: u64, actor: address }
public struct PauseEvent has copy, drop, store { enabled: bool }
public struct DenyListChangeEvent has copy, drop, store { address: address, added: bool }

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

/// Create a new SupplyManagerCap and authorize it, there can only be one SupplyManagerCap at a time.
public fun new_supply_manager<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    ctx: &mut TxContext,
): SupplyManagerCap<T> {
    assert!(!df::exists_(&treasury.id, SupplyManagerKey {}), ESupplyManagerEntryAlreadyExists);

    let id = object::new(ctx);
    let supply_manager_cap = SupplyManagerCap { id };

    df::add(&mut treasury.id, SupplyManagerKey {}, object::id(&supply_manager_cap) );

    supply_manager_cap
}

/// Authorize a supply manager again.
public fun authorize_supply_manager<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    supply_manger_id: ID
) {
    assert!(!df::exists_(&treasury.id, SupplyManagerKey {}), ESupplyManagerEntryAlreadyExists);
    df::add(&mut treasury.id, SupplyManagerKey {}, supply_manger_id );
}

/// Unauthorize the current supply manager by removing its entry from the Treasury.
public fun unauthorize_supply_manager<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
) {
    assert!(df::exists_(&treasury.id, SupplyManagerKey {}), ENoSupplyManagerSet);
    df::remove<SupplyManagerKey, ID>(&mut treasury.id, SupplyManagerKey {});
}

/// Adds the given address to the deny list, preventing it from interacting with the specified
/// coin type as an input to a transaction. Additionally at the start of the next epoch, the
/// address will be unable to receive objects of this coin type.
public fun block_address<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    deny_list: &mut DenyList,
    address: address,
    ctx: &mut TxContext,
) {
    coin::deny_list_v1_add(deny_list, treasury.borrow_deny_cap_mut(), address, ctx);
    event::emit(DenyListChangeEvent { address, added: true });
}

/// Removes an address from the deny list. Similar to `block_address`, the effect for input
/// objects will be immediate, but the effect for receiving objects will be delayed until the
/// next epoch.
public fun unblock_address<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    deny_list: &mut DenyList,
    address: address,
    ctx: &mut TxContext,
) {
    coin::deny_list_v1_remove(deny_list, treasury.borrow_deny_cap_mut(), address, ctx);
    event::emit(DenyListChangeEvent { address, added: false });
}

/// Pause all transfers.
entry fun pause_transfers<T>(
    treasury: &mut Treasury<T>,
    _: &AdminCap,
    deny_list: &mut DenyList,
    ctx: &mut TxContext
) {
    if (!deny_list_v1_is_global_pause_enabled_next_epoch<T>(deny_list)) {
        coin::deny_list_v1_enable_global_pause(deny_list,  treasury.borrow_deny_cap_mut(), ctx);
        event::emit(PauseEvent { enabled: true });
    };
}

/// Unpause all transfers.
entry fun unpause_transfers<T>(
    treasury: &mut Treasury<T>, 
    _: &AdminCap,
    deny_list: &mut DenyList,
    ctx: &mut TxContext
) {
    if (deny_list_v1_is_global_pause_enabled_next_epoch<T>(deny_list)) {
        coin::deny_list_v1_disable_global_pause(deny_list, treasury.borrow_deny_cap_mut(), ctx);
        event::emit(PauseEvent { enabled: false });
    };
}

/// Mint tokens using a SupplyManagerCap.
public fun mint<T>(
    treasury: &mut Treasury<T>,
    supply_manager_cap: &SupplyManagerCap<T>,
    deny_list: &DenyList,
    amount: u64,
    recipient: address,
    ctx: &mut TxContext,
) {
    assert_authorized_supply_manager(treasury, supply_manager_cap);

    assert!(amount > 0, EZeroAmount);
    assert!(!deny_list_v1_is_global_pause_enabled_next_epoch<T>(deny_list), EPaused);
    assert!(!deny_list_v1_contains_next_epoch<T>(deny_list, ctx.sender()), EDeniedAddress);
    assert!(!deny_list_v1_contains_next_epoch<T>(deny_list, recipient), EDeniedAddress);
    
    treasury.borrow_treasury_cap_mut_internal().mint_and_transfer(amount, recipient, ctx);
    event::emit(MintEvent { amount, recipient });
}

/// Burn tokens using a SupplyManagerCap.
public fun burn<T>(
    treasury: &mut Treasury<T>,
    supply_manager_cap: &SupplyManagerCap<T>,
    deny_list: &DenyList,
    coin: Coin<T>,
    ctx: &mut TxContext,
) {
    assert_authorized_supply_manager(treasury, supply_manager_cap);

    assert!(!deny_list_v1_is_global_pause_enabled_next_epoch<T>(deny_list), EPaused);
    assert!(!deny_list_v1_contains_next_epoch<T>(deny_list, ctx.sender()), EDeniedAddress);

    let amount = coin.value();
    assert!(amount > 0, EZeroAmount);

    treasury.borrow_treasury_cap_mut_internal().burn(coin);
    event::emit(BurnEvent { amount, actor: ctx.sender() });
}

/// Take the CoinMetadata.
public fun take_metadata<T>(treasury: &mut Treasury<T>, _: &AdminCap): CoinMetadata<T> {
    assert!(dof::exists_with_type<_, CoinMetadata<T>>(&treasury.id, CoinMetadataKey {}), EMissingCoinMetadata);
    dof::remove(&mut treasury.id, CoinMetadataKey {})
}

/// Set the CoinMetadata.
public fun set_metadata<T>(treasury: &mut Treasury<T>, _: &AdminCap, coin_metadata: CoinMetadata<T>) {
    dof::add(&mut treasury.id, CoinMetadataKey {}, coin_metadata)
}

/// Get an immutable reference to the CoinMetadata.
public fun borrow_metadata<T>(treasury: &Treasury<T>): &CoinMetadata<T> {
    assert!(dof::exists_with_type<_, CoinMetadata<T>>(&treasury.id, CoinMetadataKey {}), EMissingCoinMetadata);
    dof::borrow(&treasury.id, CoinMetadataKey {})
}

/// Get an immutable reference to the TreasuryCap.
public fun borrow_treasury_cap<T>(treasury: &Treasury<T>): &TreasuryCap<T> {
    assert!(dof::exists_with_type<_, TreasuryCap<T>>(&treasury.id, TreasuryCapKey {}), EMissingTreasuryCap);
    dof::borrow(&treasury.id, TreasuryCapKey {})
}

/// Get a mutable reference to the TreasuryCap.
public fun borrow_treasury_cap_mut<T>(treasury: &mut Treasury<T>, _: &AdminCap): &mut TreasuryCap<T> {
    assert!(dof::exists_with_type<_, TreasuryCap<T>>(&treasury.id, TreasuryCapKey {}), EMissingTreasuryCap);
    dof::borrow_mut(&mut treasury.id, TreasuryCapKey {})
}

/// Get a mutable reference to the TreasuryCap. Without the AdminCap only for internal use.
fun borrow_treasury_cap_mut_internal<T>(treasury: &mut Treasury<T>): &mut TreasuryCap<T> {
    assert!(dof::exists_with_type<_, TreasuryCap<T>>(&treasury.id, TreasuryCapKey {}), EMissingTreasuryCap);
    dof::borrow_mut(&mut treasury.id, TreasuryCapKey {})
}

// Internal helper to get a mutable reference to the DenyCapV1 and return `EMissingDenyCapV1` if not found.
fun borrow_deny_cap_mut<T>(treasury: &mut Treasury<T>): &mut DenyCapV1<T> {
    assert!(dof::exists_with_type<_, DenyCapV1<T>>(&treasury.id, DenyCapV1Key {}), EMissingDenyCapV1);
    dof::borrow_mut(&mut treasury.id, DenyCapV1Key {})
}

/// Internal helper to assert that the provided supply manager cap matches the authorized
/// supply manager recorded in the treasury. Aborts with `ENoSupplyManagerSet` or `ESupplyManagerNotAuthorized` if
/// the dynamic field is missing or the IDs do not match.
fun assert_authorized_supply_manager<T>(treasury: &Treasury<T>, supply_manager_cap: &SupplyManagerCap<T>) {
    assert!(df::exists_(&treasury.id, SupplyManagerKey {}), ENoSupplyManagerSet);
    let authorized_id = df::borrow<SupplyManagerKey, ID>(&treasury.id, SupplyManagerKey {});
    assert!(object::id(supply_manager_cap) == *authorized_id, ESupplyManagerNotAuthorized);
}

// Event accessors
public fun mint_event_amount(e: &MintEvent): u64 { e.amount }
public fun mint_event_recipient(e: &MintEvent): address { e.recipient }
public fun burn_event_amount(e: &BurnEvent): u64 { e.amount }
public fun burn_event_actor(e: &BurnEvent): address { e.actor }
public fun pause_event_enabled(e: &PauseEvent): bool { e.enabled }
public fun deny_list_change_event_address(e: &DenyListChangeEvent): address { e.address }
public fun deny_list_change_event_added(e: &DenyListChangeEvent): bool { e.added }
