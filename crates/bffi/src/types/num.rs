//! JavaScript number conversions with three explicit policies.
//!
//! A JavaScript number is an `f64`. Converting it to a fixed-width integer
//! can lose data, so [`JsNumber`] never converts implicitly:
//!
//! - **strict** ([`JsNumber::try_into_i32`] and friends) - the default:
//!   NaN, infinities and out-of-range values are errors; fractional values
//!   truncate toward zero *only* when the truncated result is in range.
//!   The 64-bit targets ([`JsNumber::try_into_i64`] /
//!   [`JsNumber::try_into_u64`]) are stricter still: the value must be a
//!   whole number with a magnitude of at most 2^53, the exact-integer
//!   limit of `f64`; anything else would silently lose precision;
//! - **saturating** ([`JsNumber::to_i32_saturating`] and friends) - the
//!   value clamps to the target range, NaN becomes `0` (the Rust `as`-cast
//!   contract);
//! - **JS semantics** ([`JsNumber::to_i32_js`] / [`JsNumber::to_u32_js`]) -
//!   the exact ECMAScript `ToInt32`/`ToUint32` operations behind `x | 0`
//!   and `x >>> 0`: truncation, then reduction modulo 2^32.
//!
//! Rust -> JS conversions are lossless `From` impls for every integer type
//! whose full range `f64` represents exactly (`i8`..`i32`, `u8`..`u32`,
//! `f32`) and checked helpers for `i64`/`u64`, whose magnitude can exceed
//! the 2^53 exact-integer limit of `f64`.

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
use bffi_core::{BffiError, ErrorCode};
use std::fmt;

/// Why a conversion failed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum ConversionError {
    /// The value is NaN or an infinity and the target type has no
    /// equivalent representation.
    NotFinite,
    /// The value is outside the range of the target type.
    OutOfRange,
}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFinite => f.write_str("value is NaN or infinite"),
            Self::OutOfRange => f.write_str("value is out of range for the target type"),
        }
    }
}

impl std::error::Error for ConversionError {}

impl From<ConversionError> for BffiError {
    fn from(error: ConversionError) -> Self {
        BffiError::with_source(ErrorCode::NumberOutOfRange, error.to_string(), error)
    }
}

