//! Integration tests for `bffi::callback::invoke_wait` (marshal-and-wait,
//! Phase 1): a native worker thread invokes a registered callback, the
//! call is marshalled through the event loop, and the JS-thread stand-in
//! delivers the outcome by pumping.
//!
//! Isolation strategy (same as `callback-threading.rs`): the JS-thread
//! binding is PROCESS-GLOBAL, so ONE dedicated helper thread binds
//! itself on first use and serves the pump jobs - a pump must run on
//! the BOUND thread for the job's inner `invoke` to pass the JS-thread
//! gate. The queue-touching tests share a static mutex: the queue is
//! process-global too, and a concurrent pump would execute another
//! test's marshal job (breaking the timeout test).
//!
//! The unbound-process delegation half of the contract lives in the
//! lib unit tests (`src/registry.rs`); the stopped-loop contract in
//! `callback-wait-shutdown.rs`, its own process (stop is sticky).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use bffi::bffi_callback::{
    CallbackError, CallbackSig, Value, ValueType, bind_js_callback, register, revoke,
};
use bffi::set_js_thread;

type Job = Box<dyn FnOnce() + Send>;

// --- JS-bound stand-ins -------------------------------------------------
//
// The JS-bound dispatch transmutes the bound pointer to the declared C
// shape; plain `extern "C"` fns stand in for real `bun:ffi` JSCallback
// pointers. They are called ON the JS-thread stand-in (inside the
// marshal job), exactly where the real dispatch runs.

/// `(i32) -> i32`: doubles the argument.
extern "C" fn fake_js_double(x: i32) -> i32 {
    x.wrapping_mul(2)
}

/// `(cstring) -> i32`: the string's UTF-8 length.
extern "C" fn fake_js_len(text: *const std::os::raw::c_char) -> i32 {
    if text.is_null() {
        return 0;
    }
    // SAFETY: the caller passes a NUL-terminated string that outlives
    // the call.
    unsafe { std::ffi::CStr::from_ptr(text) }.count_bytes() as i32
}

/// `() -> u8`: the `bun:ffi` spelling of a bool return.
extern "C" fn fake_js_ping() -> u8 {
    1
}

/// Serializes the queue-touching tests (see the module docs).
static QUEUE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The JS-thread stand-in: binds itself on first use and serves jobs
/// from a channel (a channel server, NOT `run()` - these tests pump).
fn js_thread() -> &'static Sender<Job> {
    static JS_THREAD: std::sync::OnceLock<Sender<Job>> = std::sync::OnceLock::new();
    JS_THREAD.get_or_init(|| {
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

/// Establishes the binding before any foreign-thread spawn: whichever
/// test runs first initializes the shared stand-in.
fn bind_js_thread() {
    on_js_thread(set_js_thread).unwrap();
}

/// Spawns a worker that runs `f` and reports completion through the
/// returned flag (the pump loop watches it).
fn spawn_worker<R>(
    f: impl FnOnce() -> R + Send + 'static,
) -> (thread::JoinHandle<R>, Arc<AtomicBool>)
where
    R: Send + 'static,
{
    let done = Arc::new(AtomicBool::new(false));
    let worker_done = Arc::clone(&done);
    let handle = thread::spawn(move || {
        let result = f();
        worker_done.store(true, Ordering::Release);
        result
    });
    (handle, done)
}

/// Pumps on the bound JS thread until the worker finished.
fn pump_until(done: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "the worker never woke up");
        on_js_thread(bffi::pump);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn invoke_wait_delivers_a_result_across_threads() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = register(
        CallbackSig::new(ValueType::I32, &[ValueType::I32, ValueType::I32]),
        Arc::new(|args: &[Value]| match args {
            [Value::I32(a), Value::I32(b)] => Value::I32(a + b),
            _ => unreachable!("invoke checks the signature"),
        }),
    )
    .expect("callback table has room");

    let (worker, done) = spawn_worker(move || {
        let started = Instant::now();
        let result = bffi::invoke_wait(
            handle,
            &[Value::I32(2), Value::I32(3)],
            Duration::from_secs(10),
        );
        (result, started.elapsed())
    });
    pump_until(&done);

    let (result, elapsed) = worker.join().unwrap();
    assert_eq!(result, Ok(Value::I32(5)));
    assert!(
        elapsed < Duration::from_secs(10),
        "the result must arrive within the timeout budget: {elapsed:?}"
    );
}

#[test]
fn invoke_wait_times_out_when_nobody_pumps() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = register(
        CallbackSig::new(ValueType::Bool, &[]),
        Arc::new(|_| Value::Bool(true)),
    )
    .expect("callback table has room");

    let started = Instant::now();
    let result = thread::spawn(move || bffi::invoke_wait(handle, &[], Duration::from_millis(150)))
        .join()
        .expect("worker must not panic");

    assert_eq!(result, Err(CallbackError::Timeout));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(140),
        "the timeout must not return early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the timeout must not hang: {elapsed:?}"
    );

    // The marshal job is still queued: drain it so the late fill lands
    // in the abandoned slot and the queue stays clean for other tests.
    on_js_thread(bffi::pump);
}

