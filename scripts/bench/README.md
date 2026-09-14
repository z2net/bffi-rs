# JS boundary benchmark

Compares the two loaders of the workers example cdylib on
`sum_to(1000n)`:

- **specialized** - the generated module `examples/workers/.bffi/api.gen.ts`
  (`createApiFromJson`: hoisted symbols, preallocated out slots,
  inline argument handling);
- **generic** - `createApi` from `@z2net/bffi` over the same
  `moduleJson`.

Each loader gets 50k warmup calls, then N measured calls; the table
prints total ms, M calls/s and the ratio against the specialized
loader.

## Run

```sh
cargo build --release -p bffi-example-workers   # the cdylib under test
bun scripts/bench/bench.ts                      # default: target/release/<platform name>
bun scripts/bench/bench.ts path/to/cdylib       # explicit cdylib path
bun scripts/bench/bench.ts path/to/cdylib 2000000  # explicit call count (default 1M)
```

The default path is `target/release/bffi_example_workers.dll` on
Windows, `libbffi_example_workers.dylib` on macOS and
`libbffi_example_workers.so` on Linux.
