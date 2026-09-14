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
bffi::bffi_runtime_abi!(module = crate::module_def::MODULE);
bffi::bffi_stream_abi!();

pub mod module_def;

use std::time::Duration;

use bffi::bffi_stream::Ctx;
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

/// A slow PUSH producer: one reading per 5 ms tick, delivered with
/// backpressure through the bounded buffer. JS pulls with
/// `for await` exactly like the pull streams above.
#[bffi::bffi_stream]
async fn readings(ctx: Ctx<f64>, count: u32) -> Result<(), bffi::StreamError> {
    for index in 0..count {
        bffi::sleep(Duration::from_millis(5)).await;
        ctx.push(f64::from(index)).await?;
    }
    Ok(())
}

/// A PUSH producer delivering `Result` items: even ticks arrive as
/// values, odd ticks as item errors (`ctx.push(Ok/Err)` - the item
/// itself carries the domain outcome, the stream still completes).
#[bffi::bffi_stream]
async fn checked_readings(
    ctx: Ctx<Result<u32, String>>,
    count: u32,
) -> Result<(), bffi::StreamError> {
    for index in 0..count {
        bffi::sleep(Duration::from_millis(2)).await;
        if index % 2 == 0 {
            ctx.push(Ok(index)).await?;
        } else {
            ctx.push(Err(format!("bad tick {index}"))).await?;
        }
    }
    Ok(())
}

/// Result items: even indexes arrive as values, odd ones as item
/// errors (the JS iterator throws at the first error item). The
/// parens are the documented workaround for generic bindings in
/// impl-trait position.
#[bffi::bffi_stream]
fn flaky(count: u32) -> impl Iterator<Item = Result<u64, String>> + Send {
    (0..count).map(|index| {
        if index % 2 == 0 {
            Ok(u64::from(index))
        } else {
            Err(format!("odd index {index}"))
        }
    })
}

/// Values descending from the u64 maximum (exact BigInt delivery).
#[bffi::bffi_stream]
fn big_values(count: u32) -> impl Iterator<Item = u64> + Send {
    (0..count).map(|index| u64::MAX - u64::from(index))
}
