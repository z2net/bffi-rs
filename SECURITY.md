# Security Policy

[English](https://github.com/z2net/bffi-rs/blob/main/SECURITY.md) | [Русский](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/ru/SECURITY.md) | [简体中文](https://github.com/z2net/bffi-rs/blob/main/docs/i18n/zh-CN/SECURITY.md)

## Supported versions

| Version | Supported |
| --- | --- |
| 0.1.x | security fixes on the latest `0.1.x` release |

`0.0.x` releases and older `0.1.x` minors do not receive fixes -
upgrade to the latest patch.

## Reporting a Vulnerability

- Preferred: GitHub **private vulnerability reporting**
  (Security tab -> Report a vulnerability).
- Email: **contact@z2net.com**

Please include: description, steps to reproduce, potential impact,
affected version.

We aim to acknowledge within 72 hours (24h for reports rated
Critical). Fixes are released as coordinated disclosures - please do
not disclose publicly until a fix ships. Credit is given on request.

## Scope

- Memory safety bugs in the FFI boundary
- Handle table corruption / type confusion
- Cross-module handle confusion (registry identity)
- Panic propagation issues / the `catch_unwind` boundary policy
- Codegen bugs in the generated shims (`bffi-macros`) that could
  hide `unsafe`
- **Loader manifest integrity**: a tampered `.bffi/bffi.api.json`
  vs a built cdylib (ABI handshake bypass), platform-package
  integrity verification bypass
- **Path traversal** in artifact resolution (`libraryPath`,
  `crate.dir`, platform package resolution)
- Malformed wire payloads: integer overflow, allocation DoS via
  declared lengths, unbounded nesting
- Race conditions, use-after-free, callback deadlocks (including
  re-entrant `invoke_wait`)
- Supply chain of the release pipeline (release-npm.yml /
  release-native.yml): artifact substitution, provenance gaps

Out of scope:
- Bugs in Bun itself (report to oven-sh/bun)
- Bugs in the Rust toolchain
- A native module behaving maliciously once loaded: loading a
  cdylib is arbitrary code execution BY DESIGN - the loader
  handshake protects against accidental staleness and tampered
  manifests, not against a hostile binary you chose to load

## Trust model (short version)

- The `.bffi/` config and manifest are TRUSTED INPUT: never commit
  or load a `.bffi/bffi.json` you did not review. `libraryPath`
  bypasses platform resolution and is an explicit trust decision
  (`trust: "explicit"`).
- bffi provides **panic containment** (release shims convert Rust
  panics into JS errors where possible), NOT **process isolation**:
  a native module runs inside the Bun process. Memory corruption,
  `abort`, segfaults or allocator damage in native code can take
  down the host regardless.
- `e.nativeStack` is gated behind `RUST_BACKTRACE` - keep it off in
  production (it leaks paths and code structure).
- The release `panic = "unwind"` profile is REQUIRED: `panic =
  "abort"` in a module's `[profile.release]` breaks the
  containment policy and aborts the host (`bffi doctor` checks
  this).
