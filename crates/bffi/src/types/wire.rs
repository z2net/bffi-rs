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
//! | 9   | wire composite - SIGNATURE MARKER ONLY: declares "any record or sequence" in a callback signature; never emitted as a value tag |
//! | 10  | exact `u64` (8 bytes LE)                   |
//! | 11  | error item (UTF-8 message, `u32` LE length + bytes) |
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
/// The composite-signature marker: appears ONLY in callback
/// signature bytes, declaring "any record or sequence" parameter or
/// return. Never emitted as a value tag - composite values travel as
/// their real `TAG_RECORD`/`TAG_SEQ` records.
pub const TAG_WIRE: u8 = 9;
/// The exact `u64` value (`8` bytes LE): JS decodes it as a
/// non-negative `bigint` (`BigInt64`/`BigUint64` agree below
/// `i64::MAX`; `U64` is the exact carrier above it).
pub const TAG_U64: u8 = 10;
/// The error item (a UTF-8 message with a `u32` LE length prefix):
/// the wire form of a `Result` item's `Err` inside a stream. The JS
/// side decodes it into an `Error` instance.
pub const TAG_ERROR: u8 = 11;

/// The default maximum declared length/count the decoder accepts: a
/// `Str`/`Bytes`/error payload length or a record/sequence element
/// count above it is rejected before any slicing or looping happens.
/// Hardening bound for corrupted or hostile buffers; raise or lower it
/// for a process via [`set_max_wire_payload`].
pub const MAX_WIRE_PAYLOAD: u32 = 64 * 1024 * 1024;

/// The default maximum record/sequence nesting depth the decoder
/// accepts; deeper composites are rejected. Recursion bound for
/// hostile buffers; adjust via [`set_max_wire_depth`].
pub const MAX_WIRE_DEPTH: u32 = 64;

static WIRE_PAYLOAD_LIMIT: AtomicU32 = AtomicU32::new(MAX_WIRE_PAYLOAD);
static WIRE_DEPTH_LIMIT: AtomicU32 = AtomicU32::new(MAX_WIRE_DEPTH);

/// Returns the wire payload limit currently in effect.
#[must_use]
pub fn max_wire_payload() -> u32 {
    WIRE_PAYLOAD_LIMIT.load(Ordering::Relaxed)
}

/// Sets the wire payload limit for the process. `0` is rejected (the
/// previous limit stays in effect); `u32::MAX` is the natural ceiling.
pub fn set_max_wire_payload(limit: u32) {
    if limit == 0 {
        return;
    }
    WIRE_PAYLOAD_LIMIT.store(limit, Ordering::Relaxed);
}

/// Returns the wire nesting depth limit currently in effect.
#[must_use]
pub fn max_wire_depth() -> u32 {
    WIRE_DEPTH_LIMIT.load(Ordering::Relaxed)
}

/// Sets the wire nesting depth limit for the process. `0` is rejected
/// (the previous limit stays in effect).
pub fn set_max_wire_depth(limit: u32) {
    if limit == 0 {
        return;
    }
    WIRE_DEPTH_LIMIT.store(limit, Ordering::Relaxed);
}

thread_local! {
    /// `(tag offset, declared count, chained depth)` of the most
    /// recent record/sequence header decoded on this thread. A header
    /// CHAINS onto its parent (nesting depth grows) only when it sits
    /// exactly at the parent's first-child slot (`start == parent_tag
    /// + 5`) and the parent declared a single field/item; anything
    /// else (a sibling, a fresh top-level decode at offset 0, a new
    /// payload region) restarts the depth at 1. That distinguishes
    /// nested composites from wide sequences without any cooperation
    /// from the generated recursive decoders, which this file cannot
    /// see into.
    static LAST_COMPOSITE: Cell<(usize, u32, usize)> = const { Cell::new((usize::MAX, 0, 0)) };
}

/// Registers one record/sequence header entry at `start` (the tag
/// byte) that declared `count` children, and enforces the wire depth
/// limit on the chained nesting depth.
fn enter_composite(start: usize, count: u32, what: &str) -> Result<(), BffiError> {
    let depth = LAST_COMPOSITE.with(|cell| {
        let (last_start, last_count, last_depth) = cell.get();
        let depth = if start == last_start.wrapping_add(5) && last_count == 1 {
            last_depth + 1
        } else {
            1
        };
        cell.set((start, count, depth));
        depth
    });
    let limit = max_wire_depth();
    if depth > limit as usize {
        return Err(wire_error(&format!(
            "wire: {what} nesting depth {depth} exceeds the wire depth limit of {limit}"
        )));
    }
    Ok(())
}

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
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