#[test]
fn invoke_wait_delivers_signature_errors_through_the_slot() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let expected = CallbackSig::new(ValueType::I32, &[ValueType::I32]);
    let handle =
        register(expected.clone(), Arc::new(|_| Value::I32(0))).expect("callback table has room");

    // Empty args against a one-parameter signature: the arity mismatch
    // is detected by the inner invoke ON the JS thread and must cross
    // the slot unchanged.
    let (worker, done) =
        spawn_worker(move || bffi::invoke_wait(handle, &[], Duration::from_secs(10)));
    pump_until(&done);

    let result = worker.join().unwrap();
    assert_eq!(
        result,
        Err(CallbackError::SignatureMismatch {
            expected,
            got: Vec::new(),
        })
    );
}

#[test]
fn invoke_wait_reports_revocation_during_the_wait() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = register(
        CallbackSig::new(ValueType::Bool, &[]),
        Arc::new(|_| Value::Bool(true)),
    )
    .expect("callback table has room");

    let (worker, done) =
        spawn_worker(move || bffi::invoke_wait(handle, &[], Duration::from_secs(10)));
    // Revoke AFTER the marshal job reached the queue (deterministic
    // order): the job still runs, the inner invoke reports the dead
    // handle, and the error crosses the slot.
    let deadline = Instant::now() + Duration::from_secs(10);
    while bffi::pending() == 0 {
        assert!(Instant::now() < deadline, "the marshal job never arrived");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(revoke(handle));
    pump_until(&done);

    let result = worker.join().unwrap();
    assert_eq!(result, Err(CallbackError::InvalidHandle(handle)));
}

#[test]
fn late_result_after_a_timeout_keeps_the_path_usable() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = register(
        CallbackSig::new(ValueType::I32, &[ValueType::I32]),
        Arc::new(|args: &[Value]| match args {
            [Value::I32(a)] => Value::I32(a * 2),
            _ => unreachable!("invoke checks the signature"),
        }),
    )
    .expect("callback table has room");

    // First call: nobody pumps, the timeout fires, the job stays queued.
    let timed_out = thread::spawn(move || {
        bffi::invoke_wait(handle, &[Value::I32(21)], Duration::from_millis(100))
    })
    .join()
    .expect("worker must not panic");
    assert_eq!(timed_out, Err(CallbackError::Timeout));

    // The late job now runs; its outcome lands in the abandoned slot.
    on_js_thread(bffi::pump);

    // A second call - with pumping this time - works normally.
    let (worker, done) =
        spawn_worker(move || bffi::invoke_wait(handle, &[Value::I32(21)], Duration::from_secs(10)));
    pump_until(&done);

    let result = worker.join().unwrap();
    assert_eq!(result, Ok(Value::I32(42)));
}

#[test]
fn invoke_wait_on_the_js_thread_delegates_without_a_marshal_job() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = register(
        CallbackSig::new(ValueType::Bool, &[]),
        Arc::new(|_| Value::Bool(true)),
    )
    .expect("callback table has room");

    // Called ON the bound thread: the direct path, even with a zero
    // timeout - nothing may be queued and nothing may time out.
    let result = on_js_thread(move || bffi::invoke_wait(handle, &[], Duration::ZERO));
    assert_eq!(result, Ok(Value::Bool(true)));
    assert_eq!(
        bffi::pending(),
        0,
        "no marshal job for the JS-thread caller"
    );
}

