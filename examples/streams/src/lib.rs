//! Example bffi-rs native module: the B2 stream surface.
//!
//! `#[bffi_stream]` exports hand JavaScript an
//! `AsyncIterableIterator` over a Rust `Iterator` (pull-chunk: the
//! JS side pulls up to `max` items per call through the generic
//! `bffi_stream_next` export):
//!
//! - `numbers` streams a generated range of numbers;
//! - `samples` streams records (the B1 composites compose);
//! - `fizzbuzz` shows a transform over an item sequence.
//!
//! The pull contract: the iterator runs on the JS thread during
//! `next()` and must not block indefinitely; long-running producers
//! arrive with the push model (B2.2).
//!
//! Aggregation lives in [`module_def`]; the `emit-json` binary
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free) plus the two JS-facing stream exports
// (bffi_stream_next/bffi_stream_drop).
bffi::bffi_runtime_abi!();
bffi::bffi_stream_abi!();

pub mod module_def;

use bffi::{BffiEnum, BffiRecord, BffiWire};

/// The measurement axis.
#[derive(BffiEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Along the X axis.
    Horizontal,
    /// Along the Y axis.
    Vertical,
}

/// One stream item.
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Sample {
    /// Where it was taken.
    pub at: f64,
    /// The station label.
    pub label: String,
    /// The axis it was taken along.
    pub axis: Axis,
}

/// Streams `1..=count` (an empty or huge count behaves as written).
#[bffi::bffi_stream]
fn numbers(count: u32) -> impl Iterator<Item = i32> + Send {
    (1..=count).map(|n| n as i32)
}

/// Streams `count` alternating-axis samples.
#[bffi::bffi_stream]
fn samples(count: u32) -> impl Iterator<Item = Sample> + Send {
    (0..count).map(|index| Sample {
        at: f64::from(index),
        label: format!("station-{index}"),
        axis: if index % 2 == 0 {
            Axis::Horizontal
        } else {
            Axis::Vertical
        },
    })
}

/// The classic transform: items of `1..=count` mapped to their
/// fizzbuzz word (a stream of strings).
#[bffi::bffi_stream]
fn fizzbuzz(count: u32) -> impl Iterator<Item = String> + Send {
    (1..=count).map(|n| match (n % 3, n % 5) {
        (0, 0) => "fizzbuzz".to_owned(),
        (0, _) => "fizz".to_owned(),
        (_, 0) => "buzz".to_owned(),
        _ => n.to_string(),
    })
}
