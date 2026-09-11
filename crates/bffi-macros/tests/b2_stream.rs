//! B2 acceptance: `#[bffi_stream]` over number and record items -
//! the generated spawn shim registers the stream, chunks decode back
//! in order, and the descriptors carry the `AsyncIterableIterator`
//! types.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use bffi::bffi_dts::TsType;
use bffi::{BffiEnum, BffiRecord, BffiWire};

// The generic stream exports (bffi_stream_next / bffi_stream_drop):
// the JS pipeline pulls chunks through them.
bffi::bffi_stream_abi!();

/// A reading axis.
#[derive(BffiEnum, Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// One stream item.
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Sample {
    pub at: f64,
    pub axis: Axis,
}

/// A fixed numeric sequence.
#[bffi::bffi_stream]
fn numbers() -> impl Iterator<Item = i32> + Send {
    1..=5
}

/// Records filtered out of a generated sequence.
#[bffi::bffi_stream]
fn samples(count: u32) -> impl Iterator<Item = Sample> + Send {
    (0..count).map(|index| Sample {
        at: f64::from(index),
        axis: if index % 2 == 0 {
            Axis::Horizontal
        } else {
            Axis::Vertical
        },
    })
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

#[test]
fn number_stream_round_trips_in_order() {
    let mut handle = 0_u64;
    assert_eq!(bffi_numbers(&mut handle), bffi::ErrorCode::Ok);
    let mut seen = Vec::new();
    loop {
        let mut chunk_handle = 0_u64;
        assert_eq!(
            bffi_stream_next(handle, 2, &mut chunk_handle),
            bffi::ErrorCode::Ok.as_u32()
        );
        if chunk_handle == 0 {
            break;
        }
        let bytes = read_buffer(bffi::Handle::from_raw(chunk_handle));
        use bffi::bffi_types::wire as w;
        let (count, mut offset) = w::decode_seq_header(&bytes, 0).expect("seq header");
        for _ in 0..count {
            let (value, next) = w::decode_i32(&bytes, offset).expect("item");
            offset = next;
            seen.push(value);
        }
    }
    assert_eq!(seen, [1, 2, 3, 4, 5]);
}

#[test]
fn record_stream_round_trips_with_params() {
    let mut handle = 0_u64;
    assert_eq!(bffi_samples(3, &mut handle), bffi::ErrorCode::Ok);
    let mut seen = Vec::new();
    loop {
        let mut chunk_handle = 0_u64;
        assert_eq!(
            bffi_stream_next(handle, 2, &mut chunk_handle),
            bffi::ErrorCode::Ok.as_u32()
        );
        if chunk_handle == 0 {
            break;
        }
        let bytes = read_buffer(bffi::Handle::from_raw(chunk_handle));
        let (count, mut offset) =
            bffi::bffi_types::wire::decode_seq_header(&bytes, 0).expect("seq header");
        for _ in 0..count {
            let (value, next) = Sample::bffi_wire_decode(&bytes, offset).expect("item");
            offset = next;
            seen.push(value);
        }
    }
    assert_eq!(
        seen,
        vec![
            Sample {
                at: 0.0,
                axis: Axis::Horizontal
            },
            Sample {
                at: 1.0,
                axis: Axis::Vertical
            },
            Sample {
                at: 2.0,
                axis: Axis::Horizontal
            },
        ]
    );
}

#[test]
fn descriptors_carry_the_stream_types() {
    assert_eq!(bffi_meta_numbers::FUNCTION.ret, TsType::StreamNumber);
    assert_eq!(
        bffi_meta_samples::FUNCTION.ret,
        TsType::StreamRecord("Sample")
    );
    assert_eq!(bffi_meta_samples::FUNCTION.params[0].ty, TsType::Number);
    assert_eq!(
        bffi_meta_numbers::FUNCTION.abi.out,
        Some(bffi::AbiOut::Handle)
    );
}
