//! Multi-isolate integration: TWO registered JS threads in one
//! process, each draining its own slot queue, with targeted
//! `invoke_wait` delivery for JS-bound entries.
//!
//! The routing proof: a JS-bound entry bound on isolate A carries A's
//! thread id, so its `invoke_wait` job is queued on A's slot queue.
//! The job itself (`invoke_js_entry`) re-checks the isolate - if the
//! delivery crossed isolates (landed on B), the call reports
//! `WrongThread` instead of reaching the entry. Both entries below
//! use the opaque pointer token `0`, which the dispatch rejects with
//! `NullPointer` AFTER the isolate gate - so `NullPointer` proves the
//! job ran on the OWNING thread, and `Timeout`/`WrongThread` would
//! prove the opposite.
//!
//! Isolation: this binary owns its whole process - the stand-in
//! threads register themselves and deregister when they end.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use bffi::bffi_callback::{CallbackError, CallbackSig, ValueType, bind_js_callback, invoke_wait};

type Job = Box<dyn FnOnce() + Send>;

/// One live JS isolate: registers itself, then drains its own slot
/// queue (plus the global queue) exactly like the JS pump does. The
/// [`Isolate::release`] half ends the drain; dropping the handle
/// joins the thread (which deregisters it).
struct Isolate {
    sender: Sender<Job>,
    release: Option<Sender<()>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Isolate {
    /// Spawns and registers a new JS-thread stand-in. The `Barrier`
    /// makes sure the registration is visible before [`spawn_isolate`]
    /// returns.
    fn start(name: &'static str, registered: Arc<Barrier>) -> Self {
        let (job_sender, job_receiver) = channel::<Job>();
        let (release_sender, release_receiver) = channel::<()>();
        let barrier = Arc::clone(&registered);
        let handle = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                bffi::set_js_thread().expect("the isolate must register itself");
                barrier.wait();
                // The loop-side twin of the JS pump: drain the event
                // loop (own slot queue first, then the global queue)
                // and run the on-thread jobs, until released. A
                // DISCONNECTED release channel is also an exit - the
                // Isolate handle was dropped.
                loop {
                    let _ = bffi::pump();
                    while let Ok(job) = job_receiver.try_recv() {
                        job();
                    }
                    match release_receiver.try_recv() {
                        Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                }
            })
            .expect("spawning an isolate must not fail");
        Self {
            sender: job_sender,
            release: Some(release_sender),
            handle: Some(handle),
        }
    }

    /// Runs `f` on this isolate's thread and waits for the result.
    fn on_thread<R>(&self, f: impl FnOnce() -> R + Send + 'static) -> R
    where
        R: Send + 'static,
    {
        let (reply_sender, reply_receiver) = channel();
        self.sender
            .send(Box::new(move || {
                let _ = reply_sender.send(f());
            }))
            .expect("the isolate must be alive");
        reply_receiver.recv().expect("the isolate must answer")
    }
}

impl Drop for Isolate {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(handle) = self.handle.take() {
            handle.join().expect("the isolate must end cleanly");
        }
    }
}

/// Binds a JS-bound entry ON the given isolate's thread and returns
/// its handle. `ptr = 0` (the opaque-null token) makes the eventual
/// dispatch report `NullPointer` after the isolate gate - the marker
/// of a correctly-routed delivery.
fn bind_on(isolate: &Isolate) -> bffi::Handle {
    isolate.on_thread(|| {
        bind_js_callback(CallbackSig::new(ValueType::I32, &[ValueType::I32]), 0).expect("bind ok")
    })
}

#[test]
fn invoke_wait_delivers_each_entry_to_its_own_isolate() {
    let registered = Arc::new(Barrier::new(3));
    let a = Isolate::start("bffi-js-a", Arc::clone(&registered));
    let b = Isolate::start("bffi-js-b", Arc::clone(&registered));
    let _guard = registered.wait();

    let handle_a = bind_on(&a);
    let handle_b = bind_on(&b);

    // Both waits run on THIS (unregistered, native) thread; each job
    // must land on its OWNING isolate. `NullPointer` = the job ran on
    // the owner and reached the null-pointer check after the isolate
    // gate; `WrongThread` would mean a crossed delivery, `Timeout` a
    // lost one.
    let (outcome_a, outcome_b) = {
        let h_a = handle_a;
        let h_b = handle_b;
        thread::scope(|scope| {
            let wait_a =
                scope.spawn(|| invoke_wait(h_a, &[bffi::Value::I32(1)], Duration::from_secs(5)));
            let wait_b =
                scope.spawn(|| invoke_wait(h_b, &[bffi::Value::I32(2)], Duration::from_secs(5)));
            (
                wait_a.join().expect("wait a"),
                wait_b.join().expect("wait b"),
            )
        })
    };
    assert_eq!(outcome_a, Err(CallbackError::NullPointer));
    assert_eq!(outcome_b, Err(CallbackError::NullPointer));
}

#[test]
fn invoke_wait_reports_loop_stopped_when_the_owner_is_gone() {
    let registered = Arc::new(Barrier::new(2));
    let a = Isolate::start("bffi-js-a", Arc::clone(&registered));
    let _guard = registered.wait();

    let handle = bind_on(&a);
    // The isolate ends: its slot queue is retired with the thread.
    drop(a);

    // The targeted delivery now has nowhere to go - immediate
    // failure, not a timeout.
    let outcome = invoke_wait(handle, &[bffi::Value::I32(1)], Duration::from_secs(5));
    assert_eq!(outcome, Err(CallbackError::LoopStopped));
}
