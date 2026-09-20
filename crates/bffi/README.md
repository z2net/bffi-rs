# bffi

[![Crates.io](https://img.shields.io/crates/v/bffi)](https://crates.io/crates/bffi)
[![License: MIT](https://img.shields.io/badge/License-MIT-3DA639)](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.98.0-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)

The single Rust crate of [bffi-rs](https://github.com/z2net/bffi-rs) -
a native-binding framework for [Bun](https://bun.sh), the `napi-rs`
equivalent built on `bun:ffi` and a thin C ABI.

Write a Rust module, annotate it, and import fully typed functions
from TypeScript. One dependency, feature-gated:

```toml
[dependencies]
bffi = "0.1.0"
```

## Quick start

```rust
use bffi::bffi;

/// Adds two numbers.
#[bffi]
pub fn add(a: u32, b: u32) -> u32 {
    a.wrapping_add(b)
}
```

The `#[bffi]` macro generates the C ABI shim (status + out-parameter,
copy by default, panics converted to JS errors) plus a const
descriptor consumed by the `@z2net/bffi` TypeScript loader - no
hand-written bindings anywhere. The generated paths target this
crate's namespaces (`::bffi::core`, ...) out of the box; an optional
`crate = "<name>"` redirects them to another facade, and
`crate = "direct"` selects the pre-merge crate roots.

## Features

| Feature | Unlocks | Enables |
| --- | --- | --- |
| `core` *(default)* | generational handles, Registry, boundary policy | - |
| `types` *(default)* | conversions, SIMD UTF-8, the wire codec | `core` |
| `error` *(default)* | `BffiError` -> JS Error mapping | `core` |
| `dts` *(default)* | descriptor IR + renderers | - |
| `object` *(default)* | `ObjectWrap<T>` ownership | `core` |
| `build` *(default)* | runtime ABI exports, loader JSON, d.ts emitters | core/types/error/dts |
| `callback` *(default)* | two-direction callbacks + generic ABI | core/types/build |
| `event-loop` *(default)* | the job queue (`pump`/`run`/`marshal`) | core/callback |
| `async` *(default)* | Rust futures as JS Promises | core/types/build/event-loop |
| `stream` *(default)* | Rust iterators as JS async iterators (pull-chunk) | core/types/build |
| `macros` *(default)* | `#[bffi]`, `#[bffi_async]` (dep: `bffi-macros`) | core/types/build/dts |
| `class` *(default)* | `#[bffi_class]`, `#[bffi_impl]` | macros/object |
| `tokio` | poll async tasks on a tokio runtime | async |

Everything is on by default; trim with `default-features = false` +
the slices you need.

## Async

Annotate an `async fn` and await it from JavaScript as a `Promise`:

```rust
use bffi::bffi_async::sleep;
use std::time::Duration;

/// Doubles after a short delay.
#[bffi::bffi_async]
pub async fn double_async(x: u64) -> u64 {
    sleep(Duration::from_millis(15)).await;
    x * 2
}
```

Cancellation is cooperative, timeouts are first-class combinators,
and tokio is an opt-in executor (`features = ["tokio"]`).

## Streams

Annotate an iterator-returning fn and consume it with `for await`:

```rust
/// Streams `1..=count`.
#[bffi::bffi_stream]
fn numbers(count: u32) -> impl Iterator<Item = i32> + Send {
    (1..=count).map(|n| n as i32)
}
```

Slow producers use the push shape - an async fn over a `Ctx<T>`
channel with backpressure (`ctx.push(item).await` parks the
producer while the JS side has not drained):

```rust
#[bffi::bffi_stream]
async fn readings(ctx: bffi::stream::Ctx<f64>, count: u32) -> Result<(), MyError> {
    for i in 0..count {
        bffi::sleep(Duration::from_millis(5)).await;
        ctx.push(f64::from(i)).await?;
    }
    Ok(())
}
```

Items ride the shared wire codec (numbers, strings, bytes, exact
`u64`, records/enums, `Result` items as `Error` values).

## Classes

```rust
use bffi::{bffi_class, bffi_impl, bffi_constructor};

/// A counter.
#[bffi_class(tag = 0x0150)]
pub struct Counter {
    pub value: u32,
}

#[bffi_impl]
impl Counter {
    /// Creates a counter.
    #[bffi_constructor]
    pub fn new(start: u32) -> Self {
        Self { value: start }
    }
}
```

The JS side gets a constructor with methods and a `release()`
(`FinalizationRegistry` covers GC).

## The JS side

The TypeScript integration ships separately:
[`@z2net/bffi`](https://www.npmjs.com/package/@z2net/bffi) (the
pipeline + typed loader), [`@z2net/bffi-cli`](https://www.npmjs.com/package/@z2net/bffi-cli)
(scaffold/build/publish) and
[`@z2net/bffi-native`](https://www.npmjs.com/package/@z2net/bffi-native)
(the published reference module). One JS call takes a crate from
`cargo build` to a fully typed API - see the
[bffi-rs repo](https://github.com/z2net/bffi-rs) for the complete
documentation and examples.

## Safety model

- Copy by default across the FFI boundary; zero-copy only through
  the explicit `bffi::unsafe_zero_copy` door.
- Generational handles + type tags: a stale handle can never reach a
  reused slot.
- Panics never cross as undefined behavior: release builds convert
  them into JS errors.
- The public API is 100% safe.

## Minimum Bun version

The TypeScript side requires Bun >= 1.4.0 (enforced at runtime).

## License

MIT - see [LICENSE](https://github.com/z2net/bffi-rs/blob/main/LICENSE).
