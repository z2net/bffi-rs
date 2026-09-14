//! Example bffi-rs native module: the event-loop surface of the
//! framework (`bffi-event-loop`).
//!
//! The jobs are intentionally trivial - the point is the DELIVERY
//! machinery crossing the bffi boundary, verified end-to-end by the
//! `@z2net/bffi` pipeline:
//!
//! - `enqueue_job` / `loop_pump` - the non-blocking queue + drain
//!   pair (each job executes exactly once);
//! - `loop_pending` / `loop_executed` - the monitoring probes;
//! - `marshal_status` - the wrong-thread delivery path: `12`
//!   (`WrongThread`) while no runner is up, `0` once a runner drains;
//! - `loop_run` / `loop_stop` - the blocking drain for a worker
//!   thread and its sticky shutdown.
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
bffi::bffi_runtime_abi!(module = crate::module_def::MODULE);

pub mod module_def;

use std::sync::atomic::{AtomicI64, Ordering};

use bffi::{
    BffiError, ErrorCode, Job, bffi, enqueue, executed_total, marshal, pending, pump, run, stop,
};

/// The value stored by the last executed job (read through
/// [`last_result`]).
static LAST_RESULT: AtomicI64 = AtomicI64::new(0);

/// The domain error of the module: wraps the event-loop error text.
#[derive(Debug)]
pub struct LoopError(String);

impl std::fmt::Display for LoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoopError {}

impl From<LoopError> for bffi::BffiError {
    fn from(error: LoopError) -> Self {
        bffi::BffiError::new(bffi::ErrorCode::DomainError, error.0)
    }
}

impl From<bffi::EventLoopError> for LoopError {
    fn from(error: bffi::EventLoopError) -> Self {
        Self(error.to_string())
    }
}

/// The shared failure epilogue of the status-returning exports:
/// stores the error as the thread-local last error and returns its
/// numeric code.
fn store_code(error: BffiError) -> u32 {
    let code = error.code;
    bffi::set_last_error(error);
    code.as_u32()
}

/// Puts one job on the queue: the job stores `x * 2` into the
/// process-wide result slot when it executes (see [`last_result`]).
///
/// # Errors
///
/// [`LoopError`] once [`loop_stop`] has been called (sticky).
#[bffi]
pub fn enqueue_job(x: i64) -> Result<(), LoopError> {
    let job: Job = Box::new(move || LAST_RESULT.store(x * 2, Ordering::Relaxed));
    enqueue(job)?;
    Ok(())
}

/// The value stored by the last executed job (`0` before the first).
#[bffi]
pub fn last_result() -> i64 {
    LAST_RESULT.load(Ordering::Relaxed)
}

/// Drains the queue without blocking; returns the number of jobs
/// executed by THIS call (`0` on an empty queue). The JS side calls
/// it in a loop (`pumpUntil` from `@z2net/bffi`) to deliver async
/// resolutions; here it demonstrates the raw mechanics.
#[bffi]
pub fn loop_pump() -> u64 {
    pump()
}

/// The number of jobs waiting for execution.
#[bffi]
pub fn loop_pending() -> u64 {
    pending()
}

/// The number of jobs executed since process start, across all
/// runners.
#[bffi]
pub fn loop_executed() -> u64 {
    executed_total()
}

/// Marshals one job (`x * 2` into the result slot) onto the RUNNING
/// loop and returns the raw `ErrorCode` numeric value: `0` = accepted
/// (the job executes on the runner thread), `12` = `WrongThread` (no
/// runner is up). The status variant keeps the exact code observable;
/// the last error carries the message.
#[bffi]
pub fn marshal_status(x: i64) -> u32 {
    let job: Job = Box::new(move || LAST_RESULT.store(x * 2, Ordering::Relaxed));
    match marshal(job) {
        Ok(()) => ErrorCode::Ok.as_u32(),
        Err(error) => store_code(BffiError::from(error)),
    }
}

/// Blocks the calling thread draining the queue until [`loop_stop`];
/// returns the number of jobs executed by THIS runner. Intended for
/// the worker thread of the e2e test.
#[bffi]
pub fn loop_run() -> u64 {
    run()
}

/// Stops the loop for good (sticky): `loop_run` returns and later
/// `enqueue_job` calls report "the event loop has been stopped".
#[bffi]
pub fn loop_stop() {
    stop();
}
