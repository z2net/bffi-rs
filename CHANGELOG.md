# Changelog

All notable changes to bffi-rs are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the versioning is [SemVer](https://semver.org/) (`0.x` may break at
any minor).

## [0.1.3] - unreleased

### Multi-isolate JS threads (Bun 1.4 workers)

- The single process-global JS-thread slot is now a REGISTRATION
  TABLE: every JS isolate (the main script and each `Worker`) binds
  its own thread with `set_js_thread()`; registrations are
  idempotent and die with the thread.
- The event loop keeps the global queue for untargeted jobs and adds
  per-isolate slot queues (`enqueue_to`): JS-bound callback waits,
  stream wakes and async resolver deliveries are TARGETED to the
  owning isolate - a job never crosses an isolate boundary. A
  departed worker fails fast (LoopStopped) instead of hanging.
- New explicit shutdown half: `unset_js_thread()` /
  `unsetJsThread()` / the `bffi_callback_unset_thread` export. A
  Worker calls it before exiting (a TERMINATED worker thread never
  runs TLS destructors on Windows).
- New example `examples/workers`: two isolates on one native
  library, concurrent native calls, `invoke_wait` cross-isolate
  delivery with a proof the callback ran in the worker, and the
  graceful-death path.

### Specialized codegen (hot path)

- Generated modules emit one wrapper per function/method/field/
  constructor with the symbol and out-slot TypedArray hoisted, exact
  positional parameters and inlined encode/decode - micro
  benchmark of a single out-slot call: **3.8x** (8.8M -> 33.7M
  calls/s).
- Emitted signatures are annotated through `TsOf<...>` against the
  embedded schema literal, so `tsc` type-checks the generated
  factory with the same `ApiOf` mapping consumers see.
- Classes emit real class bodies (`handle`, `release()`, getters,
  methods) instead of runtime-built wrappers.

### Dispose protocol and ergonomics

- `Symbol.dispose` on class instances (= `release()`), on
  `bindJsCallback` results (= idempotent revoke + JSCallback close -
  the trampoline used to leak), on stream iterators (= early drop +
  wake-trampoline close) and on the whole Api (= retires every
  JSCallback trampoline of the library through a per-lib registry).
- `streamToWeb()`: adapter from a bffi stream into a native
  `ReadableStream` (pipelines with CompressionStream and friends);
  cancelling the reader releases the native stream early.
- `installMemoryPressureGC()`: hooks the Bun 1.4
  `process.on("memoryPressure")` event to run a synchronous GC pass,
  driving FinalizationRegistry finalizers so native handles release
  before the OS applies pressure.

### Platforms

- New platform package `@z2net/bffi-native-win32-arm64-msvc` (Bun
  1.4 ships native Windows ARM64; the CI matrix and the
  optionalDependencies pins include it).
- 32-bit systems (i686, armv7) are explicitly unsupported - Bun
  itself ships 64-bit builds only; the resolver error says so.

### Compatibility notes

- Multiple JS isolates are now first-class; code that relied on "the
  first binder wins forever" should call `unsetJsThread()` on
  shutdown or treat later registrations as additional workers.
- Generated modules no longer import `createApi`; the generic
  factory remains available as `createApi`/`createApiFromLib` for
  hand-rolled loaders.

## [0.1.2] - unreleased

### Release alignment

- The npm family (`@z2net/bffi`, `@z2net/bffi-cli`,
  `@z2net/bffi-native` and the seven platform packages) is aligned
  at one version: 0.1.2; `bun.lock` re-synced (the platform
  `optionalDependencies` pins move 0.1.0 -> 0.1.2 - publish order
  is platform packages first).

### Breaking

- `#[bffi]` / `#[bffi_async]` / class macros without options emit
  facade paths (`::bffi::{core, types, dts, object, build, r#async}`);
  `crate = "bffi"` is accepted (identical to the default),
  `crate = "direct"` selects the pre-merge roots.
- `E` in `Result<T, E>` must satisfy `Into<BffiError>`; the shim
  returns the converted error's `status_u32()` (u32), and the
  exported status of every sync/class shim is the raw `u32`.
- `ModuleDef` grew the `records`, `enums` and `errors` tables -
  every `ModuleDef` literal needs the new fields.
- `#[derive(BffiError)]` variants carry user codes `0x1000..=0xFFFF`
  that replace the framework `13 (DomainError)` in the ABI status.

### Added

