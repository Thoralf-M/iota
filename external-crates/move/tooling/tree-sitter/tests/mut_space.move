// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

module a::m;
public struct S {
    f: u64,
}
public fun foo(x: & mut S): & mut u64 {
    & mut x.f;
    &
    mut x.f
}
