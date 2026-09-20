//! Criterion benches for the native callback round trip
//! (`bffi::bffi_callback`): one registered `(i32) -> i32` closure
//! invoked through its opaque handle. The bench process stays UNBOUND
//! (no `set_js_thread` anywhere), so `invoke` and `invoke_wait` both
//! take the direct path - the table lookup + signature check + closure
//! call - with the wait adding only its thread-gate/slot overhead.

#![allow(missing_docs)] // the criterion_group! expansion is undocumented
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bffi::bffi_callback::{CallbackSig, Value, ValueType, invoke, invoke_wait, register, revoke};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

/// The argument every round trip passes (doubled by the closure).
const ARG: i32 = 21;

/// The expected doubled result of the registered closure.
const EXPECTED: i32 = 42;

/// The wait budget of `invoke_wait`: the process is unbound, so the
/// direct path fills the outcome long before this deadline.
const WAIT: Duration = Duration::from_secs(5);

fn bench_callback(c: &mut Criterion) {
    // One registration per process: the `(i32) -> i32` closure the
    // round trips call through its handle.
    let handle = register(
        CallbackSig::new(ValueType::I32, &[ValueType::I32]),
        Arc::new(|args: &[Value]| match args {
            [Value::I32(x)] => Value::I32(x.wrapping_mul(2)),
            _ => unreachable!("invoke checks the signature before calling"),
        }),
    )
    .expect("table has room");

    // Correctness gate: both dispatches return exactly the doubled
    // argument before anything is timed.
    assert_eq!(
        invoke(handle, &[Value::I32(ARG)]).expect("live handle"),
        Value::I32(EXPECTED),
        "the invoke round trip must double the argument"
    );
    assert_eq!(
        invoke_wait(handle, &[Value::I32(ARG)], WAIT).expect("live handle"),
        Value::I32(EXPECTED),
        "the invoke_wait round trip must double the argument"
    );

    // The steady-state round trip: lookup, signature check, call.
    c.bench_function("callback/invoke", |b| {
        b.iter(|| {
            let out = invoke(black_box(handle), &[Value::I32(ARG)]).expect("live handle");
            black_box(out)
        })
    });

    // The same round trip through the wait dispatch: on the unbound
    // process it takes the direct path (plus its slot machinery).
    c.bench_function("callback/invoke_wait_direct", |b| {
        b.iter(|| {
            let out =
                invoke_wait(black_box(handle), &[Value::I32(ARG)], WAIT).expect("live handle");
            black_box(out)
        })
    });

    assert!(revoke(handle), "cleanup");
}

criterion_group!(benches, bench_callback);
criterion_main!(benches);
