// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

module 0x1::m {
    fun f() {
        x[];
        x[1, f()];
        x[1, 2, 3];
    }
}
