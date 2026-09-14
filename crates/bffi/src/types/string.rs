//! UTF-8 string conversion, copy by default (DESIGN §6.3).
//!
//! Byte sequences handed over from the JS side (e.g. a `cstring` argument)
//! are valid only for the duration of the call; [`bytes_to_string`]
//! therefore **copies** into an owned [`String`] after validating UTF-8.
//! The zero-copy counterpart lives behind [`super::unsafe_zero_copy`].

// Internal module aliases (the pre-merge crate names).
use super::utf8;
use crate::bffi_core;
use bffi_core::{BffiError, ErrorCode};

fn invalid_utf8() -> BffiError {
    BffiError::new(ErrorCode::InvalidUtf8, "byte sequence is not valid UTF-8")
}

/// Validates `bytes` as UTF-8 and returns an owned copy.
///
/// Copy by default: the result never aliases `bytes`, so the caller may
/// free or reuse the source buffer immediately after the call.
///
/// # Errors
///
/// [`ErrorCode::InvalidUtf8`] (as [`BffiError`]) when the sequence is not
/// valid UTF-8.
pub fn bytes_to_string(bytes: &[u8]) -> Result<String, BffiError> {
    if !utf8::validate(bytes) {
        return Err(invalid_utf8());
    }
    let copy = bytes.to_vec();
    // SAFETY: `utf8::validate` - the SIMD UTF-8 validator in
    // `super::utf8` (SSSE3/NEON with a scalar DFA fallback, semantics
    // identical to `std::str::from_utf8`) - returned `true` above for
    // exactly these bytes, and `copy` is a byte-for-byte copy of them,
    // so `copy` is valid UTF-8.
    Ok(unsafe { String::from_utf8_unchecked(copy) })
}

/// Copies a `&str` into an owned byte vector.
///
/// The symmetric partner of [`bytes_to_string`]: the result never aliases
/// the input, ready to cross the boundary (e.g. handed to the JS side with
/// transferred ownership).
#[must_use]
pub fn string_to_bytes(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}
