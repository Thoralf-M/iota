// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2025 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

/// Module: example
module example::example;

use dependency::dependency::f;

public fun g(): u64 { f() }
