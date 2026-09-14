//! A registered `run` (a JS thread stand-in) drains its slot queue
//! AND the global queue, and exits cleanly on the process-wide stop.
//!
//! ISOLATION NOTE: `stop` is process-global and STICKY - once called,
//! every later `enqueue` in this process fails. That is why this test
//! lives in its own binary (the established isolation pattern of this
//! crate's test suite) instead of `multi-js-threads.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

#[test]
fn registered_run_drains_the_global_queue_and_exits_on_stop() {
    static RAN: AtomicBool = AtomicBool::new(false);
    let (id_tx, id_rx) = channel();
    let handle = thread::spawn(move || {
        bffi::set_js_thread().expect("register");
        id_tx.send(()).expect("send");
        bffi::run()
    });
    id_rx.recv().expect("registered");

    // An untargeted job lands on the global queue; the registered
    // runner drains it together with its (empty) slot queue.
    bffi::enqueue(Box::new(|| {
        RAN.store(true, Ordering::Release);
    }))
    .expect("the loop is running");
    while !RAN.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(1));
    }

    // Stop ends the registered runner's blocking loop.
    bffi::stop();
    let executed = handle.join().expect("runner joins");
    assert!(
        executed >= 1,
        "the registered runner drained the global job"
    );
    assert!(bffi::enqueue(Box::new(|| {})).is_err(), "stop is sticky");
}
