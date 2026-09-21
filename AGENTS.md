# AGENTS.md - Rules for AI agents and contributors

[English](https://github.com/z2net/bffi-rs/blob/main/AGENTS.md) | [Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/AGENTS.md) | [简体中文](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/zh-CN/AGENTS.md)

This file defines how humans and AI agents must work on **bffi-rs**.

Repository: https://github.com/z2net/bffi-rs  
Contact: contact@z2net.com

---

## 1. Project purpose

`bffi-rs` is a **Bun-only** native binding framework written in Rust.

It is the Bun equivalent of `napi-rs`, but:

- targets **only Bun** (no Node.js / Deno compatibility);
- does **not** depend on Node-API;
- uses `bun:ffi` and a thin C ABI layer;
- is organized as a **bottom-up module stack** (foundation modules first, facade last) inside a small 3-crate workspace.

Primary goals: safety at the FFI boundary, clear ownership, good DX, and long-term maintainability.

Read `docs/DESIGN.md` before making architectural changes.

---

## 2. Hard rules

1. **Bun only**  
   Do not add Node.js or Deno compatibility layers.

2. **Safety first**
   - Default path = copy data, never zero-copy.
   - Zero-copy is allowed only via `bffi::unsafe_zero_copy`.
   - All `extern "C"` functions must be thin and wrapped with `catch_unwind`.

3. **Handles**  
   Use Generational Index + type-tag (`u64`).  
   Never expose raw Rust references or complex types across the C ABI.

4. **Panics**
   - Dev builds may abort (easier debugging).
   - Prod builds must convert panics into JS `Error`.

5. **Minimum Bun version**  
   `1.4.2` (the 1.4.1 Windows `bun:ffi` JIT fix and the 1.4.2 musl/long-running-process fixes)

6. **Rust / Cargo version**  
   Project is pinned to **Cargo / Rust 1.98.0**.  
   Do not bump without an explicit decision and CI update.

7. **No secrets in the repo**  
   Anything under `.grok`, `.claude`, `.codex`, `.opencode`, `.zcode`, `.hermes`, `.mcp`, `.mimosa`, `.env`, keys, tokens, etc. must stay out of git (see `.gitignore`).

8. **License**  
   MIT. Keep SPDX headers where appropriate.

---

## 3. Repository structure

```
bffi-rs/
├── AGENTS.md                    # this file
├── README.md
├── LICENSE
├── SECURITY.md
├── CONTACT.md
├── CHANGELOG.md
├── Cargo.toml                  # workspace (3 members)
├── deny.toml                   # cargo-deny / cargo-audit policy (CI supply-chain job)
├── rust-toolchain.toml         # pinned 1.98.0
├── package.json                # Bun workspace / scripts
├── bun.lock
├── tsconfig.json
├── .oxlintrc.json              # linter config
├── lefthook.yml                # git hooks (lint, fmt, commit-msg)
├── .gitattributes              # golden-file diff policy
├── .gitignore
├── .github/
│   ├── ISSUE_TEMPLATE/
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── workflows/              # ci, fuzz, bench, release-native, release-crates, release-npm
├── crates/
│   ├── bffi/                    # the runtime stack as layered modules (core, types, error,
│   │                            #   object, callback, dts, build, event_loop, async, stream)
│   │                            #   + the public facade; CALLING-CONVENTION.md lives here
│   ├── bffi-macros/             # all proc-macros: #[bffi], #[bffi_async], #[bffi_stream],
│   │                            #   #[bffi_class]/#[bffi_impl], derives (BffiRecord/BffiEnum/
│   │                            #   BffiError); src/support/ = shared macro internals,
│   │                            #   src/class/ = the class family
│   └── bffi-native/             # reference cdylib (runtime ABI; -> @z2net/bffi-native packages)
├── fuzz/                        # self-contained cargo-fuzz workspace (nightly; fuzz.yml)
├── docs/
│   ├── DESIGN.md                # architecture & decisions
│   ├── BINDING-GUI.md           # GUI / event-driven libraries guide
│   ├── CONTRIBUTING.md
│   ├── CODE_OF_CONDUCT.md
│   └── i18n/                    # ru / zh-CN translations (README, DESIGN, AGENTS, ...)
├── packages/                      # JS-side: bffi (@z2net/bffi), bffi-cli, native
└── scripts/                      # commit-msg hook + bench driver

Examples live in a separate repository:
https://github.com/z2net/bffi-examples (each example is a standalone
crate and an e2e suite against the published packages).
```

The runtime stack is kept as small, single-responsibility modules
(`bffi_core`, `bffi_types`, `bffi_error`, `bffi_object`, `bffi_callback`,
`bffi_dts`, `bffi_build`, `bffi_event_loop`, `bffi_async`, `bffi_stream`)
inside `crates/bffi/src/`, layered bottom-up with the `bffi` facade on
top. New crates must follow the naming scheme `bffi-*` and be added to
the workspace; a new module must keep the bottom-up layering and its
feature gate.

---

## 4. Development workflow

### Setup

```bash
# Rust
rustup toolchain install 1.98.0
rustup default 1.98.0

# Bun
bun install
```

### Common commands

```bash
bun run lint          # oxlint
bun run typecheck     # tsc
bun run build         # builds the reference cdylib (release)
bun run test:js       # runs the package unit tests (bun test packages)
bun run ci            # full CI parity: lint, typecheck, fmt, clippy, tests, JS tests
cargo check
cargo test
cargo fmt
cargo clippy
```

### Commit style

We use **Conventional Commits**:

```
feat: add generational handle table
fix: prevent panic across FFI boundary
docs: update DESIGN.md decisions
refactor(core): simplify catch_unwind helper
test: cover buffer copy path
chore: pin rust-toolchain to 1.98.0
```

Breaking changes must use `BREAKING CHANGE:` in the footer or `!` after the type.

### Branching and releases

- `main` - production branch; PRs into `main` are created only by the project owner, from `dev/main`.
- `dev/main` - integration branch; all feature work lands here via PRs.
- Features are developed in `dev/<feature>` branches (kebab-case), cut from and merged back into `dev/main`.
- PR `dev/<feature>` → `dev/main` requires 1 approval and green CI (`.github/workflows/ci.yml`; `bun run ci` locally).
- Release tags `v<semver>` (annotated) are placed only on `main`, only by the owner.

Full rules: [docs/CONTRIBUTING.md](https://github.com/z2net/bffi-rs/blob/main/docs/CONTRIBUTING.md) → "Branching and releases".

### Pull requests

- One logical change per PR.
- CI must pass.
- Update docs when behavior or public API changes.
- Reference related issues.

---

## 5. Rules for AI agents

When working on this repository an agent **must**:

1. Read `DESIGN.md` and this file before large changes.
2. Prefer small, reviewable diffs.
3. Never commit secrets, personal AI configs, or `.env` files.
4. Not introduce Node/Deno compatibility.
5. Keep the layered architecture: foundation modules first, the `bffi` facade last; `bffi_*` module boundaries inside `crates/bffi` must not be bypassed.
6. Preserve the safety model (copy by default, explicit unsafe zero-copy, generational handles).
7. Run `cargo fmt`, `cargo clippy`, and tests when possible.
8. Update `DESIGN.md` or docs if a decision changes.

When unsure about architecture, prefer asking (or opening a draft PR) instead of inventing a new pattern.

---

## 6. Contact

- Issues & discussions: GitHub
- Direct contact: **contact@z2net.com**

---

## 7. Quick reference - accepted decisions

| Topic         | Decision                                     |
| ------------- | -------------------------------------------- |
| Macro         | `#[bffi]`: shim (debug bare / release catch_unwind) + bffi_meta_* descriptor |
| `#[bffi]` returns | primitives/bigints via out-param; `String`/`Vec<u8>`/`CopiedBuf` (and `Option` of those) as buffer handles; `Result<T, E>` -> DomainError(13) |
| Min Bun       | 1.4.2                                        |
| Rust/Cargo    | 1.98.0                                       |
| Handles       | Generational Index + type-tag                |
| Error format  | `BffiError` = code + message + source; domain errors convert losslessly via `From` |
| Boundary strings | UTF-8 canonical (`bun:ffi cstring`)          |
| Table locking | Lock-free; hazard-pointer reclamation        |
| UTF-8 checks  | SIMD (x86 SSSE3, aarch64 NEON); scalar ref   |
| Buffers       | Copy by default                              |
| Zero-copy     | Only via `bffi::unsafe_zero_copy`            |
| Event loop    | `run()` drains blocking; `pump()` drains non-blocking; `marshal` = wrong-thread path (code 12) |
| TS types      | IR (ModuleDef/FunctionDef/ClassDef) + deterministic render; export_name = bffi_-prefix |
| Class macros | `#[bffi_class]`/`#[bffi_impl]` over ObjectWrap (tags 0x0100-0x01FF): field getters, `&self` methods, generated release; metadata split bffi_meta_<name> + bffi_meta_<name>_impl::CLASS; E005-E008 |
| Macro support | `bffi-macros::support`: shared model/mapping/codegen internals of the proc-macro crate (no runtime code, no ABI) |
| Panic (prod)  | Convert to JS Error                          |
| Panic (dev)   | May abort                                    |
| Compatibility | Bun only                                     |
| License       | MIT                                          |
| Facade        | `bffi`: flat re-exports of the stack; `unsafe_zero_copy` is the only zero-copy door; macro expansions name `::bffi::{core,types,dts,object,build,r#async}` by default (`crate = "<name>"` redirects, `crate = "direct"` selects the pre-merge roots) |
| Async         | `#[bffi_async]`: spawn shim returns a task handle; N-worker executor; cooperative cancel + timeout; resolve via event-loop enqueue; tokio opt-in; tags 0x0500-0x05FF; composite returns ride the wire channel (`Promise<Record>` / `Promise<Vec<T>>` via `AsyncValue::Wire`); `E: Into<BffiError>` contract |
| Streams       | `#[bffi_stream]` (B2): pull (`impl Iterator<Item = T> + Send`) or push (`async fn(ctx: Ctx<T>, ...)`, bounded 256, backpressure) as a JS `AsyncIterableIterator<T>`; generic `bffi_stream_next(handle, max)` (TAG_SEQ buffer, 0 = done; 14 = Pending retry) + `bffi_stream_drop` + `bffi_stream_set_wake` (event-loop wake trampoline, best-effort); tag 0x0600; push producers deliver `Result` items (`ctx.push(Ok/Err)`) |
| Object ownership | `ObjectWrap<T>` over global `Registry` (tag 0x0100-0x01FF); release frees the slot |
| Callbacks | `register`/`revoke` + `bind_js_callback`; tags 0x0200-0x0201; wrong-thread reject; `invoke_wait` marshals a callback onto the JS thread from ANY native thread with a mandatory timeout (`Timeout = 15`) - both tables (native closures and JS-bound handles) |
| Build ABI | Runtime exports (`bffi_error_*`, `bffi_buffer` pair, `bffi_types_free`) via `bffi_runtime_abi!()` in the user crate; tags 0x0400-0x04FF; canonical contract: bffi/CALLING-CONVENTION.md |
| Descriptor ABI | `AbiSig` (exact C widths + out slot) on `FunctionDef`/`MethodDef`; getter `export_name` + out on `FieldDef`; `release_export` on `ClassDef` |
| Wire codec | `bffi::types::wire`: one `[tag][payload]` table for async payloads and callback sigs/args/results |
| Composites (B1+B4) | Records/enums/`Vec<T>` (incl. `Vec<Vec<u8>>`) sync + async; `Option<Record>`/`Option<Vec<T>>` returns = `| null` over the 0-handle empty-buffer convention; item/field matrix rejects deeper nesting |
| Option parameters (sync) | `Option<&str>` (NULL cstring), `Option<&[u8]>` (ptr+len+flag triple), `Option<prim>` (`f64`+flag), `Option<i64/u64>` (width+flag), `Option<record/Vec<T>>` (`len == 0`); ABI names `opt_number`/`opt_i64`/`opt_u64`/`opt_ptr_len`; nested `Option` and async paths stay rejected |
| Typed errors | `#[derive(BffiError)]`: user codes 0x1000-0xFFFF replace status 13; variant = JS `e.name`, fields = `e.payload` (TAG_RECORD); rich accessors best-effort; loader JSON `errors` table |
| Callback ABI | Generic exports via `bffi_callback_abi!()` (`bffi_callback_set_thread`/`_bind`/`_invoke`/`_revoke`) in the user crate; wire-encoded; CALLING-CONVENTION.md §9 |
| Loader JSON | `bffi::build::loader_json`: canonical deterministic schema v1 from the aggregated `ModuleDef` |
| TS API codegen | `bun bffi codegen <json> -o <ts>`: deterministic renderer; embeds the schema literal; `ApiOf<>` derives exact types over `packages/bffi` |
| Platform distribution | napi-rs-style platform npm packages (optionalDependencies exact pins, `bffi pack`, resolvePlatformBinary) |
| Reference native module | `crates/bffi-native` -> `@z2net/bffi-native` platform package family |
