//! Example bffi-rs native module: the multi-isolate surface.
//!
//! Bun 1.4 workers are real threads of one process, so a process may
//! host several JS isolates. The exports here are intentionally
//! trivial - the point is the THREAD TOPOLOGY crossing the bffi
//! boundary, verified end-to-end by the `@z2net/bffi` pipeline:
//!
//! - `bind_js_thread` - registers the CALLING thread as a JS thread
//!   (multi-isolate: every Worker registers its own; idempotent);
//! - `sum_to` - CPU-bound native work, callable from BOTH isolates
//!   concurrently (the reason workers exist);
//! - `spawn_invoke_wait` / `wait_code` / `wait_value` - a DETACHED
//!   native thread marshals a JS-bound callback onto its OWNING
//!   isolate with `invoke_wait`; the outcome (code + value) lands in
//!   process-wide slots the test polls;
//! - `loop_pump` - the non-blocking drain a JS isolate calls on its
//!   own ticks (the worker pumps its slot queue with it).
//!
//! Aggregation lives in [`module_def`] (single source); the
//! `emit-json` binary materializes `.bffi/bffi.api.json` from it for
//! the `@z2net/bffi` pipeline.

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free): the JS pipeline drains errors and reads buffers
// through them.
bffi::bffi_runtime_abi!();

// The four JS-facing generic callback exports (bffi_callback_*):
// set_thread/bind/invoke/revoke. The JS side composes them through
// `bindJsCallback`/`invokeCallback`/`revokeCallback`/`setJsThread`.
bffi::bffi_callback_abi!();

pub mod module_def;

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::time::Duration;

use bffi::{
    BffiError, ErrorCode, Handle, Value, ValueType, bffi, invoke_wait, pump, register,
    set_js_thread, unset_js_thread,
};

/// The "no outcome yet" sentinel of [`wait_code`].
pub const WAIT_PENDING: u32 = u32::MAX;

/// The code of the last `spawn_invoke_wait` outcome.
static WAIT_CODE: AtomicU32 = AtomicU32::new(WAIT_PENDING);

/// The value delivered by the last successful outcome.
static WAIT_VALUE: AtomicI32 = AtomicI32::new(0);

/// The shared failure epilogue of the status-returning exports:
/// stores the error as the thread-local last error and returns its
/// numeric code.
fn store_code(error: BffiError) -> u32 {
    let code = error.code;
    bffi::set_last_error(error);
    code.as_u32()
}

/// The domain error of the module (the spawn path cannot fail today,
/// but the Result shape keeps the boundary contract explicit).
#[derive(Debug)]
pub struct SpawnError(String);

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SpawnError {}

impl From<SpawnError> for BffiError {
    fn from(error: SpawnError) -> Self {
        BffiError::new(ErrorCode::DomainError, error.0)
    }
}

/// Registers the CURRENT thread as a JS thread of this process
/// (multi-isolate: the main script and every Worker call this once on
/// their own thread; idempotent; the registration dies with the
/// thread). Returns the raw `ErrorCode` numeric value (`0` = bound).
#[bffi]
pub fn bind_js_thread() -> u32 {
    match set_js_thread() {
        Ok(()) => ErrorCode::Ok.as_u32(),
        Err(error) => store_code(BffiError::from(error)),
    }
}

/// Deregisters the CURRENT thread: the explicit shutdown half of
/// [`bind_js_thread`]. A Worker calls this right before exiting so
/// its slot queue retires immediately and further targeted deliveries
/// to it fail fast (LoopStopped) instead of timing out. Idempotent.
#[bffi]
pub fn unbind_js_thread() -> u32 {
    match unset_js_thread() {
        Ok(()) => ErrorCode::Ok.as_u32(),
        Err(error) => store_code(BffiError::from(error)),
    }
}

/// CPU-bound native work: sums `1..=n`. Callable from BOTH isolates
/// concurrently - the reason a worker exists.
#[bffi]
pub fn sum_to(n: u64) -> u64 {
    (1..=n).sum()
}

/// Registers a native doubling callback `i32(i32)`; the JS side
/// invokes it by handle (kept from the callbacks example so the
/// workers test can pin the JS -> Rust direction too).
#[bffi]
pub fn callback_register() -> Result<u64, SpawnError> {
    let handle = register(doubling_sig(), Arc::new(doubling_body))
        .map_err(|error| SpawnError(error.to_string()))?;
    Ok(handle.as_u64())
}

/// Spawns a DETACHED native thread that marshals the JS-bound
/// callback behind `handle` with `invoke_wait(handle, [a], timeout)`:
/// the job is delivered to the callback's OWNING isolate (the thread
/// that bound it), never across an isolate boundary. The outcome
/// lands in the process-wide slots ([`wait_code`] / [`wait_value`]).
#[bffi]
pub fn spawn_invoke_wait(handle: u64, a: i32, timeout_ms: u64) -> Result<(), SpawnError> {
    WAIT_CODE.store(WAIT_PENDING, Ordering::Release);
    WAIT_VALUE.store(0, Ordering::Release);
    std::thread::spawn(move || {
        let outcome = invoke_wait(
            Handle::from_raw(handle),
            &[Value::I32(a)],
            Duration::from_millis(timeout_ms),
        );
        match outcome {
            Ok(Value::I32(value)) => {
                WAIT_VALUE.store(value, Ordering::Release);
                WAIT_CODE.store(ErrorCode::Ok.as_u32(), Ordering::Release);
            }
            Ok(_) => {
                WAIT_CODE.store(ErrorCode::Error.as_u32(), Ordering::Release);
            }
            Err(error) => {
                let code = BffiError::from(error).code;
                WAIT_CODE.store(code.as_u32(), Ordering::Release);
            }
        }
    });
    Ok(())
}

/// The code of the last `spawn_invoke_wait` outcome
/// ([`WAIT_PENDING`] = the detached thread has not reported yet).
#[bffi]
pub fn wait_code() -> u32 {
    WAIT_CODE.load(Ordering::Acquire)
}

/// The value delivered by the last successful outcome (`0` before).
#[bffi]
pub fn wait_value() -> i32 {
    WAIT_VALUE.load(Ordering::Acquire)
}

/// The non-blocking drain: a JS isolate calls this on its own ticks
/// to execute queued jobs (its slot queue first, then the global
/// queue). Returns the number of jobs executed by THIS call.
#[bffi]
pub fn loop_pump() -> u64 {
    pump()
}

/// The signature of the example callbacks: `i32(i32)`.
fn doubling_sig() -> bffi::CallbackSig {
    bffi::CallbackSig::new(ValueType::I32, &[ValueType::I32])
}

/// The doubling body: `invoke` checks the signature before calling,
/// so the fallback arm is unreachable but required for exhaustiveness.
fn doubling_body(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::I32(a)) => Value::I32(a.wrapping_mul(2)),
        _ => Value::I32(0),
    }
}
