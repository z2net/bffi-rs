//! **Unsafe-by-name zero-copy views - read every word of this module.**
//!
//! Everything else in this crate copies (DESIGN §6.3). Zero-copy is only
//! allowed through this module, and the module name is the warning label:
//! a value produced here borrows the caller's buffer instead of owning a
//! copy, which is exactly the dangerous direction.
//!
//! # Contracts
//!
//! A `ZeroCopyStr` / `ZeroCopyBuf` view:
//!
//! 1. borrows memory the *caller* controls - it must not outlive the FFI
//!    call that produced the buffer. The `'a` lifetime enforces this at
//!    compile time as long as the raw pointer is only dereferenced inside
//!    the boundary layer (`bffi-build`) for the duration of the call;
//! 2. aliases memory that JS may legally keep mutating between calls -
//!    never store a view in Rust state, never spawn a thread with it;
//! 3. still validates UTF-8 for [`str_view`] - zero-copy means *no copy*,
//!    never *no checks*.
//!
//! The constructors here are safe because they take a borrowed slice; the
//! genuinely unsafe step (turning a raw `(ptr, len)` pair from the ABI
//! into a `&[u8]`) lives in `bffi-build`, immediately above this module.

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
use bffi_core::{BffiError, ErrorCode};
use std::ops::Deref;

/// A borrowed `&str` view over caller-owned UTF-8 bytes - no copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ZeroCopyStr<'a>(&'a str);

/// A borrowed `&[u8]` view over caller-owned bytes - no copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ZeroCopyBuf<'a>(&'a [u8]);

/// Borrows `bytes` as a UTF-8 string view without copying.
///
/// The view still borrows `bytes` - only the UTF-8 validation is now
/// *enforced* (checked [`str::from_utf8`]) instead of assumed: the
/// view is rejected up front when the bytes are not valid UTF-8.
///
/// # Errors
///
/// [`ErrorCode::InvalidUtf8`] (as [`BffiError`]) when the bytes are not
/// valid UTF-8 - the check is mandatory in the zero-copy path too.
pub fn str_view(bytes: &[u8]) -> Result<ZeroCopyStr<'_>, BffiError> {
    std::str::from_utf8(bytes)
        .map(ZeroCopyStr)
        .map_err(|_| BffiError::new(ErrorCode::InvalidUtf8, "byte sequence is not valid UTF-8"))
}

/// Borrows `bytes` as a byte view without copying (infallible).
#[must_use]
pub fn buf_view(bytes: &[u8]) -> ZeroCopyBuf<'_> {
    ZeroCopyBuf(bytes)
}

impl ZeroCopyStr<'_> {
    /// The borrowed string.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0
    }
}

impl Deref for ZeroCopyStr<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl ZeroCopyBuf<'_> {
    /// The borrowed bytes.
    #[must_use]
    pub const fn as_slice(&self) -> &[u8] {
        self.0
    }
}

impl Deref for ZeroCopyBuf<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{ZeroCopyStr, buf_view, str_view};
    use crate::bffi_core::ErrorCode;

    #[test]
    fn str_view_rejects_invalid_utf8() {
        let invalid: &[&[u8]] = &[
            &[0xFF],                   // invalid lead byte
            &[0x80],                   // lone continuation
            &[0xC3],                   // truncated two-byte sequence
            &[0xED, 0xA0, 0x80],       // surrogate
            &[0xF4, 0x90, 0x80, 0x80], // beyond U+10FFFF
        ];
        for bytes in invalid {
            let error = str_view(bytes).expect_err("invalid utf-8 must be rejected");
            assert_eq!(error.code, ErrorCode::InvalidUtf8, "for {bytes:?}");
        }
    }

    #[test]
    fn str_view_borrows_valid_utf8_without_copy() {
        let source = "🚀 views borrow".as_bytes().to_vec();
        let view = str_view(&source).expect("valid utf-8");
        assert_eq!(view.as_str(), "🚀 views borrow");
        assert_eq!(view.as_str().len(), source.len());
        let typed: ZeroCopyStr<'_> = view;
        assert_eq!(&*typed, "🚀 views borrow");
        assert_eq!(buf_view(&[1, 2]).as_slice(), [1, 2]);
    }
}
