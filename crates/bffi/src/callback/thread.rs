//! The JS-thread policy of the callback layer (multi-isolate).
//!
//! A callback is invoked only on a JS thread. Since Bun 1.4 workers
//! are real threads of the same process, a process may host SEVERAL
//! JS isolates - the main script and one per `Worker`. Each isolate
//! registers its own thread with [`set_js_thread`] (called on the
//! thread it owns); a registration is per-thread state held in a
//! process-wide table, and the thread's death (a terminated worker)
//! deregisters it automatically through the thread-local guard.
//!
//! [`ensure_js_thread`] is the gate every invocation path must pass.
//! While the process has NO registered JS threads it admits every
//! caller, so pure-Rust usage and unit tests need no binding ritual.
//!
//! The wait-depth gate ([`try_begin_wait`] / [`end_wait`]) tracks the
//! marshal-and-wait sections per thread: a NESTED `invoke_wait` on a
//! thread already parked inside one can never be satisfied (the
//! parked thread cannot also drain the loop), so it fails fast with
//! [`CallbackError::ReentrantWait`] instead of burning the timeout.
//!
//! Delivery targeting lives in the event loop: JS-facing entries
//! (JS-bound callbacks, stream wakes, async resolvers) record the
//! thread that created them and are marshalled to that thread's
//! queue, so a job never crosses an isolate boundary.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use super::error::CallbackError;

/// The registered JS threads of this process, keyed by numeric id.
///
/// Registration is a cold-path event (once per isolate at startup),
/// so a mutex around a set matches the event-loop crate's documented
/// trade-off: the lock-free machinery exists for the handle tables
/// that see CAS traffic on every FFI call.
static JS_THREADS: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

/// Source of per-thread numeric ids; starts at 0 so the first assigned
/// id is 1 and `0` stays free as the unbound sentinel.
static NEXT_THREAD_SEQ: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// This thread's stable numeric id, assigned on first use.
    static THREAD_SEQ: Cell<u64> = const { Cell::new(0) };

    /// The registration guard: `Some` while this thread is a
    /// registered JS thread. Dropping it (thread exit, a terminated
    /// worker) deregisters the thread.
    static REGISTRATION: std::cell::RefCell<Option<RegistrationGuard>> =
        const { std::cell::RefCell::new(None) };
}

/// The TLS marker that keeps a thread registered; dropping it
/// deregisters the thread and retires its event-loop queue.
struct RegistrationGuard;

impl Drop for RegistrationGuard {
    fn drop(&mut self) {
        let id = current_thread_id();
        let removed = with_threads(|set| set.remove(&id));
        if removed {
            // Retire the per-thread event-loop queue; jobs still
            // queued for this isolate are dropped with it (their
            // waiters observe the documented timeout path).
            #[cfg(feature = "event-loop")]
            crate::bffi_event_loop::js_thread_gone(id);
        }
    }
}

/// Runs `f` against the registration set, recovering from a poisoned
/// lock (registration and lookup never hold it across user code, so
/// poisoning cannot happen in practice - the crate-wide pattern).
fn with_threads<T>(f: impl FnOnce(&mut HashSet<u64>) -> T) -> T {
    let mut guard = JS_THREADS.lock().unwrap_or_else(PoisonError::into_inner);
    let set = guard.get_or_insert_with(HashSet::new);
    f(set)
}

/// Registers the CURRENT thread as a JS thread of this process.
///
/// Every JS isolate (the main script and each Bun `Worker`) calls
/// this once on its own thread. Registration is idempotent for the
/// calling thread; other threads are unaffected - unlike the
/// pre-multi-isolate policy, later registrations never reject or
/// overwrite earlier ones.
///
/// # Errors
///
/// Never fails today; the [`CallbackError`] return keeps the ABI
/// shape stable for future admission policies.
pub fn set_js_thread() -> Result<(), CallbackError> {
    let id = current_thread_id();
    let fresh = REGISTRATION.with(|slot| {
        let mut guard = slot.borrow_mut();
        if guard.is_some() {
            false
        } else {
            *guard = Some(RegistrationGuard);
            true
        }
    });
    if fresh {
        with_threads(|set| set.insert(id));
        #[cfg(feature = "event-loop")]
        crate::bffi_event_loop::js_thread_registered(id);
    }
    Ok(())
}

