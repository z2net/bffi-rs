#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]
//! # bffi-event-loop
//!
//! The event loop abstraction of the bffi-rs framework (DESIGN §7:
//! `enqueue`/`marshal` queue; blocking `run()`, non-blocking `pump()`).
//!
//! Native code cannot hook Bun's real event loop, so this crate is the
//! honest next thing: thread-safe job queues plus two drains -
//!
//! - background threads deliver work with [`enqueue`] / [`marshal`]
//!   (untargeted: any JS thread may run it) or [`enqueue_to`]
//!   (targeted: exactly one registered JS isolate);
//! - one thread per JS isolate (registered with
//!   `bffi::callback::set_js_thread()`) drains its OWN queue with
//!   [`run`] / [`pump`] - every job executes under
//!   [`bffi::core::run_extern_body`], so a panicking job becomes a
//!   stored last error and the loop lives on;
//! - [`stop`] ends the (sticky) loop for the whole process;
//!   [`pump`] drains whatever is queued without waiting - the
//!   Bun-tick integration that calls it periodically lives in the JS
//!   loader, not here.
//!
//! ## Queue model
//!
//! Two kinds of queues serve one contract:
//!
//! - the **global queue** receives untargeted jobs - pure-Rust work
//!   any registered JS thread may run, plus the legacy single-isolate
//!   path (an unbound process drains only this queue);
//! - a **per-thread slot queue** exists per registered JS thread and
//!   receives targeted jobs - deliveries whose target is one specific
//!   isolate (its `JSCallback` trampolines): JS-bound callback waits,
//!   stream wakes, async resolvers. A job never crosses an isolate
//!   boundary; targeting an unregistered thread (a terminated worker)
//!   fails with [`EventLoopError::NotRunning`].
//!
//! A registered runner drains its slot queue and then the global
//! queue; an unregistered runner drains only the global queue (the
//! legacy behavior, unchanged for pure-Rust usage and tests). All
//! wakeup sources notify every slot condvar, so a registered runner
//! never misses an untargeted arrival.
//!
//! The queues are `Mutex<VecDeque>` + `Condvar` on purpose: the
//! lock-free machinery of P0 exists for handle tables (CAS traffic on
//! every FFI call); a job queue sees a handful of transitions per job
//! and does not need it (documented trade-off, see the README).
//!
//! ## Example
//!
//! ```
//! use std::sync::atomic::{AtomicU32, Ordering};
//!
//! static RESULT: AtomicU32 = AtomicU32::new(0);
//!
//! bffi::enqueue(Box::new(|| {
//!     RESULT.store(41 + 1, Ordering::Relaxed);
//! }))
//! .expect("not stopped");
//! // No runner exists yet: the job is definitely still queued.
//! assert_eq!(bffi::pending(), 1);
//!
//! let handle = std::thread::spawn(bffi::run);
//! // Wait until the runner actually picked the job up (stop() is
//! // sticky - calling it before `run()` starts would skip the job).
//! while RESULT.load(Ordering::Relaxed) == 0 {
//!     std::thread::yield_now();
//! }
//! bffi::stop();
//! assert_eq!(handle.join().ok(), Some(1));
//! assert_eq!(RESULT.load(Ordering::Relaxed), 42);
//! assert!(bffi::enqueue(Box::new(|| {})).is_err());
//! ```

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
// The workspace restriction lints (expect/unwrap/panic) target production
// code; tests assert invariants and intentionally trigger panics.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// A unit of work delivered to the loop; executed on the thread that
/// runs [`run`].
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// Everything that can go wrong at the event-loop surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventLoopError {
    /// No runner is active, so the job could not be delivered to the
    /// JS thread. Maps to `ErrorCode::WrongThread` - the caller asked
    /// for marshalling and there was no one to marshal to. For
    /// [`enqueue_to`] this also covers a target isolate whose thread
    /// is gone (a terminated worker).
    NotRunning,
    /// The loop has been stopped; it is sticky and never restarts.
    /// Maps to `ErrorCode::Error`.
    Stopped,
}

