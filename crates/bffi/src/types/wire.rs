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

// ---------------------------------------------------------------------------
// Value-level helpers: one `[tag][payload]` record each.
//
// The `encode_*` writers append a complete value record; the
// `decode_*` readers validate the tag at `offset`, return the value
// and the offset PAST the record. These are the primitives the
// `#[derive(BffiRecord)]` / `#[derive(BffiEnum)]` expansions call, so
// the generated code stays tiny and reviewable. Malformed input is a
// `BffiError` with `InvalidArgument` - decode failures never panic
// and never read out of bounds.
// ---------------------------------------------------------------------------

use crate::bffi_core::{BffiError, ErrorCode};

/// Builds the canonical malformed-wire error.
fn wire_error(what: &str) -> BffiError {
    BffiError::new(ErrorCode::InvalidArgument, what)
}

/// Reads the tag byte at `offset` and checks it against `expected`;
/// returns the payload offset (one past the tag).
fn expect_tag(bytes: &[u8], offset: usize, expected: u8, what: &str) -> Result<usize, BffiError> {
    let tag = *bytes.get(offset).ok_or_else(|| wire_error(what))?;
    if tag != expected {
        return Err(wire_error(what));
    }
    Ok(offset + 1)
}

/// Appends one `i32` value record.
pub fn encode_i32(out: &mut Vec<u8>, value: i32) {
    out.push(TAG_I32);
    push_i32_le(out, value);
}

/// Appends one `i64` value record.
pub fn encode_i64(out: &mut Vec<u8>, value: i64) {
    out.push(TAG_I64);
    push_i64_le(out, value);
}

/// Appends one `f64` value record.
pub fn encode_f64(out: &mut Vec<u8>, value: f64) {
    out.push(TAG_F64);
    push_f64_le(out, value);
}

/// Appends one `bool` value record.
pub fn encode_bool(out: &mut Vec<u8>, value: bool) {
    out.push(TAG_BOOL);
    push_bool(out, value);
}

/// Appends one UTF-8 string value record.
pub fn encode_str(out: &mut Vec<u8>, value: &str) {
    out.push(TAG_STR);
    push_u32_le(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

/// Appends one raw-bytes value record.
pub fn encode_bytes(out: &mut Vec<u8>, value: &[u8]) {
    out.push(TAG_BYTES);
    push_u32_le(out, value.len() as u32);
    out.extend_from_slice(value);
}

/// Appends one record header (tag + field count); the field records
/// follow.
pub fn encode_record_header(out: &mut Vec<u8>, field_count: usize) {
    out.push(TAG_RECORD);
    push_u32_le(out, field_count as u32);
}

/// Appends one sequence header (tag + item count); the item records
/// follow.
pub fn encode_seq_header(out: &mut Vec<u8>, item_count: usize) {
    out.push(TAG_SEQ);
    push_u32_le(out, item_count as u32);
}

/// Decodes one `i32` value record at `offset`.
pub fn decode_i32(bytes: &[u8], offset: usize) -> Result<(i32, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_I32, "wire: expected i32")?;
    let value = read_i32_le(bytes, at).ok_or_else(|| wire_error("wire: truncated i32"))?;
    Ok((value, at + 4))
}

/// Decodes one `i64` value record at `offset`.
pub fn decode_i64(bytes: &[u8], offset: usize) -> Result<(i64, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_I64, "wire: expected i64")?;
    let value = read_i64_le(bytes, at).ok_or_else(|| wire_error("wire: truncated i64"))?;
    Ok((value, at + 8))
}

/// Decodes one `f64` value record at `offset`.
pub fn decode_f64(bytes: &[u8], offset: usize) -> Result<(f64, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_F64, "wire: expected f64")?;
    let value = read_f64_le(bytes, at).ok_or_else(|| wire_error("wire: truncated f64"))?;
    Ok((value, at + 8))
}

/// Decodes one `bool` value record at `offset`.
pub fn decode_bool(bytes: &[u8], offset: usize) -> Result<(bool, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_BOOL, "wire: expected bool")?;
    let value = read_bool(bytes, at).ok_or_else(|| wire_error("wire: truncated bool"))?;
    Ok((value, at + 1))
}

/// Decodes one borrowed UTF-8 string value record at `offset`
/// (validated: invalid UTF-8 is `InvalidUtf8`).
pub fn decode_str(bytes: &[u8], offset: usize) -> Result<(&str, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_STR, "wire: expected string")?;
    let (payload, end) = decode_len_prefixed(bytes, at, "string")?;
    let text = std::str::from_utf8(payload)
        .map_err(|_| BffiError::new(ErrorCode::InvalidUtf8, "wire: invalid UTF-8 in string"))?;
    Ok((text, end))
}

/// Decodes one borrowed raw-bytes value record at `offset`.
pub fn decode_bytes(bytes: &[u8], offset: usize) -> Result<(&[u8], usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_BYTES, "wire: expected bytes")?;
    decode_len_prefixed(bytes, at, "bytes")
}

/// Shared length-prefixed payload reader for `Str`/`Bytes`.
fn decode_len_prefixed<'a>(
    bytes: &'a [u8],
    at: usize,
    what: &str,
) -> Result<(&'a [u8], usize), BffiError> {
    let len = read_u32_le(bytes, at)
        .ok_or_else(|| wire_error(&format!("wire: truncated {what} length")))?
        as usize;
    let start = at + 4;
    let end = start
        .checked_add(len)
        .ok_or_else(|| wire_error("wire: oversized length"))?;
    let payload = bytes
        .get(start..end)
        .ok_or_else(|| wire_error("wire: truncated payload"))?;
    Ok((payload, end))
}

