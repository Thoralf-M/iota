// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[test_only]
module iota::url_tests;

use iota::url;

const EUrlStringMismatch: u64 = 1;

#[test]
fun test_basic_url() {
    // url strings are not currently validated
    let url_str = x"414243454647".to_ascii_string();

    let url = url::new_unsafe(url_str);
    assert!(url::inner_url(&url) == url_str, EUrlStringMismatch);
}