impl fmt::Display for EventLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => write!(f, "no event loop is running to marshal onto"),
            Self::Stopped => write!(f, "the event loop has been stopped"),
        }
    }
}

impl std::error::Error for EventLoopError {}

/// Unified-format conversion: a marshal attempt without a runner maps
/// to the dedicated `WrongThread` code (P2), a stop to `Error`; the
/// domain error is preserved as the source.
impl From<EventLoopError> for bffi_core::BffiError {
    fn from(error: EventLoopError) -> Self {
        use bffi_core::{BffiError, ErrorCode};
        let code = match &error {
            EventLoopError::NotRunning => ErrorCode::WrongThread,
            EventLoopError::Stopped => ErrorCode::Error,
        };
        BffiError::with_source(code, error.to_string(), error)
    }
}

/// One waitable job queue.
type WaitQueue = (Mutex<VecDeque<Job>>, Condvar);

static STOPPED: AtomicBool = AtomicBool::new(false);
static RUNNERS: AtomicUsize = AtomicUsize::new(0);
static EXECUTED: AtomicU64 = AtomicU64::new(0);

fn queue() -> &'static WaitQueue {
    static QUEUE: std::sync::OnceLock<WaitQueue> = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| (Mutex::new(VecDeque::new()), Condvar::new()))
}

/// The per-JS-thread slot queues, keyed by the callback layer's
/// thread ids. Registration events come from [`js_thread_registered`]
/// / [`js_thread_gone`] - the callback layer's TLS guard drives them,
/// so a terminated worker's queue is retired with its thread.
fn thread_queues() -> MutexGuard<'static, HashMap<u64, Arc<WaitQueue>>> {
    static THREAD_QUEUES: std::sync::OnceLock<Mutex<HashMap<u64, Arc<WaitQueue>>>> =
        std::sync::OnceLock::new();
    THREAD_QUEUES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Wakes every registered slot runner: an untargeted delivery or a
/// stop concerns all of them (they drain the global queue too).
fn notify_slots() {
    for slot in thread_queues().values() {
        slot.1.notify_all();
    }
}

/// Opens a slot queue for the newly registered JS thread `id`.
pub(crate) fn js_thread_registered(id: u64) {
    thread_queues().insert(id, Arc::new((Mutex::new(VecDeque::new()), Condvar::new())));
}

/// Retires the slot queue of a deregistered JS thread `id`. Jobs
/// still queued for the isolate drop with the slot - their waiters
/// observe the documented timeout path.
pub(crate) fn js_thread_gone(id: u64) {
    thread_queues().remove(&id);
}

/// The calling thread's slot queue, if it is a registered JS thread.
fn own_slot() -> Option<Arc<WaitQueue>> {
    let id = crate::bffi_callback::current_thread_id();
    if crate::bffi_callback::is_js_thread(id) {
        thread_queues().get(&id).cloned()
    } else {
        None
    }
}

/// Pops one job from `q` - the lock guard drops within the call.
fn pop(q: &WaitQueue) -> Option<Job> {
    q.0.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
}

/// Puts `job` on the global queue for any runner (untargeted).
///
/// Deliberately infallible w.r.t. capacity (the queue is unbounded)
/// and callable from any thread.
///
/// # Errors
///
/// [`EventLoopError::Stopped`] once [`stop`] has been called (sticky).
pub fn enqueue(job: Job) -> Result<(), EventLoopError> {
    if STOPPED.load(Ordering::Acquire) {
        return Err(EventLoopError::Stopped);
    }
    queue()
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(job);
    queue().1.notify_one();
    notify_slots();
    Ok(())
}

