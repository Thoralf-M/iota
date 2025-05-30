// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

module base_addr::friend_module {

    public struct A<T> {
        field1: u64,
        field2: T
    }

    public fun friend_call(): u64 { base_addr::base::friend_fun(1) }

    public fun return_0(): u64 { 0 }

    fun non_public_fun(y: u64): u64 { y }

    // Reorder the functions
    public fun plus_1(x: u64): u64 { x + 1 }
}