/// Strict integer conversion: truncates toward zero, then range-checks
/// against exact f64 bounds. All bounds up to 32 bits are powers of two
/// minus one - exactly representable in `f64`.
macro_rules! strict_int {
    ($(#[$meta:meta])* $name:ident, $ty:ty, $lo:expr, $hi:expr) => {
        $(#[$meta])*
        pub fn $name(self) -> Result<$ty, ConversionError> {
            let truncated = self.truncated()?;
            if !($lo..=$hi).contains(&truncated) {
                return Err(ConversionError::OutOfRange);
            }
            Ok(truncated as $ty)
        }
    };
}

/// Saturating family: Rust's `as`-cast *is* the saturating policy
/// (clamp to bounds, NaN -> 0), so each method is an explicit cast.
macro_rules! saturating_int {
    ($(#[$meta:meta])* $name:ident, $ty:ty) => {
        $(#[$meta])*
        #[must_use]
        pub fn $name(self) -> $ty {
            self.0 as $ty
        }
    };
}

/// A JavaScript number (`f64`) with explicit conversion policies.
///
/// Construct from the JS side with [`JsNumber::new`] or the lossless
/// `From` impls; read it back with the strict, saturating or JS-semantics
/// conversion methods.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct JsNumber(f64);

impl JsNumber {
    /// Wraps a raw `f64` (what the JS side passed across the ABI).
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Returns the underlying `f64`.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Shared strict-integer front half: rejects NaN/infinities, truncates
    /// toward zero.
    fn truncated(self) -> Result<f64, ConversionError> {
        if self.0.is_finite() {
            Ok(self.0.trunc())
        } else {
            Err(ConversionError::NotFinite)
        }
    }

    strict_int!(
        /// Strict conversion to `i8`. Fractional values truncate toward
        /// zero; NaN, infinities and out-of-range values are errors.
        try_into_i8,
        i8,
        -128.0,
        127.0
    );
    strict_int!(
        /// Strict conversion to `i16`. See [`JsNumber::try_into_i8`].
        try_into_i16,
        i16,
        -32_768.0,
        32_767.0
    );
    strict_int!(
        /// Strict conversion to `i32`. See [`JsNumber::try_into_i8`].
        try_into_i32,
        i32,
        -2_147_483_648.0,
        2_147_483_647.0
    );
    strict_int!(
        /// Strict conversion to `u8`: the value must be non-negative.
        /// Fractional values truncate toward zero; NaN, infinities and
        /// out-of-range values are errors.
        try_into_u8,
        u8,
        0.0,
        255.0
    );
    strict_int!(
        /// Strict conversion to `u16`. See [`JsNumber::try_into_u8`].
        try_into_u16,
        u16,
        0.0,
        65_535.0
    );
    strict_int!(
        /// Strict conversion to `u32`. See [`JsNumber::try_into_u8`].
        try_into_u32,
        u32,
        0.0,
        4_294_967_295.0
    );

    /// Strict conversion to `i64`.
    ///
    /// The value must be an exact integer: non-integral values are
    /// rejected (dropping the fraction is a lossy cast), and so are
    /// finite values with a magnitude above the 2^53 exact-integer
    /// limit of `f64` (past it, whole numbers silently round - e.g.
    /// `2^60` decays to a nearby representable value). Reconstruct
    /// such numbers on the JS side from `BigInt` instead.
    ///
    /// # Errors
    ///
    /// [`ConversionError::NotFinite`] for NaN/infinities,
    /// [`ConversionError::OutOfRange`] for non-integral values and for
    /// |value| above 2^53.
    pub fn try_into_i64(self) -> Result<i64, ConversionError> {
        const EXACT_LIMIT: f64 = 9_007_199_254_740_992.0; // 2^53
        let truncated = self.truncated()?;
        if self.0 != truncated {
            return Err(ConversionError::OutOfRange);
        }
        // 2^53 is the last exactly representable integer magnitude;
        // beyond it `as i64` would silently round.
        if !(-EXACT_LIMIT..=EXACT_LIMIT).contains(&truncated) {
            return Err(ConversionError::OutOfRange);
        }
        Ok(truncated as i64)
    }

    /// Strict conversion to `u64`: the value must be a non-negative
    /// exact integer. See [`JsNumber::try_into_i64`] for why
    /// non-integral values and magnitudes above 2^53 are rejected.
    ///
    /// # Errors
    ///
    /// [`ConversionError::NotFinite`] for NaN/infinities,
    /// [`ConversionError::OutOfRange`] for negative, non-integral, or
    /// > 2^53 values.
    pub fn try_into_u64(self) -> Result<u64, ConversionError> {
        const EXACT_LIMIT: f64 = 9_007_199_254_740_992.0; // 2^53
        let truncated = self.truncated()?;
        if self.0 != truncated {
            return Err(ConversionError::OutOfRange);
        }
        if !(0.0..=EXACT_LIMIT).contains(&truncated) {
            return Err(ConversionError::OutOfRange);
        }
        Ok(truncated as u64)
    }

    /// Strict conversion to `f32`: succeeds only when the round-trip
    /// `f64 -> f32 -> f64` is exact. NaN is rejected; infinities convert
    /// (they survive the round-trip).
    ///
    /// # Errors
    ///
    /// [`ConversionError::NotFinite`] for NaN,
    /// [`ConversionError::OutOfRange`] when precision would be lost.
    pub fn try_into_f32(self) -> Result<f32, ConversionError> {
        if self.0.is_nan() {
            return Err(ConversionError::NotFinite);
        }
        let narrowed = self.0 as f32;
        if f64::from(narrowed) == self.0 {
            Ok(narrowed)
        } else {
            Err(ConversionError::OutOfRange)
        }
    }

    saturating_int!(
        /// Saturating conversion to `i8`: out-of-range values clamp to the
        /// type bounds, NaN becomes `0` (the Rust `as`-cast contract).
        to_i8_saturating,
        i8
    );
    saturating_int!(
        /// Saturating conversion to `i16`. See [`JsNumber::to_i8_saturating`].
        to_i16_saturating,
        i16
    );
    saturating_int!(
        /// Saturating conversion to `i32`. See [`JsNumber::to_i8_saturating`].
        to_i32_saturating,
        i32
    );
    saturating_int!(
        /// Saturating conversion to `i64`. See [`JsNumber::to_i8_saturating`].
        to_i64_saturating,
        i64
    );
    saturating_int!(
        /// Saturating conversion to `u8`. See [`JsNumber::to_i8_saturating`].
        to_u8_saturating,
        u8
    );
    saturating_int!(
        /// Saturating conversion to `u16`. See [`JsNumber::to_i8_saturating`].
        to_u16_saturating,
        u16
    );
    saturating_int!(
        /// Saturating conversion to `u32`. See [`JsNumber::to_i8_saturating`].
        to_u32_saturating,
        u32
    );
    saturating_int!(
        /// Saturating conversion to `u64`. See [`JsNumber::to_i8_saturating`].
        to_u64_saturating,
        u64
    );

    /// ECMAScript `ToInt32`: the operation behind `x | 0`.
    ///
    /// NaN and infinities become `0`; otherwise the value truncates toward
    /// zero and reduces modulo 2^32 (result in `i32`).
    #[must_use]
    pub fn to_i32_js(self) -> i32 {
        js_mod_32(self.0).map_or(0, |m| {
            if m >= 2_147_483_648.0 {
                (m - 4_294_967_296.0) as i32
            } else {
                m as i32
            }
        })
    }

    /// ECMAScript `ToUint32`: the operation behind `x >>> 0`.
    ///
    /// NaN and infinities become `0`; otherwise the value truncates toward
    /// zero and reduces modulo 2^32 (result in `u32`).
    #[must_use]
    pub fn to_u32_js(self) -> u32 {
        js_mod_32(self.0).map_or(0, |m| m as u32)
    }

    /// Checked `i64 -> f64`: succeeds only for values the `f64` format
    /// represents exactly (|value| <= 2^53).
    ///
    /// # Errors
    ///
    /// [`ConversionError::OutOfRange`] when |value| exceeds 2^53.
    pub fn try_from_i64(value: i64) -> Result<Self, ConversionError> {
        const LIMIT: i64 = 9_007_199_254_740_992; // 2^53
        if (-(LIMIT)..=LIMIT).contains(&value) {
            Ok(Self(value as f64))
        } else {
            Err(ConversionError::OutOfRange)
        }
    }

    /// Checked `u64 -> f64`; see [`JsNumber::try_from_i64`].
    ///
    /// # Errors
    ///
    /// [`ConversionError::OutOfRange`] when value exceeds 2^53.
    pub fn try_from_u64(value: u64) -> Result<Self, ConversionError> {
        const LIMIT: u64 = 9_007_199_254_740_992; // 2^53
        if value <= LIMIT {
            Ok(Self(value as f64))
        } else {
            Err(ConversionError::OutOfRange)
        }
    }

    /// Lossy `i64 -> f64` (round-to-nearest, exactly what JS
    /// `Number(bigValue)` produces).
    #[must_use]
    pub fn from_i64_lossy(value: i64) -> Self {
        Self(value as f64)
    }

    /// Lossy `u64 -> f64`; see [`JsNumber::from_i64_lossy`].
    #[must_use]
    pub fn from_u64_lossy(value: u64) -> Self {
        Self(value as f64)
    }
}

/// `ToInt32`/`ToUint32` core: NaN/infinity gate plus exact reduction into
/// `[0, 2^32)`. `%` on `f64` is IEEE fmod - exact for any finite inputs -
/// so huge values reduce without precision surprises.
fn js_mod_32(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let truncated = value.trunc();
    let mut m = truncated % 4_294_967_296.0;
    if m < 0.0 {
        m += 4_294_967_296.0;
    }
    Some(m)
}

macro_rules! lossless_from {
    ($($ty:ty),* $(,)?) => {
        $(
            impl From<$ty> for JsNumber {
                fn from(value: $ty) -> Self {
                    Self(f64::from(value))
                }
            }
        )*
    };
}

// Every type whose full range f64 represents exactly.
lossless_from!(i8, i16, i32, u8, u16, u32, f32);

impl From<f64> for JsNumber {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl From<JsNumber> for f64 {
    fn from(number: JsNumber) -> Self {
        number.0
    }
}