/// Deregisters the CURRENT thread.
///
/// The explicit shutdown half of [`set_js_thread`]: a JS isolate that
/// is about to exit (a `Worker` before `terminate`) calls this so its
/// slot queue retires immediately and further targeted deliveries
/// fail fast. WITHOUT this call the registration is reclaimed only by
/// the TLS guard on a normal thread exit - and a TERMINATED worker
/// thread (Windows `TerminateThread` semantics) never runs TLS
/// destructors, leaving a zombie registration whose deliveries time
/// out. Idempotent for an unregistered thread.
///
/// # Errors
///
/// Never fails today; the [`CallbackError`] return keeps the ABI
/// shape stable for future admission policies.
pub fn unset_js_thread() -> Result<(), CallbackError> {
    // Dropping the guard runs the deregistration path (table removal
    // + slot queue retirement) exactly like a thread exit would.
    REGISTRATION.with(|slot| {
        drop(slot.borrow_mut().take());
    });
    Ok(())
}

/// Ensures the CURRENT thread is a registered JS thread.
///
/// - No JS threads registered: `Ok` (pure-Rust usage and unit tests).
/// - Registered and current is one of them: `Ok` (ANY isolate may run
///   isolate-independent Rust work).
/// - Registered and current is not: `Err(CallbackError::WrongThread)`.
pub fn ensure_js_thread() -> Result<(), CallbackError> {
    let registered = with_threads(|set| !set.is_empty());
    if !registered {
        return Ok(());
    }
    let id = current_thread_id();
    let member = with_threads(|set| set.contains(&id));
    if member {
        Ok(())
    } else {
        Err(CallbackError::WrongThread)
    }
}

/// Whether `id` is currently a registered JS thread.
#[cfg(feature = "event-loop")]
pub(crate) fn is_js_thread(id: u64) -> bool {
    with_threads(|set| set.contains(&id))
}

/// The calling thread's id when it is a registered JS thread, else
/// `0`. JS-facing registrations (JS-bound callbacks, stream wakes,
/// async resolvers) record this value so deliveries can be targeted;
/// a binding from an unbound (pure-Rust) context records the legacy
/// untargeted sentinel.
#[cfg(feature = "event-loop")]
pub(crate) fn binding_thread() -> u64 {
    let id = current_thread_id();
    if is_js_thread(id) { id } else { 0 }
}

/// The numeric id of the calling thread, assigning one on first use.
///
/// `std::thread::ThreadId` has no stable numeric accessor
/// (`ThreadId::as_u64` is feature-gated `thread_id_value`), so ids are
/// assigned per thread on first use from a process-wide counter -
/// monotonic and never reused, matching `ThreadId` semantics.
pub(crate) fn current_thread_id() -> u64 {
    THREAD_SEQ.with(|seq| {
        let assigned = seq.get();
        if assigned == 0 {
            let fresh = NEXT_THREAD_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
            seq.set(fresh);
            fresh
        } else {
            assigned
        }
    })
}