/// Puts `job` on the slot queue of the JS thread `id` - targeted
/// delivery for work that may only run in one isolate (its
/// `JSCallback` trampolines).
///
/// # Errors
///
/// [`EventLoopError::Stopped`] once [`stop`] has been called (sticky);
/// [`EventLoopError::NotRunning`] when `id` is not a registered JS
/// thread (never registered, or its worker already terminated).
pub fn enqueue_to(id: u64, job: Job) -> Result<(), EventLoopError> {
    if STOPPED.load(Ordering::Acquire) {
        return Err(EventLoopError::Stopped);
    }
    let slot = thread_queues().get(&id).cloned();
    let Some(slot) = slot else {
        return Err(EventLoopError::NotRunning);
    };
    slot.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(job);
    slot.1.notify_one();
    Ok(())
}

/// Marshals `job` onto the running loop: the wrong-thread delivery
/// path (criterion 5.3, P2 half - the P1 half was the reject inside
/// `bffi::callback::invoke`).
///
/// # Errors
///
/// [`EventLoopError::NotRunning`] when no runner is active - the
/// caller decides how to surface it (retry, reject, log); the
/// conversion to [`bffi::core::BffiError`] maps it to
/// `ErrorCode::WrongThread`. [`EventLoopError::Stopped`] like
/// [`enqueue`].
pub fn marshal(job: Job) -> Result<(), EventLoopError> {
    if RUNNERS.load(Ordering::Acquire) == 0 {
        return Err(EventLoopError::NotRunning);
    }
    enqueue(job)
}

/// Runs every queued job of one queue through the boundary policy.
/// Returns the number of jobs executed by THIS call.
fn drain(q: &WaitQueue) -> u64 {
    let mut executed = 0_u64;
    while let Some(job) = pop(q) {
        executed += 1;
        EXECUTED.fetch_add(1, Ordering::Relaxed);
        let _ = bffi_core::boundary::run_extern_body(|| {
            job();
            bffi_core::ErrorCode::Ok
        });
    }
    executed
}

/// Blocks the calling thread and drains the queues until [`stop`] is
/// called. Every job executes under [`bffi_core::run_extern_body`]: a
/// panic becomes a stored last error and the loop continues.
///
/// A registered JS thread (see `bffi::callback::set_js_thread`)
/// drains its OWN slot queue first and the global queue second, and
/// waits for new work on its slot condvar - it is the executor of its
/// isolate. An unregistered thread keeps the legacy behavior: it
/// drains only the global queue; the first unregistered runner waits
/// for [`stop`], additional ones drain and exit.
///
/// Nested/parallel `run` calls are allowed but discouraged: additional
/// runners only drain the queues and exit once they are empty, while
/// a primary runner keeps waiting for `stop`. `run` after `stop`
/// returns `0` immediately (stop is sticky). The return value is the
/// number of jobs executed by THIS runner (panicking jobs included).
#[must_use]
pub fn run() -> u64 {
    if STOPPED.load(Ordering::Acquire) {
        return 0;
    }
    let own = own_slot();
    RUNNERS.fetch_add(1, Ordering::AcqRel);
    let primary = RUNNERS.load(Ordering::Acquire) == 1;
    let mut executed = 0_u64;
    match own {
        // Registered executor of one isolate.
        Some(slot) => loop {
            executed += drain(&slot);
            executed += drain(queue());
            if STOPPED.load(Ordering::Acquire) {
                break;
            }
            // Wait for the next arrival under the slot mutex (all
            // wakeup sources notify this condvar - targeted jobs,
            // untargeted jobs, stop). The emptiness check and the
            // wait share the lock, so an arrival in between cannot be
            // missed.
            let mut queued = slot.0.lock().unwrap_or_else(PoisonError::into_inner);
            if queued.is_empty() {
                queued = slot.1.wait(queued).unwrap_or_else(PoisonError::into_inner);
            }
        },
        // Unregistered: the legacy single-queue loop.
        None => loop {
            let job = {
                let mut q = queue().0.lock().unwrap_or_else(PoisonError::into_inner);
                loop {
                    if let Some(job) = q.pop_front() {
                        break Some(job);
                    }
                    if STOPPED.load(Ordering::Acquire) {
                        break None;
                    }
                    // Additional runners are drain-only: they exit on an
                    // empty queue instead of waiting forever.
                    if !primary {
                        break None;
                    }
                    q = queue().1.wait(q).unwrap_or_else(PoisonError::into_inner);
                }
            };
            let Some(job) = job else { break };
            executed += 1;
            EXECUTED.fetch_add(1, Ordering::Relaxed);
            let _ = bffi_core::boundary::run_extern_body(|| {
                job();
                bffi_core::ErrorCode::Ok
            });
        },
    }
    RUNNERS.fetch_sub(1, Ordering::AcqRel);
    executed
}

