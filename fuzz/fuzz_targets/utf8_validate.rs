#![no_main]

//! Differential target for the SIMD UTF-8 validator
//! (`crates/bffi/src/types/utf8.rs`). The validator itself is
//! `pub(crate)`, so the target drives it through its public entry,
//! `bffi::types::bytes_to_string` (returns Ok exactly when the
//! validator accepts).
//!
//! Invariant: bffi's verdict must equal `std::str::from_utf8`'s verdict
//! on every input; any disagreement is a bug (panic = crash report).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    match bffi::types::bytes_to_string(data) {
        Ok(text) => {
            assert!(
                std::str::from_utf8(data).is_ok(),
                "bffi accepted, std rejected: {data:?}"
            );
            // Copy by default: the accepted copy must carry the exact
            // bytes over.
            assert_eq!(text.as_bytes(), data, "accepted text differs: {data:?}");
        }
        Err(_) => {
            assert!(
                std::str::from_utf8(data).is_err(),
                "bffi rejected, std accepted: {data:?}"
            );
        }
    }
});
