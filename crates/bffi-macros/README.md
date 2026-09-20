# bffi-macros

[![License: MIT](https://img.shields.io/badge/License-MIT-3DA639)](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.98.0-DEA584?logo=rust&logoColor=white)](https://github.com/z2net/bffi-rs/blob/main/rust-toolchain.toml)

The single ABI generator of [bffi-rs](https://github.com/z2net/bffi-rs/blob/main/README.md) -
the Bun-only native binding framework. The `#[bffi]` attribute macro owns everything that crosses
the FFI boundary: no hand-written shims, no drift between the Rust signature and the generated C
ABI (see [docs/DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)).

The macro runs in the USER crate: the expansion lands wherever `#[bffi]` is used and names
the `bffi` facade (`::bffi::{core,types,dts,object,build,r#async}`) in that crate's
namespace (`crate = "direct"` selects the pre-merge roots). The generated shims are
tested directly from Rust - no Bun is needed for this crate's test suite.

**Status:** complete for the documented surface - `#[bffi]` on a plain `fn` emits the function unchanged, an
`extern "C"` shim under the boundary policy, and a const `bffi-dts` descriptor; returns
cover primitives, bigints, buffer payloads (`String`/`Vec<u8>`/`CopiedBuf`, `Option` of
those) and `Result<T, E>` through the err channel. Class declarations (`#[bffi_class]` /
`#[bffi_impl]`) and `#[bffi_async]` / `#[bffi_stream]` live in this same crate
(`src/class/`, `async_fn`, `stream_fn`).

---

## Contents

| Module                                                        | Provides                                                         |
| ------------------------------------------------------------- | ---------------------------------------------------------------- |
| [`src/model.rs`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi-macros/src/model.rs)     | `FnModel` - the parsed, validated function signature             |
| [`src/mapping.rs`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi-macros/src/mapping.rs) | Type classification and TypeScript name mapping                  |
| [`src/errors.rs`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi-macros/src/errors.rs)   | `MacroDiagnostic` - stable `E001`..`E004` compile-time codes     |
| [`src/shim.rs`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi-macros/src/shim.rs)       | `extern "C"` shim codegen under the boundary policy              |
| [`src/meta.rs`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi-macros/src/meta.rs)       | Const `bffi_meta_*` descriptor codegen for `bffi-dts`            |

---

## What `#[bffi]` expands to

One attribute produces exactly three artifacts:

```text
#[bffi]
/// Adds two numbers.
fn add(a: u32, b: u32) -> u32 { a + b }

// 1. The original item, unchanged (docs stay on it).

// 2. The C ABI shim - two cfg variants of one symbol:
#[unsafe(no_mangle)]
pub extern "C" fn bffi_add(a: u32, b: u32, __ret: *mut u32)
    -> ::bffi_core::ErrorCode { /* ... */ }

// 3. The const descriptor consumed by bffi-dts / bffi-build:
pub mod bffi_meta_add {
    pub const FUNCTION: ::bffi_dts::FunctionDef = ::bffi_dts::FunctionDef {
        js_name: "add",
        export_name: "bffi_add",
        docs: &["Adds two numbers."],
        params: &[ /* ParamDef { name, ty } ... */ ],
        ret: ::bffi_dts::TsType::Number,
        abi: ::bffi_dts::AbiSig {
            params: &[ /* one AbiType per parameter ... */ ],
            out: ::std::option::Option::Some(
                ::bffi_dts::AbiOut::Prim(::bffi_dts::AbiPrim::U32),
            ),
        },
    };
}
```

**Build-profile policy (DESIGN §6.5).** The two cfg variants differ only in how the body
is executed:

- `#[cfg(debug_assertions)]` - the body runs bare. A panic escapes and aborts the
  process, keeping stack traces intact while debugging.
- `#[cfg(not(debug_assertions))]` - the body runs inside
  `::bffi_core::boundary::run_extern_body`. A panic unwinds no further: it becomes
  `ErrorCode::Panic` plus a stored thread-local last error.

**Out-param contract.** The return value is written through `__ret: *mut T`, appended
after the parameters; `()` returns have no out-parameter. A null `__ret` returns
`ErrorCode::NullPointer`. Buffer payloads travel as `u64` handles
(`Option`'s `None` writes the `0` handle); `Result`'s error path returns
`ErrorCode::DomainError` (13).

**`&str` conversion (DESIGN §6.3).** A `&str` parameter arrives as a `*const c_char`
pointing at a NUL-terminated cstring (`bun:ffi` convention). The shim null-checks the
pointer, converts with `CStr::from_ptr`, and validates UTF-8 through
`bffi_types::unsafe_zero_copy::str_view` - invalid UTF-8 returns
`ErrorCode::InvalidUtf8`.

**`&[u8]` parameters (borrowed buffers).** A `&[u8]` parameter arrives as a
`ptr: *const u8` + `len: u64` pair (in parameter order). bun:ffi keeps the
`TypedArray` pointer valid for the duration of the call; a null pointer is allowed
only when `len == 0` (empty view), otherwise the shim returns
`ErrorCode::NullPointer` with the last error set. The view is zero-copy and
borrowed: it never outlives the call, so owned returns copy
(`CopiedBuf::from_slice`). The descriptor carries ONE `Uint8Array`
`ParamDef` - the `(ptr, len)` pair is ABI-level only.

**Buffer returns (P2).** `String` / `Vec<u8>` / `CopiedBuf` / `Option` of these are
copied into the [`bffi-build`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/build)
transient-buffer table; the shim returns a handle the JS side reads through the
`bffi_buffer` / `bffi_buffer_length` pair and releases with `bffi_types_free`
(the full ABI contract lives in that crate's CALLING-CONVENTION.md).

**Result err channel (P2).** `Result<T, E>` transports `Ok` like a plain `T`; `Err(e)`
stores `BffiError` with code `DomainError`, message `e.to_string()` and `e` as the
source. `E` must implement `std::error::Error + Send + Sync` - the trait bound
surfaces in the expansion if violated.

**Errors.** Every failure stores a `BffiError` via `::bffi::core::set_last_error` and
returns the matching `ErrorCode`; success returns `ErrorCode::Ok` and stores nothing.

## Attribute options: `crate = "..."`

`#[bffi]` takes one optional option:

- `#[bffi]` - default: the expansion names the [`bffi`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi)
  facade namespaces (`::bffi::core`, `::bffi::types`, `::bffi::dts`,
  `::bffi::build`, `::bffi::r#async`), so a dependency on `bffi`
  alone suffices.
- `#[bffi(crate = "<name>")]` - redirects the same namespaces to
  another facade: the expansion names `::<name>::{core, types, dts,
  build, r#async}`. `crate = "bffi"` is accepted and identical to
  the default.
- `#[bffi(crate = "direct")]` - **pre-merge roots**: the expansion
  names the historical direct dependencies (`::bffi_core`,
  `::bffi_types`, `::bffi_dts`, `::bffi_build`). The pre-merge
  crates are not published; this mode exists for in-workspace
  development.

Anything else (unknown keys, non-literal or invalid `crate` values,
duplicates) is rejected with `E004`.

## Type matrix v2

| Rust type                        | Param | Return | TsType    |
| -------------------------------- | ----- | ------ | --------- |
| `i8` `i16` `i32` `u8` `u16` `u32` `f32` `f64` | yes | yes | `number`  |
| `i64` `u64`                      | yes   | yes    | `bigint`  |
| `bool`                           | yes   | yes    | `boolean` |
| `&str` (borrowed; lifetimes ok)  | yes   | -      | `string`  |
| `&[u8]` (borrowed; lifetimes ok) | yes   | -      | `Uint8Array` |
| `()`                             | -     | yes    | `void`    |
| `String`                         | -     | yes    | `string`  |
| `Vec<u8>`, `CopiedBuf`           | -     | yes    | `Uint8Array` |
| `Option<String>`                 | -     | yes    | `string \| null` |
| `Option<Vec<u8>>`, `Option<CopiedBuf>` | - | yes   | `Uint8Array \| null` |
| `Result<T, E>`                   | -     | yes    | `T`'s kind; `Err` -> code 13 |

Everything else is rejected at compile time - `E002` for parameters, `E003` for returns.
Owned buffers (`String`/`Vec<u8>`) as parameters, structs and non-buffer `Option`/`Vec`
are future work.

Shape violations are rejected too (`E001`): `async`, generic, `unsafe`, method receivers
(`self`), variadic, `extern`, and `const` functions are outside the rules.

## Diagnostics

Rejections carry stable codes - do not renumber, the golden `.stderr` files in
[`tests/ui`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi-macros/tests/ui) lock them:

| Code   | Meaning                                                          |
| ------ | ---------------------------------------------------------------- |
| `E001` | unsupported function shape (async/generic/unsafe/self/variadic/extern/const) |
| `E002` | unsupported parameter type                                       |
| `E003` | unsupported return type                                          |
| `E004` | unknown option; only `crate = "..."` is supported                |

Format - `bffi[<code>]: <message>` first line, then `  = help: ` and `  = note: ` lines:

```text
error: bffi[E002]: unsupported type `Vec < u8 >` for parameter `data`
  = help: supported: i8|i16|i32|i64|u8|u16|u32|u64|f32|f64|bool|&str|&[u8]|()
  = note: borrowed `&[u8]` is the only buffer parameter; owned buffers are return-only (CALLING-CONVENTION.md)
  = note: boundary rules: DESIGN.md (https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)
```

This is the compile-time counterpart of the runtime `BffiError` scheme
(code + message + source, in [`bffi-core`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/core));
mapping runtime errors to JS is [`bffi-error`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/error)'s
job - the macro never depends on it.

## Requirements on the user crate

- Default: a dependency on the [`bffi`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi)
  facade - the expansion names `::bffi::core`, `::bffi::types`,
  `::bffi::dts`, and `::bffi::build` (plus `::bffi::r#async` for
  `#[bffi_async]`), which the facade re-exports.
- `crate = "direct"`: dependencies on the pre-merge `bffi-core`,
  `bffi-types`, and `bffi-dts` - the expansion names `::bffi_core`,
  `::bffi_types`, and `::bffi_dts` at the call site. Buffer and
  `Result` returns additionally name `::bffi_build` - add it when
  those returns are used (or unconditionally). The pre-merge crates
  are not published.
- Unique function names: each shim is `#[unsafe(no_mangle)]`, so two `#[bffi]`
  functions with the same name collide at link time as duplicate symbols.
- Rust **edition 2024**: the generated shims use `#[unsafe(no_mangle)]`, which
  only the 2024 edition and later accept.

## Quick start

In a downstream crate, depend on [`bffi`](https://crates.io/crates/bffi)
(the facade re-exports these macros) and annotate plain functions;
render `.d.ts` from the descriptors (as in
[`tests/descriptor.rs`](https://github.com/z2net/bffi-rs/blob/main/crates/bffi-macros/tests/descriptor.rs)):

```rust
use bffi::dts::{FunctionDef, ModuleDef};

#[bffi::bffi]
/// Adds two numbers.
fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[bffi::bffi]
/// Handles a name.
fn greet(who: &str) -> u32 {
    who.len() as u32
}

static FNS: &[FunctionDef] = &[bffi_meta_add::FUNCTION, bffi_meta_greet::FUNCTION];

let rendered = bffi::dts::render(&ModuleDef { name: "math", fns: FNS });
// /** Adds two numbers. */
// export function add(a: number, b: number): number;
// export function greet(who: string): number;
```

At runtime the JS side links `bffi_add` / `bffi_greet` through `bun:ffi` - the typed
loader ships with the `bffi` crate and the `@z2net/bffi` npm package.

## Testing

```sh
cargo test -p bffi-macros          # everything
```

Suites:

- unit tests per module (model, mapping, errors, shim, meta);
- `tests/ui` - trybuild compile-fail goldens (`.stderr`) locking the `E001`-`E004`
  messages;
- `tests/shim.rs` - direct calls into the generated shims: out-parameter writes, null
  out-pointer, bigint paths, the `&str` cstring path;
- `tests/descriptor.rs` - the const `FunctionDef` matches the expected literal and
  renders through `bffi_dts::render` (the §6.1 integration);
- `tests/panic.rs` - panic becomes `ErrorCode::Panic`; release-only
  (`#![cfg(not(debug_assertions))]`, DESIGN §6.5), exercised by the release job of the
  CI matrix.

## What does _not_ belong here

Per DESIGN §8-9 (one responsibility per module) - the homes are the
feature-gated modules of the [`bffi`](https://crates.io/crates/bffi)
facade:

| Concern                                          | Home                        |
| ------------------------------------------------ | --------------------------- |
| Runtime ABI exports (dealloc, buffer pairs, loader) | `bffi::build`            |
| Event-loop marshalling                           | `bffi::bffi_event_loop`      |
| Object ownership                                 | `bffi::object`               |
| Callbacks                                        | `bffi::bffi_callback`        |
| TS IR (ModuleDef / FunctionDef / render)         | `bffi::dts`                  |

## Requirements

- Rust **1.98.0** (pinned workspace-wide via `rust-toolchain.toml`)
- No Bun is needed to use or test this crate - no Bun e2e in P1, the shims are tested
  from Rust

## License

MIT - see [LICENSE](https://github.com/z2net/bffi-rs/blob/main/LICENSE).
