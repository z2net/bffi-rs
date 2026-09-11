//! # bffi-stream
//!
//! Pull-based streams over user iterators (B2.1): a
//! `#[bffi_stream]` export hands JavaScript a stream handle; the
//! generic `bffi_stream_next` export pulls chunks. The items are
//! pre-encoded wire records (`Vec<u8>` of one complete
//! `[tag][payload]` each), so the chunk crosses as one `TAG_SEQ`
//! payload through the ordinary transient-buffer channel - no new
//! ABI shapes.
//!
//! The contract is **pull-chunk** (B2.1): `next` runs on the JS
//! thread under the entry mutex, so the iterator must not block
//! indefinitely; long-running producers arrive with the push model
//! (B2.2) or by wrapping work in an `#[bffi_async]` task. A panic
//! inside the iterator poisons the stream: `catch_unwind` marks the
//! entry errored and every later `next` reports that error (the
//! table survives; only the stream is dead).
//!
//! The stream table reuses the global [`Registry`] under tag
//! `0x0600` (claimed in [`bffi::core::handle`]), mirroring the task
//! table of `bffi-async`.

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
use std::sync::{Arc, Mutex, OnceLock};

use bffi_core::{Handle, Registry, TypeTag};

pub mod abi;

/// The type tag of the stream table.
const STREAM_TAG: TypeTag = TypeTag(0x0600);

/// The live state of one stream: the shared iterator is consumed
/// under the entry mutex; `Done`/`Errored` are terminal.
pub(crate) struct StreamEntry {
    state: Mutex<StreamState>,
}

enum StreamState {
    /// The iterator is alive: up to `max` items per pull.
    Items(Box<dyn Iterator<Item = Vec<u8>> + Send>),
    /// The iterator is exhausted (or the stream was dropped early).
    Done,
    /// A panic poisoned the iterator; the message reports it.
    Errored(String),
}

/// Everything that can go wrong at the stream surface.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamError {
    /// The stream table has no free slot.
    TableFull,
    /// Table initialization ever failed (sticky).
    TagInUse,
    /// The handle is null, stale or foreign.
    InvalidHandle,
    /// The iterator panicked on an earlier pull; the message reports
    /// the panic.
    Errored(String),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TableFull => write!(f, "the bffi-stream table is full"),
            Self::TagInUse => write!(f, "the bffi-stream tag is already declared"),
            Self::InvalidHandle => write!(f, "unknown or stale stream handle"),
            Self::Errored(message) => write!(f, "the stream failed: {message}"),
        }
    }
}

impl std::error::Error for StreamError {}

/// Unified-format conversion onto existing codes; the domain error
/// is preserved as the source.
impl From<StreamError> for bffi_core::BffiError {
    fn from(error: StreamError) -> Self {
        use bffi_core::ErrorCode;
        let code = match &error {
            StreamError::TableFull => ErrorCode::TableFull,
            StreamError::TagInUse => ErrorCode::InvalidTag,
            StreamError::InvalidHandle => ErrorCode::InvalidHandle,
            StreamError::Errored(_) => ErrorCode::Error,
        };
        let message = error.to_string();
        bffi_core::BffiError::with_source(code, message, error)
    }
}

/// The result of one chunk pull: `Done` when the iterator is
/// exhausted, otherwise up to `max` pre-encoded item records.
#[derive(Debug, PartialEq, Eq)]
pub enum Chunk {
    /// The stream is exhausted.
    Done,
    /// Up to `max` items, each one complete wire record.
    Items(Vec<Vec<u8>>),
}

fn tables() -> Result<(), bffi_core::RegistryError> {
    static TABLES: OnceLock<Result<(), bffi_core::RegistryError>> = OnceLock::new();
    *TABLES.get_or_init(|| {
        Registry::global()
            .declare::<StreamEntry>(STREAM_TAG)
            .map_err(|_| bffi_core::RegistryError::TagAlreadyRegistered(STREAM_TAG))
    })
}

/// Registers a stream over `iter` and returns its handle.
///
/// # Errors
///
/// [`StreamError::TableFull`] when the stream table has no free
/// slot, or [`StreamError::TagInUse`] if table initialization ever
/// failed.
pub fn spawn(iter: Box<dyn Iterator<Item = Vec<u8>> + Send>) -> Result<Handle, StreamError> {
    tables().map_err(|_| StreamError::TagInUse)?;
    Registry::global()
        .insert(
            STREAM_TAG,
            Arc::new(StreamEntry {
                state: Mutex::new(StreamState::Items(iter)),
            }),
        )
        .map_err(|_| StreamError::TableFull)
}