- **B1 composites**: `#[derive(BffiRecord)]` / `#[derive(BffiEnum)]`,
  wire tags `TAG_RECORD (7)` / `TAG_SEQ (8)`, record/enum/`Vec<T>`
  parameters and returns, `ModuleDef.records` / `ModuleDef.enums`,
  TS interfaces and enum unions in `.d.ts`, `ApiOf` resolution and
  the `examples/records` e2e suite.
- **B2 streams**: `#[bffi_stream]` in both shapes - pull
  (`impl Iterator<Item = T> + Send`) and push (`async fn(ctx:
  Ctx<T>, ...)`, bounded 256, backpressure) - as a JS
  `AsyncIterableIterator<T>`; wire `TAG_U64 (10)` / `TAG_ERROR (11)`;
  `ErrorCode::Pending (14)`; `Result` items arrive as `T | Error`;
  `examples/streams` e2e.
- **B3 typed errors**: `#[derive(BffiError)]` (user codes, variant
  name, payload record, `RUST_BACKTRACE`-gated backtrace), the rich
  `TAG_ERROR` envelope, `e.code` / `e.name` / `e.payload` /
  `e.nativeStack` on the JS side (best-effort), `ModuleDef.errors`
  and the loader-JSON `errors` table; `examples/errors` e2e.
- **B4 composites completion**: `Vec<Vec<u8>>` (`Uint8Array[]`),
  `Option<Record>` / `Option<Vec<T>>` returns (`| null` over the
  0-handle convention), push `ctx.push(Ok/Err)` pinned by e2e, and
  async composite returns (`Promise<Sample>`, `Promise<number[]>`)
  via `AsyncValue::Wire`.
- **A1 async Option**: `Option<T>` async returns in every shape
  (`Option` of buffers, records, sequences; `Result<Option<T>, E>`);
  `None` maps to `null` across nine `Promise<... | null>` descriptor
  spellings.
- **A2 stream wake**: the `bffi_stream_set_wake` ABI export
  (best-effort) fires the JS wake trampoline through the event loop
  when a push producer delivers into an empty buffer or completes -
  a pull waiting on `Pending` resumes instantly; the bounded timer
  stays as the lost-wake-up fallback.
- **A3 stress e2e**: mixed concurrency (24 parallel tasks with a
  failure and a timeout) and interleaved push streams.
- **Phase 3.5 `invoke_wait` + JS-bound handles**: the marshalled
  callback invocation (any native thread, bounded timeout) now
  dispatches both callback tables - natively registered closures
  and JS-bound callbacks (`bffi_callback_bind`); `Value` grows
  `Unit`/`Str`/`Bytes`. The `examples/wry` webview binding drops
  its forwarder workaround and calls the JS-bound handler
  directly.
- **Phase 3 `examples/wry`**: a full webview window driven from
  Bun - wry 0.57 + winit 0.30 pinned, the window and event loop
  on a dedicated native thread, IPC round trips through
  `invoke_wait`, graceful shutdown, window-free Rust tests plus
  an env-gated real-window e2e (`BFFI_WRY_E2E=1`).
- **Phase 4 guide**: `docs/BINDING-GUI.md` - the threading model,
  the `invoke_wait` contract, IPC round trips, keep-alive and
  shutdown, packaging notes (WebView2 / GTK / WKWebView).
- The repository `AGENTS.md`, `DESIGN.md`, `CALLING-CONVENTION.md`
  (sections 4, 9.1, 10, 11) and the README document every contract
  above.

### Fixed

- Stream shims return the `u32` status under `catch_unwind` (the
  E-contract change had left the release path on `ErrorCode`).
- The loader-JSON golden regeneration now uses the correct test
  target name (a dash-vs-underscore typo had silently skipped the
  overwrite step).
- Async `Result` err channel converts through `Into<BffiError>` -
  derived user codes no longer flatten into a `Display` message.

## [0.1.1] - 2026-08

- crates.io release of `bffi` / `bffi-macros` (the facade crate
  covering the whole stack), CI feature-slice checks, doctor
  hardening.

## [0.1.0] - 2026-07

- First public cut: the `#[bffi]` / `#[bffi_async]` /
  `#[bffi_class]` macro family, the runtime ABI (`bffi_error_*`,
  the buffer pair), the typed loader + codegen pipeline
  (`@z2net/bffi`), the `bffi` CLI, the platform-package flow and
  the reference native module family.
