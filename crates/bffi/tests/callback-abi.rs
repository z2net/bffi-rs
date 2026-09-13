//! Integration tests for the generic callback ABI helpers
//! (`bffi::bffi_callback::abi`): wire encoding/decoding round-trips, the
//! bind/invoke/revoke bodies end-to-end (including the transient-buffer
//! result path), and the error contract (malformed bytes ->
//! `InvalidArgument` with a stored message, dead handles ->
//! `InvalidHandle`).
//!
//! The process stays UNBOUND here (see `src/registry.rs` tests): no
//! test binds the JS thread, so `invoke` admits every caller. The
//! sticky-binding path is covered by `tests/threading.rs` and - across
//! the real ABI - by the JS e2e suite.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use bffi::bffi_build::runtime::{buffer_len, buffer_ptr, free_buffer};
use bffi::bffi_callback::abi::{bind_body, decode_args, encode_result, invoke_body, revoke_body};
use bffi::bffi_callback::{CallbackSig, Value, ValueType, register};
use bffi::bffi_core::ErrorCode;
use bffi::bffi_core::take_last_error;
use bffi::bffi_types::wire;

fn i32_args(values: &[i32]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.push(wire::TAG_I32);
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[test]
fn decode_args_round_trips_the_full_matrix() {
    let mut bytes = Vec::new();
    Value::Unit.encode_into(&mut bytes);
    Value::I32(-2).encode_into(&mut bytes);
    Value::I64(1).encode_into(&mut bytes);
    Value::F64(1.5).encode_into(&mut bytes);
    Value::Bool(true).encode_into(&mut bytes);
    Value::Str("héllo".to_owned()).encode_into(&mut bytes);
    Value::Bytes(bffi::CopiedBuf::from_slice(&[9, 8])).encode_into(&mut bytes);

    assert_eq!(
        decode_args(&bytes).expect("valid records"),
        vec![
            Value::Unit,
            Value::I32(-2),
            Value::I64(1),
            Value::F64(1.5),
            Value::Bool(true),
            Value::Str("héllo".to_owned()),
            Value::Bytes(bffi::CopiedBuf::from_slice(&[9, 8])),
        ]
    );
    assert!(decode_args(&[]).expect("empty args are valid").is_empty());
}

#[test]
fn bind_body_accepts_the_extended_tag_matrix() {
    // The wry IPC signature: `unit(str)` - a void-returning callback
    // taking one cstring.
    let mut slot: u64 = 0;
    let status = bind_body(wire::TAG_UNIT, &[wire::TAG_STR], 0xBEEF, &mut slot);
    assert_eq!(status, ErrorCode::Ok);
    let info = bffi::bffi_callback::js_callback(bffi::bffi_core::Handle::from_raw(slot))
        .expect("the bound slot is readable");
    assert_eq!(
        info.sig,
        CallbackSig::new(ValueType::Unit, &[ValueType::Str])
    );
    assert!(revoke_body(slot) == ErrorCode::Ok.as_u32());

    // `Bytes` binds too (the dispatch is what rejects it later).
    let mut slot: u64 = 0;
    let status = bind_body(wire::TAG_BYTES, &[], 0, &mut slot);
    assert_eq!(status, ErrorCode::Ok);
    assert!(revoke_body(slot) == ErrorCode::Ok.as_u32());
}

#[test]
fn decode_args_rejects_unknown_and_truncated_records() {
    let error = decode_args(&[0xFF]).expect_err("unknown tag");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert!(
        error.message.contains("malformed callback arguments"),
        "unexpected message: {}",
        error.message
    );

    // A truncated i32 record (tag without the full payload).
    let error = decode_args(&[wire::TAG_I32, 1, 2]).expect_err("truncated payload");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
}

#[test]
fn encode_result_writes_one_wire_record() {
    assert_eq!(
        encode_result(Value::I32(5)),
        vec![wire::TAG_I32, 5, 0, 0, 0]
    );
    assert_eq!(encode_result(Value::Bool(false)), vec![wire::TAG_BOOL, 0]);
}

#[test]
fn bind_body_stores_the_signature_and_pointer() {
    let mut slot: u64 = 0;
    let status = bind_body(wire::TAG_I32, &[wire::TAG_I32], 0xDEAD_BEEF, &mut slot);
    assert_eq!(status, ErrorCode::Ok);
    assert_ne!(slot, 0, "a fresh handle must be written");

    let info = bffi::bffi_callback::js_callback(bffi::bffi_core::Handle::from_raw(slot))
        .expect("the bound slot is readable");
    assert_eq!(
        info.sig,
        CallbackSig::new(ValueType::I32, &[ValueType::I32])
    );
    assert_eq!(info.ptr, 0xDEAD_BEEF);

    assert!(revoke_body(slot) == ErrorCode::Ok.as_u32());
}

#[test]
fn bind_body_rejects_unknown_signature_tags() {
    let mut slot: u64 = 0;
    let status = bind_body(200, &[wire::TAG_I32], 0, &mut slot);
    assert_eq!(status, ErrorCode::InvalidArgument);
    let error = take_last_error().expect("a last error is stored");
    assert!(error.message.contains("unknown callback return tag 200"));

    // Tag 50 is outside the framework-wide wire table.
    let status = bind_body(wire::TAG_I32, &[50], 0, &mut slot);
    assert_eq!(status, ErrorCode::InvalidArgument);
    let error = take_last_error().expect("a last error is stored");
    assert!(
        error
            .message
            .contains("unknown callback parameter tag 50 at index 0")
    );
}

#[test]
fn bind_body_rejects_a_null_out_pointer() {
    let status = bind_body(wire::TAG_I32, &[], 0, std::ptr::null_mut());
    assert_eq!(status, ErrorCode::NullPointer);
}

#[test]
fn invoke_body_returns_the_result_through_the_buffer_table() {
    let handle = register(
        CallbackSig::new(ValueType::I32, &[ValueType::I32]),
        Arc::new(|args: &[Value]| match args {
            [Value::I32(a)] => Value::I32(a * 2),
            _ => unreachable!("invoke checks the signature"),
        }),
    )
    .expect("table has room");

    let mut slot: u64 = 0;
    let args = i32_args(&[21]);
    let status = invoke_body(handle.as_u64(), &args, &mut slot);
    assert_eq!(status, ErrorCode::Ok);
    assert_ne!(slot, 0, "the result buffer handle must be written");

    let len = buffer_len(bffi::bffi_core::Handle::from_raw(slot));
    assert_eq!(len, 5, "one i32 wire record");
    let ptr = buffer_ptr(bffi::bffi_core::Handle::from_raw(slot));
    assert!(!ptr.is_null());
    // SAFETY: the pointer is valid until the free below and covers
    // exactly `len` bytes (CALLING-CONVENTION.md §5).
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    assert_eq!(bytes, &[wire::TAG_I32, 42, 0, 0, 0]);

    assert!(free_buffer(bffi::bffi_core::Handle::from_raw(slot)));
    assert!(revoke_body(handle.as_u64()) == ErrorCode::Ok.as_u32());
}

#[test]
fn invoke_body_maps_malformed_args_to_invalid_argument() {
    let handle = register(
        CallbackSig::new(ValueType::I32, &[ValueType::I32]),
        Arc::new(|_| Value::I32(0)),
    )
    .expect("table has room");

    let mut slot: u64 = 0;
    let status = invoke_body(handle.as_u64(), &[0xFF], &mut slot);
    assert_eq!(status, ErrorCode::InvalidArgument);
    let error = take_last_error().expect("a last error is stored");
    assert!(error.message.contains("malformed callback arguments"));
}

#[test]
fn invoke_body_maps_signature_mismatch_to_invalid_argument() {
    let handle = register(
        CallbackSig::new(ValueType::I32, &[ValueType::I32]),
        Arc::new(|_| Value::I32(0)),
    )
    .expect("table has room");

    // An EMPTY argument record is a valid wire encoding of "no
    // arguments" - and a deliberate arity mismatch.
    let mut slot: u64 = 0;
    let status = invoke_body(handle.as_u64(), &[], &mut slot);
    assert_eq!(status, ErrorCode::InvalidArgument);
    let error = take_last_error().expect("a last error is stored");
    assert!(
        error.message.contains("callback signature mismatch"),
        "unexpected message: {}",
        error.message
    );
}

#[test]
fn invoke_body_maps_dead_handles_to_invalid_handle() {
    let handle = register(
        CallbackSig::new(ValueType::Bool, &[]),
        Arc::new(|_| Value::Bool(true)),
    )
    .expect("table has room");
    assert!(bffi::bffi_callback::revoke(handle));

    let mut slot: u64 = 0;
    let status = invoke_body(handle.as_u64(), &[], &mut slot);
    assert_eq!(status, ErrorCode::InvalidHandle);
}

#[test]
fn revoke_body_reports_ok_once_then_invalid_handle() {
    let mut slot: u64 = 0;
    let status = bind_body(wire::TAG_BOOL, &[], 0, &mut slot);
    assert_eq!(status, ErrorCode::Ok);

    assert_eq!(revoke_body(slot), ErrorCode::Ok.as_u32());
    assert_eq!(revoke_body(slot), ErrorCode::InvalidHandle.as_u32());
}