/// Stops the loop for good: wakes every runner (the current job
/// finishes, queued jobs stay queued), makes [`enqueue`] fail with
/// [`EventLoopError::Stopped`] and turns [`run`] into a no-op.
/// Idempotent.
///
/// The STOPPED flag is an atomic, but the NOTIFICATION must be
/// serialized against each waiter's "check STOPPED -> enter wait"
/// critical section: a runner holds the queue (or slot) mutex
/// between checking the flag and blocking on the condvar, so stop()
/// acquires the same mutexes before notifying. Without this, a stop
/// landing in that window is LOST and the runner sleeps forever
/// (the lost-wakeup race the event-loop doctest intermittently
/// caught).
pub fn stop() {
    STOPPED.store(true, Ordering::Release);
    {
        let _held = queue().0.lock().unwrap_or_else(PoisonError::into_inner);
        queue().1.notify_all();
    }
    for slot in thread_queues().values() {
        let _held = slot.0.lock().unwrap_or_else(PoisonError::into_inner);
        slot.1.notify_all();
    }
}

/// Number of jobs waiting for execution across the global queue and
/// every slot queue (a monotonic snapshot, for monitoring and tests).
#[must_use]
pub fn pending() -> u64 {
    let mut total = queue()
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len() as u64;
    for slot in thread_queues().values() {
        total += slot.0.lock().unwrap_or_else(PoisonError::into_inner).len() as u64;
    }
    total
}

/// Number of jobs executed since process start, across all runners
/// (for monitoring and tests).
#[must_use]
pub fn executed_total() -> u64 {
    EXECUTED.load(Ordering::Relaxed)
}

/// Whether at least one runner is inside [`run`].
#[must_use]
pub fn is_running() -> bool {
    RUNNERS.load(Ordering::Acquire) > 0
}

