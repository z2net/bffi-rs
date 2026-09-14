//! Criterion benches for the object ownership path: the
//! [`ObjectWrap::wrap`] / `get` / `release` cycle over the global
//! generational-handle registry, single-threaded.

#![allow(missing_docs)] // the criterion_group! expansion is undocumented
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bffi::{ObjectWrap, TypeTag};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;

/// The object type stored behind the wrap.
struct BenchObject {
    payload: [u64; 4],
}

/// Builds one fresh stored value.
fn make() -> BenchObject {
    BenchObject {
        payload: [1, 2, 3, 4],
    }
}

/// The unique tag this bench claims for `BenchObject` (inside the
/// `bffi-object` range `0x0100..=0x01FF`).
const TAG: TypeTag = TypeTag(0x0180);

fn bench_handles(c: &mut Criterion) {
    // One wrap per process: `new` declares the tag, a second claim
    // with the same tag would fail with `TagInUse`.
    let wrap = ObjectWrap::<BenchObject>::new(TAG).expect("a unique tag in the object range");

    // Steady-state read path: resolve an existing handle.
    let pinned = wrap.wrap(make()).expect("room");
    c.bench_function("handles/get", |b| {
        b.iter(|| {
            let got: Arc<BenchObject> = wrap.get(black_box(pinned)).expect("live handle");
            black_box(got.payload[0])
        })
    });

    // The full ownership cycle: insert, resolve, remove.
    c.bench_function("handles/wrap_get_release", |b| {
        b.iter(|| {
            let handle = wrap.wrap(make()).expect("room");
            let got: Arc<BenchObject> = wrap.get(handle).expect("live handle");
            black_box(got.payload[0]);
            wrap.release(handle).expect("live handle");
        })
    });

    wrap.release(pinned).expect("cleanup");
}

criterion_group!(benches, bench_handles);
criterion_main!(benches);
