//! Integration tests of the async core over the public API.
//!
//! Delivery tests stop short of invoking resolver trampolines: fake
//! pointers would segfault. The trampoline path is exercised by the
//! real bun:ffi e2e suite (the async example in the bffi-examples
//! repo).
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use bffi::bffi_async::{AsyncValue, cancel, pending_tasks, sleep, spawn, timeout};

fn ok(value: AsyncValue) -> Result<AsyncValue, bffi::bffi_core::BffiError> {
    Ok(value)
}

/// Waits (with a deadline) until `check` holds.
fn wait_for(what: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct GateState {
    opened: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// Stays pending until the test opens the gate and wakes the stored
/// waker (the canonical pending-then-woken future).
struct Gate {
    state: Arc<GateState>,
    done: Arc<AtomicBool>,
    registered: bool,
}

impl Future for Gate {
    type Output = Result<AsyncValue, bffi::bffi_core::BffiError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.state.opened.load(Ordering::SeqCst) {
            self.done.store(true, Ordering::SeqCst);
            return Poll::Ready(ok(AsyncValue::I32(7)));
        }
        *self.state.waker.lock().expect("lock") = Some(cx.waker().clone());
        self.registered = true;
        // Lost-wakeup guard: the gate may have opened while the waker
        // was being registered.
        if self.state.opened.load(Ordering::SeqCst) {
            self.done.store(true, Ordering::SeqCst);
            return Poll::Ready(ok(AsyncValue::I32(7)));
        }
        Poll::Pending
    }
}

#[test]
fn ready_futures_run_to_completion() {
    let done = Arc::new(AtomicU32::new(0));
    for _ in 0..256 {
        let done = Arc::clone(&done);
        spawn(async move {
            done.fetch_add(1, Ordering::SeqCst);
            ok(AsyncValue::Unit)
        })
        .expect("room");
    }
    wait_for("256 ready tasks", || done.load(Ordering::SeqCst) == 256);
    wait_for("live count back to zero", || pending_tasks() == 0);
}

#[test]
fn woken_pending_futures_complete() {
    let state = Arc::new(GateState {
        opened: AtomicBool::new(false),
        waker: Mutex::new(None),
    });
    let done = Arc::new(AtomicBool::new(false));
    spawn(Gate {
        state: Arc::clone(&state),
        registered: false,
        done: Arc::clone(&done),
    })
    .expect("room");

    wait_for("the future registered its waker", || {
        state.waker.lock().expect("lock").is_some()
    });
    state.opened.store(true, Ordering::SeqCst);
    let waker = state.waker.lock().expect("lock").take();
    waker.expect("waker registered").wake();
    wait_for("woken task executed", || done.load(Ordering::SeqCst));
}

#[test]
fn cancel_before_first_poll_drops_the_future() {
    static DROPPED: AtomicBool = AtomicBool::new(false);
    struct DropFlag;

    impl Drop for DropFlag {
        fn drop(&mut self) {
            DROPPED.store(true, Ordering::SeqCst);
        }
    }

    struct Held(DropFlag);
    impl Future for Held {
        type Output = Result<AsyncValue, bffi::bffi_core::BffiError>;
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    let task = spawn(Held(DropFlag)).expect("room");
    assert!(cancel(task), "cancel wins while running");
    assert!(!cancel(task), "second cancel is a no-op");
    wait_for("the future dropped", || DROPPED.load(Ordering::SeqCst));
    wait_for("live count drops to zero", || pending_tasks() == 0);
}

#[test]
fn panic_inside_a_future_does_not_kill_the_executor() {
    struct Panicky;
    impl Future for Panicky {
        type Output = Result<AsyncValue, bffi::bffi_core::BffiError>;
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            panic!("async boom");
        }
    }

    spawn(Panicky).expect("room");
    wait_for("the panicking task finished", || pending_tasks() == 0);

    // The executor survives: a healthy task still completes.
    let done = Arc::new(AtomicBool::new(false));
    let done2 = Arc::clone(&done);
    spawn(async move {
        done2.store(true, Ordering::SeqCst);
        ok(AsyncValue::Bool(true))
    })
    .expect("room");
    wait_for("healthy task executed after the panic", || {
        done.load(Ordering::SeqCst)
    });
}

#[test]
fn result_errors_become_the_task_failure() {
    struct Failing;
    impl Future for Failing {
        type Output = Result<AsyncValue, bffi::bffi_core::BffiError>;
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(Err(bffi::bffi_core::BffiError::new(
                bffi::bffi_core::ErrorCode::Error,
                "domain failure",
            )))
        }
    }
    spawn(Failing).expect("room");
    wait_for("failing task finished", || pending_tasks() == 0);
}

#[test]
fn sleep_completes_after_the_deadline() {
    let start = Instant::now();
    spawn(async move {
        sleep(Duration::from_millis(25)).await;
        ok(AsyncValue::Unit)
    })
    .expect("room");
    wait_for("sleep task finished", || pending_tasks() == 0);
    assert!(
        start.elapsed() >= Duration::from_millis(20),
        "the deadline must be respected"
    );
}

#[test]
fn timeout_fires_and_drops_the_inner_future() {
    static INNER_DROPPED: AtomicBool = AtomicBool::new(false);
    struct DropOnCancel;
    impl Drop for DropOnCancel {
        fn drop(&mut self) {
            INNER_DROPPED.store(true, Ordering::SeqCst);
        }
    }

    struct PendingHeld(DropOnCancel);
    impl Future for PendingHeld {
        type Output = Result<AsyncValue, bffi::bffi_core::BffiError>;
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    spawn(timeout(
        Duration::from_millis(10),
        PendingHeld(DropOnCancel),
    ))
    .expect("room");
    wait_for("timed-out task finished", || pending_tasks() == 0);
    wait_for("the inner future dropped", || {
        INNER_DROPPED.load(Ordering::SeqCst)
    });
}

#[test]
fn timeout_does_not_fire_when_the_future_finishes_first() {
    spawn(timeout(Duration::from_secs(10), async {
        ok(AsyncValue::I64(-42))
    }))
    .expect("room");
    wait_for("fast task finished before its deadline", || {
        pending_tasks() == 0
    });
}

#[test]
fn many_workers_execute_concurrent_tasks() {
    // More tasks than workers: everything must still execute.
    let done = Arc::new(AtomicU32::new(0));
    for _ in 0..64 {
        let done = Arc::clone(&done);
        spawn(async move {
            std::thread::sleep(Duration::from_millis(2));
            done.fetch_add(1, Ordering::SeqCst);
            ok(AsyncValue::Unit)
        })
        .expect("room");
    }
    wait_for("64 blocking tasks", || done.load(Ordering::SeqCst) == 64);
}
