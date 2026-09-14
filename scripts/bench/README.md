# JS boundary benchmark

Compares the two loaders of the reference cdylib (`bffi-native`) on
`add(1, 2)`:

- **specialized** - the generated module
  `crates/bffi-native/.bffi/api.gen.ts` (`createApiFromJson`: hoisted
  symbols, preallocated out slots, inline argument handling);
- **generic** - `createApi` from `@z2net/bffi` over the same
  `moduleJson`.

Each loader gets 50k warmup calls, then N measured calls; the table
prints total ms, M calls/s and the ratio against the specialized
loader.

## Run

```sh
cargo build --release -p bffi-native            # the cdylib under test
bun scripts/bench/bench.ts                      # default: target/release/<platform name>
bun scripts/bench/bench.ts path/to/cdylib       # explicit cdylib path
bun scripts/bench/bench.ts path/to/cdylib 2000000  # explicit call count (default 1M)
```

The default path is `target/release/bffi_native.dll` on Windows,
`libbffi_native.dylib` on macOS and `libbffi_native.so` on Linux.
