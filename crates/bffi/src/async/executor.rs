//! The N-worker executor: polls spawned futures on background
//! threads, never touching JavaScript.
//!
//! Threading contract (the Bun-support invariant): workers only poll
//! futures and record outcomes; delivering an outcome to JS happens
//! through an event-loop job, which the JS thread executes while
//! draining (`pump()` / `run()`).

// Internal module aliases (the pre-merge crate names).
use crate::bffi_build;
use crate::bffi_core;
use crate::bffi_event_loop;
use crate::bffi_types;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

use bffi_core::{ErrorCode, Handle, panic_message};

use super::task::{self, Outcome, TaskRecord};
use super::value::AsyncValue;

/// A spawned future as the executor stores it: the unified output
/// keeps every codegen path identical (`Result` errors and caught
/// panics both become `Err`).
pub(crate) type BoxFuture =
    Pin<Box<dyn Future<Output = Result<AsyncValue, bffi_core::BffiError>> + Send + 'static>>;

/// Hard cap on worker threads; the default is the machine parallelism
/// clamped into `1..=MAX_WORKERS`.
const MAX_WORKERS: usize = 4;

struct Executor {
    ready: Mutex<VecDeque<Handle>>,
    signal: Condvar,
    parked: Mutex<HashMap<Handle, BoxFuture>>,
    live_tasks: AtomicUsize,
}

fn executor() -> &'static Executor {
    static EXECUTOR: OnceLock<Executor> = OnceLock::new();
    EXECUTOR.get_or_init(|| {
        let executor = Executor {
            ready: Mutex::new(VecDeque::new()),
            signal: Condvar::new(),
            parked: Mutex::new(HashMap::new()),
            live_tasks: AtomicUsize::new(0),
        };
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, MAX_WORKERS);
        for index in 0..workers {
            let _unused = std::thread::Builder::new()
                .name(format!("bffi-async-worker-{index}"))
                .spawn(worker_loop);
        }
        executor
    })
}

/// Wakes the executor when the future's own machinery (a channel, a
/// timer, a socket) decides the task is ready to poll again.
struct TaskWake(Handle);

impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        schedule(self.0);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        schedule(self.0);
    }
}

fn schedule(handle: Handle) {
    let executor = executor();
    executor
        .ready
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push_back(handle);
    executor.signal.notify_one();
}

/// Removes a parked future without polling it (cancellation): the
/// `drop` that follows is the cancellation.
fn drop_parked(handle: Handle) {
    let _dropped = executor()
        .parked
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&handle);
}

pub(crate) fn live_tasks() -> u64 {
    executor().live_tasks.load(Ordering::Acquire) as u64
}

/// Registers and schedules a task; returns its handle.
pub(crate) fn launch(future: BoxFuture) -> Result<Handle, bffi_core::RegistryError> {
    let handle = task::register()?;
    executor();
    executor().live_tasks.fetch_add(1, Ordering::AcqRel);
    executor()
        .parked
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(handle, future);
    schedule(handle);
    Ok(handle)
}

fn worker_loop() {
    loop {
        let handle = {
            let executor = executor();
            let mut ready = executor
                .ready
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if let Some(handle) = ready.pop_front() {
                    break handle;
                }
                ready = executor
                    .signal
                    .wait(ready)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        poll_task(handle);
    }
}

/// Takes the parked future (if it is still parked and not cancelled),
/// polls it once, and records the outcome.
fn poll_task(handle: Handle) {
    let Some(record) = task::record(handle) else {
        drop_parked(handle);
        return;
    };
    if record.cancel_requested() {
        // `cancel` won the transition and already delivered; the only
        // thing left here is dropping the future.
        drop_parked(handle);
        return;
    }
    let Some(mut future) = executor()
        .parked
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&handle)
    else {
        return; // not parked: cancelled concurrently, future already dropped
    };

    let waker = Waker::from(Arc::new(TaskWake(handle)));
    let mut context = Context::from_waker(&waker);
    // SAFETY: the future is never moved after this pin; only polled
    // in place or dropped.
    let poll = catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(&mut context)));

    match poll {
        Err(payload) => {
            // SAFETY: the Box payload must be dereferenced explicitly
            // so `panic_message` sees the payload object.
            let error = bffi_core::BffiError::new(ErrorCode::Panic, panic_message(&*payload));
            if record.mark_failed(error) {
                task_finished(handle);
                try_deliver(&record);
            }
        }
        Ok(Poll::Ready(Ok(value))) => {
            if record.mark_completed(value) {
                task_finished(handle);
                try_deliver(&record);
            }
        }
        Ok(Poll::Ready(Err(error))) => {
            if record.mark_failed(error) {
                task_finished(handle);
                try_deliver(&record);
            }
        }
        Ok(Poll::Pending) => {
            if record.cancel_requested() {
                // Cancellation arrived while the future was parked
                // inside a poll; dropping it here is the cancel.
                drop(future);
                if record.mark_cancelled() {
                    task_finished(handle);
                    try_deliver(&record);
                }
            } else {
                executor()
                    .parked
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(handle, future);
            }
        }
    }
}

