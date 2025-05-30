// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

// This test verifies the correct deletion of the Config's setting object after the epoch it was
// created. THe value is set only in one epoch.

//# init --accounts A B --addresses test=0x0

//# publish --sender A
module test::regulated_coin {
    use iota::coin;
    use iota::deny_list::DenyList;

    public struct REGULATED_COIN has drop {}

    fun init(otw: REGULATED_COIN, ctx: &mut TxContext) {
        let (mut treasury_cap, deny_cap, metadata) = coin::create_regulated_currency_v1(
            otw,
            9,
            b"RC",
            b"REGULATED_COIN",
            b"A new regulated coin",
            option::none(),
            true,
            ctx
        );
        let coin = coin::mint(&mut treasury_cap, 10000, ctx);
        transfer::public_transfer(coin, tx_context::sender(ctx));
        transfer::public_transfer(deny_cap, tx_context::sender(ctx));
        transfer::public_freeze_object(treasury_cap);
        transfer::public_freeze_object(metadata);
    }

    entry fun assert_address_deny_status(
        deny_list: &DenyList,
        addr: address,
        expected: bool,
    ) {
        let status = coin::deny_list_v1_contains_next_epoch<REGULATED_COIN>(deny_list, addr);
        assert!(status == expected, 0);
    }

    entry fun assert_global_pause_status(
        deny_list: &DenyList,
        expected: bool,
    ) {
        let status =
            coin::deny_list_v1_is_global_pause_enabled_next_epoch<REGULATED_COIN>(deny_list);
        assert!(status == expected, 0);
    }
}

// Deny account B.
//# run iota::coin::deny_list_v1_add --args object(0x403) object(1,2) @B --type-args test::regulated_coin::REGULATED_COIN --sender A

// Enable global pause.
//# run iota::coin::deny_list_v1_enable_global_pause --args object(0x403) object(1,2) --type-args test::regulated_coin::REGULATED_COIN --sender A

// View the setting objects
//# view-object 2,1

//# view-object 3,0

//# advance-epoch

// Undeny account B.
//# run iota::coin::deny_list_v1_remove --args object(0x403) object(1,2) @B --type-args test::regulated_coin::REGULATED_COIN --sender A

// Disable global pause.
//# run iota::coin::deny_list_v1_disable_global_pause --args object(0x403) object(1,2) --type-args test::regulated_coin::REGULATED_COIN --sender A

// Verify the setting objects are still present
//# view-object 2,1

//# view-object 3,0

//# advance-epoch

// Undeny account B.
//# run iota::coin::deny_list_v1_remove --args object(0x403) object(1,2) @B --type-args test::regulated_coin::REGULATED_COIN --sender A

// Disable global pause.
//# run iota::coin::deny_list_v1_disable_global_pause --args object(0x403) object(1,2) --type-args test::regulated_coin::REGULATED_COIN --sender A

// Verify the setting objects are deleted
//# view-object 2,1

//# view-object 3,0

// Assert that the address is no longer denied.
//# run test::regulated_coin::assert_address_deny_status --args immshared(0x403) @B false --sender A

// Assert that global pause is disabled.
//# run test::regulated_coin::assert_global_pause_status --args immshared(0x403) false --sender A
