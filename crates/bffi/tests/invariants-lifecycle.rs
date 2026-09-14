//! Lifecycle state-machine invariant tests (security review), through
//! the PUBLIC API only.
//!
//! Existing coverage this file deliberately does NOT duplicate:
//! - JS-bound + native revoke during a pending `invoke_wait`: pinned in
//!   `callback-wait.rs :: invoke_wait_reports_revocation_during_the_wait`
//!   and `:: invoke_wait_js_bound_reports_revocation_during_the_wait`
//!   (the marshal job's fresh lookup crosses `InvalidHandle`; no call
//!   through a dead slot).
//! - revoke-before/after invoke, revoke-exactly-once:
//!   `callback-callback_lifecycle.rs`.
//! - cancel before the first poll + second-cancel-is-false:
//!   `async-core.rs :: cancel_before_first_poll_drops_the_future`
//!   (plus the unit-level state machine in `async/task.rs`).
//! - error drain set/take/clear:
//!   `error-mapping.rs` and the unit test `core/error.rs ::
//!   last_error_can_be_set_and_taken`.
//!
//! Isolation model: this binary NEVER binds a JS thread and NEVER
//! pumps/runs the loop, so delivery jobs (which carry fake resolver
//! pointers) only accumulate in the queue and are observed by COUNT
//! (`bffi::pending()` deltas) - they never execute. While unbound,
//! `invoke` admits every thread (pure-Rust usage mode).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bffi::bffi_async::{AsyncError, AsyncValue};
use bffi::bffi_callback::{CallbackError, CallbackSig, Value, ValueType, invoke, register, revoke};
use bffi::bffi_core::{BffiError, ErrorCode};
use bffi::bffi_error::{JsErrorName, take_last_error_shape};
use bffi::{attach, cancel, pending_tasks, set_last_error, spawn, take_last_error};