/// Non-blocking drain: pops and executes every queued job through
/// [`bffi::core::boundary::run_extern_body`], exactly like [`run`]
/// does (the `EXECUTED` counter advances; a panicking job becomes a
/// stored last error and the drain continues), and stops once the
/// queues are empty. It never touches a condvar - it does not wait
/// and it does not wake runners - so it is safe from any thread and
/// concurrently with a blocking [`run`]: the queue mutexes serialize
/// pops, and only the popper runs the job it popped, so every job
/// executes exactly once.
///
/// A registered JS thread drains its OWN slot queue first, then the
/// global queue; an unregistered thread drains only the global queue
/// (the legacy behavior). Returns the number of jobs executed by
/// THIS call. The Bun-tick integration (calling `pump` periodically
/// from a JS loader) is loader-side work, not native.
#[must_use]
pub fn pump() -> u64 {
    let own = own_slot();
    let mut executed = match &own {
        Some(slot) => drain(slot),
        None => 0,
    };
    executed += drain(queue());
    executed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the queue-touching unit tests: libtest runs them in
    /// parallel, and `pending` / `pump` observe the process-wide
    /// queues, so concurrent enqueue-drain cycles would race.
    static QUEUE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn pump_drains_the_queue_without_blocking() {
        let _guard = QUEUE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

        // Phase 1: three queued jobs, one non-blocking drain.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        for _ in 0..3 {
            enqueue(Box::new(|| {
                COUNTER.fetch_add(1, Ordering::Relaxed);
            }))
            .expect("unit tests never stop the loop");
        }
        assert_eq!(pending(), 3);
        assert_eq!(pump(), 3);
        assert_eq!(COUNTER.load(Ordering::Relaxed), 3);
        assert_eq!(pending(), 0);

        // Phase 2: an empty queue drains nothing.
        assert_eq!(pump(), 0);

        // Phase 3: a panicking job becomes a stored last error and
        // the drain continues with the next job.
        enqueue(Box::new(|| panic!("pump boom"))).expect("unit tests never stop the loop");
        enqueue(Box::new(|| {})).expect("unit tests never stop the loop");
        assert_eq!(pump(), 2, "the panic is contained, the drain continues");
        assert_eq!(pending(), 0, "the loop state stays healthy");
        // pump ran the jobs on THIS thread, so the last error is in
        // this thread's slot.
        let error = bffi_core::take_last_error().expect("the panic must be stored");
        assert_eq!(error.code, bffi_core::ErrorCode::Panic);
        assert_eq!(error.message, "pump boom");
    }

    #[test]
    fn pump_and_pending_report_an_empty_queue() {
        let _guard = QUEUE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(pump(), 0);
        assert_eq!(pending(), 0);
    }

    #[test]
    fn targeted_delivery_reaches_only_the_target_slot() {
        let _guard = QUEUE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

        static HITS: AtomicUsize = AtomicUsize::new(0);
        // Register a stand-in JS thread that stays alive and drains
        // its own slot queue until the targeted job lands.
        let (id_tx, id_rx) = std::sync::mpsc::channel();
        let target = std::thread::spawn(move || {
            crate::bffi_callback::set_js_thread().expect("bind ok");
            id_tx
                .send(crate::bffi_callback::current_thread_id())
                .expect("send id");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while HITS.load(Ordering::Relaxed) < 1 && std::time::Instant::now() < deadline {
                let _ = pump();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            HITS.load(Ordering::Relaxed)
        });
        let target_id = id_rx.recv().expect("target id");

        enqueue_to(
            target_id,
            Box::new(|| {
                HITS.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .expect("the target is registered");

        let hits = target.join().expect("target thread must not panic");
        assert_eq!(hits, 1, "the slot job is drained by its owner");
        assert_eq!(pending(), 0);
    }

    #[test]
    fn targeted_delivery_to_an_unknown_thread_fails() {
        let _guard = QUEUE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let result = enqueue_to(9_999_999, Box::new(|| {}));
        assert_eq!(result, Err(EventLoopError::NotRunning));
    }

    #[test]
    fn unregistered_runners_never_see_targeted_jobs() {
        let _guard = QUEUE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

        static RAN: AtomicUsize = AtomicUsize::new(0);
        // A registered stand-in that keeps its slot open until the
        // parent finished asserting.
        let (id_tx, id_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let target = std::thread::spawn(move || {
            crate::bffi_callback::set_js_thread().expect("bind ok");
            id_tx
                .send(crate::bffi_callback::current_thread_id())
                .expect("send id");
            let _ = release_rx.recv();
        });
        let target_id = id_rx.recv().expect("target id");

        enqueue_to(
            target_id,
            Box::new(|| {
                RAN.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .expect("the target is registered");

        // THIS thread is unregistered: its pump drains only the
        // global queue and must not touch the slot job.
        assert_eq!(pump(), 0);
        assert_eq!(RAN.load(Ordering::Relaxed), 0);

        // The registrant departs; its slot (with the queued job) is
        // retired, so the job never runs and the slot is closed.
        drop(release_tx);
        target.join().expect("registrant must not panic");
        assert!(
            enqueue_to(target_id, Box::new(|| {})).is_err(),
            "the departed thread's slot is closed"
        );
        assert_eq!(RAN.load(Ordering::Relaxed), 0, "the job died with the slot");
        assert_eq!(pending(), 0);
    }
}
