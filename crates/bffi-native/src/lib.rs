//! The bffi-rs reference native library: a minimal cdylib expanding
//! the runtime ABI (`bffi_error_*`, the `bffi_buffer` pair,
//! `bffi_types_free`).
//!
//! This is the library the napi-rs-style platform npm packages carry
//! (`@z2net/bffi-native-<triple>`, assembled by `bffi pack` from the
//! release artifacts). It is deliberately tiny - the point is a REAL
//! bffi surface crossing the boundary, loadable through the
//! `@z2net/bffi` loader without building anything locally:
//!
//! - [`add`] - primitives and the `__ret` out-parameter;
//! - [`shout`] - the cstring (`&str`) parameter path and a string
//!   return (transient-buffer handle);
//! - [`version`] - the crate version as a static string return.
//!
//! Aggregation lives in [`module_def`] (single source); the
//! `emit-json` binary materializes `.bffi/bffi.api.json` from it.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free): the JS pipeline drains errors and reads buffers
// through them.
bffi::bffi_runtime_abi!();

pub mod module_def;

use bffi::bffi;

/// Adds two numbers.
#[bffi]
pub fn add(a: u32, b: u32) -> u32 {
    a.wrapping_add(b)
}

/// Returns an uppercased greeting (string return through the
/// transient-buffer pair).
#[bffi]
pub fn shout(name: &str) -> String {
    format!("HELLO {name}!")
}

/// The version of this library (static string return).
#[bffi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
