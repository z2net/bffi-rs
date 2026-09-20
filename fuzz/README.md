# Fuzzing (cargo-fuzz)

libFuzzer targets for the safety-critical byte-level surfaces of the
`bffi` facade.

This directory is a **self-contained cargo workspace**, deliberately
detached from the repository workspace: cargo-fuzz requires a **nightly**
Rust toolchain, while the repository workspace is pinned to 1.98.0 via
`rust-toolchain.toml` at the repo root (which this directory never
touches). `fuzz/rust-toolchain.toml` pins `channel = "nightly"`, and
rustup resolves the closest toolchain file - so every command below,
run from inside `fuzz/`, automatically uses nightly and leaves the
repository pin alone.

## Layout

- `Cargo.toml` - workspace root (single `bffi-fuzz` crate, empty
  `[workspace]` table for isolation) + path dependency on
  `../crates/bffi` (default features);
- `rust-toolchain.toml` - nightly for this directory only;
- `fuzz_targets/` - one `[[bin]]` per target.

## Targets

| Target | Public surface exercised | Invariant |
| ------ | ------------------------ | --------- |
| `wire_decode` | `bffi::types::wire` decoders (`decode_i32`, `decode_i64`, `decode_f64`, `decode_bool`, `decode_str`, `decode_bytes`, `decode_u64`, `decode_u64_lenient`, `decode_number`, `decode_error`, `decode_error_rich`, `decode_record_header`, `decode_seq_header`, `decode_variant`) | never panics, never reads out of bounds, every successful decode advances the offset; decode errors on malformed input are expected and fine |
| `utf8_validate` | `bffi::types::bytes_to_string` (the public entry of the SIMD validator) | the verdict must agree with `std::str::from_utf8` on every input |
| `handle_ops` | `bffi::core::{Handle, HandleTable}` and `bffi::ObjectWrap` (fixed tag) | a deterministic op program of insert/get/remove/contains/wrap/get/release never panics; released handles never resolve again, double release is a clean error |
| `callback_args` | `bffi::bffi_callback::abi::decode_args` (the callback argument decoder, incl. the composite `Wire` walker) | malformed argument buffers surface as clean errors - never a panic, never an out-of-bounds read |

## Run locally

```sh
# one-time
cargo install cargo-fuzz --locked

# from this directory - nightly is picked up from rust-toolchain.toml
cd fuzz
cargo fuzz run wire_decode
cargo fuzz run utf8_validate
cargo fuzz run handle_ops
cargo fuzz run callback_args

# bounded smoke run, the same shape CI uses
cargo fuzz run wire_decode -- -max_total_time=120
```

Crashing inputs land in `artifacts/<target>/`, the growing corpus in
`corpus/<target>/` (both gitignored).

## CI

`.github/workflows/fuzz.yml` runs each target for 120 seconds on a
weekly schedule (plus manual dispatch) as a smoke/continuity check and
uploads crash artifacts on failure. The workflow is intentionally
separate from `ci.yml` and not wired into it, because it needs nightly
while the main workspace stays on the 1.98.0 pin.