/// Pulls up to `max` pre-encoded item records. An empty `max` yields
/// an empty chunk (the JS side always sends a positive budget).
///
/// # Errors
///
/// [`StreamError::InvalidHandle`] for null/stale/foreign handles,
/// [`StreamError::Errored`] when the iterator panicked on an earlier
/// pull (or this one - the panic is caught here, the stream is
/// marked errored and stays dead).
pub fn next_chunk(handle: Handle, max: usize) -> Result<Chunk, StreamError> {
    let entry = Registry::global()
        .get_typed::<StreamEntry>(handle)
        .ok_or(StreamError::InvalidHandle)?;
    let mut state = entry
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match &mut *state {
        StreamState::Done => Ok(Chunk::Done),
        StreamState::Errored(message) => Err(StreamError::Errored(message.clone())),
        StreamState::Items(iter) => {
            let mut items = Vec::new();
            for _ in 0..max {
                // Catch the panic so the entry mutex never stays
                // poisoned and the stream reports a clean, sticky
                // error instead of taking down the host.
                let next = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| iter.next()));
                match next {
                    Ok(Some(item)) => items.push(item),
                    Ok(None) => {
                        *state = StreamState::Done;
                        break;
                    }
                    Err(payload) => {
                        let message = bffi_core::panic_message(payload.as_ref());
                        *state = StreamState::Errored(message.clone());
                        return Err(StreamError::Errored(message));
                    }
                }
            }
            Ok(Chunk::Items(items))
        }
    }
}

/// Releases the stream behind `handle` (an early exit before the
/// iterator is exhausted); `true` when a live stream was dropped.
/// The GC finalizer and an explicit JS `return()` both route here;
/// dropping twice or after exhaustion reports `false` and stays
/// silent.
#[must_use]
pub fn drop_stream(handle: Handle) -> bool {
    match Registry::global().get_typed::<StreamEntry>(handle) {
        Some(entry) => {
            let mut state = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            matches!(*state, StreamState::Items(_)) && {
                *state = StreamState::Done;
                true
            }
        }
        None => false,
    }
}

/// Whether the handle names a live (not yet exhausted or errored)
/// stream.
#[must_use]
pub fn is_live(handle: Handle) -> bool {
    match Registry::global().get_typed::<StreamEntry>(handle) {
        Some(entry) => {
            let state = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            matches!(*state, StreamState::Items(_))
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{Chunk, StreamError, drop_stream, is_live, next_chunk, spawn};

    fn numbers(range: std::ops::Range<u32>) -> Box<dyn Iterator<Item = Vec<u8>> + Send> {
        use crate::bffi_types::wire as w;
        Box::new(range.map(move |n| {
            let mut out = Vec::new();
            w::encode_i32(&mut out, n as i32);
            out
        }))
    }

    #[test]
    fn chunks_yield_in_order_until_done() {
        let handle = spawn(numbers(0..5)).expect("spawned");
        assert!(is_live(handle));

        let mut seen = Vec::new();
        while let Chunk::Items(items) = next_chunk(handle, 2).expect("chunk") {
            seen.extend(items);
        }
        assert_eq!(seen.len(), 5);
        assert!(!is_live(handle), "exhausted streams are not live");
    }

    #[test]
    fn max_limits_the_chunk_and_the_pull_resumes() {
        let handle = spawn(numbers(0..4)).expect("spawned");
        match next_chunk(handle, 3).expect("first chunk") {
            Chunk::Items(items) => assert_eq!(items.len(), 3),
            Chunk::Done => panic!("stream is not exhausted"),
        }
        match next_chunk(handle, 3).expect("second chunk") {
            Chunk::Items(items) => assert_eq!(items.len(), 1),
            Chunk::Done => panic!("one item remains"),
        }
        assert_eq!(next_chunk(handle, 3).expect("done"), Chunk::Done);
    }

    #[test]
    fn empty_max_yields_an_empty_chunk() {
        let handle = spawn(numbers(0..1)).expect("spawned");
        assert_eq!(
            next_chunk(handle, 0).expect("empty budget"),
            Chunk::Items(Vec::new())
        );
    }

    #[test]
    fn invalid_and_dropped_handles_report_cleanly() {
        assert!(matches!(
            next_chunk(crate::bffi_core::Handle::NULL, 4),
            Err(StreamError::InvalidHandle)
        ));
        let handle = spawn(numbers(0..2)).expect("spawned");
        assert!(drop_stream(handle), "first drop wins");
        assert!(!drop_stream(handle), "second drop stays silent");
        assert_eq!(next_chunk(handle, 4).expect("done"), Chunk::Done);
    }

    #[test]
    fn a_panicking_iterator_poisons_only_its_own_stream() {
        struct Boom(usize);
        impl Iterator for Boom {
            type Item = Vec<u8>;
            fn next(&mut self) -> Option<Vec<u8>> {
                if self.0 == 0 {
                    panic!("stream boom");
                }
                self.0 -= 1;
                Some(Vec::new())
            }
        }
        let healthy = spawn(numbers(0..1)).expect("spawned");
        let poisoned = spawn(Box::new(Boom(0))).expect("spawned");

        let error = next_chunk(poisoned, 4).expect_err("poisoned");
        assert!(
            matches!(error, StreamError::Errored(ref message) if message.contains("stream boom")),
            "the panic message surfaces: {error:?}"
        );
        // Sticky: the stream stays dead.
        assert!(matches!(
            next_chunk(poisoned, 4),
            Err(StreamError::Errored(_))
        ));
        // The table (and other streams) survive.
        assert_eq!(
            next_chunk(healthy, 4).expect("healthy chunk"),
            Chunk::Items(vec![vec![crate::bffi_types::wire::TAG_I32, 0, 0, 0, 0]])
        );
    }
}
