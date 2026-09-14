//! Integration tests for the `invoke_wait` re-entrancy gate
//! (CALLING-CONVENTION.md §9.1, deadlock contract): a JS thread that
//! is blocked inside a native call can never drain the loop, so a
//! nested marshal-and-wait must fail FAST with `ReentrantWait`
//! (status `16`) instead of burning the timeout.
//!
//! Covered here (public surface): the same-thread direct path stays
//! wait-free - the owning JS thread invoking its OWN JS-bound entry
//! succeeds instantly, no marshal job, no park - and the
//! `ReentrantWait` -> `ReentrantCall` (16) status mapping. The nested
//! fast-fail itself drives the crate-internal wait-depth helpers and
//! lives in the lib unit tests (`src/callback/thread.rs`).
//!
//! Isolation (same as `callback-wait.rs`): this binary owns its whole
//! process; the JS-thread stand-in binds itself on first use.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use bffi::bffi_callback::{
    CallbackError, CallbackSig, Value, ValueType, bind_js_callback, invoke_wait,
};
use bffi::{BffiError, ErrorCode, set_js_thread};

type Job = Box<dyn FnOnce() + Send>;

/// `(i32) -> i32`: doubles the argument (a JSCallback stand-in).
extern "C" fn fake_js_double(x: i32) -> i32 {
    x.wrapping_mul(2)
}

/// The JS-thread stand-in: binds itself on first use and serves jobs
/// from a channel (a channel server, NOT `run()` - these tests pump
/// nothing; both paths under test are wait-free).
fn js_thread() -> &'static Sender<Job> {
    static JS_THREAD: std::sync::OnceLock<Sender<Job>> = std::sync::OnceLock::new();
    JS_THREAD.get_or_init(|| {
        let (sender, receiver) = channel::<Job>();
        thread::Builder::new()
            .name("bffi-js-reentrant".to_owned())
            .spawn(move || {
                set_js_thread().expect("the JS-thread stand-in must bind itself");
                for job in receiver {
                    job();
                }
            })
            .expect("spawning the JS-thread stand-in must not fail");
        sender
    })
}

/// Runs `f` on the JS-thread stand-in and waits for its result.
fn on_js_thread<R>(f: impl FnOnce() -> R + Send + 'static) -> R
where
    R: Send + 'static,
{
    let (reply_sender, reply_receiver) = channel();
    js_thread()
        .send(Box::new(move || {
            let _ = reply_sender.send(f());
        }))
        .expect("the JS-thread stand-in must be alive");
    reply_receiver
        .recv()
        .expect("the JS-thread stand-in must answer")
}

#[test]
fn invoke_wait_on_the_owning_js_thread_is_direct_and_succeeds() {
    // Bind ON the JS thread so the entry records ITS id, with a real
    // stand-in pointer so the direct call produces a value.
    let handle = on_js_thread(|| {
        bind_js_callback(
            CallbackSig::new(ValueType::I32, &[ValueType::I32]),
            fake_js_double as extern "C" fn(i32) -> i32 as usize,
        )
        .expect("bind ok")
    });

    // The calling thread IS the owning JS thread: the direct
    // synchronous path - no marshal job, no park, no wait-depth gate.
    // A deadlock here would burn the 30s budget; success must be
    // near-instant.
    let (result, elapsed) = on_js_thread(move || {
        let started = Instant::now();
        let result = invoke_wait(handle, &[Value::I32(21)], Duration::from_secs(30));
        (result, started.elapsed())
    });

    assert_eq!(result, Ok(Value::I32(42)));
    assert!(
        elapsed < Duration::from_secs(1),
        "the same-thread direct path must not park: {elapsed:?}"
    );
}

#[test]
fn reentrant_wait_maps_onto_the_reentrant_call_status() {
    let error = BffiError::from(CallbackError::ReentrantWait);
    assert_eq!(error.code, ErrorCode::ReentrantCall);
    assert_eq!(error.code.as_u32(), 16);
    assert!(
        error.source.is_some(),
        "the domain error must survive as the source"
    );
    assert_eq!(
        error.message,
        "re-entrant invoke_wait: the calling JS thread is inside a native call; the loop cannot drain - restructure so native work runs off the JS thread"
    );
}
