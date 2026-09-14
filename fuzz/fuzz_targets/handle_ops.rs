#![no_main]

//! Op-program target over bffi's PUBLIC handle surfaces:
//! - a standalone generational table `bffi::core::HandleTable`
//!   (insert/get/remove/contains), and
//! - `bffi::ObjectWrap<u64>` over the global Registry with a fixed tag
//!   (wrap/get/release, double release, stale handles).
//!
//! Input bytes drive a deterministic little op program. Invariants:
//! nothing ever panics, a freshly issued handle always resolves, and a
//! released handle never resolves again (release must invalidate, not
//! resurrect).

use libfuzzer_sys::fuzz_target;
use std::sync::{Arc, OnceLock};

use bffi::core::{Handle, HandleTable, TypeTag};
use bffi::ObjectWrap;

/// Fixed tag for the `ObjectWrap` under test (the bffi-object range
/// 0x0100..=0x01FF). The global Registry allows exactly one
/// declaration per tag per process, so the wrap is created lazily,
/// once, and reused across fuzz iterations.
const WRAP_TAG: TypeTag = TypeTag(0x01F0);

/// Tag of the standalone table under test (user range 0x8000..=0xFFFF).
const TABLE_TAG: TypeTag = TypeTag(0x8001);

/// Cap on simultaneously wrapped-but-unreleased objects, so a long
/// campaign cannot grow the global Registry without bound. Retiring
/// the oldest tracked handle also deterministically exercises the
/// release/stale path.
const WRAP_LIVE_CAP: usize = 512;

fn wrap() -> &'static ObjectWrap<u64> {
    static WRAP: OnceLock<ObjectWrap<u64>> = OnceLock::new();
    WRAP.get_or_init(|| {
        // The fuzz process owns the tag: nothing else can have claimed it.
        ObjectWrap::<u64>::new(WRAP_TAG).expect("WRAP_TAG 0x01F0 is free in the fuzz process")
    })
}

/// Reads up to 8 bytes as a little-endian u64 (zero padded).
fn take_u64(bytes: &[u8]) -> (u64, &[u8]) {
    let n = bytes.len().min(8);
    let mut raw = [0_u8; 8];
    raw[..n].copy_from_slice(&bytes[..n]);
    (u64::from_le_bytes(raw), &bytes[n..])
}

/// Reads 8 arbitrary bytes as a raw handle. Garbage handles (wrong tag,
/// wild generation/index, null) must be rejected cleanly, never panic.
fn take_handle(bytes: &[u8]) -> (Handle, &[u8]) {
    let (raw, rest) = take_u64(bytes);
    (Handle::from_raw(raw), rest)
}

/// Reads a small byte chunk as the value of a table insert.
fn take_chunk(bytes: &[u8]) -> (Vec<u8>, &[u8]) {
    let (len, rest) = take_u64(bytes);
    let len = (len as usize) % 64;
    let n = rest.len().min(len);
    (rest[..n].to_vec(), &rest[n..])
}

fuzz_target!(|data: &[u8]| {
    // A fresh standalone table per input keeps the op program
    // deterministic; the ObjectWrap persists (global Registry).
    let table = HandleTable::<Vec<u8>>::with_capacity(TABLE_TAG, 4096);
    let mut wrap_live: Vec<Handle> = Vec::new();

    let mut cursor = data;
    while let Some((&op, rest)) = cursor.split_first() {
        cursor = rest;
        match op % 6 {
            // table insert: a fresh handle must resolve immediately
            0 => {
                let (value, rest) = take_chunk(cursor);
                cursor = rest;
                if let Ok(handle) = table.insert(Arc::new(value)) {
                    assert_eq!(handle.tag(), TABLE_TAG, "table must stamp its tag");
                    assert!(table.get(handle).is_some(), "fresh insert must resolve");
                    assert!(table.contains(handle), "fresh insert must be contained");
                }
            }
            // table get with arbitrary (garbage / stale / live) handles
            1 => {
                let (handle, rest) = take_handle(cursor);
                cursor = rest;
                let _ = table.get(handle);
            }
            // table remove: a successful release must invalidate
            2 => {
                let (handle, rest) = take_handle(cursor);
                cursor = rest;
                if table.remove(handle).is_some() {
                    assert!(!table.contains(handle), "removed handle still contained");
                    assert!(table.get(handle).is_none(), "removed handle still resolves");
                }
            }
            // table contains probe
            3 => {
                let (handle, rest) = take_handle(cursor);
                cursor = rest;
                let _ = table.contains(handle);
            }
            // ObjectWrap::wrap + immediate get round trip
            4 => {
                let (value, rest) = take_u64(cursor);
                cursor = rest;
                if wrap_live.len() >= WRAP_LIVE_CAP {
                    let oldest = wrap_live.remove(0);
                    assert!(
                        wrap().release(oldest).is_ok(),
                        "tracked handle must release"
                    );
                    assert!(
                        wrap().get(oldest).is_err(),
                        "released handle must not resolve"
                    );
                }
                if let Ok(handle) = wrap().wrap(value) {
                    assert_eq!(handle.tag(), WRAP_TAG, "wrap must stamp its tag");
                    assert_eq!(*wrap().get(handle).expect("fresh wrap must resolve"), value);
                    wrap_live.push(handle);
                }
            }
            // ObjectWrap::release: the handle goes stale, a second
            // release is an error - never a resurrection, never a panic
            _ => {
                let (handle, rest) = take_handle(cursor);
                cursor = rest;
                if wrap().release(handle).is_ok() {
                    assert!(
                        wrap().get(handle).is_err(),
                        "released handle must not resolve"
                    );
                    assert!(wrap().release(handle).is_err(), "double release must fail");
                    if let Some(at) = wrap_live.iter().position(|&h| h == handle) {
                        wrap_live.swap_remove(at);
                    }
                }
            }
        }
    }
});
