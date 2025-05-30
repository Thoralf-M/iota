// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[defines_primitive(u64)]
module std::u64;

use std::string::String;

/// Returns the bitwise not of the value.
/// Each bit that is 1 becomes 0. Each bit that is 0 becomes 1.
public fun bitwise_not(x: u64): u64 {
    x ^ max_value!()
}

/// Return the larger of `x` and `y`
public fun max(x: u64, y: u64): u64 {
    std::macros::num_max!(x, y)
}

/// Return the smaller of `x` and `y`
public fun min(x: u64, y: u64): u64 {
    std::macros::num_min!(x, y)
}

/// Return the absolute value of x - y
public fun diff(x: u64, y: u64): u64 {
    std::macros::num_diff!(x, y)
}

/// Calculate x / y, but round up the result.
public fun divide_and_round_up(x: u64, y: u64): u64 {
    std::macros::num_divide_and_round_up!(x, y)
}

/// Return the value of a base raised to a power
public fun pow(base: u64, exponent: u8): u64 {
    std::macros::num_pow!(base, exponent)
}

/// Get a nearest lower integer Square Root for `x`. Given that this
/// function can only operate with integers, it is impossible
/// to get perfect (or precise) integer square root for some numbers.
///
/// Example:
/// ```
/// math::sqrt(9) => 3
/// math::sqrt(8) => 2 // the nearest lower square root is 4;
/// ```
///
/// In integer math, one of the possible ways to get results with more
/// precision is to use higher values or temporarily multiply the
/// value by some bigger number. Ideally if this is a square of 10 or 100.
///
/// Example:
/// ```
/// math::sqrt(8) => 2;
/// math::sqrt(8 * 10000) => 282;
/// // now we can use this value as if it was 2.82;
/// // but to get the actual result, this value needs
/// // to be divided by 100 (because sqrt(10000)).
///
///
/// math::sqrt(8 * 1000000) => 2828; // same as above, 2828 / 1000 (2.828)
/// ```
public fun sqrt(x: u64): u64 {
    std::macros::num_sqrt!<u64, u128>(x, 64)
}

/// Try to convert a `u64` to a `u8`. Returns `None` if the value is too large.
public fun try_as_u8(x: u64): Option<u8> {
    std::macros::try_as_u8!(x)
}

/// Try to convert a `u64` to a `u16`. Returns `None` if the value is too large.
public fun try_as_u16(x: u64): Option<u16> {
    std::macros::try_as_u16!(x)
}

/// Try to convert a `u64` to a `u32`. Returns `None` if the value is too large.
public fun try_as_u32(x: u64): Option<u32> {
    std::macros::try_as_u32!(x)
}

public fun to_string(x: u64): String {
    std::macros::num_to_string!(x)
}

/// Maximum value for a `u64`
public macro fun max_value(): u64 {
    0xFFFF_FFFF_FFFF_FFFF
}

/// Loops applying `$f` to each number from `$start` to `$stop` (exclusive)
public macro fun range_do($start: u64, $stop: u64, $f: |u64|) {
    std::macros::range_do!($start, $stop, $f)
}

/// Loops applying `$f` to each number from `$start` to `$stop` (inclusive)
public macro fun range_do_eq($start: u64, $stop: u64, $f: |u64|) {
    std::macros::range_do_eq!($start, $stop, $f)
}

/// Loops applying `$f` to each number from `0` to `$stop` (exclusive)
public macro fun do($stop: u64, $f: |u64|) {
    std::macros::do!($stop, $f)
}

/// Loops applying `$f` to each number from `0` to `$stop` (inclusive)
public macro fun do_eq($stop: u64, $f: |u64|) {
    std::macros::do_eq!($stop, $f)
}