/// Builds the canonical malformed-wire error.
fn wire_error(what: &str) -> BffiError {
    BffiError::new(ErrorCode::InvalidArgument, what)
}

/// A type that crosses the boundary as one wire-encoded value: the
/// `#[derive(BffiRecord)]` / `#[derive(BffiEnum)]` expansions
/// implement this over the value-level helpers below.
pub trait BffiWire: Sized {
    /// Appends this value as one complete wire record.
    fn bffi_wire_encode(&self, out: &mut Vec<u8>);

    /// Decodes one wire record at `offset`; returns the value and the
    /// offset past it.
    fn bffi_wire_decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), BffiError>;
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

/// Appends one exact `u64` value record.
pub fn encode_u64(out: &mut Vec<u8>, value: u64) {
    out.push(TAG_U64);
    out.extend_from_slice(&value.to_le_bytes());
}

/// Decodes one exact `u64` value record at `offset`.
pub fn decode_u64(bytes: &[u8], offset: usize) -> Result<(u64, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_U64, "wire: expected u64")?;
    let tail = bytes
        .get(at..at + 8)
        .ok_or_else(|| wire_error("wire: truncated u64"))?;
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(tail);
    Ok((u64::from_le_bytes(raw), at + 8))
}

/// Decodes one exact `u64` value record at `offset`: `TAG_U64` or
/// `TAG_I64` (the JS encoder picks the tag by value - bigints below
/// `i64::MAX` ride `I64`, where the signed view agrees - so exact
/// `u64` fields decode tolerantly and exactly; a negative `i64`
/// payload is an error).
pub fn decode_u64_lenient(bytes: &[u8], offset: usize) -> Result<(u64, usize), BffiError> {
    let tag = *bytes
        .get(offset)
        .ok_or_else(|| wire_error("wire: expected u64"))?;
    if tag == TAG_U64 {
        return decode_u64(bytes, offset);
    }
    if tag == TAG_I64 {
        let (value, next) = decode_i64(bytes, offset)?;
        let value =
            u64::try_from(value).map_err(|_| wire_error("wire: negative i64 payload for u64"))?;
        return Ok((value, next));
    }
    Err(wire_error("wire: expected u64 or i64"))
}

/// Decodes one error-message record at `offset` (the wire form of a
/// `Result` item's `Err`); validated UTF-8.
pub fn decode_error(bytes: &[u8], offset: usize) -> Result<(String, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_ERROR, "wire: expected error")?;
    let (payload, end) = decode_len_prefixed(bytes, at, "error message")?;
    let message = std::str::from_utf8(payload)
        .map_err(|_| BffiError::new(ErrorCode::InvalidUtf8, "wire: invalid UTF-8 in error"))?
        .to_owned();
    Ok((message, end))
}

/// Appends one error-message value record (the wire form of a
/// `Result` item's `Err`).
pub fn encode_error(out: &mut Vec<u8>, message: &str) {
    out.push(TAG_ERROR);
    push_u32_le(out, message.len() as u32);
    out.extend_from_slice(message.as_bytes());
}

/// Appends one rich-error envelope: `[u32 code][str variant][str
/// message][record|unit]` - the wire form of a `Result` item's `Err`
/// for `#[derive(BffiError)]` types (the code is the user code, the
/// variant the enum variant name, the record the payload fields;
/// ad-hoc errors use code 13, an empty variant and a unit payload).
pub fn encode_error_rich(
    out: &mut Vec<u8>,
    code: u32,
    variant: &str,
    message: &str,
    payload: Option<&[u8]>,
) {
    out.push(TAG_ERROR);
    push_u32_le(out, code);
    push_u32_le(out, variant.len() as u32);
    out.extend_from_slice(variant.as_bytes());
    push_u32_le(out, message.len() as u32);
    out.extend_from_slice(message.as_bytes());
    match payload {
        Some(record) => out.extend_from_slice(record),
        None => out.push(TAG_UNIT),
    }
}

