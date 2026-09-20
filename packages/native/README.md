# @z2net/bffi-native

The [bffi-rs](https://github.com/z2net/bffi-rs) reference native
module, distributed with the napi-rs-style platform-package
layout: this main package is pure TypeScript; every platform
ships as its own npm package carrying the prebuilt cdylib.
npm/Bun installs only the one matching the running platform
through `optionalDependencies`.

**Bun only** (>= 1.4.2).

## The package family

| Package | Platform | Binary inside (artifact convention) |
| --- | --- | --- |
| `@z2net/bffi-native` | - (this package, pure TS) | - |
| `@z2net/bffi-native-win32-x64-msvc` | Windows x64 | `bffi_native.dll` |
| `@z2net/bffi-native-win32-arm64-msvc` | Windows arm64 | `bffi_native.dll` |
| `@z2net/bffi-native-linux-x64-gnu` | Linux x64 (glibc) | `libbffi_native.so` |
| `@z2net/bffi-native-linux-x64-musl` | Linux x64 (musl) | `libbffi_native.so` |
| `@z2net/bffi-native-linux-arm64-gnu` | Linux arm64 (glibc) | `libbffi_native.so` |
| `@z2net/bffi-native-linux-arm64-musl` | Linux arm64 (musl) | `libbffi_native.so` |
| `@z2net/bffi-native-darwin-aarch64` | macOS arm64 | `libbffi_native.dylib` |
| `@z2net/bffi-native-darwin-x64` | macOS x64 | `libbffi_native.dylib` |

This package pins all eight platform packages in
`optionalDependencies` with EXACT versions (no caret - a loose pin
would let npm pair a JS update with a stale binary). npm/Bun installs
only the entry matching the running `os`/`cpu`; the others are
skipped, which is expected and harmless.

32-bit systems (i686, armv7) are NOT supported: Bun itself ships
64-bit builds only.

## Where the binary comes from

The Rust source is `crates/bffi-native` in the repository - a
minimal cdylib expanding the runtime ABI
(`bffi_build::bffi_runtime_abi!()`) with a deliberately tiny surface
(one function per boundary path):

| Export | Rust | Boundary path |
| --- | --- | --- |
| `add(a, b) -> u32` | primitives | the `__ret` out-parameter |
| `shout(name) -> string` | `&str` -> `String` | cstring parameter in; string return through the transient-buffer pair |
| `version() -> string` | static string | buffer return |

The build matrix
([.github/workflows/release-native.yml](https://github.com/z2net/bffi-rs/blob/main/.github/workflows/release-native.yml),
manual trigger) compiles the crate on all eight triples:
`windows-latest` (msvc x64), `windows-11-arm` (msvc arm64),
`ubuntu-latest` (glibc x64),
`ubuntu-24.04-arm` (glibc arm64), the musl pair on the same
runners (the crates are pure Rust, so musl needs only
`rustup target add` - no extra system packages) and the macOS
pair (`macos-13` builds the x64 side, `macos-latest` arm64); it
uploads the cdylibs; each is then assembled with `bffi pack` from
[@z2net/bffi-cli](https://github.com/z2net/bffi-rs/blob/main/packages/bffi-cli)
- which renames the binary to the artifact convention, writes the
`os`/`cpu`/`libc` package.json and the `{ path }` entry shim.

Publish order matters: **platform packages first, then this main
package** - a main release with missing platform pins is the
classic napi-rs-style distribution breakage.

## Usage

```ts
import { createNative } from "@z2net/bffi-native";

const native = await createNative(); // resolves + dlopens the platform binary

native.add(3, 4);        // => 7
native.shout("bffi");    // => "HELLO bffi!"
native.version();        // => "0.2.0"
```

What `createNative()` does:

1. `resolvePlatformBinary("@z2net/bffi-native", { binary: "bffi_native" })`
   (from `@z2net/bffi`) resolves the installed platform package and
   returns the absolute binary path - the artifact convention makes
   this a pure lookup, no package code has to execute; the async
   return carries the integrity verification (Bun's file readers are
   async);
2. `createApiFromJson(path)` (the generated
   [api.gen.ts](https://github.com/z2net/bffi-rs/blob/main/packages/native/src/api.gen.ts))
   dlopens the library and builds the typed API object.

An explicit library path overrides resolution (tests, locally built
artifacts):

```ts
const native = await createNative("D:/path/to/bffi_native.dll");
```

## Rebuilding locally

```sh
cargo build --release -p bffi-native
bunx @z2net/bffi-cli pack --src target/release/bffi_native.dll \
  --triple win32-x64-msvc --name @z2net/bffi-native
```

The loader JSON (`crates/bffi-native/.bffi/bffi.api.json`) is emitted
by the crate's `emit-json` binary from the single `ModuleDef`
aggregation; `packages/native/src/api.gen.ts` is generated from it.

## License

MIT - see [LICENSE](./LICENSE).
