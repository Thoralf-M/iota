// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[test_only]
module iota::kiosk_test_utils;

use iota::coin::{Self, Coin};
use iota::iota::IOTA;
use iota::kiosk::{Self, Kiosk, KioskOwnerCap};
use iota::package::{Self, Publisher};
use iota::transfer_policy::{Self as policy, TransferPolicy, TransferPolicyCap};

public struct OTW has drop {}
public struct Asset has key, store { id: UID }

/// Prepare: dummy context
public fun ctx(): TxContext { tx_context::dummy() }

/// Prepare: accounts
/// Alice, Bob and my favorite guy - Carl
public fun folks(): (address, address, address) { (@0xA11CE, @0xB0B, @0xCA51) }

/// Get the Publisher object.
public fun get_publisher(ctx: &mut TxContext): Publisher {
    package::test_claim(OTW {}, ctx)
}

/// Prepare: TransferPolicy<Asset>
public fun get_policy(ctx: &mut TxContext): (TransferPolicy<Asset>, TransferPolicyCap<Asset>) {
    let publisher = get_publisher(ctx);
    let (policy, cap) = policy::new(&publisher, ctx);
    return_publisher(publisher);
    (policy, cap)
}

/// Prepare: Get IOTA
public fun get_iota(amount: u64, ctx: &mut TxContext): Coin<IOTA> {
    coin::mint_for_testing(amount, ctx)
}

/// Prepare: Asset
public fun get_asset(ctx: &mut TxContext): (Asset, ID) {
    let uid = object::new(ctx);
    let id = uid.to_inner();
    (Asset { id: uid }, id)
}

/// Prepare: Kiosk
public fun get_kiosk(ctx: &mut TxContext): (Kiosk, KioskOwnerCap) {
    kiosk::new(ctx)
}

public fun return_publisher(publisher: Publisher) {
    publisher.burn_publisher()
}

/// Cleanup: TransferPolicy
public fun return_policy(
    policy: TransferPolicy<Asset>,
    cap: TransferPolicyCap<Asset>,
    ctx: &mut TxContext,
): u64 {
    let profits = policy.destroy_and_withdraw(cap, ctx);
    profits.burn_for_testing()
}

/// Cleanup: Kiosk
public fun return_kiosk(kiosk: Kiosk, cap: KioskOwnerCap, ctx: &mut TxContext): u64 {
    let profits = kiosk.close_and_withdraw(cap, ctx);
    profits.burn_for_testing()
}

/// Cleanup: vector<Asset>
public fun return_assets(mut assets: vector<Asset>) {
    while (assets.length() > 0) {
        let Asset { id } = assets.pop_back();
        id.delete()
    };

    assets.destroy_empty()
}
