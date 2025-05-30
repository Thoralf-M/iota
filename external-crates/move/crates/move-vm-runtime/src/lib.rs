// Copyright (c) The Diem Core Contributors
// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

//! The core Move VM logic.
//!
//! It is a design goal for the Move VM to be independent of the Diem
//! blockchain, so that other blockchains can use it as well. The VM isn't there
//! yet, but hopefully will be there soon.

pub mod data_cache;
mod interpreter;
mod loader;
pub mod logging;
pub mod move_vm;
pub mod native_extensions;
pub mod native_functions;
pub mod runtime;
pub mod session;
#[macro_use]
mod tracing;
mod tracing2;

// Only include debugging functionality in debug builds
#[cfg(any(debug_assertions, feature = "tracing"))]
mod debug;

#[cfg(test)]
mod unit_tests;
