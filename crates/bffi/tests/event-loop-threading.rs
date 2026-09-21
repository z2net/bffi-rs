//! Integration tests: the event loop as the wrong-thread marshal path
//! for `bffi-callback` (criterion 5.3, P2 half).
//!
//! The JS thread is a dedicated helper thread (libtest gives every
//! `#[test]` its own thread) that binds itself with `set_js_thread`
//! and then lives inside `run()`. `stop()` is never called here - it
//! is global and sticky, and the helper must stay usable for every
//! test in the binary; the parked thread ends with the process.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bffi::bffi_callback::{CallbackSig, Value, ValueType, invoke, register, set_js_thread};
use bffi::bffi_event_loop::marshal;

type Direct = Box<dyn FnOnce() + Send>;

/// Executed-job total of the helper runner, kept so the `#[must_use]`
/// return of `run()` lands somewhere observable.
static HELPER_EXECUTED: AtomicU64 = AtomicU64::new(0);

fn js_thread() -> &'static std::sync::mpsc::Sender<Direct> {
    static JS_THREAD: std::sync::OnceLock<std::sync::mpsc::Sender<Direct>> =
        std::sync::OnceLock::new();
    JS_THREAD.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Direct>();
        std::thread::Builder::new()
            .name("bffi-js-thread".into())
            .spawn(move || {
                set_js_thread().expect("the helper thread binds first in this binary");
                // The helper never stops (stop is global and sticky);
                // its executed count lands in HELPER_EXECUTED.
                let executed = bffi::bffi_event_loop::run();
                HELPER_EXECUTED.fetch_add(executed, Ordering::Relaxed);
            })
            .expect("helper thread spawns");
        // Serve direct (non-loop) jobs too, so tests can run code on
        // the bound thread without a marshal round-trip.
        std::thread::spawn(move || {
            for job in rx {
                job();
            }
        });
        // Wait until the runner is inside run(): marshal() checks
        // is_running(), and the spawn above does not synchronize with
        // the runner's entry.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !bffi::bffi_event_loop::is_running() {
            assert!(
                std::time::Instant::now() < deadline,
                "the helper runner never started"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        tx
    })
}

/// Sends `job` through the marshal path and waits for its reply.
fn on_loop<R: Send + 'static>(make: impl FnOnce() -> R + Send + 'static) -> R {
    let (tx, rx) = std::sync::mpsc::channel();
    marshal(Box::new(move || {
        let value = make();
        tx.send(value).expect("receiver alive");
    }))
    .expect("the helper runner is active");
    rx.recv_timeout(Duration::from_secs(10))
        .expect("the loop executed the job")
}

#[test]
fn marshal_delivers_invoke_onto_the_js_thread() {
    // Make sure the helper (and therefore the runner) exists first:
    // libtest starts both tests in parallel, and this test must not
    // depend on the other one winning the initialization race.
    js_thread();

    let handle = register(
        CallbackSig::new(ValueType::I32, &[ValueType::I32]),
        Arc::new(|args: &[Value]| {
            let Value::I32(x) = args[0] else {
                unreachable!("signature-checked")
            };
            Value::I32(x * 2)
        }),
    )
    .expect("callback table has room");

    let doubled = on_loop(move || invoke(handle, &[Value::I32(21)]))
        .expect("the callback must pass every gate on the JS thread");
    assert_eq!(doubled, Value::I32(42));

    // Revoke, then marshal an invoke of the dead handle: the loop
    // executes it, but the callback itself is gone (criterion 5.2).
    assert!(bffi::bffi_callback::revoke(handle));
    let dead = on_loop(move || invoke(handle, &[Value::I32(1)]));
    assert_eq!(
        dead,
        Err(bffi::bffi_callback::CallbackError::InvalidHandle(handle))
    );
}

#[test]
fn wrong_thread_invoke_outside_the_loop_maps_to_code_12() {
    // Make sure the helper (and therefore the binding) exists first:
    // whichever test runs first creates it.
    js_thread();

    let handle = register(
        CallbackSig::new(ValueType::Bool, &[]),
        Arc::new(|_| Value::Bool(true)),
    )
    .expect("callback table has room");

    let invoked = std::thread::spawn(move || invoke(handle, &[]))
        .join()
        .expect("spawned thread must not panic");
    assert_eq!(
        invoked,
        Err(bffi::bffi_callback::CallbackError::WrongThread)
    );

    let converted = bffi::bffi_core::BffiError::from(invoked.unwrap_err());
    assert_eq!(converted.code, bffi::bffi_core::ErrorCode::WrongThread);
    assert_eq!(converted.message, "callback invoked from a non-JS thread");
}

/// Regression coverage for the lost-wakeup window in the registered
/// runner's park: an untargeted enqueue landing between the runner's
/// global drain and its condvar wait notifies a condvar nobody waits
/// on yet, and the job sat on the global queue until the next
/// arrival. Each round-trip forces the runner through the
/// drain-then-park cycle, so a regression turns into a timeout here
/// (best-effort: the window is a few instructions wide, and the
/// wall-clock bound keeps a starved CI runner honest).
#[test]
fn marshal_round_trips_survive_the_park_window() {
    js_thread();

    for round in 0..300 {
        let (tx, rx) = std::sync::mpsc::channel();
        marshal(Box::new(move || {
            tx.send(round).expect("receiver alive");
        }))
        .expect("the helper runner is active");
        let delivered = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the loop executed every round-trip job");
        assert_eq!(delivered, round);
    }
}