/// Waits (with a deadline) until `check` holds.
fn wait_for(what: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

// --- 7: revoke DURING an in-flight invoke ------------------------------

/// A callback body that revokes ITSELF mid-invoke. Pinned behavior:
/// the in-flight invoke completes deterministically with the body's
/// value (the registry hands the invoke an owning `Arc`, so the entry
/// outlives its own removal), the inner `revoke` wins the slot removal
/// exactly once, and every later use of the handle is InvalidHandle.
#[test]
fn self_revoke_during_invoke_completes_and_lands_invalid() {
    let handle_slot: Arc<OnceLock<bffi::Handle>> = Arc::new(OnceLock::new());
    let inner_revoked = Arc::new(AtomicBool::new(false));

    let slot = Arc::clone(&handle_slot);
    let flag = Arc::clone(&inner_revoked);
    let handle = register(
        CallbackSig::new(ValueType::I32, &[]),
        Arc::new(move |_| {
            let own = slot.get().copied().expect("handle recorded before invoke");
            flag.store(revoke(own), Ordering::SeqCst);
            Value::I32(91)
        }),
    )
    .expect("callback table has room");
    handle_slot.set(handle).expect("set once");

    // The in-flight invoke completes; the body ran and won the removal.
    let result = invoke(handle, &[]).expect("in-flight invoke must complete");
    assert_eq!(result, Value::I32(91));
    assert!(
        inner_revoked.load(Ordering::SeqCst),
        "the body's self-revoke must win the slot removal"
    );

    // Terminal: every later use is InvalidHandle, revoke is a no-op.
    assert_eq!(
        invoke(handle, &[]).err(),
        Some(CallbackError::InvalidHandle(handle))
    );
    assert!(!revoke(handle), "second revoke after self-revoke is false");
}

/// Variant: a body revoking a DIFFERENT live handle mid-invoke. The
/// victim's invoke must be InvalidHandle right after; the revoking
/// invoke is unaffected.
#[test]
fn revoke_of_another_handle_during_invoke_is_immediate() {
    let victim = register(
        CallbackSig::new(ValueType::Bool, &[]),
        Arc::new(|_| Value::Bool(true)),
    )
    .expect("callback table has room");

    let killer = register(
        CallbackSig::new(ValueType::I32, &[]),
        Arc::new(move |_| {
            assert!(revoke(victim), "the mid-invoke revoke of the victim wins");
            Value::I32(1)
        }),
    )
    .expect("callback table has room");

    assert_eq!(
        invoke(killer, &[]).expect("revoking invoke completes"),
        Value::I32(1)
    );
    assert_eq!(
        invoke(victim, &[]).err(),
        Some(CallbackError::InvalidHandle(victim))
    );
}

// --- 8: cancelled tasks deliver Cancelled exactly once -----------------
//
// The delivery queue is never drained here, so `bffi::pending()`
// deltas count EXACTLY how many delivery jobs were enqueued - the
// observable form of "the outcome is Cancelled exactly once" without
// real resolver trampolines. The three delivery-counting tests share
// a static mutex (the `callback-wait.rs` pattern): the queue is
// process-global, and the queue is only safe to measure while no
// other test can enqueue into the delta window.

/// Serializes the delivery-counting tests (see the section docs).
static DELIVERY_LOCK: Mutex<()> = Mutex::new(());

/// A future that is polled once (or not at all, if cancel wins first)
/// and then stays pending forever; records polls and drops.
struct PendingForever {
    polls: Arc<AtomicU32>,
    dropped: Arc<AtomicBool>,
}

impl Drop for PendingForever {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl std::future::Future for PendingForever {
    type Output = Result<AsyncValue, BffiError>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        std::task::Poll::Pending
    }
}

/// Attach BEFORE cancel: the cancel itself enqueues exactly ONE
/// delivery (the stored Cancelled outcome); the second cancel is a
/// no-op that enqueues nothing; a second attach is rejected and cannot
/// produce a second delivery.
#[test]
fn cancel_delivers_cancelled_exactly_once() {
    let _guard = DELIVERY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let polls = Arc::new(AtomicU32::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let task = spawn(PendingForever {
        polls: Arc::clone(&polls),
        dropped: Arc::clone(&dropped),
    })
    .expect("task table has room");

    // Attach while running: no outcome yet, so NOTHING may be enqueued
    // (an attach-time attempt must not consume the delivery slot).
    let before = bffi::pending();
    attach(task, 1, 2).expect("attach to a running task");
    assert_eq!(
        bffi::pending() - before,
        0,
        "attach on a running task enqueues nothing"
    );

    // THE invariant: exactly one Cancelled delivery, exactly once.
    assert!(cancel(task), "cancel wins on a running task");
    assert_eq!(
        bffi::pending() - before,
        1,
        "cancel must enqueue exactly one delivery job"
    );

    // Second cancel: terminal state already reached, false, no second
    // delivery.
    assert!(!cancel(task), "second cancel is a no-op");
    assert_eq!(
        bffi::pending() - before,
        1,
        "no second delivery after a second cancel"
    );

    // The future is dropped at the poll boundary and never resolves.
    wait_for("the cancelled future dropped", || {
        dropped.load(Ordering::SeqCst)
    });
    assert_eq!(pending_tasks(), 0, "the cancelled task is not live anymore");
    assert!(
        polls.load(Ordering::SeqCst) <= 1,
        "a cancelled future is polled at most once, never to completion"
    );

    // A resolver pair attaches at most once: no second delivery path.
    assert_eq!(attach(task, 3, 4).err(), Some(AsyncError::AlreadyAttached));
    assert_eq!(bffi::pending() - before, 1);

    // Bogus and foreign handles never win a cancel.
    assert!(!cancel(bffi::Handle::NULL));
    assert!(!cancel(bffi::Handle::new(
        bffi::TypeTag(0x0500),
        7,
        123_456
    )));
}

/// Cancel DURING a poll (the executor's cancel-during-poll window):
/// the cancel wins the transition while the worker is parked inside
/// poll; when the future later returns Pending it must be dropped,
/// the lost `mark_cancelled` must not double-finish or re-deliver.
#[test]
fn cancel_during_poll_never_double_delivers() {
    let _guard = DELIVERY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    struct GateState {
        released: Mutex<bool>,
        signal: Condvar,
    }
    let state = Arc::new(GateState {
        released: Mutex::new(false),
        signal: Condvar::new(),
    });
    let entered = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicU32::new(0));
    let dropped = Arc::new(AtomicBool::new(false));

    struct Gated {
        state: Arc<GateState>,
        entered: Arc<AtomicBool>,
        polls: Arc<AtomicU32>,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for Gated {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    impl std::future::Future for Gated {
        type Output = Result<AsyncValue, BffiError>;
        fn poll(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.entered.store(true, Ordering::SeqCst);
            let mut released = self
                .state
                .released
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !*released {
                let (woken, timeout) = self
                    .state
                    .signal
                    .wait_timeout(released, Duration::from_millis(50))
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                released = woken;
                if timeout.timed_out() && Instant::now() >= deadline {
                    // Never blocks the worker forever: fail via the
                    // outer deadline asserts instead.
                    break;
                }
            }
            std::task::Poll::Pending
        }
    }

    let task = spawn(Gated {
        state: Arc::clone(&state),
        entered: Arc::clone(&entered),
        polls: Arc::clone(&polls),
        dropped: Arc::clone(&dropped),
    })
    .expect("task table has room");

    // The worker is now INSIDE poll (parked on the gate).
    wait_for("the future entered poll", || entered.load(Ordering::SeqCst));

    let before = bffi::pending();
    attach(task, 1, 2).expect("attach while parked in poll");
    assert!(cancel(task), "cancel wins while the worker is inside poll");
    assert!(!cancel(task));
    assert_eq!(
        bffi::pending() - before,
        1,
        "exactly one Cancelled delivery despite the in-flight poll"
    );

    // Release the gate: the poll returns Pending into a cancelled
    // task - the future is dropped and the lost transition must not
    // enqueue a second delivery or double-finish the task.
    *state
        .released
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    state.signal.notify_all();
    wait_for("the future dropped after the gate", || {
        dropped.load(Ordering::SeqCst)
    });
    wait_for("live count back to zero", || pending_tasks() == 0);
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "no further poll after cancel"
    );
    assert_eq!(
        bffi::pending() - before,
        1,
        "the post-cancel Pending must not deliver again"
    );
}

/// Attach AFTER cancel: the stored Cancelled outcome is delivered at
/// attach time (exactly one job), and a second attach is rejected.
#[test]
fn late_attach_after_cancel_delivers_the_stored_outcome_once() {
    let _guard = DELIVERY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let task = spawn(PendingForever {
        polls: Arc::new(AtomicU32::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    })
    .expect("task table has room");

    assert!(cancel(task), "cancel wins");
    assert!(!cancel(task), "second cancel is a no-op");
    assert_eq!(pending_tasks(), 0);

    // No resolvers were attached at cancel time: nothing queued yet.
    let before = bffi::pending();
    attach(task, 1, 2).expect("late attach delivers the stored outcome");
    assert_eq!(
        bffi::pending() - before,
        1,
        "the stored Cancelled outcome is delivered exactly once at attach"
    );

    assert_eq!(
        attach(task, 3, 4).err(),
        Some(AsyncError::AlreadyAttached),
        "a second attach cannot produce a second delivery"
    );
    assert_eq!(bffi::pending() - before, 1);
}

// --- 10: the last-error slot REPLACES under consecutive failures -------

/// Pinned behavior: `set_last_error` REPLACES any previous error
/// (errno-style, newest wins) - a second failed call must not append
/// or drop the new error, and the drain hands out exactly the newest
/// one and clears the slot. Mirrors the unit pin in
/// `core/error.rs :: last_error_can_be_set_and_taken`, here over both
/// public drain surfaces (`take_last_error` and the JS-facing
/// `take_last_error_shape`).
#[test]
fn second_failed_call_replaces_the_last_error() {
    // Fresh slot on this test thread.
    assert!(take_last_error().is_none(), "test threads start clean");

    // "First failed call".
    set_last_error(BffiError::new(ErrorCode::InvalidHandle, "first failure"));
    // "Second failed call": REPLACES, does not queue.
    set_last_error(BffiError::new(ErrorCode::DomainError, "second failure"));

    let drained = take_last_error().expect("the newest error is stored");
    assert_eq!(drained.code, ErrorCode::DomainError, "newest error wins");
    assert_eq!(drained.message, "second failure");

    // The drain cleared the slot: the replaced error is gone for good.
    assert!(take_last_error().is_none());

    // Same replace semantics on the JS-facing shape drain.
    set_last_error(BffiError::new(ErrorCode::NumberOutOfRange, "js first"));
    set_last_error(BffiError::new(ErrorCode::InvalidUtf8, "js second"));
    let shape = take_last_error_shape().expect("shape of the newest error");
    assert_eq!(
        shape.name,
        JsErrorName::TypeError,
        "InvalidUtf8 maps to SyntaxError"
    );
    assert_eq!(shape.message, "js second");
    assert!(take_last_error_shape().is_none(), "drain clears the slot");
}
