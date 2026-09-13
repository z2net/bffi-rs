//! Example bffi-rs native module: the async surface of the framework
//! (`bffi-async`).
//!
//! The point is NOT the computations themselves - they are REAL
//! Rust futures crossing the bffi boundary, so the `.bffi` pipeline
//! (`@z2net/bffi`) is verified end-to-end for the async paths:
//!
//! - `#[bffi_async]` fns become spawn shims: JS gets a task handle
//!   that the loader wraps into a `Promise`;
//! - values, strings, domain errors and panics travel the delivery
//!   path (worker -> event loop -> JS thread);
//! - `timeout` cancels the inner future; JS can cancel through the
//!   raw handle (`spawn_slow` + `cancel_task`);
//! - `loop_pump` is the delivery driver: promises settle only while
//!   the JS side drains the event loop (DESIGN §7).
//!
//! Aggregation lives in [`module_def`] (single source); the
//! `emit-json` binary materializes `.bffi/bffi.api.json` from it for
//! the `@z2net/bffi` pipeline.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free): the JS pipeline drains errors and reads buffers
// through them.
bffi::bffi_runtime_abi!();

// The two JS-facing async exports (bffi_async_attach/bffi_async_cancel):
// attach the resolve/reject trampolines, request cancellation.
bffi::bffi_async_abi!();

pub mod module_def;

use std::time::Duration;

use bffi::BffiRecord;
use bffi::Handle;
use bffi::r#async::{
    AsyncValue, sleep as async_sleep, spawn as async_spawn, timeout as async_timeout,
};

/// A task's final report (delivered as `Promise<Report>` on the JS
/// side).
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Report {
    /// The computed value.
    pub value: u64,
    /// A human-readable label.
    pub label: String,
}

/// The domain error of the module: a plain message wrapper.
#[derive(Debug)]
pub struct ExampleError(String);

impl std::fmt::Display for ExampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExampleError {}

impl From<ExampleError> for bffi::BffiError {
    fn from(error: ExampleError) -> Self {
        bffi::BffiError::new(bffi::ErrorCode::DomainError, error.0)
    }
}

/// Doubles `x` after a short sleep: the `#[bffi_async]` macro turns
/// this into a spawn shim returning a task handle; JS awaits
/// `Promise<bigint>`.
#[bffi::bffi_async]
pub async fn double_async(x: u64) -> u64 {
    async_sleep(Duration::from_millis(15)).await;
    x * 2
}

/// Returns an uppercased greeting: string results are delivered
/// through the transient-buffer payload (the same transport as the
/// sync `String` returns, encoded into the wire record).
#[bffi::bffi_async]
pub async fn shout_async(name: String) -> String {
    format!("HELLO {name}!")
}

/// Builds a report after a short sleep: composite results ride the
/// wire channel (`Promise<Report>` on the JS side, decoded through
/// the module's record table).
#[bffi::bffi_async]
pub async fn report_async(value: u64) -> Report {
    async_sleep(Duration::from_millis(10)).await;
    Report {
        value,
        label: format!("report-{value}"),
    }
}

/// Collects a number sequence: `Vec<T>` async returns ride the same
/// wire channel as sequences (`Promise<number[]>` on the JS side).
#[bffi::bffi_async]
pub async fn ticks_async(count: u32) -> Vec<f64> {
    async_sleep(Duration::from_millis(10)).await;
    (0..count).map(f64::from).collect()
}

/// An optional report: `None` (zero value) arrives as `null` - the
/// `Option<Report>` async return (`Promise<Report | null>`).
#[bffi::bffi_async]
pub async fn maybe_report(value: u64) -> Option<Report> {
    async_sleep(Duration::from_millis(10)).await;
    if value == 0 {
        None
    } else {
        Some(Report {
            value,
            label: format!("report-{value}"),
        })
    }
}

/// A failing task: `Result` rejects the promise with the domain
/// message.
#[bffi::bffi_async]
pub async fn fail_async() -> Result<u64, ExampleError> {
    Err(ExampleError("domain failure".to_owned()))
}

/// A panicking task: the executor's `catch_unwind` converts the panic
/// into a rejection with the panic message.
#[bffi::bffi_async]
#[allow(clippy::panic)]
pub async fn panic_async() -> u64 {
    panic!("async boom")
}

/// A task that never completes on its own: a 50 ms deadline drops the
/// inner future and rejects the promise with "task timed out".
#[bffi::bffi_async]
pub async fn timed_async() -> Result<u64, ExampleError> {
    let outcome = async_timeout(Duration::from_millis(50), async {
        async_sleep(Duration::from_secs(60)).await;
        Ok(AsyncValue::I64(0))
    })
    .await;
    match outcome {
        Ok(value) => Ok(match value {
            AsyncValue::I64(v) => u64::try_from(v).unwrap_or(0),
            _ => 0,
        }),
        // The timeout error's Display is exactly "task timed out".
        Err(error) => Err(ExampleError(error.to_string())),
    }
}

/// Spawns a slow task by hand (no macro): the RAW task handle comes
/// back as a plain `u64`, so JS can cancel it through [`cancel_task`]
/// before wrapping it into a promise with `wrapTask`.
#[bffi::bffi]
pub fn spawn_slow(ms: u64) -> Result<u64, ExampleError> {
    let task = async_spawn(async move {
        async_sleep(Duration::from_millis(ms)).await;
        Ok(AsyncValue::I64(i64::try_from(ms).unwrap_or(i64::MAX)))
    })
    .map_err(|error| ExampleError(error.to_string()))?;
    Ok(task.as_u64())
}

/// Requests cooperative cancellation of a task: `1` = a running task
/// was cancelled (the attached promise rejects with "task
/// cancelled"), `0` = invalid/stale handle or already finished.
#[bffi::bffi]
pub fn cancel_task(task: u64) -> u32 {
    u32::from(bffi::bffi_async::cancel(Handle::from_raw(task)))
}

/// The number of spawned-but-unfinished tasks (a probe for tests).
#[bffi::bffi]
pub fn async_pending() -> u64 {
    bffi::bffi_async::pending_tasks()
}

/// Drains the event loop without blocking: the JS side calls this in
/// a loop (`pumpUntil` from `@z2net/bffi`) while a task promise is
/// pending - the resolution job executes on the JS thread during the
/// drain, and THAT is what settles the promise.
#[bffi::bffi]
pub fn loop_pump() -> u64 {
    bffi::bffi_event_loop::pump()
}
