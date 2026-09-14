#![no_main]

//! Fuzz target for the wire codec decoders
//! (`crates/bffi/src/types/wire.rs`), driven through the public
//! `bffi::types::wire` re-exports.
//!
//! Invariants:
//! - the decoders never panic and never read out of bounds (a decode
//!   error is the EXPECTED outcome on malformed input and is fine);
//! - every successful decode strictly advances the offset, so decoding
//!   cannot loop forever (libFuzzer's timeout is the outer net).

use libfuzzer_sys::fuzz_target;

use bffi::types::wire;

/// Walker recursion bound. Mirrors the decoder's own MAX_WIRE_DEPTH
/// limit; kept here as well so the fuzz walker is stack-safe even if
/// the decoder's bound were ever raised.
const WALK_DEPTH_CAP: u32 = wire::MAX_WIRE_DEPTH;

/// Decodes one value record at `offset` by tag dispatch, recursing into
/// record/sequence children exactly the way a `#[derive(BffiRecord)]`
/// expansion does. Returns the offset past the value.
fn walk_value(bytes: &[u8], offset: usize, depth: u32) -> Result<usize, ()> {
    if depth >= WALK_DEPTH_CAP {
        return Err(());
    }
    let tag = *bytes.get(offset).ok_or(())?;
    let next = match tag {
        wire::TAG_UNIT => offset + 1,
        wire::TAG_I32 => wire::decode_i32(bytes, offset).map_err(drop)?.1,
        wire::TAG_I64 => wire::decode_i64(bytes, offset).map_err(drop)?.1,
        wire::TAG_F64 => wire::decode_f64(bytes, offset).map_err(drop)?.1,
        wire::TAG_BOOL => wire::decode_bool(bytes, offset).map_err(drop)?.1,
        wire::TAG_STR => wire::decode_str(bytes, offset).map_err(drop)?.1,
        wire::TAG_BYTES => wire::decode_bytes(bytes, offset).map_err(drop)?.1,
        wire::TAG_U64 => wire::decode_u64(bytes, offset).map_err(drop)?.1,
        wire::TAG_ERROR => wire::decode_error(bytes, offset).map_err(drop)?.1,
        wire::TAG_RECORD => {
            let (count, mut next) = wire::decode_record_header(bytes, offset).map_err(drop)?;
            for _ in 0..count {
                next = walk_value(bytes, next, depth + 1)?;
            }
            next
        }
        wire::TAG_SEQ => {
            let (count, mut next) = wire::decode_seq_header(bytes, offset).map_err(drop)?;
            for _ in 0..count {
                next = walk_value(bytes, next, depth + 1)?;
            }
            next
        }
        // Unknown tags must be rejected cleanly, never panicked on.
        _ => return Err(()),
    };
    // A successful decode must consume at least the tag byte: an
    // offset that does not advance would loop forever.
    assert!(next > offset, "wire decoder did not advance past {offset}");
    Ok(next)
}

fuzz_target!(|data: &[u8]| {
    // 1. Tag-dispatched recursive walk (the generated-decoder pattern).
    let _ = walk_value(data, 0, 0);

    // 2. Every scalar entry point at the buffer edges: clean errors on
    //    malformed input, never a panic, never an OOB read.
    for offset in [0, data.len()] {
        let _ = wire::decode_i32(data, offset);
        let _ = wire::decode_i64(data, offset);
        let _ = wire::decode_f64(data, offset);
        let _ = wire::decode_bool(data, offset);
        let _ = wire::decode_str(data, offset);
        let _ = wire::decode_bytes(data, offset);
        let _ = wire::decode_u64(data, offset);
        let _ = wire::decode_error(data, offset);
        let _ = wire::decode_record_header(data, offset);
        let _ = wire::decode_seq_header(data, offset);
    }

    // 3. The tolerant decoders and the rich-error envelope.
    let _ = wire::decode_u64_lenient(data, 0);
    let _ = wire::decode_number(data, 0);
    let _ = wire::decode_variant(data, 0, &["Idle", "Running"]);
    let _ = wire::decode_error_rich(data, 0);
});