/// One decoded rich-error envelope.
#[derive(Debug)]
pub struct ErrorEnvelope {
    /// The user-defined code (0 for ad-hoc domain errors).
    pub code: u32,
    /// The variant name (empty for ad-hoc domain errors).
    pub variant: String,
    /// The human-readable message.
    pub message: String,
    /// The payload record bytes (`None` when absent); decode with the
    /// module's record descriptor.
    pub payload: Option<Vec<u8>>,
}

/// Decodes one rich-error envelope at `offset`.
pub fn decode_error_rich(bytes: &[u8], offset: usize) -> Result<(ErrorEnvelope, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_ERROR, "wire: expected error")?;
    let read_str = |at: usize| -> Result<(String, usize), BffiError> {
        let (payload, end) = decode_len_prefixed(bytes, at, "error field")?;
        let value = std::str::from_utf8(payload)
            .map_err(|_| BffiError::new(ErrorCode::InvalidUtf8, "wire: invalid UTF-8 in error"))?
            .to_owned();
        Ok((value, end))
    };
    let code = read_u32_le(bytes, at).ok_or_else(|| wire_error("wire: truncated error code"))?;
    let (variant, at) = read_str(at + 4)?;
    let (message, at) = read_str(at)?;
    // The payload record (documented as the envelope's last element)
    // is captured raw: it is a self-describing TAG_RECORD decoded by
    // the module's record descriptor on the JS side.
    let payload = bytes
        .get(at..)
        .filter(|tail| tail.first() == Some(&TAG_RECORD))
        .map(|tail| tail.to_vec());
    let next = match &payload {
        Some(tail) => at + tail.len(),
        None => at + 1, // the TAG_UNIT marker
    };
    Ok((
        ErrorEnvelope {
            code,
            variant,
            message,
            payload,
        },
        next,
    ))
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

/// Decodes one number-ish value record: `TAG_I32` or `TAG_F64`
/// (the JS encoder picks the width by value - whole small numbers
/// ride `i32` - so number fields decode tolerantly and exactly).
pub fn decode_number(bytes: &[u8], offset: usize) -> Result<(f64, usize), BffiError> {
    let tag = *bytes
        .get(offset)
        .ok_or_else(|| wire_error("wire: expected a number"))?;
    if tag == TAG_I32 {
        let (value, next) = decode_i32(bytes, offset)?;
        return Ok((f64::from(value), next));
    }
    if tag == TAG_F64 {
        return decode_f64(bytes, offset);
    }
    Err(wire_error("wire: expected i32 or f64"))
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

/// Shared length-prefixed payload reader for `Str`/`Bytes` and the
/// error-message/error-field payloads. Validates the declared length
/// against the wire payload limit and against the buffer bounds
/// BEFORE any subsequence is taken.
fn decode_len_prefixed<'a>(
    bytes: &'a [u8],
    at: usize,
    what: &str,
) -> Result<(&'a [u8], usize), BffiError> {
    let len = read_u32_le(bytes, at)
        .ok_or_else(|| wire_error(&format!("wire: truncated {what} length header")))?;
    let limit = max_wire_payload();
    if len > limit {
        return Err(wire_error(&format!(
            "wire: {what} declared length {len} exceeds the wire payload limit of {limit}"
        )));
    }
    let len = len as usize;
    let start = at + 4;
    let end = start.checked_add(len).ok_or_else(|| {
        wire_error(&format!(
            "wire: {what} declared length {len} exceeds payload"
        ))
    })?;
    if end > bytes.len() {
        return Err(wire_error(&format!(
            "wire: {what} declared length {len} exceeds payload"
        )));
    }
    Ok((&bytes[start..end], end))
}

/// Decodes one record header at `offset`; returns the field count and
/// the offset of the first field record. The declared count is
/// validated against the wire payload limit, and chained nesting
/// (a header at its `count == 1` parent's first-child slot) is
/// validated against the wire depth limit.
pub fn decode_record_header(bytes: &[u8], offset: usize) -> Result<(usize, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_RECORD, "wire: expected record")?;
    let count = read_u32_le(bytes, at)
        .ok_or_else(|| wire_error("wire: truncated record field count header"))?;
    let limit = max_wire_payload();
    if count > limit {
        return Err(wire_error(&format!(
            "wire: record declared field count {count} exceeds the wire payload limit of {limit}"
        )));
    }
    enter_composite(offset, count, "record")?;
    Ok((count as usize, at + 4))
}

