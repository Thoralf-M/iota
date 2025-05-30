// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

module kek::kek {
    public struct Kek {
        a: u8,
        b: u64,
    }

    public fun destroy(k1: Kek, k2: Kek) {
        let Kek { a: _, .. } = k1;
        let Kek { .. } = k2;
    }
}
