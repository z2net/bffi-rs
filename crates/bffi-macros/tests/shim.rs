//! Acceptance tests for the generated `extern "C"` shims
//! (kanboard 7.1): every shim validates its pointers, copies `&str`
//! inputs through the cstring path, and transports results via
//! out-parameters under the boundary policy.
//!
//! The shims are safe functions: pointer *parameters* do not make an
//! `fn` unsafe, and every dereference inside the shim is guarded by
//! the shim's own checks.

#![allow(clippy::expect_used, clippy::unwrap_used)]
// The probe namespace modules mirror the facade namespaces; the
// generated shims carry the docs.
#![allow(missing_docs)]

use bffi::{CopiedBuf, ErrorCode, take_last_error};

#[bffi_macros::bffi]
/// Adds two numbers.
fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[bffi_macros::bffi]
fn widen(x: i32) -> i64 {
    x as i64
}

#[bffi_macros::bffi]
fn flip(x: u64) -> bool {
    x == 0
}

#[bffi_macros::bffi]
fn shout(phrase: &str) -> u32 {
    phrase.len() as u32
}

#[bffi_macros::bffi]
/// Touches nothing.
fn touch(x: u32) {
    let _ = x;
}

#[bffi_macros::bffi]
/// Echoes the word with an exclamations mark.
fn echo_word(word: &str) -> String {
    format!("{word}!")
}

#[bffi_macros::bffi]
fn raw_bytes() -> Vec<u8> {
    vec![1, 2, 3]
}

#[bffi_macros::bffi]
fn copied() -> CopiedBuf {
    CopiedBuf::from_slice(b"owned")
}

#[bffi_macros::bffi]
fn maybe_word(flag: bool) -> Option<String> {
    if flag { Some("yes".to_owned()) } else { None }
}

/// The `E` side of the err channel: a real domain error.
#[derive(Debug)]
struct DivError {
    divisor: u32,
}

impl std::fmt::Display for DivError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "division by {}", self.divisor)
    }
}

impl std::error::Error for DivError {}

#[bffi_macros::bffi]
fn checked_div(a: u32, b: u32) -> Result<u32, DivError> {
    a.checked_div(b).ok_or(DivError { divisor: b })
}

#[bffi_macros::bffi]
fn unit_result(flag: bool) -> Result<(), DivError> {
    flag.then_some(()).ok_or(DivError { divisor: 1 })
}

#[bffi_macros::bffi]
fn wild(_: u32) -> u32 {
    7
}

#[bffi_macros::bffi]
fn slice_len(data: &[u8]) -> u32 {
    data.len() as u32
}

#[bffi_macros::bffi]
fn echo_bytes(data: &[u8]) -> CopiedBuf {
    CopiedBuf::from_slice(data)
}

// Facade-only mode without the facade: `crate = "bffi_macros_probe"`
// makes the generated shim resolve through `::bffi_macros_probe::{core,
// types, dts, build}` instead of the direct deps. The `extern crate
// self` alias puts THIS crate into the extern prelude under the probe
// name, so the absolute paths resolve to the re-export modules below.
extern crate self as bffi_macros_probe;

pub mod core {
    pub use bffi::core::*;
}
pub mod types {
    pub use bffi::types::*;
}
pub mod dts {
    pub use bffi::dts::*;
}
pub mod build {
    pub use bffi::build::*;
}

#[bffi_macros::bffi(crate = "bffi_macros_probe")]
/// Triples through the probe namespaces.
fn triple(x: u32) -> u32 {
    x * 3
}

#[bffi_macros::bffi(crate = "bffi_macros_probe")]
/// Greets through the probe namespaces (cstring + buffer paths).
fn probe_shout(phrase: &str) -> String {
    format!("{phrase}!")
}

/// Converts `bytes` into a NUL-terminated cstring pointer for the
/// generated `&str` shim parameters. `into_raw` leaks on purpose:
/// test allocations are never reclaimed.
fn cstring(bytes: &[u8]) -> *const std::os::raw::c_char {
    std::ffi::CString::new(bytes)
        .expect("test bytes contain no interior NUL")
        .into_raw()
        .cast_const()
}

