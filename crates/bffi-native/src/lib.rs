//! The bffi-rs reference native library: a minimal cdylib expanding
//! the runtime ABI (`bffi_error_*`, the `bffi_buffer` pair,
//! `bffi_types_free`).
//!
//! This is the library the napi-rs-style platform npm packages carry
//! (`@z2net/bffi-native-<triple>`, assembled by `bffi pack` from the
//! release artifacts). It is deliberately tiny - the point is a REAL
//! bffi surface crossing the boundary, loadable through the
//! `@z2net/bffi` loader without building anything locally:
//!
//! - [`add`] - primitives and the `__ret` out-parameter;
//! - [`shout`] - the cstring (`&str`) parameter path and a string
//!   return (transient-buffer handle);
//! - [`version`] - the crate version as a static string return.
//!
//! A test-only export ([`bffi_test_roundtrip_string`], the real
//! bun:ffi JSCallback cstring round trip) plus the workers-e2e pair
//! ([`bffi_test_invoke_on_thread`], the native-thread `invoke_wait`
//! targeted delivery, and [`bffi_test_pump`], the non-blocking drain
//! a Worker isolate pumps with) are also present but NOT registered
//! in [`module_def`]: they stay out of the public demo API and do not
//! change `exportsHash`.
//!
//! Aggregation lives in [`module_def`] (single source); the
//! `emit-json` binary materializes `.bffi/bffi.api.json` from it.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free): the JS pipeline drains errors and reads buffers
// through them.
bffi::bffi_runtime_abi!(module = crate::module_def::MODULE);

// The generic callback ABI exports (bffi_callback_set_thread/
// _unset_thread/_bind/_invoke/_revoke): the JS-bound callback surface
// the e2e suite drives through [`bffi_test_roundtrip_string`].
bffi::bffi_callback_abi!();

pub mod module_def;

// The crate-root import the `#[bffi]` expansions resolve through.
use bffi::bffi;

use std::time::Duration;

use bffi::{BffiError, CopiedBuf, ErrorCode, Handle, Value, invoke_wait, pump, set_last_error};

/// Stores `error` as the thread-local last error and returns its code:
/// the shared failure epilogue (mirrors `bffi-callback`'s `abi::store`).
fn store(error: BffiError) -> ErrorCode {
    let code = error.code;
    set_last_error(error);
    code
}

/// Adds two numbers.
#[bffi]
pub fn add(a: u32, b: u32) -> u32 {
    a.wrapping_add(b)
}

/// Returns an uppercased greeting (string return through the
/// transient-buffer pair).
#[bffi]
pub fn shout(name: &str) -> String {
    format!("HELLO {name}!")
}

/// The version of this library (static string return).
#[bffi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The body behind [`bffi_test_roundtrip_string`]: invokes the
/// JS-bound callback behind `handle` with one `"ping"` argument
/// (`invoke_wait` takes the direct path - this runs on the JS thread)
/// and stores the `pong:` reply as a transient-buffer handle (same
/// pattern as `bffi-callback`'s `abi::invoke_body`).
#[allow(clippy::not_unsafe_ptr_arg_deref)]
fn roundtrip_string_body(handle: u64, __ret: *mut u64) -> ErrorCode {
    if __ret.is_null() {
        return store(BffiError::new(
            ErrorCode::NullPointer,
            "output pointer is null",
        ));
    }
    let outcome = invoke_wait(
        Handle::from_raw(handle),
        &[Value::Str("ping".to_owned())],
        Duration::from_secs(5),
    );
    let text = match outcome {
        Ok(Value::Str(text)) => text,
        Ok(_) => {
            return store(BffiError::new(
                ErrorCode::InvalidArgument,
                "the JS callback returned a non-string value",
            ));
        }
        Err(error) => return store(BffiError::from(error)),
    };
    match bffi::build::runtime::store_bytes(CopiedBuf::from_vec(
        format!("pong:{text}").into_bytes(),
    )) {
        Ok(buffer) => {
            // SAFETY: `__ret` is non-null (checked above) and valid
            // for one `u64` write per the bun:ffi out-parameter
            // contract.
            unsafe { ::std::ptr::write(__ret, buffer.as_u64()) };
            ErrorCode::Ok
        }
        Err(error) => store(BffiError::from(error)),
    }
}

