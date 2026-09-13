//! # bffi-types
//!
//! Type conversion between JavaScript values and Rust, built on the
//! primitives of `bffi-core` (see `docs/DESIGN.md` §6.3, §8).
//!
//! This crate is the **policy** layer: it defines what a conversion means
//! (copy or zero-copy, strict or coercing) over plain safe Rust types
//! (`f64`, `&[u8]`, `&str`, `String`, `Vec<u8>`). The **syntax** layer -
//! the actual `extern "C"` entry points, raw pointer handling and
//! `bun:ffi` declarations - belongs to `bffi-build` (P2), which will call
//! these converters right after validating its pointers.
//!
//! ## Copy by default
//!
//! Per DESIGN §6.3, data crossing the boundary is **copied by default**;
//! zero-copy exists only behind the explicitly named
//! [`unsafe_zero_copy`] module and still validates UTF-8 where strings are
//! involved. Zero-copy *never* happens implicitly: every zero-copy type
//! borrows its input (`ZeroCopyStr<'a>` mirrors `&'a [u8]`), so the
//! borrow checker keeps views from outliving the FFI call.
//!
//! ## Numbers: three conversion policies
//!
//! A JavaScript number is an `f64`. Converting it to a fixed-width integer
//! can overflow, so [`JsNumber`] offers three explicit policies:
//!
//! | Policy                | Example (`3e9` to `i32`)     | Use when                    |
//! |-----------------------|------------------------------|-----------------------------|
//! | strict (`try_into_*`) | `Err(OutOfRange)`            | default; losses must be visible |
//! | saturating (`to_*_saturating`) | `i32::MAX`          | clamping is acceptable      |
//! | JS semantics (`to_*_js`)       | `-1294967296` (`x|0`)| matching `Number`/typed-array coercion |
//!
//! ## Errors
//!
//! Fallible conversions return `Result<_, BffiError>` with
//! [`bffi::core::ErrorCode`] values (`InvalidUtf8`, `NumberOutOfRange`, …).
//! Recording errors into the thread-local last-error slot is the job of
//! the boundary layer (`bffi-build`), not of these converters.
//!
//! ## Example
//!
//! ```
//! use bffi::core::ErrorCode;
//! use bffi::types::{bytes_to_string, CopiedBuf, JsNumber};
//!
//! // numbers: strict is the default policy
//! let n = JsNumber::new(3.0e9);
//! assert!(n.try_into_i32().is_err());
//! assert_eq!(n.to_i32_saturating(), i32::MAX);
//! assert_eq!(n.to_i32_js(), -1_294_967_296); // JS: 3e9 | 0
//!
//! // strings: UTF-8 is validated, the result owns a copy
//! let text = bytes_to_string(b"hello \xF0\x9F\x9A\x80")?;
//! assert_eq!(text, "hello 🚀");
//!
//! // buffers: copies never alias the source
//! let mut source = vec![1_u8, 2, 3];
//! let copied = CopiedBuf::from_slice(&source);
//! source[0] = 99;
//! assert_eq!(copied.as_slice(), [1, 2, 3]);
//! # Ok::<(), bffi::core::BffiError>(())
//! ```
//!
//! ## Implementation notes
//!
//! - UTF-8 validation is SIMD-accelerated (x86_64 SSSE3, aarch64 NEON)
//!   with a scalar DFA fallback; all paths are cross-checked against
//!   `std::str::from_utf8` semantics in tests.
//! - **Unsafe is isolated and audited**: only the SIMD vector loads and
//!   the unchecked UTF-8 constructors guarded by the validator use it;
//!   every block carries a `SAFETY:` justification and all public API is
//!   safe.

// Internal module aliases (the pre-merge crate names).
// The workspace restriction lints (expect/unwrap/panic) target production
// code; tests assert invariants and intentionally trigger panics.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

pub(crate) mod utf8;

pub mod buffer;
pub mod num;
pub mod string;
pub mod unsafe_zero_copy;
pub mod utf16;
pub mod wire;

pub use buffer::CopiedBuf;
pub use num::{ConversionError, JsNumber};
pub use string::{bytes_to_string, string_to_bytes};
pub use unsafe_zero_copy::{ZeroCopyBuf, ZeroCopyStr, buf_view, str_view};
pub use utf16::{
    string_to_utf16, string_to_utf32, utf16_to_string, utf16_to_string_lossy, utf32_to_string,
};
pub use wire::BffiWire;
