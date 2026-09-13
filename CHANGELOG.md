# Changelog

All notable changes to bffi-rs are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the versioning is [SemVer](https://semver.org/) (`0.x` may break at
any minor).

## [0.1.2] - unreleased

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
- The repository `AGENTS.md`, `DESIGN.md`, `CALLING-CONVENTION.md`
  (sections 4, 10, 11) and the README document every contract above.

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
