//! The shared wire codec: the `[tag: u8][payload]` record format every
//! byte-carrying payload uses when it travels through the
//! transient-buffer table (async task results, callback arguments and
//! results).
//!
//! ONE tag table serves the whole framework: `bffi-async` and
//! `bffi-callback` quote these constants, and the JS-side decoder
//! (`packages/bffi/src/runtime/wire.ts`) mirrors them. Little-endian
//! everywhere; `i64`/`u64` payloads are exact (no `f64` narrowing);
//! `Str`/`Bytes` payloads carry a `u32` LE length prefix.
//!
//! | Tag | Payload                                   |
//! | --- | ----------------------------------------- |
//! | 0   | `Unit` (no bytes)                         |
//! | 1   | `i32` (4 bytes LE)                        |
//! | 2   | `i64` (8 bytes LE)                        |
//! | 3   | `f64` (8 bytes LE)                        |
//! | 4   | `bool` (1 byte, `0`/`1`)                  |
//! | 5   | UTF-8 string (`u32` LE length + bytes)    |
//! | 6   | raw bytes (`u32` LE length + bytes)       |
//! | 7   | record (`u32` LE field count + one value record per field, positional - the field names come from the descriptor, not the wire) |
//! | 8   | sequence (`u32` LE item count + one value record per item) |
//!
//! Tags 7-8 are the composite containers of the B1 type matrix: a
//! `#[derive(BffiRecord)]` struct serializes as one record value, a
//! `Vec<T>` crossing the boundary as one sequence value. Fields and
//! items are themselves full `[tag][payload]` records, so decoding
//! is recursive and self-describing.
//!
//! The module holds no unsafe and never panics: every reader is total
//! (`Option`), every writer is an infallible `Vec` push.

/// The `Unit` record: a single tag byte, no payload.
pub const TAG_UNIT: u8 = 0;
/// The `i32` record.
pub const TAG_I32: u8 = 1;
/// The `i64` record.
pub const TAG_I64: u8 = 2;
/// The `f64` record.
pub const TAG_F64: u8 = 3;
/// The `bool` record.
pub const TAG_BOOL: u8 = 4;
/// The UTF-8 string record (`u32` LE length prefix).
pub const TAG_STR: u8 = 5;
/// The raw-bytes record (`u32` LE length prefix).
pub const TAG_BYTES: u8 = 6;
/// The record value (`u32` LE field count + one value record per
/// field, positional): the wire form of a `#[derive(BffiRecord)]`
/// struct. Field names live in the descriptor, not the payload.
pub const TAG_RECORD: u8 = 7;
/// The sequence value (`u32` LE item count + one value record per
/// item): the wire form of a `Vec<T>` whose item type is not `u8`.
pub const TAG_SEQ: u8 = 8;

