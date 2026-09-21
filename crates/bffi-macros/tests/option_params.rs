//! Option-parameter acceptance (sync paths): `Option<&str>`,
//! `Option<&[u8]>`, `Option<prim>`, `Option<i64>`/`Option<u64>`,
//! records and sequences cross as nullable ABI slots. `None`
//! conventions: NULL cstring, `len == 0` wire payload, the
//! `(value, flag)` pair; the descriptors record the `opt_*` ABI names
//! and the `| null` TS kinds.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use bffi::bffi_dts::{AbiType, TsType};
use bffi::{BffiRecord, BffiWire};

/// A point record.
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Labels with an optional prefix.
#[bffi::bffi]
fn opt_label(label: Option<&str>) -> String {
    label.unwrap_or("none").to_owned()
}

/// Measures an optional byte view (`Some(&[])` is distinct from `None`).
#[bffi::bffi]
fn opt_view(data: Option<&[u8]>) -> u64 {
    data.map_or(u64::MAX, |d| d.len() as u64)
}

/// Wraps an optional width.
#[bffi::bffi]
fn opt_width(w: Option<u32>) -> u32 {
    w.unwrap_or(u32::MAX)
}

/// Gates an optional flag (`None` defaults to `true`).
#[bffi::bffi]
fn opt_flag(flag: Option<bool>) -> bool {
    flag.unwrap_or(true)
}

/// Echoes an optional unsigned bigint.
#[bffi::bffi]
fn opt_big(v: Option<u64>) -> u64 {
    v.unwrap_or(42)
}

/// Echoes an optional signed bigint.
#[bffi::bffi]
fn opt_signed(v: Option<i64>) -> i64 {
    v.unwrap_or(77)
}

/// Sums an optional point.
#[bffi::bffi]
fn opt_point(p: Option<Point>) -> f64 {
    p.map_or(-1.0, |pt| pt.x + pt.y)
}

/// Counts an optional sequence.
#[bffi::bffi]
fn opt_scores(s: Option<Vec<f64>>) -> u32 {
    s.map_or(u32::MAX, |v| v.len() as u32)
}

/// Reads a transient-buffer handle back into bytes.
fn read_buffer(handle: bffi::Handle) -> Vec<u8> {
    // SAFETY: `buffer_ptr` handed out the pointer to exactly
    // `buffer_len(handle)` owned bytes; the handle is still live.
    Vec::from(unsafe {
        std::slice::from_raw_parts(
            bffi::build::runtime::buffer_ptr(handle),
            bffi::build::runtime::buffer_len(handle) as usize,
        )
    })
}

#[test]
fn optional_string_some_and_none() {
    let cstring = std::ffi::CString::new("héllo").expect("utf8");
    let mut handle_val = 0_u64;
    let code = bffi_opt_label(cstring.as_ptr(), &mut handle_val);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    let label = String::from_utf8(read_buffer(bffi::Handle::from_raw(handle_val))).expect("utf8");
    assert_eq!(label, "héllo");

    // A NULL cstring is `None`, not an error.
    let mut handle_val = 0_u64;
    let code = bffi_opt_label(std::ptr::null(), &mut handle_val);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    let label = String::from_utf8(read_buffer(bffi::Handle::from_raw(handle_val))).expect("utf8");
    assert_eq!(label, "none");
}

#[test]
fn optional_view_distinguishes_some_empty_from_none() {
    let bytes = [1_u8, 2, 3];
    let mut out = 0_u64;
    let code = bffi_opt_view(bytes.as_ptr(), 3, 1, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, 3);

    // `Some(&[])`: null pointer, zero length, flag set.
    let mut out = 0_u64;
    let code = bffi_opt_view(std::ptr::null(), 0, 1, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, 0);

    // `None`: the flag is clear.
    let mut out = 0_u64;
    let code = bffi_opt_view(std::ptr::null(), 0, 0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, u64::MAX);
}

