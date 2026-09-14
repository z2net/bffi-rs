//! Example bffi-rs native module: the callback surface of the
//! framework (`bffi-callback`).
//!
//! The callback body is intentionally trivial - the point is the
//! OWNERSHIP and THREADING machinery crossing the bffi boundary,
//! verified end-to-end by the `@z2net/bffi` pipeline:
//!
//! - `callback_register` / `callback_invoke_status` - the JS -> Rust
//!   lifecycle (the JS side invokes through the generic
//!   `bffi_callback_invoke` ABI via `invokeCallback`);
//! - `callback_ptr` - the Rust -> JS direction: the opaque
//!   `bindJsCallback` pointer read back from the JS table (a LIVE
//!   call into JS is the async delivery path's job - see the async
//!   example);
//! - `bind_js_thread` / `loop_run` / `loop_stop` - the JS-thread
//!   registration (multi-isolate: every JS isolate registers its own
//!   thread) and the blocking drain a worker thread enters;
//! - `marshal_invoke` - the wrong-thread delivery: the job invokes
//!   the callback ON THE RUNNER THREAD and stores the result.
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

// The four JS-facing generic callback exports (bffi_callback_*):
// set_thread/bind/invoke/revoke. The JS side composes them through
// `bindJsCallback`/`invokeCallback`/`revokeCallback`/`setJsThread`.
bffi::bffi_callback_abi!();

pub mod module_def;

use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};

use bffi::{
    BffiError, CallbackError, CallbackSig, ErrorCode, Handle, Value, ValueType, bffi, invoke,
    js_callback, register, run, set_js_thread, stop,
};

/// The value stored by the last executed marshal job (read through
/// [`last_invoked`]).
static LAST_INVOKED: AtomicI32 = AtomicI32::new(0);

/// The domain error of the module: wraps the callback error text.
#[derive(Debug)]
pub struct InvokeError(String);

impl std::fmt::Display for InvokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvokeError {}

impl From<InvokeError> for bffi::BffiError {
    fn from(error: InvokeError) -> Self {
        bffi::BffiError::new(bffi::ErrorCode::DomainError, error.0)
    }
}

impl From<CallbackError> for InvokeError {
    fn from(error: CallbackError) -> Self {
        Self(error.to_string())
    }
}

/// The fixed signature of the example callbacks: `i32(i32)`.
fn doubling_sig() -> CallbackSig {
    CallbackSig::new(ValueType::I32, &[ValueType::I32])
}

/// The fixed body: doubles its single `i32` argument. `invoke` checks
/// the signature before calling, so the fallback arm is unreachable
/// but required for exhaustiveness.
fn doubling_body(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::I32(a)) => Value::I32(a.wrapping_mul(2)),
        _ => Value::I32(0),
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

/// Registers the native doubling callback `i32(i32)`; JS invokes it
/// by handle through `invokeCallback` (the generic ABI).
#[bffi]
pub fn callback_register() -> Result<u64, InvokeError> {
    let handle = register(doubling_sig(), Arc::new(doubling_body))?;
    Ok(handle.as_u64())
}

/// Invokes the native callback behind `handle` with one `i32` and
/// returns the raw `ErrorCode` numeric value with the last error
/// stored, so tests can pin exact codes (`0` = Ok, `12` =
/// WrongThread, `4` = InvalidHandle). The typed invoke path is the
/// generic ABI itself (`invokeCallback` on the JS side).
#[bffi]
pub fn callback_invoke_status(handle: u64, a: i32) -> u32 {
    match invoke(Handle::from_raw(handle), &[Value::I32(a)]) {
        Ok(Value::I32(_)) => ErrorCode::Ok.as_u32(),
        Ok(_) => store_code(BffiError::new(
            ErrorCode::Error,
            "callback returned an unexpected value kind",
        )),
        Err(error) => store_code(BffiError::from(error)),
    }
}

/// Reads back the JS-side callback behind `handle` (the Rust -> JS
/// direction): the opaque pointer token stored at bind time. The
/// token is NEVER dereferenced by Rust - a LIVE call into JS through
/// this pointer is the async delivery path's job (see the async
/// example); here the e2e verifies storage, identity and revocation.
#[bffi]
pub fn callback_ptr(handle: u64) -> Result<u64, InvokeError> {
    let info = js_callback(Handle::from_raw(handle))?;
    Ok(info.ptr as u64)
}

/// Registers the CURRENT thread as a JS thread of this process
/// (multi-isolate: every JS isolate registers its own thread;
/// idempotent). Returns the raw `ErrorCode` numeric value: `0` =
/// registered. The worker of the e2e test calls this before entering
/// the loop drain.
#[bffi]
pub fn bind_js_thread() -> u32 {
    match set_js_thread() {
        Ok(()) => ErrorCode::Ok.as_u32(),
        Err(error) => store_code(BffiError::from(error)),
    }
}

/// Marshals a job onto the RUNNING loop: the job invokes the callback
/// behind `handle` with `a` ON THE RUNNER THREAD (where the JS thread
/// gate passes) and stores the result into the process-wide slot (see
/// [`last_invoked`]). Returns the raw `ErrorCode` numeric value: `0` =
/// accepted, `12` = `WrongThread` (no runner is up).
#[bffi]
pub fn marshal_invoke(handle: u64, a: i32) -> u32 {
    let job: bffi::Job = Box::new(move || {
        if let Ok(Value::I32(value)) = invoke(Handle::from_raw(handle), &[Value::I32(a)]) {
            LAST_INVOKED.store(value, Ordering::Relaxed);
        }
    });
    match bffi::marshal(job) {
        Ok(()) => ErrorCode::Ok.as_u32(),
        Err(error) => store_code(BffiError::from(error)),
    }
}

/// The value stored by the last executed marshal job (`0` before the
/// first).
#[bffi]
pub fn last_invoked() -> i32 {
    LAST_INVOKED.load(Ordering::Relaxed)
}

/// Blocks the calling thread draining the loop until [`loop_stop`];
/// returns the number of jobs executed by THIS runner. Intended for
/// the worker thread of the e2e test (after `bind_js_thread`, so the
/// marshalled callback invocations pass the JS-thread gate).
#[bffi]
pub fn loop_run() -> u64 {
    run()
}

/// Stops the loop for good (sticky): `loop_run` returns.
#[bffi]
pub fn loop_stop() {
    stop()
}