#[cfg(feature = "event-loop")]
thread_local! {
    /// The per-thread `invoke_wait` nesting depth: `> 0` while this
    /// thread is parked inside a marshal-and-wait section.
    static WAIT_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Marks the calling thread as entering an `invoke_wait` wait
/// section (the `Mutex` + `Condvar` park).
///
/// `Err` means the thread is ALREADY inside a wait: a nested
/// marshal-and-wait can never be satisfied - the parked thread cannot
/// also drain the event loop its job depends on - so the caller
/// translates this into [`CallbackError::ReentrantWait`] and fails
/// fast instead of burning the whole timeout.
///
/// # Errors
///
/// `Err(())` when a wait section is already active on this thread
/// (not yet closed by [`end_wait`]).
#[cfg(feature = "event-loop")]
pub(crate) fn try_begin_wait() -> Result<(), ()> {
    WAIT_DEPTH.with(|depth| {
        if depth.get() > 0 {
            return Err(());
        }
        depth.set(depth.get() + 1);
        Ok(())
    })
}

/// Marks the calling thread as having left the wait section (the
/// [`try_begin_wait`] counterpart); a surplus call is absorbed so the
/// depth can never underflow.
#[cfg(feature = "event-loop")]
pub(crate) fn end_wait() {
    WAIT_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
}

/// The crate-wide test lock over the process-global JS-thread state.
/// Tests run in parallel threads of one process: every test that BINDS
/// the process (a keeper thread inside its body) and every test that
/// ASSUMES it unbound must hold this lock, so a parallel test thread
/// can never observe another module's bound window.
#[cfg(test)]
pub(crate) fn process_state_lock() -> std::sync::MutexGuard<'static, ()> {
    static PROCESS_STATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    PROCESS_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    // NOTE (test isolation): the JS-thread table is process-global,
    // and libtest runs tests in parallel. Every registration here
    // lives in a spawned thread whose TLS guard drops before the
    // thread joins, so no registration outlives a test - and the
    // shared process_state_lock serializes the tests against each
    // other AND against every other module's tests that bind the
    // process or assume it unbound. Integration-level wrong-thread
    // behavior is covered by `tests/threading.rs` and
    // `tests/multi-js-threads.rs`, separate processes.
    use std::time::{Duration, Instant};

    use super::{
        current_thread_id, end_wait, ensure_js_thread, is_js_thread, process_state_lock,
        set_js_thread, try_begin_wait,
    };
    use crate::bffi_callback::{
        CallbackError, CallbackSig, ValueType, bind_js_callback, invoke_wait,
    };

    #[test]
    fn unbound_ensure_is_ok() {
        let _guard = process_state_lock();
        assert!(ensure_js_thread().is_ok());
    }

    #[test]
    fn registration_is_thread_scoped_and_auto_drops() {
        let _guard = process_state_lock();

        let id = std::thread::spawn(|| {
            set_js_thread().expect("bind ok");
            assert!(ensure_js_thread().is_ok());
            current_thread_id()
        })
        .join()
        .expect("registrar thread must not panic");

        // The registrar is gone, so the process is unbound again and
        // every caller is admitted.
        assert!(!is_js_thread(id), "TLS drop must deregister");
        assert!(ensure_js_thread().is_ok());
    }

    #[test]
    fn second_registration_is_accepted() {
        let _guard = process_state_lock();

        // Two registrars stay ALIVE while the assertions run - a
        // registration lives exactly as long as its thread.
        let (id_tx, id_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let keeper = std::thread::spawn(move || {
            set_js_thread().expect("bind ok");
            assert!(ensure_js_thread().is_ok());
            id_tx.send(current_thread_id()).expect("send id");
            let _ = release_rx.recv();
        });
        let first = id_rx.recv().expect("first id");
        assert!(is_js_thread(first));

        let second = std::thread::spawn(|| {
            set_js_thread().expect("second bind ok");
            assert!(ensure_js_thread().is_ok());
            current_thread_id()
        })
        .join()
        .expect("second thread must not panic");
        assert_ne!(first, second);

        // A concurrent registration is accepted, not rejected - the
        // multi-isolate contract. But an UNREGISTERED caller while
        // registrations exist is still rejected.
        let rejected = std::thread::spawn(ensure_js_thread).join().expect("join");
        assert_eq!(rejected, Err(super::CallbackError::WrongThread));

        // Release the keeper: both registrations die with their
        // threads and the process is unbound again.
        drop(release_tx);
        keeper.join().expect("keeper must not panic");
        assert!(!is_js_thread(first));
    }

    #[test]
    fn wait_depth_refuses_nested_begins_and_absorbs_surplus_ends() {
        assert!(try_begin_wait().is_ok(), "a fresh thread has depth 0");
        assert_eq!(try_begin_wait(), Err(()), "a nested begin is refused");
        end_wait();
        end_wait(); // surplus: absorbed, the depth must not underflow
        assert!(
            try_begin_wait().is_ok(),
            "the depth must not have underflowed"
        );
        end_wait();
    }

    #[test]
    fn nested_invoke_wait_fails_fast_with_reentrant_wait() {
        let _guard = process_state_lock();

        // A live registration makes the process BOUND, so the
        // unregistered caller below takes the marshal-and-wait path -
        // the only path with a wait section to gate. The keeper never
        // pumps: the gate fires BEFORE anything is queued.
        let (id_tx, id_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let keeper = std::thread::spawn(move || {
            set_js_thread().expect("bind ok");
            id_tx.send(current_thread_id()).expect("send id");
            let _ = release_rx.recv();
        });
        let _id = id_rx.recv().expect("keeper must announce");

        // The entry is bound by the (still unregistered) test thread,
        // so it records the legacy untargeted sentinel - irrelevant:
        // the gate fires before any queue interaction.
        let handle = bind_js_callback(CallbackSig::new(ValueType::Bool, &[]), 0)
            .expect("callback table has room");

        let started = Instant::now();
        let outcome = std::thread::spawn(move || {
            // Simulated nesting: this thread is "already waiting"
            // (the outer begin succeeded), so invoke_wait must refuse
            // instead of parking for the full timeout.
            try_begin_wait().expect("the outer wait begins");
            let outcome = invoke_wait(handle, &[], Duration::from_secs(30));
            end_wait();
            outcome
        })
        .join()
        .expect("the waiting thread must not panic");
        let elapsed = started.elapsed();

        assert_eq!(outcome, Err(CallbackError::ReentrantWait));
        assert!(
            elapsed < Duration::from_secs(1),
            "the gate must fail fast, not burn the timeout: {elapsed:?}"
        );

        drop(release_tx);
        keeper.join().expect("keeper must not panic");
    }
}
