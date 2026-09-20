//! Integration tests for the stream runtime through the `bffi`
//! facade: pull chunks (`spawn_stream` + `next_chunk`), the push
//! protocol (`spawn_push` + `Ctx`), the JS-facing ABI helpers
//! (`bffi_stream::abi`), and the exact wire shape of a pulled chunk
//! (one `TAG_SEQ` transient buffer).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bffi::bffi_core::{ErrorCode, Handle};
use bffi::bffi_stream::abi;
use bffi::bffi_types::wire;
use bffi::{Chunk, Ctx, StreamError, is_live, next_chunk, spawn_push, spawn_stream};

/// One i32 item as its `[tag][payload]` record.
fn i32_item(value: i32) -> Vec<u8> {
    let mut out = Vec::new();
    wire::encode_i32(&mut out, value);
    out
}

/// Polls a future once (no executor): `Some(output)` when ready.
fn futures_poll<F: std::future::Future + Unpin>(fut: &mut F) -> Option<F::Output> {
    use std::task::Poll;
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    match std::pin::Pin::new(fut).poll(&mut cx) {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}

#[test]
fn pull_yields_the_items_in_order_across_chunks() {
    let handle = spawn_stream(Box::new((0..5).map(|n| Ok(i32_item(n))))).expect("spawned");
    assert!(is_live(handle));

    let mut seen = Vec::new();
    while let Chunk::Items(items) = next_chunk(handle, 2).expect("chunk") {
        seen.extend(items);
    }
    let decoded: Vec<i32> = seen
        .iter()
        .map(|record| wire::decode_i32(record, 0).expect("i32 record").0)
        .collect();
    assert_eq!(decoded, vec![0, 1, 2, 3, 4]);
    assert!(!is_live(handle), "an exhausted stream is not live");
}

#[test]
fn push_delivers_strings_and_completes() {
    let (handle, ctx): (Handle, Ctx<String>) = spawn_push().expect("spawned");
    assert!(
        futures_poll(&mut ctx.push("alpha".to_owned())).is_some(),
        "a push below capacity resolves immediately"
    );
    assert!(futures_poll(&mut ctx.push("beta".to_owned())).is_some());
    ctx.complete().expect("completed");

    let mut seen = Vec::new();
    while let Chunk::Items(items) = next_chunk(handle, 8).expect("chunk") {
        seen.extend(items);
    }
    let decoded: Vec<String> = seen
        .iter()
        .map(|record| {
            wire::decode_str(record, 0)
                .expect("str record")
                .0
                .to_owned()
        })
        .collect();
    assert_eq!(decoded, vec!["alpha", "beta"]);
}

#[test]
fn a_failed_producer_surfaces_the_message() {
    let (handle, ctx) = spawn_push::<u64>().expect("spawned");
    ctx.fail("producer broke").expect("failed");
    let error = next_chunk(handle, 4).expect_err("errored");
    assert!(
        matches!(&error, StreamError::Errored(message) if message.contains("producer broke")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn the_abi_helpers_drive_a_pull_stream() {
    let handle = spawn_stream(Box::new((0..3).map(|n| Ok(i32_item(n))))).expect("spawned");

    // The wake trampoline registers on a LIVE stream; exhaustion and
    // the drop invalidate it.
    assert_eq!(
        abi::set_wake_as_code(handle.as_u64(), 0x1),
        ErrorCode::Ok.as_u32()
    );

    // First chunk through the C shape: one TAG_SEQ transient buffer.
    let mut slot: u64 = 0;
    let status = abi::next_as_code(handle.as_u64(), 2, &mut slot);
    assert_eq!(status, ErrorCode::Ok.as_u32());
    assert_ne!(slot, 0, "a non-empty chunk writes its buffer handle");

    let buffer = Handle::from_raw(slot);
    let len = bffi::bffi_build::runtime::buffer_len(buffer);
    // SAFETY: the pointer is valid until the free below and covers
    // exactly `len` bytes (CALLING-CONVENTION.md §5).
    let bytes = unsafe {
        std::slice::from_raw_parts(bffi::bffi_build::runtime::buffer_ptr(buffer), len as usize)
    };
    assert_eq!(bytes[0], wire::TAG_SEQ, "the chunk crosses as one sequence");
    let (count, mut at) = wire::decode_seq_header(bytes, 0).expect("seq header");
    assert_eq!(count, 2);
    let mut seen = Vec::new();
    for _ in 0..count {
        let (value, next) = wire::decode_i32(bytes, at).expect("item record");
        seen.push(value);
        at = next;
    }
    assert_eq!(seen, vec![0, 1]);
    assert!(bffi::bffi_build::runtime::free_buffer(buffer));

    // The remainder, then exhaustion (the 0 handle = done).
    let status = abi::next_as_code(handle.as_u64(), 2, &mut slot);
    assert_eq!(status, ErrorCode::Ok.as_u32());
    assert_ne!(slot, 0, "one item remains");
    assert!(bffi::bffi_build::runtime::free_buffer(Handle::from_raw(
        slot
    )));

    let status = abi::next_as_code(handle.as_u64(), 2, &mut slot);
    assert_eq!(status, ErrorCode::Ok.as_u32());
    assert_eq!(
        slot, 0,
        "an exhausted stream signals done with the 0 handle"
    );
    // Pulls past exhaustion keep answering Ok(Done) - the slot stays
    // with its terminal state - while the DROP reports the finished
    // stream as a stale handle (nothing left to release).
    let status = abi::next_as_code(handle.as_u64(), 2, &mut slot);
    assert_eq!(status, ErrorCode::Ok.as_u32());
    assert_eq!(slot, 0);
    assert_eq!(
        abi::drop_as_code(handle.as_u64()),
        ErrorCode::InvalidHandle.as_u32()
    );

    // An EARLY drop on a live stream still reports Ok exactly once.
    let early =
        spawn_stream(Box::new(std::iter::repeat_with(|| Ok(i32_item(0))))).expect("spawned");
    assert_eq!(abi::drop_as_code(early.as_u64()), ErrorCode::Ok.as_u32());
    assert_eq!(
        abi::drop_as_code(early.as_u64()),
        ErrorCode::InvalidHandle.as_u32()
    );
}