#[test]
fn optional_number_some_and_none() {
    let mut out = 0_u32;
    let code = bffi_opt_width(7.0, 1, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, 7);

    let mut out = 0_u32;
    let code = bffi_opt_width(0.0, 0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, u32::MAX);
}

#[test]
fn optional_bool_some_false_differs_from_none() {
    let mut out = false;
    let code = bffi_opt_flag(0.0, 1, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert!(!out, "`Some(false)` must reach the callee");

    let mut out = false;
    let code = bffi_opt_flag(0.0, 0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert!(out, "`None` must take the default");
}

#[test]
fn optional_bigints_survive_the_extremes() {
    let mut out = 0_u64;
    let code = bffi_opt_big(u64::MAX, 1, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, u64::MAX);

    let mut out = 0_u64;
    let code = bffi_opt_big(0, 0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, 42);

    let mut out = 0_i64;
    let code = bffi_opt_signed(i64::MIN, 1, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, i64::MIN);

    let mut out = 0_i64;
    let code = bffi_opt_signed(0, 0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, 77);
}

#[test]
fn optional_record_none_is_the_zero_length_payload() {
    let point = Point { x: 1.0, y: 2.0 };
    let mut wire = Vec::new();
    point.bffi_wire_encode(&mut wire);

    let mut out = 0_f64;
    let code = bffi_opt_point(wire.as_ptr(), wire.len() as u64, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, 3.0);

    // `len == 0` is `None`: the mirror of the 0-handle return.
    let mut out = 0_f64;
    let code = bffi_opt_point(std::ptr::null(), 0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, -1.0);
}

#[test]
fn optional_sequence_none_is_the_zero_length_payload() {
    use bffi::bffi_types::wire as w;
    let mut wire = Vec::new();
    w::encode_seq_header(&mut wire, 2);
    w::encode_f64(&mut wire, 1.5);
    w::encode_f64(&mut wire, 2.5);

    let mut out = 0_u32;
    let code = bffi_opt_scores(wire.as_ptr(), wire.len() as u64, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, 2);

    let mut out = 0_u32;
    let code = bffi_opt_scores(std::ptr::null(), 0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());
    assert_eq!(out, u32::MAX);
}

#[test]
fn descriptors_carry_the_nullable_kinds_and_opt_abi_names() {
    assert_eq!(
        bffi_meta_opt_label::FUNCTION.params[0].ty,
        TsType::NullableString
    );
    assert_eq!(
        bffi_meta_opt_label::FUNCTION.abi.params[0],
        AbiType::Cstring
    );

    assert_eq!(
        bffi_meta_opt_view::FUNCTION.params[0].ty,
        TsType::NullableUint8Array
    );
    assert_eq!(
        bffi_meta_opt_view::FUNCTION.abi.params[0],
        AbiType::OptPtrLen
    );

    assert_eq!(
        bffi_meta_opt_width::FUNCTION.params[0].ty,
        TsType::NullableNumber
    );
    assert_eq!(
        bffi_meta_opt_width::FUNCTION.abi.params[0],
        AbiType::OptNumber
    );

    assert_eq!(
        bffi_meta_opt_flag::FUNCTION.params[0].ty,
        TsType::NullableBoolean
    );

    assert_eq!(
        bffi_meta_opt_big::FUNCTION.params[0].ty,
        TsType::NullableBigInt
    );
    assert_eq!(bffi_meta_opt_big::FUNCTION.abi.params[0], AbiType::OptU64);
    assert_eq!(
        bffi_meta_opt_signed::FUNCTION.abi.params[0],
        AbiType::OptI64
    );

    assert_eq!(
        bffi_meta_opt_point::FUNCTION.params[0].ty,
        TsType::NullableRecord("Point")
    );
    assert_eq!(bffi_meta_opt_point::FUNCTION.abi.params[0], AbiType::PtrLen);

    assert_eq!(
        bffi_meta_opt_scores::FUNCTION.params[0].ty,
        TsType::NullableNumberArray
    );
    assert_eq!(
        bffi_meta_opt_scores::FUNCTION.abi.params[0],
        AbiType::PtrLen
    );
}
