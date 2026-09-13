//! Integration test for `bffi::callback::invoke_wait` against a
//! STOPPED event loop: the job cannot be queued at all, so the wait
//! fails immediately with `CallbackError::LoopStopped` instead of
//! burning the timeout.
//!
//! Own process on purpose: `bffi::stop` is sticky process-global and
//! would break every queue-sharing test (see `callback-wait.rs`).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bffi::bffi_callback::{CallbackError, CallbackSig, Value, ValueType, register, set_js_thread};

#[test]
fn invoke_wait_after_a_loop_stop_fails_immediately_with_loop_stopped() {
    // This binary owns the process-global loop state: stop it before
    // the first marshal attempt.
    bffi::stop();

    // Bind THIS thread as the JS thread (single-test binary): the
    // spawned worker below must be a foreign thread to take the
    // marshal path - an unbound process would delegate to the sync
    // invoke and never reach the queue.
    set_js_thread().expect("this binary owns the binding");

    let handle = register(
        CallbackSig::new(ValueType::Bool, &[]),
        Arc::new(|_| Value::Bool(true)),
    )
    .expect("callback table has room");

    let started = Instant::now();
    let result = thread::spawn(move || bffi::invoke_wait(handle, &[], Duration::from_secs(5)))
        .join()
        .expect("worker must not panic");

    assert_eq!(result, Err(CallbackError::LoopStopped));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a stopped loop must fail the wait immediately, not after the timeout"
    );
}
