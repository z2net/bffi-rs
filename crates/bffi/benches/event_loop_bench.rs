//! Criterion benches for the event loop (`bffi::bffi_event_loop`):
//! the full enqueue -> pump cycle on the global queue (one job per
//! cycle, the job itself a single atomic increment) and the
//! pump-of-empty drain. The bench process registers no JS thread and
//! never stops the loop, so the drain keeps the legacy
//! global-queue-only behavior.

#![allow(missing_docs)] // the criterion_group! expansion is undocumented
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bffi::bffi_event_loop::{enqueue, pump};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

/// The job counter: incremented once per enqueued job and checked
/// against the enqueue count in the correctness gate.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The number of full enqueue -> pump cycles the gate exercises.
const GATE_CYCLES: u64 = 1_000;

fn bench_event_loop(c: &mut Criterion) {
    // Correctness gate: every enqueued job runs exactly once, one
    // pump drains the whole batch, and the drained queue pumps empty
    // before anything is timed.
    for _ in 0..GATE_CYCLES {
        enqueue(Box::new(|| {
            COUNTER.fetch_add(1, Ordering::Relaxed);
        }))
        .expect("the loop is never stopped here");
    }
    assert_eq!(pump(), GATE_CYCLES, "one pump drains the whole batch");
    assert_eq!(COUNTER.load(Ordering::Relaxed), GATE_CYCLES);
    assert_eq!(pump(), 0, "the drained queue pumps empty");

    // The full cycle: queue a job, then drain it - the pair a JS
    // tick performs.
    c.bench_function("event_loop/enqueue_pump", |b| {
        b.iter(|| {
            enqueue(Box::new(|| {
                COUNTER.fetch_add(1, Ordering::Relaxed);
            }))
            .expect("the loop is never stopped here");
            black_box(pump())
        })
    });

    // The empty drain: the fixed cost of finding nothing queued.
    c.bench_function("event_loop/pump_empty", |b| b.iter(|| black_box(pump())));
}

criterion_group!(benches, bench_event_loop);
criterion_main!(benches);