/// Reads and frees a transient buffer through the `bffi-build`
/// runtime API (the same surface the JS pair reads).
fn read_buffer(handle: bffi::Handle) -> Vec<u8> {
    // SAFETY: `buffer_ptr` handed out the pointer to exactly
    // `buffer_len(handle)` owned bytes; the handle is still live.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            bffi::build::runtime::buffer_ptr(handle),
            bffi::build::runtime::buffer_len(handle) as usize,
        )
    };
    let copy = bytes.to_vec();
    assert!(bffi::build::runtime::free_buffer(handle));
    copy
}

#[test]
fn shim_writes_out_param_and_returns_ok() {
    let mut out = 0_u32;
    let code = bffi_add(1, 2, &mut out);
    assert_eq!(code, ErrorCode::Ok);
    assert_eq!(out, 3);
    assert!(
        take_last_error().is_none(),
        "success must not store a last error"
    );
}

#[test]
fn shim_rejects_null_out_pointer() {
    let code = bffi_add(1, 2, std::ptr::null_mut());
    assert_eq!(code, ErrorCode::NullPointer);
    let error = take_last_error().expect("null out-pointer must store a last error");
    assert_eq!(error.code, ErrorCode::NullPointer);
}

#[test]
fn shim_handles_bigint_paths() {
    let mut wide = 0_i64;
    assert_eq!(bffi_widen(-5, &mut wide), ErrorCode::Ok);
    assert_eq!(wide, -5);

    let mut flag = false;
    assert_eq!(bffi_flip(0, &mut flag), ErrorCode::Ok);
    assert!(flag);
}

#[test]
fn shim_converts_cstrings_and_rejects_invalid_utf8() {
    let mut len = 0_u32;
    let hello = cstring(b"hello");
    assert_eq!(bffi_shout(hello, &mut len), ErrorCode::Ok);
    assert_eq!(len, 5);

    let invalid = cstring(&[0xFF_u8]);
    assert_eq!(bffi_shout(invalid, &mut len), ErrorCode::InvalidUtf8);
    let error = take_last_error().expect("invalid UTF-8 must store a last error");
    assert_eq!(error.code, ErrorCode::InvalidUtf8);
}

#[test]
fn shim_without_return_has_no_out_parameter() {
    // No out-parameter: the unit-returning shim takes only the fn's
    // own parameters.
    assert_eq!(bffi_touch(7), ErrorCode::Ok);
}

#[test]
fn string_return_yields_a_buffer_handle_with_the_utf8_bytes() {
    let mut handle = 0_u64;
    let code = bffi_echo_word(cstring(b"hey"), &mut handle);
    assert_eq!(code, ErrorCode::Ok);
    assert_ne!(handle, 0, "a stored string must produce a non-null handle");
    let bytes = read_buffer(bffi::Handle::from_raw(handle));
    assert_eq!(bytes, b"hey!");
}

#[test]
fn vec_and_copiedbuf_returns_yield_raw_byte_handles() {
    let mut vec_handle = 0_u64;
    assert_eq!(bffi_raw_bytes(&mut vec_handle), ErrorCode::Ok);
    assert_eq!(read_buffer(bffi::Handle::from_raw(vec_handle)), [1, 2, 3]);

    let mut copied_handle = 0_u64;
    assert_eq!(bffi_copied(&mut copied_handle), ErrorCode::Ok);
    assert_eq!(read_buffer(bffi::Handle::from_raw(copied_handle)), b"owned");
}

#[test]
fn option_none_writes_the_null_handle_and_some_stores_the_bytes() {
    let mut handle = 0_u64;
    assert_eq!(bffi_maybe_word(false, &mut handle), ErrorCode::Ok);
    assert_eq!(handle, 0, "None must write the documented 0 handle");

    assert_eq!(bffi_maybe_word(true, &mut handle), ErrorCode::Ok);
    assert_ne!(handle, 0);
    assert_eq!(read_buffer(bffi::Handle::from_raw(handle)), b"yes");
}

