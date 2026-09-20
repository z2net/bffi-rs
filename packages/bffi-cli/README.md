# @z2net/bffi-cli

Bun-only CLI for [bffi-rs](https://github.com/z2net/bffi-rs):
scaffold, build, validate, generate, package and fetch native modules
through the [`@z2net/bffi`](https://github.com/z2net/bffi-rs/blob/main/packages/bffi)
pipeline. **The CLI is a thin wrapper** - every command delegates to
the library; this document describes what each command actually does,
the flags, and the exit-code contract.

- **Bun only** (>= 1.4.2, enforced as the FIRST thing the binary does)
- single runtime dependency: `@z2net/bffi`
- License: MIT ([LICENSE](./LICENSE))

## Install

```sh
bun add -d @z2net/bffi-cli      # installs the `bffi` executable
bunx bffi --help                # or run without installing
```

The `bin` entry is `bin/bffi.ts` with a `#!/usr/bin/env bun` shebang -
npm/bun linking executes it with Bun (a runtime that cannot run
TypeScript sources will fail loudly at the shebang; that is the point
of a bun-only tool).

## Exit codes (the whole CLI lives by this contract)

| Code | Meaning | When |
| --- | --- | --- |
| `0` | success | the command completed |
| `1` | usage error | missing required flags, unknown command; also `--help` |
| `2` | input/environment failure | bad config, missing files, unsupported Bun, no cargo, HTTP/digest failures, unknown triples |

The Bun version gate runs before ANY command: an outdated runtime
prints `bffi requires Bun >= 1.4.2; found <version>` and exits `2` -
the same numeric check `@z2net/bffi` enforces at import.

## Command reference

### `bffi init` - scaffold a project

```
bffi init [--root <dir>] [--module <m>] [--crate-name <n>] [--binary <b>]
```

Creates `.bffi/bffi.json` (defaults: `module = "native"`,
`crate.dir = <module>`, `crate.name = "bffi-<module>"`, `binary =
bffi-<module>` with dashes replaced by underscores) plus a minimal
Rust crate:

- `Cargo.toml` - `cdylib + rlib`, the `emit-json` binary
  (`<binary>_emit_json`), and a single crates.io dependency on
  `bffi` (the facade crate re-exports the macros and the runtime
  ABI);
- `src/lib.rs` - one sample `#[bffi::bffi]` function to replace;
- `src/module_def.rs` - the single `ModuleDef` aggregation (THE
  source of the loader JSON);
- `src/bin/emit_json.rs` - writes `.bffi/bffi.api.json` from the
  aggregation;
- a crate-local `.gitignore` (generated working files stay out of
  git; the config is committed).

Refuses to overwrite an existing config (`2`).

### `bffi build` - compile the crate

```
bffi build [--config <p>] [--root <d>]
```

Spawns `cargo build --release -p <crate.name>` from the project root,
streaming output through. `debug: true` in the config forwards
`BFFI_DEBUG=1` to the subprocess (Rust debug shims skip the
`catch_unwind` boundary - panics abort instead of converting).
Non-zero cargo exit => `2`.

### `bffi check` - validate WITHOUT building

```
bffi check [--config <p>] [--root <d>]
```

Runs four checks, one line each (`ok   name - detail` / `FAIL ...`):

1. **config** - `.bffi/bffi.json` loads and passes schema validation;
2. **loader JSON** - `.bffi/<files[0]>` exists and validates
   (schema v1);
3. **api.gen.ts** - the generated module exists (skipped with
   `generate.apiGen: false`);
4. **artifact** - the cdylib exists at the resolved path
   (`libraryPath` from the config, else
   `<targetDir>/release/[lib]<binary>.<ext>`).

Every check green => `0`; any failure => `2` with the failing
dimension printed.

### `bffi doctor` - check + environment

```
bffi doctor [--config <p>] [--root <d>]
```

Everything `check` does, plus:

- **bun runtime** - the numeric `>= 1.4.2` comparison;
- **cargo** - `Bun.which` FIRST (a missing binary fails fast with
  "not found in PATH" instead of hanging), then `cargo --version`
  through `Bun.spawnSync` with a 15-second timeout (a hung toolchain
  reports "spawn timed out" instead of blocking CI forever).

### `bffi codegen` - loader JSON to typed TS

```
bffi codegen [--config <p>] [--root <d>]
```

Renders `.bffi/api.gen.ts` from the loader JSON: a deterministic
renderer (fixed header, LF, canonicalized field order, schema
embedded as a `const ... satisfies ModuleJson` literal from which
`ApiOf<>` derives exact types). The write is skipped when
byte-identical. Malformed JSON and schema violations exit `2` with
path-precise diagnostics.

### `bffi pack` - assemble one platform package (napi-rs style)

```
bffi pack --src <binary> --triple <t> [--name <base>] [--binary <b>] [--out <dir>]
```

Turns a built cdylib into an npm platform package:

1. reads the CWD `package.json` for `version`/`license`;
2. derives the package name `<base>-<triple>` and the output
   directory `<out>/<scopeless-base>-<triple>`;
3. **renames the binary to the artifact convention**
   `[lib]<binary>.<ext>` (`--binary bffi_mylib`, default: the
   package base with dashes replaced) - this exact name is what
   `resolvePlatformBinary` looks up in `@z2net/bffi`;
4. writes `package.json` carrying `os` / `cpu` / (`libc`) so npm/Bun
   install it ONLY on matching platforms;
5. writes the CJS entry shim `index.js` exporting
   `{ path: __dirname + "/<file>" }` - plain string concat, no
   `node:` imports (the bun-only contract holds even in generated
   code).

Triples: `win32-x64-msvc`, `linux-x64-gnu`, `linux-arm64-gnu`,
`linux-x64-musl`, `linux-arm64-musl`, `darwin-aarch64`,
`darwin-x64`; anything else exits `2` with the shipped list. Missing
`--src` is a usage error (`1`).

A main package then pins every platform package EXACTLY in
`optionalDependencies` (one shared version; publish platform-first).
See [packages/native](https://github.com/z2net/bffi-rs/blob/main/packages/native)
for a published, installable reference and
[`resolvePlatformBinary`](https://github.com/z2net/bffi-rs/blob/main/packages/bffi/README.md)
for the consumer side.

### `bffi fetch` - download a prebuilt binary

```
bffi fetch --repo <owner/repo> --tag <v> --asset <file> [--out <dir>]
```

Downloads `https://github.com/<repo>/releases/download/<tag>/<asset>`
through the global `fetch`; when a `<asset>.sha256` sidecar exists,
the digest is verified with `Bun.CryptoHasher` (mismatch => `2`,
nothing is written). Default output directory: `target/bffi`.
The handler is named `fetchCmd` so it never shadows the global
`fetch` it uses.

## Architecture notes

- `bin/bffi.ts` only routes: the Bun gate, a command table, try/catch
  -> exit codes. All logic lives in `src/` and `@z2net/bffi`.
- File operations are bun-only end to end: `Bun.file` / `Bun.write`
  (`Bun.write` copies from a `BunFile` and creates parent
  directories itself); path joining goes through the pure-string
  `joinOut` from `@z2net/bffi` - the CLI contains zero `node:` module
  imports.
- Tests: `bun test packages/bffi-cli` (arg parsing, check/doctor
  matrices over a temp project, pack assembly, exit codes).

## License

MIT - see [LICENSE](./LICENSE).