/// Terminal bookkeeping for the transition winner: the live count
/// drops, any parked future is dropped and any registered aborter is
/// discarded. Callers that lose the transition must NOT call this
/// (the winner already did).
pub(crate) fn task_finished(handle: Handle) {
    executor().live_tasks.fetch_sub(1, Ordering::AcqRel);
    drop_parked(handle);
    take_aborter(handle);
}
/// The registered aborters of tasks running on foreign executors
/// (feature `tokio`): `cancel` runs the closure to abort the tokio
/// task immediately.
type Aborter = Box<dyn FnOnce() + Send>;
type Aborters = Mutex<HashMap<Handle, Aborter>>;

static ABORTERS: OnceLock<Aborters> = OnceLock::new();

fn aborters() -> &'static Aborters {
    ABORTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers an aborter closure for `handle` (foreign-executor tasks).
// Used only by the gated runtime module; dead in a default build.
#[cfg_attr(not(feature = "tokio"), allow(dead_code))]
pub(crate) fn register_aborter(handle: Handle, abort: Aborter) {
    aborters()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(handle, abort);
}

/// Takes the aborter out (without running it).
fn take_aborter(handle: Handle) -> Option<Aborter> {
    aborters()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&handle)
}

/// Runs and removes the aborter for `handle`, if any.
#[cfg_attr(not(feature = "tokio"), allow(dead_code))]
pub(crate) fn run_aborter(handle: Handle) {
    if let Some(abort) = take_aborter(handle) {
        abort();
    }
}

/// The live-count increment for a freshly spawned task.
#[cfg_attr(not(feature = "tokio"), allow(dead_code))]
pub(crate) fn task_started() {
    executor().live_tasks.fetch_add(1, Ordering::AcqRel);
}

/// Enqueues the resolve/reject delivery when the task has an attached
/// resolver pair and its outcome was not delivered yet.
///
/// TARGETED delivery: the resolvers belong to the isolate that
/// attached them, so the job is queued on that thread's slot queue
/// (a departed worker leaves the delivery undeliverable - the
/// documented "do not stop a worker with live tasks" contract).
/// Pairs attached while the process was unbound keep the legacy
/// untargeted delivery.
pub(crate) fn try_deliver(record: &TaskRecord) {
    // Order matters: the delivery slot is taken ONLY when there is a
    // terminal outcome - an attach-time attempt on a running task
    // must not consume it.
    let Some(resolvers) = record.resolver_pair() else {
        return;
    };
    let Some(outcome) = record.outcome_snapshot() else {
        return;
    };
    if !record.take_delivery_slot() {
        return;
    }
    let job = Box::new(move || {
        deliver_on_js_thread(resolvers.resolve, resolvers.reject, outcome);
    });
    let queued = if resolvers.thread == 0 {
        bffi_event_loop::enqueue(job)
    } else {
        bffi_event_loop::enqueue_to(resolvers.thread, job)
    };
    if queued.is_err() {
        // The event loop was stopped, or the owning isolate is gone:
        // the delivery is undeliverable (documented contract - do not
        // stop the loop or a worker with live tasks).
    }
}

/// Runs on the JS thread (event-loop drain): materializes the value
/// into the transient-buffer table and invokes the attached resolver
/// trampolines.
fn deliver_on_js_thread(resolve: usize, reject: usize, outcome: Outcome) {
    match outcome {
        Outcome::Done(value) => {
            let payload = value.encode();
            let Ok(buffer) =
                bffi_build::runtime::store_bytes(bffi_types::CopiedBuf::from_vec(payload))
            else {
                // The buffer table is full: nothing to resolve with.
                reject_with(reject, "task result storage is full");
                return;
            };
            // SAFETY: `resolve` is a bun:ffi JSCallback trampoline
            // declared as `(u64) -> void`; this job runs on the JS
            // thread, so calling it is legal.
            unsafe { resolve_by_ptr(resolve, buffer.as_u64()) };
        }
        Outcome::Failed { message, .. } => reject_with(reject, &message),
        Outcome::Cancelled => reject_with(reject, "task cancelled"),
    }
}

fn reject_with(reject: usize, message: &str) {
    let Ok(cstring) = std::ffi::CString::new(message) else {
        return; // interior NUL in a message is impossible today
    };
    // SAFETY: `reject` is a bun:ffi JSCallback trampoline declared as
    // `(cstring) -> void`; this job runs on the JS thread and the
    // CString outlives the call.
    unsafe { reject_by_ptr(reject, cstring.as_ptr()) };
}

// SAFETY: the caller guarantees `ptr` is a bun:ffi JSCallback
// trampoline with the declared `(u64) -> void` signature and that the
// call happens on the JS thread.
unsafe fn resolve_by_ptr(ptr: usize, value_handle: u64) {
    // SAFETY: any usize is a valid u64; the pointer validity contract
    // is documented above.
    let resolve: extern "C" fn(u64) = unsafe { std::mem::transmute(ptr) };
    resolve(value_handle);
}

// SAFETY: the caller guarantees `ptr` is a bun:ffi JSCallback
// trampoline with the declared `(cstring) -> void` signature, that the
// call happens on the JS thread, and that the cstring outlives the
// call.
unsafe fn reject_by_ptr(ptr: usize, message: *const std::os::raw::c_char) {
    // SAFETY: any usize is a valid usize; the pointer validity
    // contract is documented above.
    let reject: extern "C" fn(*const std::os::raw::c_char) = unsafe { std::mem::transmute(ptr) };
    reject(message);
}