#[test]
fn result_ok_transports_the_inner_value_and_err_reports_code_13() {
    let mut out = 0_u32;
    assert_eq!(bffi_checked_div(10, 2, &mut out), ErrorCode::Ok);
    assert_eq!(out, 5);

    assert_eq!(bffi_checked_div(1, 0, &mut out), ErrorCode::DomainError);
    let error = take_last_error().expect("an Err must store the domain error");
    assert_eq!(error.code, ErrorCode::DomainError);
    assert_eq!(error.message, "division by 0");
    assert!(
        error.source.is_some(),
        "the domain error must survive as source"
    );
}

#[test]
fn unit_result_has_no_out_parameter_and_reports_the_err_channel() {
    assert_eq!(bffi_unit_result(true), ErrorCode::Ok);
    assert_eq!(bffi_unit_result(false), ErrorCode::DomainError);
    let error = take_last_error().expect("an Err must store the domain error");
    assert_eq!(error.message, "division by 1");
}

#[test]
fn wild_pattern_parameter_compiles_and_calls_cleanly() {
    let mut out = 0_u32;
    assert_eq!(bffi_wild(123, &mut out), ErrorCode::Ok);
    assert_eq!(out, 7);
}

#[test]
fn buffer_view_param_borrows_the_caller_bytes() {
    // A non-contiguous buffer so a copy bug would show as a wrong
    // length: the view must see exactly `len` bytes at `ptr`.
    let mut storage = [0xAA_u8, 1, 2, 3, 0xBB];

    let mut out = 0_u32;
    {
        let data = &storage[1..4];
        assert_eq!(
            bffi_slice_len(data.as_ptr(), data.len() as u64, &mut out),
            ErrorCode::Ok
        );
        assert_eq!(out, 3);
        assert!(
            take_last_error().is_none(),
            "success must not store a last error"
        );
    }

    // The borrowed bytes stay readable (and mutable) after the call:
    // the view never took ownership.
    storage[1] = 42;
    let data = &storage[1..4];
    assert_eq!(
        bffi_slice_len(data.as_ptr(), data.len() as u64, &mut out),
        ErrorCode::Ok
    );
    assert_eq!(out, 3);
}

#[test]
fn buffer_view_param_roundtrips_bytes_through_a_copiedbuf_handle() {
    let payload = [10_u8, 250, 0, 7];
    let mut handle = 0_u64;
    assert_eq!(
        bffi_echo_bytes(payload.as_ptr(), payload.len() as u64, &mut handle),
        ErrorCode::Ok
    );
    assert_ne!(handle, 0);
    // `CopiedBuf::from_slice` copies: the returned bytes outlive the
    // (already dead) borrow.
    assert_eq!(read_buffer(bffi::Handle::from_raw(handle)), [10, 250, 0, 7]);
}

#[test]
fn buffer_view_param_accepts_null_pointer_when_len_is_zero() {
    let mut out = 0_u32;
    assert_eq!(bffi_slice_len(std::ptr::null(), 0, &mut out), ErrorCode::Ok);
    assert_eq!(out, 0);
    assert!(
        take_last_error().is_none(),
        "the empty-view success must not store a last error"
    );
}

#[test]
fn buffer_view_param_rejects_null_pointer_with_nonzero_len() {
    let mut out = 0_u32;
    assert_eq!(
        bffi_slice_len(std::ptr::null(), 5, &mut out),
        ErrorCode::NullPointer
    );
    let error = take_last_error().expect("a null data pointer must store a last error");
    assert_eq!(error.code, ErrorCode::NullPointer);
    assert_eq!(error.message, "buffer argument pointer is null");
}

#[test]
fn crate_option_shim_calls_through_the_probe_namespaces() {
    let mut out = 0_u32;
    assert_eq!(bffi_triple(7, &mut out), ErrorCode::Ok);
    assert_eq!(out, 21);
}

#[test]
fn crate_option_shim_walks_the_string_and_buffer_paths() {
    let mut handle = 0_u64;
    let hey = cstring(b"hey");
    assert_eq!(bffi_probe_shout(hey, &mut handle), ErrorCode::Ok);
    assert_ne!(handle, 0);
    assert_eq!(read_buffer(bffi::Handle::from_raw(handle)), b"hey!");
}
