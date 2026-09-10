//! Minimal end-to-end: derive + `#[bffi]` over a record type.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use bffi::{BffiRecord, BffiWire};

#[derive(BffiRecord, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
}

#[bffi::bffi]
fn make() -> Point {
    Point { x: 1.0 }
}

#[bffi::bffi]
fn shift(p: Point, dx: f64) -> Point {
    Point { x: p.x + dx }
}

#[test]
fn record_return_and_param_round_trip() {
    // make: handle out -> wire -> decode.
    let mut handle_val = 0_u64;
    assert_eq!(bffi_make(&mut handle_val), bffi::ErrorCode::Ok);
    let bytes = read_buffer(bffi::Handle::from_raw(handle_val));
    let (point, end) = Point::bffi_wire_decode(&bytes, 0).expect("decode");
    assert_eq!((point.x, end), (1.0, bytes.len()));

    // shift: wire param in, handle out.
    let mut wire = Vec::new();
    point.bffi_wire_encode(&mut wire);
    let mut out = 0_u64;
    let code = bffi_shift(wire.as_ptr(), wire.len() as u64, 2.0, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok);
    let bytes = read_buffer(bffi::Handle::from_raw(out));
    let (shifted, _) = Point::bffi_wire_decode(&bytes, 0).expect("decode");
    assert_eq!(shifted.x, 3.0);
}

/// Reads a transient-buffer handle back into bytes.
fn read_buffer(handle: bffi::Handle) -> Vec<u8> {
    // SAFETY: `buffer_ptr` handed out the pointer to exactly
    // `buffer_len(handle)` owned bytes; the handle is still live.
    let bytes = Vec::from(unsafe {
        std::slice::from_raw_parts(
            bffi::build::runtime::buffer_ptr(handle),
            bffi::build::runtime::buffer_len(handle) as usize,
        )
    });
    assert!(bffi::build::runtime::free_buffer(handle));
    bytes
}
