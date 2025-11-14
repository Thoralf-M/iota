/// # OFT Implementation Module
///
/// This module provides the implementation for initializing OFT (Omnichain Fungible Token) objects.
/// It handles the two-phase creation process:
/// 1. Package initialization creates an OFTInitTicket with the necessary components
/// 2. Users consume the ticket to create either a standard OFT(mint/burn model) or an OFT adapter(escrow/release model)
///
/// The module supports two types of OFTs:
/// - **Standard OFT**: Uses a TreasuryCap to mint/burn tokens for cross-chain transfers
/// - **OFT Adapter**: Wraps existing tokens using an escrow mechanism(escrow/release model)
module oft::oft_impl;

use call::call_cap::CallCap;
use oapp::oapp::{Self, OApp, AdminCap};
use oft::oft;
use oft_common::migration::MigrationCap;
use regulated_coin::regulated_coin::{Self, SupplyManagerCap, Treasury};

// === Errors ===

const EInvalidOApp: u64 = 1;

/// One-time witness struct used during package initialization.
public struct OFT_IMPL has drop {}

/// Ticket that holds the components needed to create an OFT.
public struct OFTInitTicket has key {
    id: UID,
    /// Call capability for the OFT
    oft_cap: CallCap,
    /// Address of the OApp object
    oapp_object: address,
    /// Admin capability for the OFT & OApp
    admin_cap: AdminCap,
}

/// Package initialization function called when the module is first published.
/// Creates the initial OApp components and transfers them to the publisher.
fun init(otw: OFT_IMPL, ctx: &mut TxContext) {
    let (oft_cap, admin_cap, oapp_object) = oapp::new(&otw, ctx);
    transfer::transfer(OFTInitTicket { id: object::new(ctx), oft_cap, oapp_object, admin_cap }, ctx.sender());
}

/// Initializes a standard OFT (Omnichain Fungible Token).
/// A standard OFT uses a SupplyManagerCap to mint and burn tokens for cross-chain transfers.
///
/// **Parameters**:
/// - `ticket`: Creation ticket obtained from package initialization
/// - `oapp`: Associated OApp instance that can only be called by this OFT object with the hold of the oft_cap
/// - `supply_manager_cap`: SupplyManagerCap for the token
/// - `treasury`: Treasury object for the regulated coin
/// - `shared_decimals`: Number of decimals to use for cross-chain operations
///
/// **Returns**:
/// - `AdminCap`: Capability for managing the OFT
/// - `MigrationCap`: Capability for future migrations of this OFT
public fun init_oft(
    ticket: OFTInitTicket,
    oapp: &OApp,
    supply_manager_cap: SupplyManagerCap,
    treasury: &Treasury,
    shared_decimals: u8,
    ctx: &mut TxContext,
): (AdminCap, MigrationCap) {
    let (oft_cap, admin_cap) = destroy_oft_init_ticket(ticket, oapp);
    let coin_metadata = regulated_coin::borrow_metadata(treasury);
    let migration_cap = oft::init_oft(oapp, oft_cap, supply_manager_cap, coin_metadata, shared_decimals, ctx);

    (admin_cap, migration_cap)
}

// === Helper Functions ===

fun destroy_oft_init_ticket(ticket: OFTInitTicket, expected_oapp: &OApp): (CallCap, AdminCap) {
    let OFTInitTicket { id, oft_cap, oapp_object, admin_cap } = ticket;
    assert!(oapp_object == object::id_address(expected_oapp), EInvalidOApp);
    object::delete(id);
    (oft_cap, admin_cap)
}