/// Appends one little-endian `u32`.
pub fn push_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Appends one little-endian `i32`.
pub fn push_i32_le(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Appends one little-endian `i64`.
pub fn push_i64_le(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Appends one little-endian `f64`.
pub fn push_f64_le(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Appends one `bool` byte (`0`/`1`).
pub fn push_bool(out: &mut Vec<u8>, value: bool) {
    out.push(u8::from(value));
}

/// Reads one little-endian `u32` at `offset`; `None` past the end.
#[must_use]
pub fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let tail = bytes.get(offset..offset + 4)?;
    let mut raw = [0_u8; 4];
    raw.copy_from_slice(tail);
    Some(u32::from_le_bytes(raw))
}

/// Reads one little-endian `i32` at `offset`; `None` past the end.
#[must_use]
pub fn read_i32_le(bytes: &[u8], offset: usize) -> Option<i32> {
    let tail = bytes.get(offset..offset + 4)?;
    let mut raw = [0_u8; 4];
    raw.copy_from_slice(tail);
    Some(i32::from_le_bytes(raw))
}

/// Reads one little-endian `i64` at `offset`; `None` past the end.
#[must_use]
pub fn read_i64_le(bytes: &[u8], offset: usize) -> Option<i64> {
    let tail = bytes.get(offset..offset + 8)?;
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(tail);
    Some(i64::from_le_bytes(raw))
}

/// Reads one little-endian `f64` at `offset`; `None` past the end.
#[must_use]
pub fn read_f64_le(bytes: &[u8], offset: usize) -> Option<f64> {
    let tail = bytes.get(offset..offset + 8)?;
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(tail);
    Some(f64::from_le_bytes(raw))
}

/// Reads one `bool` byte at `offset` (`0` = false, anything else =
/// true); `None` past the end.
#[must_use]
pub fn read_bool(bytes: &[u8], offset: usize) -> Option<bool> {
    let raw = *bytes.get(offset)?;
    Some(raw != 0)
}

#[cfg(test)]
mod tests {
    use super::{
        TAG_BOOL, TAG_BYTES, TAG_F64, TAG_I32, TAG_I64, TAG_RECORD, TAG_SEQ, TAG_STR, TAG_UNIT,
        push_bool, push_f64_le, push_i32_le, push_i64_le, push_u32_le, read_bool, read_f64_le,
        read_i32_le, read_i64_le, read_u32_le,
    };

    #[test]
    fn tag_table_matches_the_documented_layout() {
        assert_eq!(TAG_UNIT, 0);
        assert_eq!(TAG_I32, 1);
        assert_eq!(TAG_I64, 2);
        assert_eq!(TAG_F64, 3);
        assert_eq!(TAG_BOOL, 4);
        assert_eq!(TAG_STR, 5);
        assert_eq!(TAG_BYTES, 6);
        // The B1 composite containers (records, Vec<T> sequences).
        // Tags are ABI: the JS mirror (packages/bffi wire.ts) must
        // agree forever - never renumber.
        assert_eq!(TAG_RECORD, 7);
        assert_eq!(TAG_SEQ, 8);
    }

    #[test]
    fn a_record_value_composes_from_child_records() {
        // Point { x: 1, y: -2 } as a hand-composed record value: the
        // shape the #[derive(BffiRecord)] encoder emits.
        let mut out = Vec::new();
        out.push(TAG_RECORD);
        push_u32_le(&mut out, 2);
        out.push(TAG_I32);
        push_i32_le(&mut out, 1);
        out.push(TAG_I32);
        push_i32_le(&mut out, -2);

        assert_eq!(out.len(), 1 + 4 + 2 * (1 + 4));
        assert_eq!(out[0], TAG_RECORD);
        assert_eq!(read_u32_le(&out, 1), Some(2));
        assert_eq!(out[5], TAG_I32);
        assert_eq!(read_i32_le(&out, 6), Some(1));
        assert_eq!(out[10], TAG_I32);
        assert_eq!(read_i32_le(&out, 11), Some(-2));
    }

    #[test]
    fn a_sequence_value_composes_from_child_records() {
        // vec![true, false] as a hand-composed sequence value.
        let mut out = Vec::new();
        out.push(TAG_SEQ);
        push_u32_le(&mut out, 2);
        out.push(TAG_BOOL);
        push_bool(&mut out, true);
        out.push(TAG_BOOL);
        push_bool(&mut out, false);

        assert_eq!(out.len(), 1 + 4 + 2 * (1 + 1));
        assert_eq!(read_u32_le(&out, 1), Some(2));
        assert_eq!(out[5], TAG_BOOL);
        assert_eq!(read_bool(&out, 6), Some(true));
        assert_eq!(out[7], TAG_BOOL);
        assert_eq!(read_bool(&out, 8), Some(false));
    }

    #[test]
    fn pushes_and_reads_round_trip_little_endian() {
        let mut out = Vec::new();
        push_u32_le(&mut out, 0xDEAD_BEEF_u32);
        push_i32_le(&mut out, -2);
        push_i64_le(&mut out, 1);
        push_f64_le(&mut out, 1.5);
        push_bool(&mut out, true);

        assert_eq!(read_u32_le(&out, 0), Some(0xDEAD_BEEF));
        assert_eq!(read_i32_le(&out, 4), Some(-2));
        assert_eq!(read_i64_le(&out, 8), Some(1));
        assert_eq!(read_f64_le(&out, 16), Some(1.5));
        assert_eq!(read_bool(&out, 24), Some(true));
        assert_eq!(out.len(), 25);
    }

    #[test]
    fn readers_past_the_end_are_none() {
        let mut out = Vec::new();
        push_i32_le(&mut out, 7);
        assert_eq!(read_i32_le(&out, 1), None);
        assert_eq!(read_i64_le(&out, 0), None);
        assert_eq!(read_f64_le(&out, 0), None);
        assert_eq!(read_u32_le(&[], 0), None);
        assert_eq!(read_bool(&[], 0), None);
    }
}
