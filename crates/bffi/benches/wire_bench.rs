//! Criterion benches for the wire codec (`bffi::types::wire`): one
//! representative value set (i32, i64, f64, bool, a ~32-byte string,
//! a 256-byte byte payload, a 4-field record, a 100-item sequence)
//! composed with the value-level helpers, and the decode side reading
//! the same buffer back field by field.

#![allow(missing_docs)] // the criterion_group! expansion is undocumented
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bffi::types::wire::{
    decode_bool, decode_bytes, decode_f64, decode_i32, decode_i64, decode_record_header,
    decode_seq_header, decode_str, encode_bool, encode_bytes, encode_f64, encode_i32, encode_i64,
    encode_record_header, encode_seq_header, encode_str,
};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// The representative ~32-byte string.
const STR_32: &str = "the quick brown fox jumps over!!";

/// The representative 256-byte payload.
const BYTES_256: [u8; 256] = [0x5A; 256];

/// The sequence length of the value set.
const SEQ_ITEMS: usize = 100;

/// The fixed scalar arguments (stable across the encode/decode pair).
const I32_VALUE: i32 = -123_456;
const I64_VALUE: i64 = i64::MIN;
const F64_VALUE: f64 = 8421.375;

/// Encodes the full representative value set into one fresh buffer:
/// the exact shape a `#[derive(BffiRecord)]` struct plus a `Vec<T>`
/// field emit through the value-level helpers.
fn encode_value_set() -> Vec<u8> {
    let mut out = Vec::with_capacity(1024);
    encode_i32(&mut out, I32_VALUE);
    encode_i64(&mut out, I64_VALUE);
    encode_f64(&mut out, F64_VALUE);
    encode_bool(&mut out, true);
    encode_str(&mut out, STR_32);
    encode_bytes(&mut out, &BYTES_256);
    // A 4-field record.
    encode_record_header(&mut out, 4);
    encode_i32(&mut out, 1);
    encode_i64(&mut out, 2);
    encode_f64(&mut out, 3.5);
    encode_str(&mut out, "four");
    // A 100-item sequence.
    encode_seq_header(&mut out, SEQ_ITEMS);
    for i in 0..SEQ_ITEMS {
        encode_i32(&mut out, i as i32);
    }
    out
}

/// Decodes the full value set from `buf` and returns the end offset;
/// self-checking, so a corrupt buffer fails the bench loudly instead
/// of timing a no-op.
fn decode_value_set(buf: &[u8]) -> usize {
    let mut offset = 0;
    let (v, next) = decode_i32(buf, offset).expect("i32 record");
    assert_eq!(v, I32_VALUE);
    offset = next;
    let (v, next) = decode_i64(buf, offset).expect("i64 record");
    assert_eq!(v, I64_VALUE);
    offset = next;
    let (v, next) = decode_f64(buf, offset).expect("f64 record");
    assert_eq!(v, F64_VALUE);
    offset = next;
    let (v, next) = decode_bool(buf, offset).expect("bool record");
    assert!(v);
    offset = next;
    let (v, next) = decode_str(buf, offset).expect("str record");
    assert_eq!(v, STR_32);
    offset = next;
    let (v, next) = decode_bytes(buf, offset).expect("bytes record");
    assert_eq!(v, &BYTES_256);
    offset = next;

    let (count, next) = decode_record_header(buf, offset).expect("record header");
    assert_eq!(count, 4);
    offset = next;
    let (v, next) = decode_i32(buf, offset).expect("record field 0");
    assert_eq!(v, 1);
    offset = next;
    let (v, next) = decode_i64(buf, offset).expect("record field 1");
    assert_eq!(v, 2);
    offset = next;
    let (v, next) = decode_f64(buf, offset).expect("record field 2");
    assert_eq!(v, 3.5);
    offset = next;
    let (v, next) = decode_str(buf, offset).expect("record field 3");
    assert_eq!(v, "four");
    offset = next;

    let (count, next) = decode_seq_header(buf, offset).expect("seq header");
    assert_eq!(count, SEQ_ITEMS);
    offset = next;
    for i in 0..count {
        let (v, next) = decode_i32(buf, offset).expect("seq item");
        assert_eq!(v, i as i32);
        offset = next;
    }
    offset
}

fn bench_wire(c: &mut Criterion) {
    // Encode: a fresh buffer per iteration (the encoder is infallible).
    c.bench_function("wire/encode_value_set", |b| {
        b.iter(|| black_box(encode_value_set()))
    });

    // Decode round trip: the pre-encoded buffer read back field by
    // field.
    let buf = encode_value_set();
    assert_eq!(
        decode_value_set(&buf),
        buf.len(),
        "the value set round-trips exactly"
    );
    c.bench_function("wire/decode_value_set", |b| {
        b.iter(|| black_box(decode_value_set(black_box(&buf))))
    });
}

criterion_group!(benches, bench_wire);
criterion_main!(benches);
