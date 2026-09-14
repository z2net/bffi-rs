# bffi-rs

<div align="center">

[![Bun](https://img.shields.io/badge/Bun-%3E%3D1.4.0-F472B6?logo=bun&logoColor=white)](https://bun.sh)
[![Rust](https://img.shields.io/badge/Rust-1.98.0-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-3DA639?logo=opensourceinitiative&logoColor=white)](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
[![GitHub Issues](https://img.shields.io/github/issues/z2net/bffi-rs)](https://github.com/z2net/bffi-rs/issues)
[![GitHub Pull Requests](https://img.shields.io/github/issues-pr/z2net/bffi-rs)](https://github.com/z2net/bffi-rs/pulls)

**English** | [Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/README.md) | [简体中文](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/zh-CN/README.md)

</div>

Binding framework for Bun - a napi-rs-equivalent for [Bun](https://bun.sh), built on `bun:ffi` and a thin C ABI. Written in Rust, bottom-up from small focused crates.

See [docs/DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md) for architecture and [AGENTS.md](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md) for the project's engineering rules.

## Documentation

- [docs/DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md) - architecture & decisions
- [crates/bffi/CALLING-CONVENTION.md](https://github.com/z2net/bffi-rs/blob/main/crates/bffi/CALLING-CONVENTION.md) - the C ABI contract (every crossing, callback exports included)
- [docs/BINDING-GUI.md](https://github.com/z2net/bffi-rs/blob/main/docs/BINDING-GUI.md) - binding GUI / event-driven libraries (wry et al.) from Bun
- [docs/CONTRIBUTING.md](https://github.com/z2net/bffi-rs/blob/main/docs/CONTRIBUTING.md) - how to contribute (branching, commits, PRs)
- [AGENTS.md](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md) - engineering rules for humans and AI agents
- [packages/bffi](https://github.com/z2net/bffi-rs/blob/main/packages/bffi) - `@z2net/bffi`: the typed loader + build pipeline (see its README)
- [packages/bffi-cli](https://github.com/z2net/bffi-rs/blob/main/packages/bffi-cli) - `@z2net/bffi-cli`: the `bffi` CLI (init, build, check, doctor, codegen, pack, fetch)
- [packages/native](https://github.com/z2net/bffi-rs/blob/main/packages/native) - `@z2net/bffi-native`: the reference native module (platform npm package family)
- [bffi-examples](https://github.com/z2net/bffi-examples) - example modules, each doubling as an e2e suite (sqlite, records, streams, errors, async, event-loop, callbacks, workers, wry)
- [SECURITY.md](https://github.com/z2net/bffi-rs/blob/main/SECURITY.md) - security policy
- [CONTACT.md](https://github.com/z2net/bffi-rs/blob/main/CONTACT.md) - contacts

## Requirements

- [Bun](https://bun.sh) >= 1.4.0 (enforced at runtime by `@z2net/bffi` and the `bffi` CLI)
- Rust 1.98.0 (pinned via `rust-toolchain.toml`; rustup installs it automatically)
- bash (for the commit-msg hook; preinstalled on macOS/Linux, Git Bash on Windows)

## Components

| Part | Purpose |
| ---- | ------- |
| `crates/bffi` | **The published crate** ([crates.io/crates/bffi](https://crates.io/crates/bffi)): the whole stack as feature-gated modules + the facade + macros re-exports |
| `crates/bffi-macros` | The proc-macro crate ([crates.io/crates/bffi-macros](https://crates.io/crates/bffi-macros)): `#[bffi]`, `#[bffi_async]`, `#[bffi_class]`, `#[bffi_impl]`, `#[bffi_constructor]` |
| `crates/bffi-native` | The reference cdylib (`add`/`shout`/`version` + runtime ABI); source of the `@z2net/bffi-native` platform package family |
| `packages/bffi` | Bun-only JS integration package: config, full pipeline (build → json → api.gen), typed loader (npm: `@z2net/bffi`) |
| `packages/bffi-cli` | The `bffi` CLI: init, build, codegen, pack, fetch, check, doctor (npm: `@z2net/bffi-cli`) |

## Getting started

```sh
bun install          # installs dependencies + git hooks (lefthook)
bun run build        # builds the reference cdylib (release)
bun run test:js      # runs the package unit tests (bun test packages)
bun run check        # oxlint + tsc + cargo check
bun run ci           # full CI parity: lint, typecheck, fmt, clippy, tests, JS tests
```

The internal modules (core, types, error, object, dts, build,
callback, event-loop, async) live inside `crates/bffi/src/` as
feature-gated modules - the published crate `bffi` is the single
dependency covering the whole stack. The crates.io packages are
published from this repository; see
[crates/bffi](https://github.com/z2net/bffi-rs/blob/main/crates/bffi)
and the [crates.io page](https://crates.io/crates/bffi).

## Generated TypeScript API

The `#[bffi]` descriptors are the single source of truth: the crate's
`emit-json` binary writes `.bffi/bffi.api.json` (schema v1) from the
aggregated `ModuleDef`, and the `@z2net/bffi` pipeline does the rest -
validate the manifest against the binary (ABI version + exports hash
handshake runs BEFORE your first call), generate `.bffi/api.gen.ts`,
resolve and `dlopen` the library. Deterministic bytes, safe to commit
and diff.

The generated factory emits one SPECIALIZED wrapper per function,
method and field - the native symbol and the out-slot are hoisted,
argument encoding and result decoding are inlined, and every
signature is annotated against the embedded schema so `tsc` checks
the generated file with the exact types you consume.

```ts
import { bffi } from "@z2net/bffi";
import type { Api } from "./.bffi/api.gen.ts";

const api: Api = await bffi();       // one call: build -> json -> gen -> dlopen
api.add(1, 2);                       // number, typed; errors throw JS Errors
const counter = new api.counter(10); // classes: FinalizationRegistry + release()
await api.compute(21);               // `#[bffi_async]` -> Promise
const sample = await api.report(7n); // records: Promise<Report> / Sample / Sample[] | null

// The dispose protocol (Bun executes `using` natively):
{
  using stream = api.numbers(10);    // streams release on dispose
  // ...
} // classes, callbacks and the Api itself dispose the same way
```

Composites cross the boundary as wire-encoded buffers, copy by
default: `#[derive(BffiRecord)]` structs, `#[derive(BffiEnum)]`
unit enums, `Vec<T>` sequences (including `Vec<Vec<u8>>`), and
`Option<Record>` / `Option<Vec<T>>` returns rendering as
`| null` (async: `Promise<Sample>`, `Promise<number[]>`).

## Typed errors

Domain errors derive `BffiError` with stable user codes in the
reserved range `0x1000..=0xFFFF`; the code replaces the framework
status in the ABI return and surfaces as `e.code` on the JS side,
with `e.name` (the variant), `e.payload` (the variant fields) and
`e.nativeStack` (a `RUST_BACKTRACE`-gated backtrace - keep it off in production) alongside. See [SECURITY.md](SECURITY.md) for the trust model: the `.bffi/` manifest is trusted input, `libraryPath` is an explicit trust decision, and bffi provides panic CONTAINMENT (not process isolation).

```rust
#[derive(BffiError, Debug)]
pub enum UsersError {
    #[bffi(code = 0x1001)]
    NotFound { id: u64 },
    #[bffi(code = 0x1002)]
    InvalidAge { age: u32, min: u32 },
}

#[bffi]
pub fn find_user(id: u64) -> Result<User, UsersError> { ... }
```

```ts
try {
  api.find_user(99n);
} catch (e) {
  e.code;    // 0x1001 (4097)
  e.name;    // "NotFound"
  e.payload; // [99n]
}
```

The pipeline, its config (`.bffi/bffi.json`) and every subtlety are
documented in
[`packages/bffi`](https://github.com/z2net/bffi-rs/blob/main/packages/bffi);
full worked examples live in
[bffi-examples](https://github.com/z2net/bffi-examples).

## Examples

The example modules live in their own repository,
[bffi-examples](https://github.com/z2net/bffi-examples) - each one is
a standalone crate there and doubles as an e2e suite against the
published packages (the repo's CI runs the suites): sqlite (the full
pipeline over rusqlite, the entry example), records, streams, errors,
async, event-loop, callbacks, workers and wry.

## Benchmarks

JS boundary throughput of the reference library `bffi-native`,
`add(1, 2)`, 1M calls per loader after a 50k warmup (`cargo build
--release -p bffi-native`, then `bun scripts/bench/bench.ts`),
measured on a GitHub Actions ubuntu-latest runner:

| Loader | Calls/s | vs specialized |
| ------ | ------- | -------------- |
| specialized (`api.gen.ts` via `createApiFromJson`) | ~172M | 1.00x |
| generic (`createApi` over the same module JSON) | ~11M | 0.07x |

The specialized codegen is the default since 0.1.3. Rust-side
criterion benches for the wire codec and the handle registry live in
`crates/bffi/benches/` (`cargo bench -p bffi`). The numbers come from
the `bench` workflow (manual dispatch on GitHub Actions runners) -
never from a maintainer's machine.

## Conventions

- Conventional Commits are enforced by a `commit-msg` hook (`scripts/commit-msg.sh`).
- Pre-commit runs oxlint, `tsc --noEmit`, `cargo fmt --check` and clippy.
- Pre-push runs the workspace tests.
- GitHub Actions CI (`.github/workflows/ci.yml`) runs on every pull request and on pushes to `main` / `dev/main` (Rust matrix: ubuntu / windows / macos, plus a JS job); `bun run ci` remains the local parity command.

## License

[MIT](https://github.com/z2net/bffi-rs/blob/main/LICENSE)
