# @z2net/bffi

Bun-only typed loader and build pipeline for
[bffi-rs](https://github.com/z2net/bffi-rs) native modules. You write
a Rust `cdylib`, annotate its functions with `#[bffi]` / `#[bffi_async]`
/ class macros, and this package turns it into a fully typed
TypeScript API - no per-function bindings written by hand, ever.

```
Rust crate  ──cargo build──►  cdylib (.dll/.so/.dylib)
    │
    └── emit-json ──►  .bffi/bffi.api.json   (schema v1: every export, typed)
                              │
@z2net/bffi  ─────────────────┼──  validate ──► .bffi/api.gen.ts (typed TS)
                              └──  dlopen ────►  typed Api object
```

- **Bun only** (>= 1.4.2, enforced at import time - see
  [Version gate](#version-gate)); no Node.js or Deno support.
- **Zero runtime dependencies.** Everything is built on `bun:ffi`,
  `Bun.file`/`Bun.write`, `Bun.resolveSync` and standard Web APIs.
- License: MIT ([LICENSE](./LICENSE)).

```sh
bun add @z2net/bffi
```

The quickest possible look - the [sqlite
example](https://github.com/z2net/bffi-examples/tree/main/sqlite):

```ts
import { bffi } from "@z2net/bffi";
import type { Api } from "./.bffi/api.gen.ts";

const api: Api = await bffi();          // build -> json -> gen -> resolve -> dlopen
api.exec(api.open(":memory:"), "SELECT 1");
```

The rest of this document explains every step and every subtlety of
that arrow chain.

---

## 1. The mental model

The framework splits a native module into **two artifacts plus one
config**:

| Artifact | Producer | Consumer |
| --- | --- | --- |
| the **cdylib** (`bffi_mylib.dll` / `libbffi_mylib.so` / `.dylib`) | `cargo build --release -p <crate>` | `bun:ffi.dlopen` (inside this package) |
| the **loader JSON** (`.bffi/bffi.api.json`, schema v1) | the crate's `emit-json` binary, from the one explicit `ModuleDef` aggregation | validation + codegen + dlopen declarations |
| the **config** (`.bffi/bffi.json`) | you (or `bffi init` from `@z2net/bffi-cli`) | the pipeline: what to build, what to generate, where the binary lands |

Every export declared in the loader JSON crosses the C ABI in ONE
uniform shape:

```
u32 bffi_<name>(<params in ABI order>, __ret: *mut <slot>) -> ErrorCode
```

- the return is ALWAYS the status code (`0` = ok);
- the actual result travels through the trailing `__ret` OUT-PARAMETER
  (buffer/string/task returns return a `u64` HANDLE in `__ret`, not
  the value);
- a non-zero status means the thread-local LAST ERROR holds a
  readable `BffiError` - drained through the `bffi_error_*` exports.

This uniformity is what lets ONE generic loader drive ANY module:
the JS side never needs per-function knowledge beyond the JSON.

## 2. The one-call pipeline

```ts
const api: Api = await bffi({ config: ".bffi/bffi.json" });
// individual steps are exported too: bffiBuild / bffiGenerate
```

`bffi()` executes five steps; each is idempotent and re-runnable:

1. **Locate the project root** - an explicit `config` path wins;
   otherwise the working directory is walked UPWARDS until a
   directory containing `.bffi/bffi.json` is found.
2. **`cargo build --release -p <crate.name>`** - spawned with
   `stdout/stderr: "inherit"` from the project root (so a workspace
   `target/` is shared between examples). Skipped entirely with
   `{ skipBuild: true }`. When the config sets `debug: true`, the
   subprocess receives `BFFI_DEBUG=1` (Rust-side debug shims run
   WITHOUT the `catch_unwind` boundary - panics abort and are easy to
   debug; release shims convert panics into `ErrorCode::Panic`).
3. **Read + validate the loader JSON** (`.bffi/bffi.api.json` by
   default; configurable through `crate.files[0]`). Schema version
   mismatch (`bffi !== 1`) is a hard error.
4. **Generate `.bffi/api.gen.ts`** - a deterministic render of the
   schema (fixed header, LF endings, canonicalized field order; the
   write is SKIPPED when byte-identical so module caches stay warm).
   The generated module imports `@z2net/bffi` BY PACKAGE NAME and
   exports `createApiFromJson(libraryPath)` + the `Api` type.
5. **Resolve + dlopen** - `<crate.targetDir>/release/[lib]<binary>.<ext>`
   (extension per running platform, `config.os` may override), then
   `dlopen` with declarations built from the schema + the
   `features` block, and build the typed API object.

Live walkthrough:
[sqlite](https://github.com/z2net/bffi-examples/tree/main/sqlite).

## 3. The config (`.bffi/bffi.json`), field by field

| Field | Type | Meaning |
| --- | --- | --- |
| `bffi` | `1` | schema version - hard requirement |
| `version` | string | the stack version (informational) |
| `module` | string | module name; lands in the generated file header and the schema |
| `os` | `"auto" \| "win32" \| "linux" \| "darwin"` | binary-resolution override (`auto` = running platform) |
| `libc` | `"auto" \| "glibc" \| "musl"` | Linux libc override for platform-package resolution (Alpine: `musl`) |
| `debug` | boolean | forwards `BFFI_DEBUG=1` to cargo + verbose logs |
| `bun` / `rust` | string | executables for spawned subprocesses (`bun`, `cargo`) |
| `crate.dir` | string | crate directory relative to the root (`"."` when the crate IS the root) |
| `crate.name` | string | cargo package name (the `-p` flag) |
| `crate.binary` | string | cdylib base name WITHOUT extension/lib prefix (`bffi_mylib`) |
| `crate.targetDir` | string? | relative target dir - set it when a WORKSPACE relocates `target/` (monorepos: `"../../target"`) |
| `features` | object? | which built-in export groups the crate actually expands: `runtime` (default true), `async`, `callbacks` - **must match the `*_abi!()` macros the crate expands**, because `bun:ffi` throws at dlopen on any declared-but-missing symbol |
| `generate.apiGen` / `generate.outFile` | boolean / string | codegen on/off (default on) and the output file (`api.gen.ts`) |
| `files` | string[] | the working files inside `.bffi` (`[bffi.api.json, api.gen.ts]`); `files[0]` is the loader JSON the pipeline reads |
| `libraryPath` | string \| null | explicit dlopen path - skips resolution entirely (tests, prebuilt platform packages) |

A real config:
[sqlite/.bffi/bffi.json](https://github.com/z2net/bffi-examples/tree/main/sqlite/.bffi/bffi.json).

## 4. How ONE call crosses the boundary

Take `#[bffi] pub fn open(path: &str) -> Result<u64, SqliteError>`
(the sqlite example,
[lib.rs](https://github.com/z2net/bffi-examples/tree/main/sqlite/src/lib.rs)).
The macro generates the shim `bffi_open(path_ptr: *const c_char,
__ret: *mut u64) -> u32`; the loader generates the JS wrapper. A call
then flows:

1. **Arguments are encoded.** Each schema parameter carries an `abi`
   name; the encoder maps JS values onto `bun:ffi` argument kinds:
   - `bool` -> `u8` (`true` = 1) - bun:ffi has no native bool;
   - `ptr_len` (`&[u8]` / `Vec<u8>`) -> TWO arguments (`ptr`, `len`);
     an EMPTY `Uint8Array` becomes `null` + `0` because bun:ffi
     REJECTS empty TypedArrays as pointers;
   - `cstring` (`&str` / `String`) -> passed verbatim; strings are
     UTF-8 by contract, bun:ffi does the NUL termination;
   - `i64`/`u64` -> `bigint` (exact; never through `number`);
   - everything else (`i8..i32`, `u8..u32`, `f32`, `f64`) -> `number`.
   The argument COUNT is validated against the schema first
   (`open: expected 1 argument(s), got 2`).
2. **The symbol is invoked** through the untyped dlopen table; every
   accessor goes through `sym()` so a missing export is a loud
   "missing export: bffi_open", never an `undefined` call.
3. **Status check.** Non-zero => `takeError()` drains the thread-local
   last error and THROWS it. The drained `Error` carries:
   - `message` - the Rust `Display` text;
   - `cause` - the optional domain SOURCE string (the `E` of
     `Result<T, E>`);
   - the JS constructor is chosen by `bffi_error_name`
     (`1` Error / `2` TypeError / `3` RangeError).
   A non-zero status WITHOUT a stored error still throws
   (`<export> failed: <code>`), never silently passes.
4. **The out-slot is read** and transported by the RETURN ABI:
   - `void` -> `undefined`;
   - primitive slots (`i32`, `u64`, ...) -> `number` / `bigint`;
   - `buffer` (String / Vec<u8> / CopiedBuf returns) -> the `__ret`
     handle is READ IMMEDIATELY: `bffi_buffer_length` +
     `bffi_buffer` copy the bytes into a fresh JS `Uint8Array` and
     `bffi_types_free` releases the native slot (copy-before-free;
     the JS side never holds native memory). `string` returns are
     `TextDecoder`-decoded; `string | null` / `Uint8Array | null`
     map an EMPTY payload to `null` (an empty payload is
     indistinguishable from `Option::None` - a documented v1
     limitation);
   - `task` (async returns) -> the `u64` handle goes to
     [`wrapTask`](#7-async-tasks-promises-and-the-pump) and the call
     returns a `Promise`.

Type mapping table (schema `ts`/`abi` -> JS):

| Rust | `ts` | `abi` | JS type |
| --- | --- | --- | --- |
| `i8..i32`, `u8..u32`, `f32`, `f64` | `number` | width | `number` |
| `i64`, `u64` | `bigint` | width | `bigint` |
| `bool` | `boolean` | `bool` (`u8` on the wire) | `boolean` |
| `&str`, `String` | `string` | `cstring` / `buffer` | `string` |
| `&[u8]`, `Vec<u8>`, `CopiedBuf` | `Uint8Array` | `ptr_len` / `buffer` | `Uint8Array` |
| `Option<String>` / `Option<CopiedBuf>` | `string \| null` / `Uint8Array \| null` | `buffer` | `string \| null` / `Uint8Array \| null` |
| async fns | `Promise<T>` | `task` | `Promise<T>` |
| classes | - | - | constructor + methods + `release()` (a `FinalizationRegistry` releases the native handle on GC; `release()` releases early and unregisters) |

## 5. The runtime exports every crate must expand

The crate (not this package!) expands `bffi_build::bffi_runtime_abi!()`,
which emits the ten symbols the loader needs on EVERY module:

- `bffi_error_take_last` / `bffi_error_name` / `bffi_error_message_ptr`
  / `bffi_error_message_len` / `bffi_error_cause_ptr` /
  `bffi_error_cause_len` / `bffi_error_free` - the error drain
  (take MOVES the thread-local error into a readable slot; free
  releases it; pointers are valid only until free);
- `bffi_buffer` / `bffi_buffer_length` - the transient-buffer pair;
- `bffi_types_free` - the shared release for both tables.

They are declared automatically (the `runtime` feature, default on).

## 6. The wire codec

Callbacks and async payloads share ONE byte format -
`[tag: u8][payload]`, little-endian
(`bffi_types::wire` on the Rust side):

| Tag | Value | Payload |
| --- | --- | --- |
| `Unit` | 0 | (none) |
| `I32` | 1 | 4 bytes LE |
| `I64` | 2 | 8 bytes LE, exact bigint - no `f64` narrowing |
| `F64` | 3 | 8 bytes LE |
| `Bool` | 4 | 1 byte |
| `Str` | 5 | `u32` LE length + UTF-8 bytes |
| `Bytes` | 6 | `u32` LE length + raw bytes |

Encoding side: an integral `number` fitting i32 encodes as `I32`,
otherwise `F64`; a `bigint` encodes as `I64` (out-of-range throws);
an empty argument list is the single-byte Unit record. Decoding side:
`Bytes` are COPIED out (the source buffer is transient).

## 7. Async: tasks, Promises and the pump

`#[bffi_async]` on the Rust side generates a spawn shim returning a
`u64` TASK HANDLE; the schema marks the return as `task`, and the
loader wraps every handle into a `Promise` via `wrapTask(lib, task)`:

- two bun:ffi `JSCallback`s are created - the resolver `(u64) -> void`
  and the rejector `(cstring) -> void` (JSCallback does not know
  `"undefined"`, so `void` it is);
- `bffi_async_attach(task, resolvePtr, rejectPtr, __ret)` registers
  the pair; the second attach on one task is rejected
  (`InvalidArgument`, "already attached");
- when the Rust future completes, the value is wire-encoded into a
  transient buffer and a job is enqueued onto the EVENT LOOP; the job
  runs on the JS thread during a drain and calls the trampolines;
  the promise settles and the payload is decoded + freed.

**The pump contract.** Bun's tick cannot be hooked from native code,
so deliveries happen ONLY while the JS side drains the loop. The
loader never starts a hidden interval; instead:

```ts
import { pumpUntil } from "@z2net/bffi";
await pumpUntil(api.compute(5), () => api.loopPump());
// loopPump is YOUR export over bffi_event_loop::pump()
```

`pumpUntil` calls `pump()` after every macrotask until the promise
settles. A live, complete demonstration (values, strings, domain
errors, panics, timeouts, cancellation, the raw-handle path):
[async](https://github.com/z2net/bffi-examples/tree/main/async).

Cancellation is COOPERATIVE: `bffi_async_cancel` drops the future at
its next poll boundary (blocking code inside is not interrupted) and
rejects the attached promise with "task cancelled".

## 8. Callbacks: both directions, one thread rule

The generic callback ABI (four exports the crate expands via
`bffi_callback::bffi_callback_abi!()`) plus four JS helpers:

| Helper | Direction | What it does |
| --- | --- | --- |
| `setJsThread(lib)` | - | registers the CALLING thread as a JS thread (multi-isolate; idempotent). `bffi()` AUTO-BINDS the loading isolate when the module ships the callback surface - the manual call is only needed for advanced setups |
| `unsetJsThread(lib)` | - | deregisters the CALLING thread (a Worker calls it before exiting; idempotent) |
| `bindJsCallback(lib, sig, fn)` | Rust -> JS | wraps `fn` into a `JSCallback` (`sig.ret`/`sig.params` are `"i32" \| "i64" \| "f64" \| "bool" \| "cstring"`), stores the pointer under a fresh handle; returns `{ handle, revoke() }` |
| `invokeCallback(lib, handle, ...args)` | JS -> Rust | wire-encodes `args`, invokes the native body registered by the crate, decodes the result |
| `revokeCallback(lib, handle)` | both | terminal revocation - a dead handle never resurrects; the second revoke throws |

As of 0.2.0 the value kinds crossing the callback boundary are
`i32`, `i64`, `u64` (exact, a non-negative `bigint` in JS even above
`i64::MAX`), `f64`, `bool` and `string` (a `cstring` both directions;
a returned pointer is call-scoped and copied out immediately).
Composites (arrays, records) cannot cross the raw JSCallback call -
`invokeCallback` carries them through the wire channel in the
JS -> Rust direction.

Subtleties worth knowing:

- **The thread gate.** While the process is UNBOUND every caller is
  admitted; once `setJsThread` ran anywhere, invocations from other
  threads are rejected with `WrongThread (12)`. There is no unbind.
- **`invoke_wait` crosses the gate - by marshalling, not by
  breaking it.** The native side can call a bound callback from ANY
  thread: the call is enqueued onto the bffi event loop, the
  calling thread parks with a mandatory timeout
  (`ErrorCode::Timeout = 15`), and the callback executes on the JS
  thread during the pump. This is how GUI-library handlers (wry et
  al.) reach JavaScript - see
  [docs/BINDING-GUI.md](https://github.com/z2net/bffi-rs/blob/main/docs/BINDING-GUI.md)
  and `bffi::invoke_wait` in
  [CALLING-CONVENTION.md section 9.1](https://github.com/z2net/bffi-rs/blob/main/crates/bffi/CALLING-CONVENTION.md).
- **Revocation while waiting is safe.** `invoke_wait` re-checks the
  handle inside the marshal job: a revoked handle surfaces as
  `InvalidHandle` through the slot, never a dead-pointer call.
- **Signature mismatches** (arity or tag shape) surface as
  `InvalidArgument (11)` with an "expected i32(i32), got ..." message.
- Booleans cross callbacks as `u8` (`1`/`0`) - same as the ABI.

Both directions live in
[callbacks](https://github.com/z2net/bffi-examples/tree/main/callbacks).

## 9. The event loop

`enqueue` (always works until stopped), `marshal` (requires a RUNNING
drain - otherwise `WrongThread`), `pump` (non-blocking drain),
`run` (blocking drain for a dedicated thread), `stop` (sticky). This
package does not drive the loop for you: async deliveries arrive
while YOU pump (see §7). The mechanics, including the exactly-once
guarantee and the sticky stop:
[event-loop](https://github.com/z2net/bffi-examples/tree/main/event-loop).

## 10. Platform packages (napi-rs style)

Prebuilt binaries ship as per-platform npm packages, napi-rs style:

```
@scope/mylib                          (pure TS; pins every platform below EXACTLY in optionalDependencies)
@scope/mylib-win32-x64-msvc           (bffi_mylib.dll;        os: win32,  cpu: x64)
@scope/mylib-linux-x64-gnu            (libbffi_mylib.so;      os: linux,  cpu: x64,  libc: glibc)
@scope/mylib-linux-x64-musl           (libbffi_mylib.so;      os: linux,  cpu: x64,  libc: musl)
@scope/mylib-linux-arm64-gnu          (libbffi_mylib.so;      os: linux,  cpu: arm64, libc: glibc)
@scope/mylib-darwin-aarch64           (libbffi_mylib.dylib;   os: darwin, cpu: arm64)
@scope/mylib-darwin-x64               (libbffi_mylib.dylib;   os: darwin, cpu: x64)
```

npm/Bun installs only the package matching `os`/`cpu`/`libc`
(optional dependencies can be skipped - that is expected and must
not break the install). Resolution:

```ts
import { resolvePlatformBinary } from "@z2net/bffi";
const path = await resolvePlatformBinary("@scope/mylib", { binary: "bffi_mylib" });
// Bun.resolveSync("@scope/mylib-<triple>", from) -> entry's directory
// -> <dir>/[lib]<binary>.<ext>                   (the ARTIFACT CONVENTION)
```

- the triple of the running platform: `win32-x64` -> `win32-x64-msvc`,
  `linux-x64` -> `linux-x64-gnu` (`libc: "musl"` swaps the suffix),
  `darwin-arm64` -> `darwin-aarch64`, `darwin-x64` -> `darwin-x64`;
- the binary INSIDE the platform package follows the artifact
  convention `[lib]<binary>.<ext>` - `bffi pack`
  ([@z2net/bffi-cli](https://github.com/z2net/bffi-rs/blob/main/packages/bffi-cli))
  renames the cdylib accordingly;
- everything missing/not-installed is one loud error with the
  package name, never a silent wrong-path dlopen.

A published, installable reference:
[packages/native](https://github.com/z2net/bffi-rs/blob/main/packages/native)
(`@z2net/bffi-native` + seven platform packages).

## 11. The version gate

`engines.bun` in package.json is advisory; the library ENFORCES
`>= 1.4.2` at import: `assertBunVersion()` runs in the public entry
and throws `@z2net/bffi requires Bun >= 1.4.2; found <version>`.
The comparison is numeric (so `1.10.0` > `1.4.2`) and intentionally
avoids `Bun.semver` (which postdates the older runtimes the gate
rejects). The CLI prints the same message and exits `2`.

## 12. Subtle rules worth memorizing

1. **Every result crosses via an out-parameter**; the C return is
   always the status. A declared arity mismatch at the JS call site
   throws before the FFI call.
2. **Copy-by-default, everywhere.** Returned bytes are copied before
   the native free; `&[u8]` parameters borrow for the call only.
   (A deliberate zero-copy door exists on the Rust side,
   `bffi::unsafe_zero_copy`, and is never the default.)
3. **Empty `Uint8Array` arguments travel as a NULL pointer with
   `len == 0`** - bun:ffi rejects empty TypedArrays as pointers.
4. **Empty payload == `Option::None`**: a `null`-able return maps an
   empty buffer to `null`. v1 cannot distinguish "empty Some" from
   "None" - do not rely on it.
5. **Strings are UTF-8, canonical.** bun:ffi cstrings handle the NUL
   termination; invalid UTF-8 is Rust-side `InvalidUtf8`.
6. **64-bit integers are `bigint`** - they never pass through
   `number`, on the ABI or in the wire codec.
7. **`JSCallback` `returns` is `"void"`** - bun:ffi does not know
   `"undefined"`.
8. **`bool` crosses as `u8`** on every layer (ABI, wire, callbacks).
9. **Errors are thread-local.** The last error is drained by the
   thread that observed the non-zero status; take MOVES it out
   (the second take gets `null`).
10. **The pump is the delivery driver** - without a drain, completed
    async tasks stay queued forever (by design; no hidden timers).

## 13. Package layout and subpath exports

```
src/
├── index.ts      the 1:1 re-export of all barrels + the version gate
├── runtime/      wire codec, error drain, buffers, wrapTask/pumpUntil,
│                 callback helpers, streams, dispose, version gate
│                                                       -> "@z2net/bffi/runtime"
├── loader/       schema types, buildDeclarations, createApi/ApiOf,
│                 platform resolution               -> "@z2net/bffi/loader"
├── pipeline/     config v1, cargo build step, bffi() orchestrator
│                                                       -> "@z2net/bffi/pipeline"
└── codegen/      schema validation + the deterministic renderer
                                                          -> "@z2net/bffi/codegen"
```

`main`/`types` point at `src/index.ts` - TypeScript sources are
shipped as-is (Bun executes TS natively). The tests
(`bun test packages/bffi`) cover the codec round-trips, declaration
building, resolution and the API factory over a mock symbol table.

## 14. See it live

- [sqlite](https://github.com/z2net/bffi-examples/tree/main/sqlite)
  - the pipeline end-to-end on a real workload;
- [async](https://github.com/z2net/bffi-examples/tree/main/async)
  - Promises, timeouts, cancellation, the pump;
- [event-loop](https://github.com/z2net/bffi-examples/tree/main/event-loop)
  - the queue/drains/marshal mechanics;
- [callbacks](https://github.com/z2net/bffi-examples/tree/main/callbacks)
  - both callback directions, the thread gate, marshal delivery;
- [records](https://github.com/z2net/bffi-examples/tree/main/records)
  - records, enums, sequences, `Option` fields and returns;
- [streams](https://github.com/z2net/bffi-examples/tree/main/streams)
  - pull and push producers, backpressure, wake-driven delivery;
- [errors](https://github.com/z2net/bffi-examples/tree/main/errors)
  - typed errors with user codes (`e.code` / `e.name` / `e.payload`);
- [wry](https://github.com/z2net/bffi-examples/tree/main/wry)
  - a webview window driven from Bun - the GUI-binding reference
    ([docs/BINDING-GUI.md](https://github.com/z2net/bffi-rs/blob/main/docs/BINDING-GUI.md)).