/// Decodes one record header at `offset`; returns the field count and
/// the offset of the first field record.
pub fn decode_record_header(bytes: &[u8], offset: usize) -> Result<(usize, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_RECORD, "wire: expected record")?;
    let count =
        read_u32_le(bytes, at).ok_or_else(|| wire_error("wire: truncated record header"))? as usize;
    Ok((count, at + 4))
}

/// Decodes one sequence header at `offset`; returns the item count
/// and the offset of the first item record.
pub fn decode_seq_header(bytes: &[u8], offset: usize) -> Result<(usize, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_SEQ, "wire: expected sequence")?;
    let count = read_u32_le(bytes, at)
        .ok_or_else(|| wire_error("wire: truncated sequence header"))? as usize;
    Ok((count, at + 4))
}

/// Decodes a string record and maps it through `variants` (the enum
/// decode: an unknown variant name is an error).
pub fn decode_variant<'a>(
    bytes: &[u8],
    offset: usize,
    variants: &[&'a str],
) -> Result<(&'a str, usize), BffiError> {
    let (name, next) = decode_str(bytes, offset)?;
    let found = variants
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
        .ok_or_else(|| wire_error("wire: unknown variant"))?;
    Ok((found, next))
}

#[cfg(test)]
mod tests {
    use super::{
        TAG_BOOL, TAG_BYTES, TAG_F64, TAG_I32, TAG_I64, TAG_RECORD, TAG_SEQ, TAG_STR, TAG_UNIT,
        decode_bool, decode_bytes, decode_f64, decode_i32, decode_i64, decode_record_header,
        decode_seq_header, decode_str, decode_variant, encode_bool, encode_bytes, encode_f64,
        encode_i32, encode_i64, encode_record_header, encode_seq_header, encode_str, push_bool,
        push_f64_le, push_i32_le, push_i64_le, push_u32_le, read_bool, read_f64_le, read_i32_le,
        read_i64_le, read_u32_le,
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
    fn value_helpers_round_trip_every_scalar_kind() {
        let mut out = Vec::new();
        encode_i32(&mut out, -7);
        encode_i64(&mut out, i64::MIN);
        encode_f64(&mut out, 0.25);
        encode_bool(&mut out, true);
        encode_str(&mut out, "héllo");
        encode_bytes(&mut out, &[1, 2, 3]);

        let mut offset = 0_usize;
        let (v, next) = decode_i32(&out, offset).expect("i32");
        assert_eq!((v, next), (-7, offset + 1 + 4));
        offset = next;
        let (v, next) = decode_i64(&out, offset).expect("i64");
        assert_eq!(v, i64::MIN);
        offset = next;
        let (v, next) = decode_f64(&out, offset).expect("f64");
        assert_eq!(v, 0.25);
        offset = next;
        let (v, next) = decode_bool(&out, offset).expect("bool");
        assert!(v);
        offset = next;
        let (v, next) = decode_str(&out, offset).expect("str");
        assert_eq!(v, "héllo");
        offset = next;
        let (v, next) = decode_bytes(&out, offset).expect("bytes");
        assert_eq!(v, &[1, 2, 3]);
        assert_eq!(next, out.len(), "the last record ends the buffer");
    }

    #[test]
    fn record_and_seq_headers_round_trip() {
        let mut out = Vec::new();
        encode_record_header(&mut out, 3);
        encode_i32(&mut out, 1);
        encode_seq_header(&mut out, 1);
        encode_i32(&mut out, 2);

        let (count, at) = decode_record_header(&out, 0).expect("record header");
        assert_eq!((count, at), (3, 5));
        let (_, at) = decode_i32(&out, at).expect("field");
        let (count, at) = decode_seq_header(&out, at).expect("seq header");
        assert_eq!((count, at), (1, 5 + 5 + 5));
        let (_, end) = decode_i32(&out, at).expect("item");
        assert_eq!(end, out.len());
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        let mut out = Vec::new();
        encode_i32(&mut out, 1);
        // Wrong tag: an i32 reader over a bool record.
        let mut wrong = Vec::new();
        encode_bool(&mut wrong, true);
        assert!(decode_i32(&wrong, 0).is_err());
        // Wrong tag byte at the first position.
        assert!(decode_i32(&[TAG_BOOL, 0, 0, 0, 0], 0).is_err());
        // Truncated payloads of every width.
        assert!(decode_i32(&out[..3], 0).is_err());
        assert!(decode_i64(&[TAG_I64, 0, 0], 0).is_err());
        assert!(decode_f64(&[TAG_F64, 0, 0], 0).is_err());
        // Empty input everywhere.
        assert!(decode_bool(&[], 0).is_err());
        assert!(decode_str(&[], 0).is_err());
        assert!(decode_record_header(&[], 0).is_err());
        assert!(decode_seq_header(&[], 0).is_err());
        // Length prefix promising more than the buffer holds.
        let mut truncated = Vec::new();
        truncated.push(TAG_STR);
        push_u32_le(&mut truncated, 100);
        truncated.extend_from_slice(b"ab");
        assert!(decode_str(&truncated, 0).is_err());
        // Invalid UTF-8 in a string record.
        let mut bad_utf8 = Vec::new();
        bad_utf8.push(TAG_STR);
        push_u32_le(&mut bad_utf8, 1);
        bad_utf8.extend_from_slice(&[0xFF]);
        assert!(decode_str(&bad_utf8, 0).is_err());
    }

    #[test]
    fn variant_decode_rejects_unknown_names() {
        let mut out = Vec::new();
        encode_str(&mut out, "Running");
        let (name, end) = decode_variant(&out, 0, &["Idle", "Running"]).expect("known");
        assert_eq!((name, end), ("Running", out.len()));
        assert!(
            decode_variant(&out, 0, &["Idle"]).is_err(),
            "unknown variant"
        );
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
