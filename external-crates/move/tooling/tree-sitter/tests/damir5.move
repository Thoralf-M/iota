// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

module beep::boop {
    fun f(x: MyEnum): u8 {
        match (x) {
            MyEnum::Variant(1, true) => 1,
            MyEnum::Variant(_, _) => 1,
            MyEnum::OtherVariant(_, 3) => 2,
            // Now exhaustive since this will match all values of MyEnum::OtherVariant
            MyEnum::OtherVariant(..) => 2,

        }
    }
}