/// Test-only export backing
/// `packages/bffi/test/e2e-string-return.test.ts`: the JS side binds a
/// `(string) -> string` callback and then calls this synchronously on
/// the JS thread - the full cstring round trip (the `Value::Str`
/// argument crosses the real bun:ffi JSCallback as a cstring, the JS
/// return crosses back through bun:ffi's call-scoped transcode
/// buffer, and the reply rides the transient-buffer pair).
///
/// Deliberately NOT registered in [`module_def`]: it is not part of
/// the public demo API and must not change `exportsHash`.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn bffi_test_roundtrip_string(handle: u64, __ret: *mut u64) -> u32 {
    #[cfg(debug_assertions)]
    {
        roundtrip_string_body(handle, __ret).as_u32()
    }
    #[cfg(not(debug_assertions))]
    {
        bffi::boundary::run_extern_body_or(
            || roundtrip_string_body(handle, __ret).as_u32(),
            ErrorCode::Panic.as_u32(),
        )
    }
}

/// The body behind [`bffi_test_invoke_on_thread`]: invokes the
/// JS-bound callback behind `handle` from a PLAIN native thread (the
/// export spawns it and joins it before returning), so `invoke_wait`
/// takes the marshal path and the job is delivered TARGETED to the
/// callback's owning isolate - the Worker that bound the callback
/// must drain its slot queue ([`bffi_test_pump`]) for the reply to
/// land. The `pong:` reply is stored as a transient-buffer handle,
/// same as [`bffi_test_roundtrip_string`].
#[allow(clippy::not_unsafe_ptr_arg_deref)]
fn invoke_on_thread_body(handle: u64, __ret: *mut u64) -> ErrorCode {
    if __ret.is_null() {
        return store(BffiError::new(
            ErrorCode::NullPointer,
            "output pointer is null",
        ));
    }
    // The spawned thread only owns `handle` by value (a raw
    // `__ret` would be `!Send`); the reply storage runs HERE, after
    // the join, so the last error lands in the caller's thread-local.
    let joined = std::thread::spawn(move || {
        invoke_wait(
            Handle::from_raw(handle),
            &[Value::Str("ping".to_owned())],
            Duration::from_secs(10),
        )
    })
    .join();
    let text = match joined {
        Ok(Ok(Value::Str(text))) => text,
        Ok(Ok(_)) => {
            return store(BffiError::new(
                ErrorCode::InvalidArgument,
                "the JS callback returned a non-string value",
            ));
        }
        Ok(Err(error)) => return store(BffiError::from(error)),
        Err(_) => {
            return store(BffiError::new(
                ErrorCode::Panic,
                "the invoke thread panicked",
            ));
        }
    };
    match bffi::build::runtime::store_bytes(CopiedBuf::from_vec(
        format!("pong:{text}").into_bytes(),
    )) {
        Ok(buffer) => {
            // SAFETY: `__ret` is non-null (checked above) and valid
            // for one `u64` write per the bun:ffi out-parameter
            // contract.
            unsafe { ::std::ptr::write(__ret, buffer.as_u64()) };
            ErrorCode::Ok
        }
        Err(error) => store(BffiError::from(error)),
    }
}

/// Test-only export backing `packages/bffi/test/e2e-workers.test.ts`:
/// the main isolate passes a handle bound inside a Worker; a plain
/// native thread runs `invoke_wait`, the job is marshaled TARGETED to
/// the owning worker's slot queue, and the worker's pump loop
/// delivers it - the reply carries the worker-unique marker, which is
/// the multi-isolate delivery proof. Blocks until the reply lands or
/// the 10s timeout expires.
///
/// Deliberately NOT registered in [`module_def`] (same policy as
/// [`bffi_test_roundtrip_string`]).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn bffi_test_invoke_on_thread(handle: u64, __ret: *mut u64) -> u32 {
    #[cfg(debug_assertions)]
    {
        invoke_on_thread_body(handle, __ret).as_u32()
    }
    #[cfg(not(debug_assertions))]
    {
        bffi::boundary::run_extern_body_or(
            || invoke_on_thread_body(handle, __ret).as_u32(),
            ErrorCode::Panic.as_u32(),
        )
    }
}

/// Test-only export: the non-blocking event-loop drain for the
/// workers e2e - each Worker isolate pumps its own slot queue with it
/// on a timer (the same role `loop_pump` plays in the
/// examples/workers suite), which is what executes the jobs
/// [`bffi_test_invoke_on_thread`] marshals to it.
///
/// Deliberately NOT registered in [`module_def`].
#[unsafe(no_mangle)]
pub extern "C" fn bffi_test_pump() -> u64 {
    #[cfg(debug_assertions)]
    {
        pump()
    }
    #[cfg(not(debug_assertions))]
    {
        bffi::boundary::run_extern_body_or(pump, 0)
    }
}
