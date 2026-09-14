//! Integration tests for the process-global JS-thread table.
//!
//! The registration is PROCESS-GLOBAL state: every JS isolate binds
//! its own thread with `set_js_thread` (the multi-isolate policy),
//! repeats from a bound thread are idempotent, and a thread that is
//! NOT registered is rejected with `CallbackError::WrongThread` -
//! including when it merely probes with `ensure_js_thread`. A binding
//! attempt from a foreign thread is no longer a rejection: it
//! REGISTERS that thread (that is how a second Worker joins), and the
//! registration dies with the thread.
//!
//! Isolation strategy: libtest runs every `#[test]` on its own thread,
//! so a test body cannot bind "the main test thread" - two tests would
//! bind two different harness threads and each would reject the other
//! (observed empirically before this harness was adopted). This binary
//! therefore funnels every call that must observe or establish the
//! binding through ONE dedicated helper thread - the test stand-in for
//! the JS thread, bound on first use - while foreign-thread assertions
//! run on plain spawned threads. Tests stay
//! order-independent and parallel-safe: whichever test runs first
//! establishes the binding through the helper, and every foreign-thread
//! spawn happens only after a completed helper
//! call, so the binding always exists by then.
//!
//! The "unbound -> Ok" half of the contract is NOT covered here (the
//! binding may already exist by the time these tests run); it is
//! asserted by the unit tests in `src/callback/thread.rs`, whose
//! spawned registrars deregister themselves when they end.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::thread;

use bffi::bffi_callback::{CallbackError, ensure_js_thread, set_js_thread};

type Job = Box<dyn FnOnce() + Send>;

/// Runs `f` on the shared JS-thread stand-in and waits for its result.
fn on_js_thread<R>(f: impl FnOnce() -> R + Send + 'static) -> R
where
    R: Send + 'static,
{
    static JS_THREAD: OnceLock<Sender<Job>> = OnceLock::new();
    let sender = JS_THREAD.get_or_init(|| {
        let (sender, receiver) = channel::<Job>();
        thread::Builder::new()
            .name("bffi-js-thread".to_owned())
            .spawn(move || {
                set_js_thread().expect("the JS-thread stand-in must bind itself");
                for job in receiver {
                    job();
                }
            })
            .expect("spawning the JS-thread stand-in must not fail");
        sender
    });
    let (reply_sender, reply_receiver) = channel();
    sender
        .send(Box::new(move || {
            let _ = reply_sender.send(f());
        }))
        .expect("the JS-thread stand-in must be alive");
    reply_receiver
        .recv()
        .expect("the JS-thread stand-in must answer")
}

#[test]
fn set_js_thread_is_idempotent_on_the_bound_thread() {
    on_js_thread(set_js_thread).unwrap();
    on_js_thread(set_js_thread).unwrap();
    on_js_thread(ensure_js_thread).unwrap();
}

#[test]
fn foreign_thread_is_rejected() {
    // Establish the binding first (blocking): from here on, foreign
    // threads must observe a BOUND process, never an unbound one.
    on_js_thread(set_js_thread).unwrap();

    // Invocation from a foreign thread must be rejected...
    let invoked = thread::spawn(ensure_js_thread).join().unwrap();
    assert_eq!(invoked.err(), Some(CallbackError::WrongThread));

    // ...while a binding ATTEMPT from a foreign thread is ACCEPTED:
    // it registers that thread (the multi-isolate policy - this is
    // how a second Worker joins). The registration lives exactly as
    // long as the registering thread.
    let (release_sender, release_receiver) = channel::<()>();
    let registrar = thread::spawn(move || {
        let bound = set_js_thread();
        let _ = release_receiver.recv();
        bound
    });
    // Let the registrar bind, then release it and collect its result.
    thread::sleep(std::time::Duration::from_millis(20));
    drop(release_sender);
    let rebound = registrar.join().unwrap();
    assert_eq!(rebound.ok(), Some(()));

    // The first binding is unchanged: the bound thread still admits
    // itself, and the registrar is gone (deregistered by its TLS
    // guard).
    on_js_thread(ensure_js_thread).unwrap();
    let after = thread::spawn(ensure_js_thread).join().unwrap();
    assert_eq!(after.err(), Some(CallbackError::WrongThread));
}