#[test]
fn invoke_wait_js_bound_delivers_a_result_across_threads() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    // The JS-bound handle of a `(i32) -> i32` JSCallback stand-in.
    let handle = bind_js_callback(
        CallbackSig::new(ValueType::I32, &[ValueType::I32]),
        fake_js_double as extern "C" fn(i32) -> i32 as usize,
    )
    .expect("callback table has room");

    let (worker, done) =
        spawn_worker(move || bffi::invoke_wait(handle, &[Value::I32(21)], Duration::from_secs(10)));
    pump_until(&done);

    let result = worker.join().unwrap();
    assert_eq!(result, Ok(Value::I32(42)));
}

#[test]
fn invoke_wait_js_bound_crosses_strings_as_cstrings() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = bind_js_callback(
        CallbackSig::new(ValueType::I32, &[ValueType::Str]),
        fake_js_len as extern "C" fn(*const std::os::raw::c_char) -> i32 as usize,
    )
    .expect("callback table has room");

    let (worker, done) = spawn_worker(move || {
        bffi::invoke_wait(
            handle,
            &[Value::Str("héllo".to_owned())],
            Duration::from_secs(10),
        )
    });
    pump_until(&done);

    let result = worker.join().unwrap();
    assert_eq!(result, Ok(Value::I32(6)));
}

#[test]
fn invoke_wait_js_bound_times_out_without_a_pump() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = bind_js_callback(
        CallbackSig::new(ValueType::Bool, &[]),
        fake_js_ping as extern "C" fn() -> u8 as usize,
    )
    .expect("callback table has room");
    let worker_handle = handle;

    let started = Instant::now();
    let result =
        thread::spawn(move || bffi::invoke_wait(worker_handle, &[], Duration::from_millis(150)))
            .join()
            .expect("worker must not panic");
    assert_eq!(result, Err(CallbackError::Timeout));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(140),
        "the timeout must not return early: {elapsed:?}"
    );

    // The marshal job is still queued; revoke BEFORE draining so the
    // late job never calls through the stand-in pointer.
    assert!(revoke(handle));
    on_js_thread(bffi::pump);
}

#[test]
fn invoke_wait_js_bound_reports_revocation_during_the_wait() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = bind_js_callback(
        CallbackSig::new(ValueType::Bool, &[]),
        fake_js_ping as extern "C" fn() -> u8 as usize,
    )
    .expect("callback table has room");

    let (worker, done) =
        spawn_worker(move || bffi::invoke_wait(handle, &[], Duration::from_secs(10)));
    // Revoke AFTER the marshal job reached the queue (deterministic
    // order): the job's fresh lookup reports the dead handle and the
    // stand-in pointer is never called.
    let deadline = Instant::now() + Duration::from_secs(10);
    while bffi::pending() == 0 {
        assert!(Instant::now() < deadline, "the marshal job never arrived");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(revoke(handle));
    pump_until(&done);

    let result = worker.join().unwrap();
    assert_eq!(result, Err(CallbackError::InvalidHandle(handle)));
}

#[test]
fn invoke_wait_js_bound_rejects_wrong_typing_without_a_queue() {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bind_js_thread();

    let handle = bind_js_callback(
        CallbackSig::new(ValueType::Bool, &[]),
        fake_js_ping as extern "C" fn() -> u8 as usize,
    )
    .expect("callback table has room");

    // The mismatch is detected on the CALLING thread: the error comes
    // back immediately (no marshal job, no timeout wait).
    let result = thread::spawn(move || {
        bffi::invoke_wait(handle, &[Value::I32(1)], Duration::from_millis(150))
    })
    .join()
    .expect("worker must not panic");
    assert_eq!(
        result,
        Err(CallbackError::SignatureMismatch {
            expected: CallbackSig::new(ValueType::Bool, &[]),
            got: vec![ValueType::I32],
        })
    );
    assert_eq!(
        bffi::pending(),
        0,
        "a wrong-typed JS-bound call must not be queued"
    );
}