/// Decodes one sequence header at `offset`; returns the item count
/// and the offset of the first item record. The declared count is
/// validated against the wire payload limit, and chained nesting is
/// validated against the wire depth limit (see
/// [`decode_record_header`]).
pub fn decode_seq_header(bytes: &[u8], offset: usize) -> Result<(usize, usize), BffiError> {
    let at = expect_tag(bytes, offset, TAG_SEQ, "wire: expected sequence")?;
    let count = read_u32_le(bytes, at)
        .ok_or_else(|| wire_error("wire: truncated sequence item count header"))?;
    let limit = max_wire_payload();
    if count > limit {
        return Err(wire_error(&format!(
            "wire: sequence declared item count {count} exceeds the wire payload limit of {limit}"
        )));
    }
    enter_composite(offset, count, "sequence")?;
    Ok((count as usize, at + 4))
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
        BffiError, MAX_WIRE_DEPTH, MAX_WIRE_PAYLOAD, TAG_BOOL, TAG_BYTES, TAG_ERROR, TAG_F64,
        TAG_I32, TAG_I64, TAG_RECORD, TAG_SEQ, TAG_STR, TAG_U64, TAG_UNIT, TAG_WIRE, decode_bool,
        decode_bytes, decode_error, decode_error_rich, decode_f64, decode_i32, decode_i64,
        decode_record_header, decode_seq_header, decode_str, decode_u64, decode_u64_lenient,
        decode_variant, encode_bool, encode_bytes, encode_error, encode_error_rich, encode_f64,
        encode_i32, encode_i64, encode_record_header, encode_seq_header, encode_str, encode_u64,
        max_wire_depth, max_wire_payload, push_bool, push_f64_le, push_i32_le, push_i64_le,
        push_u32_le, read_bool, read_f64_le, read_i32_le, read_i64_le, read_u32_le,
        set_max_wire_depth, set_max_wire_payload,
    };
    use std::sync::{Mutex, PoisonError};

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
        // The callback-signature composite marker (never a value tag).
        assert_eq!(TAG_WIRE, 9);
        // The exact u64 carrier and the stream error item.
        assert_eq!(TAG_U64, 10);
        assert_eq!(TAG_ERROR, 11);
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
        let _serial = serial();
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
        let _serial = serial();
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
        let _serial = serial();
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
    fn u64_round_trips_exactly_and_error_carries_the_message() {
        let _serial = serial();
        let mut out = Vec::new();
        // Above i64::MAX: the value only a U64 tag carries exactly.
        encode_u64(&mut out, u64::MAX);
        encode_u64(&mut out, 42);
        encode_error(&mut out, "stream failed");

        let (value, next) = decode_u64(&out, 0).expect("u64");
        assert_eq!((value, next), (u64::MAX, 9));
        let (value, next) = decode_u64(&out, next).expect("u64 small");
        assert_eq!((value, next), (42, 18));
        let (message, end) = decode_error(&out, next).expect("error");
        assert_eq!((message.as_str(), end), ("stream failed", out.len()));

        // Malformed shapes are clean errors.
        assert!(decode_u64(&[TAG_U64, 0, 0], 0).is_err());
        // Length prefix promising more than the buffer holds.
        assert!(decode_error(&[TAG_ERROR, 5, 0, 0, 0], 0).is_err());
    }

    #[test]
    fn lenient_u64_decode_accepts_both_exact_carriers() {
        let mut out = Vec::new();
        // Small u64 as the JS encoder sends it: the I64 carrier.
        encode_i64(&mut out, 9_007_199_254_740_993);
        // Large u64: the U64 carrier.
        encode_u64(&mut out, u64::MAX);

        let (value, next) = decode_u64_lenient(&out, 0).expect("i64 carrier");
        assert_eq!((value, next), (9_007_199_254_740_993, 9));
        let (value, next) = decode_u64_lenient(&out, next).expect("u64 carrier");
        assert_eq!((value, next), (u64::MAX, 18));
        assert_eq!(next, out.len());

        // A negative i64 payload is not a u64.
        let mut negative = Vec::new();
        encode_i64(&mut negative, -1);
        assert!(decode_u64_lenient(&negative, 0).is_err());
        // Wrong tags are clean errors.
        assert!(decode_u64_lenient(&[TAG_BOOL, 0], 0).is_err());
        assert!(decode_u64_lenient(&[TAG_STR, 0, 0, 0, 0], 0).is_err());
        assert!(decode_u64_lenient(&[], 0).is_err());
    }

    #[test]
    fn rich_error_envelope_round_trips_code_variant_message_payload() {
        use super::decode_error_rich;
        let _serial = serial();
        let mut payload_record = Vec::new();
        payload_record.push(TAG_RECORD);
        push_u32_le(&mut payload_record, 1);
        payload_record.push(TAG_I64);
        push_i64_le(&mut payload_record, 777);

        let mut out = Vec::new();
        encode_error_rich(
            &mut out,
            0x1001,
            "NotFound",
            "no row 777",
            Some(&payload_record),
        );

        let (envelope, end) = decode_error_rich(&out, 0).expect("envelope");
        assert_eq!(envelope.code, 0x1001);
        assert_eq!(envelope.variant, "NotFound");
        assert_eq!(envelope.message, "no row 777");
        assert_eq!(envelope.payload.as_deref(), Some(payload_record.as_slice()));
        assert_eq!(end, out.len());
    }

    #[test]
    fn rich_error_envelope_without_payload_decodes_unit() {
        let _serial = serial();
        let mut out = Vec::new();
        encode_error_rich(&mut out, 13, "", "domain failure", None);
        let (envelope, end) = decode_error_rich(&out, 0).expect("envelope");
        assert_eq!((envelope.code, envelope.variant.as_str()), (13, ""));
        assert_eq!(envelope.message, "domain failure");
        assert!(envelope.payload.is_none(), "unit marker = no payload");
        assert_eq!(end, out.len());
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

    // -----------------------------------------------------------------------
    // Hostile-input limits: declared lengths/counts and nesting depth.
    // -----------------------------------------------------------------------

    /// Serializes every test that decodes or mutates the global wire
    /// limits: the limits are process-global atomics, so limit-mutation
    /// tests must not overlap limit-sensitive decodes.
    static LIMIT_LOCK: Mutex<()> = Mutex::new(());

    /// Lock guard for [`LIMIT_LOCK`].
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        LIMIT_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Restores the default payload limit when the test scope ends (also
    /// on a failed assert - the tests run in parallel).
    struct ResetPayloadLimit;
    impl Drop for ResetPayloadLimit {
        fn drop(&mut self) {
            set_max_wire_payload(MAX_WIRE_PAYLOAD);
        }
    }

    /// Restores the default depth limit when the test scope ends.
    struct ResetDepthLimit;
    impl Drop for ResetDepthLimit {
        fn drop(&mut self) {
            set_max_wire_depth(MAX_WIRE_DEPTH);
        }
    }

    /// Builds exactly `depth` chained record headers: `depth - 1`
    /// one-field wrappers around a terminal empty record
    /// (`[RECORD 1] * (depth - 1) [RECORD 0]`).
    fn nested_records(depth: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..depth.saturating_sub(1) {
            out.push(TAG_RECORD);
            push_u32_le(&mut out, 1);
        }
        out.push(TAG_RECORD);
        push_u32_le(&mut out, 0);
        out
    }

    /// The sequence twin of [`nested_records`].
    fn nested_seqs(depth: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..depth.saturating_sub(1) {
            out.push(TAG_SEQ);
            push_u32_le(&mut out, 1);
        }
        out.push(TAG_SEQ);
        push_u32_le(&mut out, 0);
        out
    }

    /// Walks nested record headers the way a `#[derive(BffiRecord)]`
    /// decode does: each header's fields are decoded inside the scope
    /// of its parent header.
    fn recurse_record_headers(bytes: &[u8], offset: usize) -> Result<usize, BffiError> {
        let (count, mut next) = decode_record_header(bytes, offset)?;
        for _ in 0..count {
            next = recurse_record_headers(bytes, next)?;
        }
        Ok(next)
    }

    /// The sequence twin of [`recurse_record_headers`].
    fn recurse_seq_headers(bytes: &[u8], offset: usize) -> Result<usize, BffiError> {
        let (count, mut next) = decode_seq_header(bytes, offset)?;
        for _ in 0..count {
            next = recurse_seq_headers(bytes, next)?;
        }
        Ok(next)
    }

    #[test]
    fn limit_setters_reject_zero_and_report_the_effective_limit() {
        let _serial = serial();
        let _reset = ResetPayloadLimit;
        let _reset_depth = ResetDepthLimit;

        assert_eq!(max_wire_payload(), MAX_WIRE_PAYLOAD);
        assert_eq!(max_wire_depth(), MAX_WIRE_DEPTH);

        set_max_wire_payload(0);
        assert_eq!(max_wire_payload(), MAX_WIRE_PAYLOAD, "0 is rejected");
        set_max_wire_depth(0);
        assert_eq!(max_wire_depth(), MAX_WIRE_DEPTH, "0 is rejected");

        set_max_wire_payload(16);
        assert_eq!(max_wire_payload(), 16);
        set_max_wire_depth(2);
        assert_eq!(max_wire_depth(), 2);
    }

    #[test]
    fn truncated_length_headers_name_the_tag() {
        let _serial = serial();
        // String: tag + 2 of the 4 length bytes.
        let mut str_rec = Vec::new();
        str_rec.push(TAG_STR);
        push_u32_le(&mut str_rec, 5);
        let err = decode_str(&str_rec[..3], 0).unwrap_err();
        assert!(err.message.contains("string"), "{}", err.message);
        assert!(err.message.contains("truncated"), "{}", err.message);

        // Bytes: same shape.
        let mut bytes_rec = Vec::new();
        bytes_rec.push(TAG_BYTES);
        push_u32_le(&mut bytes_rec, 4);
        let err = decode_bytes(&bytes_rec[..3], 0).unwrap_err();
        assert!(err.message.contains("bytes"), "{}", err.message);
        assert!(err.message.contains("truncated"), "{}", err.message);

        // Error message: same shape.
        let err = decode_error(&[TAG_ERROR, 1, 0], 0).unwrap_err();
        assert!(err.message.contains("error message"), "{}", err.message);
        assert!(err.message.contains("truncated"), "{}", err.message);

        // Rich-error envelope: code reads, the variant length is cut.
        let err = decode_error_rich(&[TAG_ERROR, 1, 0, 0, 0, 1, 0], 0).unwrap_err();
        assert!(err.message.contains("error field"), "{}", err.message);
        assert!(err.message.contains("truncated"), "{}", err.message);
    }

    #[test]
    fn declared_lengths_beyond_the_buffer_name_the_violation() {
        let _serial = serial();
        let mut out = Vec::new();
        out.push(TAG_STR);
        push_u32_le(&mut out, 100);
        out.extend_from_slice(b"ab");
        let err = decode_str(&out, 0).unwrap_err();
        assert!(
            err.message.contains("declared length 100 exceeds payload"),
            "{}",
            err.message
        );

        let mut out = Vec::new();
        out.push(TAG_BYTES);
        push_u32_le(&mut out, u32::MAX - 3);
        out.extend_from_slice(&[0xAA; 8]);
        let err = decode_bytes(&out, 0).unwrap_err();
        assert!(err.message.contains("declared length"), "{}", err.message);

        let mut out = Vec::new();
        out.push(TAG_ERROR);
        push_u32_le(&mut out, 9);
        out.extend_from_slice(b"abc");
        let err = decode_error(&out, 0).unwrap_err();
        assert!(
            err.message
                .contains("error message declared length 9 exceeds payload"),
            "{}",
            err.message
        );
    }

    #[test]
    fn declared_lengths_above_the_payload_limit_are_rejected() {
        let _serial = serial();
        let _reset = ResetPayloadLimit;

        // u32::MAX is above the default 64 MiB limit and is rejected
        // before the buffer bounds are even consulted.
        let mut out = Vec::new();
        out.push(TAG_BYTES);
        push_u32_le(&mut out, u32::MAX);
        out.extend_from_slice(&[0xAA; 8]);
        let err = decode_bytes(&out, 0).unwrap_err();
        assert!(
            err.message.contains("exceeds the wire payload limit"),
            "{}",
            err.message
        );

        // A lowered limit rejects payloads that are fully present.
        set_max_wire_payload(8);
        let mut small = Vec::new();
        small.push(TAG_STR);
        push_u32_le(&mut small, 16);
        small.extend_from_slice(&[b'x'; 16]);
        let err = decode_str(&small, 0).unwrap_err();
        assert!(
            err.message.contains("exceeds the wire payload limit of 8"),
            "{}",
            err.message
        );

        // At-or-under the limit keeps decoding.
        let mut ok = Vec::new();
        ok.push(TAG_STR);
        push_u32_le(&mut ok, 4);
        ok.extend_from_slice(b"okay");
        assert_eq!(decode_str(&ok, 0).unwrap(), ("okay", 9));
    }

    #[test]
    fn declared_counts_above_the_payload_limit_are_rejected() {
        let _serial = serial();
        let _reset = ResetPayloadLimit;

        // u32::MAX declared fields/items cannot be legitimate.
        let mut rec = Vec::new();
        rec.push(TAG_RECORD);
        push_u32_le(&mut rec, u32::MAX);
        let err = decode_record_header(&rec, 0).unwrap_err();
        assert!(
            err.message.contains("record declared field count"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("exceeds the wire payload limit"),
            "{}",
            err.message
        );

        let mut seq = Vec::new();
        seq.push(TAG_SEQ);
        push_u32_le(&mut seq, u32::MAX);
        let err = decode_seq_header(&seq, 0).unwrap_err();
        assert!(
            err.message.contains("sequence declared item count"),
            "{}",
            err.message
        );

        // A lowered limit rejects counts above it even when the items
        // are all present.
        set_max_wire_payload(2);
        let mut rec = Vec::new();
        rec.push(TAG_RECORD);
        push_u32_le(&mut rec, 3);
        encode_i32(&mut rec, 1);
        encode_i32(&mut rec, 2);
        encode_i32(&mut rec, 3);
        let err = decode_record_header(&rec, 0).unwrap_err();
        assert!(
            err.message.contains("exceeds the wire payload limit of 2"),
            "{}",
            err.message
        );

        // A count at the limit still reads its header.
        let mut rec = Vec::new();
        rec.push(TAG_RECORD);
        push_u32_le(&mut rec, 2);
        encode_i32(&mut rec, 1);
        encode_i32(&mut rec, 2);
        let (count, _) = decode_record_header(&rec[..5], 0).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn rich_error_field_lengths_go_through_the_payload_limit() {
        let _serial = serial();
        let _reset = ResetPayloadLimit;

        let mut out = Vec::new();
        out.push(TAG_ERROR);
        push_u32_le(&mut out, 0x1001);
        push_u32_le(&mut out, u32::MAX);
        let err = decode_error_rich(&out, 0).unwrap_err();
        assert!(
            err.message.contains("error field declared length"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("exceeds the wire payload limit"),
            "{}",
            err.message
        );

        set_max_wire_payload(4);
        let mut out = Vec::new();
        out.push(TAG_ERROR);
        push_u32_le(&mut out, 0x1001);
        push_u32_le(&mut out, 8);
        out.extend_from_slice(b"NotFound");
        let err = decode_error_rich(&out, 0).unwrap_err();
        assert!(
            err.message.contains("exceeds the wire payload limit of 4"),
            "{}",
            err.message
        );
    }

    #[test]
    fn record_and_seq_nesting_is_depth_limited() {
        // The depth limit is a process-global: serialize the whole
        // test so concurrent limit-mutation tests cannot interfere.
        let _serial = serial();

        // At the default limit: 64 nested records decode, 65 error.
        let ok_buf = nested_records(MAX_WIRE_DEPTH as usize);
        assert!(recurse_record_headers(&ok_buf, 0).is_ok());

        let deep = nested_records(MAX_WIRE_DEPTH as usize + 1);
        let err = recurse_record_headers(&deep, 0).unwrap_err();
        assert!(
            err.message.contains("record nesting depth"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("exceeds the wire depth limit"),
            "{}",
            err.message
        );

        // Sequences are depth-limited through the same counter.
        let ok_seq = nested_seqs(MAX_WIRE_DEPTH as usize);
        assert!(recurse_seq_headers(&ok_seq, 0).is_ok());
        let deep_seq = nested_seqs(MAX_WIRE_DEPTH as usize + 1);
        let err = recurse_seq_headers(&deep_seq, 0).unwrap_err();
        assert!(
            err.message.contains("sequence nesting depth"),
            "{}",
            err.message
        );

        // A lowered limit takes effect.
        let _reset = ResetDepthLimit;
        set_max_wire_depth(2);
        let three = nested_records(3);
        assert!(recurse_record_headers(&three, 0).is_err());
        let two = nested_records(2);
        assert!(recurse_record_headers(&two, 0).is_ok());
    }
}
