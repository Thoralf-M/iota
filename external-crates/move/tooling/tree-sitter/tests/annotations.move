// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[allow(unused_const, unused_variable)]

#[allow(unused_const, unused_variable)]
#[test, expected_failure(abort_code = ::staking::staking_tests::ENotImplemented)]
#[test, expected_failure(abort_code = other_module::ENotFound)]
#[expected_failure(arithmetic_error, location = pkg_addr::other_module)]

// #[allow(unused_const, unused_variable)]
module c::k {}
