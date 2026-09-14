//! Integration tests for the numeric conversion policies.
//!
//! Reference vectors follow the ECMAScript specification (ToInt32 /
//! ToUint32 / Number coercion), verified against engine semantics.

// Tests assert invariants; the workspace restriction lints target
// production code.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use bffi::bffi_core::BffiError;
use bffi::bffi_types::{ConversionError, JsNumber};

/// Literal-argument helper (keeps test call sites uniform).
fn num(value: f64) -> JsNumber {
    JsNumber::new(value)
}

#[test]
fn strict_conversions_accept_in_range_values() {
    assert_eq!(num(127.0).try_into_i8(), Ok(i8::MAX));
    assert_eq!(num(-128.9).try_into_i8(), Ok(i8::MIN)); // trunc toward zero
    assert_eq!(num(32_767.0).try_into_i16(), Ok(i16::MAX));
    assert_eq!(num(2_147_483_647.0).try_into_i32(), Ok(i32::MAX));
    assert_eq!(num(9.0).try_into_u8(), Ok(9));
    assert_eq!(num(65_535.0).try_into_u16(), Ok(u16::MAX));
    assert_eq!(num(4_294_967_295.0).try_into_u32(), Ok(u32::MAX));
    assert_eq!(num(4_294_967_295.4).try_into_u32(), Ok(u32::MAX)); // trunc
    assert_eq!(num(-0.9).try_into_u8(), Ok(0)); // -0.9 truncs to -0 == 0
}

#[test]
fn strict_conversions_reject_out_of_range_and_non_finite() {
    assert_eq!(num(128.0).try_into_i8(), Err(ConversionError::OutOfRange));
    assert_eq!(num(-129.0).try_into_i8(), Err(ConversionError::OutOfRange));
    assert_eq!(
        num(-1.0).try_into_u8(),
        Err(ConversionError::OutOfRange),
        "unsigned targets reject negatives"
    );
    assert_eq!(
        num(2_147_483_648.0).try_into_i32(),
        Err(ConversionError::OutOfRange)
    );
    assert_eq!(
        num(f64::NAN).try_into_i32(),
        Err(ConversionError::NotFinite)
    );
    assert_eq!(
        num(f64::INFINITY).try_into_i64(),
        Err(ConversionError::NotFinite)
    );
    assert_eq!(
        num(f64::NEG_INFINITY).try_into_u64(),
        Err(ConversionError::NotFinite)
    );
}

#[test]
fn strict_i64_u64_enforce_the_exact_integer_limit() {
    // 2^53 is exactly representable in f64 and stays legal.
    assert_eq!(num(9_007_199_254_740_992.0).try_into_i64(), Ok(1 << 53)); // 2^53, exact
    assert_eq!(
        num(-9_007_199_254_740_992.0).try_into_i64(),
        Ok(-(1_i64 << 53))
    );
    // 2^53 + 2 is the first f64 above the exact-integer limit (2^53 + 1
    // rounds to 2^53 in the f64 format, so it cannot name the boundary):
    // converting it would silently lose precision, so it is rejected.
    assert_eq!(
        num(9_007_199_254_740_994.0).try_into_i64(),
        Err(ConversionError::OutOfRange),
        "2^53 + 2 as f64 is beyond the exact-integer limit"
    );
    // 2^60 as f64 would previously cast lossily; it must error now.
    assert_eq!(
        num(1_152_921_504_606_846_976.0).try_into_i64(),
        Err(ConversionError::OutOfRange),
        "2^60 silently loses precision as f64"
    );
    // The old ±2^63 power-of-two bounds sit beyond the exact-integer
    // limit too.
    assert_eq!(
        num(-9_223_372_036_854_775_808.0).try_into_i64(),
        Err(ConversionError::OutOfRange),
        "i64::MIN as f64 (2^63) is beyond the exact-integer limit"
    );
    assert_eq!(
        num(9_223_372_036_854_775_808.0).try_into_i64(),
        Err(ConversionError::OutOfRange),
        "2^63 is one past i64::MAX"
    );
    // u64 mirrors i64: values beyond 2^53 lose precision as f64.
    assert_eq!(num(0.0).try_into_u64(), Ok(0));
    assert_eq!(num(9_007_199_254_740_992.0).try_into_u64(), Ok(1_u64 << 53));
    assert_eq!(
        num(18_446_744_073_709_551_615.0).try_into_u64(),
        Err(ConversionError::OutOfRange),
        "f64 cannot represent u64::MAX; next representable is 2^64"
    );
    assert_eq!(
        num(9_223_372_036_854_775_808.0).try_into_u64(),
        Err(ConversionError::OutOfRange),
        "2^63 as f64 is beyond the exact-integer limit"
    );
}

#[test]
fn strict_i64_u64_reject_non_integral_values() {
    // Fractional f64 values must not truncate into the 64-bit targets:
    // dropping the fraction is data loss the caller did not ask for.
    assert_eq!(num(9.5).try_into_i64(), Err(ConversionError::OutOfRange));
    assert_eq!(num(-0.9).try_into_i64(), Err(ConversionError::OutOfRange));
    assert_eq!(num(9.5).try_into_u64(), Err(ConversionError::OutOfRange));
    assert_eq!(
        num(4_294_967_295.9).try_into_u64(),
        Err(ConversionError::OutOfRange)
    );
    // The <=32-bit strict policy keeps its truncating behavior.
    assert_eq!(num(-128.9).try_into_i8(), Ok(i8::MIN)); // trunc toward zero
    assert_eq!(num(4_294_967_295.4).try_into_u32(), Ok(u32::MAX)); // trunc
}

#[test]
fn strict_f32_requires_exact_roundtrip() {
    assert_eq!(num(0.5).try_into_f32(), Ok(0.5_f32));
    assert_eq!(
        num(0.1).try_into_f32(),
        Err(ConversionError::OutOfRange),
        "0.1_f64 != 0.1_f32 as f64"
    );
    assert_eq!(
        num(f64::NAN).try_into_f32(),
        Err(ConversionError::NotFinite)
    );
    assert_eq!(num(f64::INFINITY).try_into_f32(), Ok(f32::INFINITY));
    assert_eq!(
        num(1.0e300).try_into_f32(),
        Err(ConversionError::OutOfRange)
    );
}

#[test]
fn saturating_family_clamps_and_maps_nan_to_zero() {
    assert_eq!(num(3.0e9).to_i32_saturating(), i32::MAX);
    assert_eq!(num(-3.0e9).to_i32_saturating(), i32::MIN);
    assert_eq!(num(300.0).to_i8_saturating(), i8::MAX);
    assert_eq!(num(f64::NAN).to_u32_saturating(), 0);
    assert_eq!(num(f64::INFINITY).to_i64_saturating(), i64::MAX);
    assert_eq!(num(f64::NEG_INFINITY).to_i16_saturating(), i16::MIN);
    assert_eq!(num(-1.0).to_u8_saturating(), 0);
    assert_eq!(num(1.0e300).to_u64_saturating(), u64::MAX);
}

#[test]
fn js_semantics_match_ecmascript_toint32_touint32() {
    // 3e9 | 0 === -1294967296; 3e9 >>> 0 === 3000000000
    assert_eq!(num(3.0e9).to_i32_js(), -1_294_967_296);
    assert_eq!(num(3.0e9).to_u32_js(), 3_000_000_000);
    // -1 >>> 0 === 4294967295
    assert_eq!(num(-1.0).to_i32_js(), -1);
    assert_eq!(num(-1.0).to_u32_js(), 4_294_967_295);
    // NaN / infinities collapse to 0
    assert_eq!(num(f64::NAN).to_i32_js(), 0);
    assert_eq!(num(f64::INFINITY).to_u32_js(), 0);
    assert_eq!(num(f64::NEG_INFINITY).to_i32_js(), 0);
    // truncation happens before the modulo
    assert_eq!(num(0.9).to_i32_js(), 0);
    assert_eq!(num(-0.9).to_i32_js(), 0);
    // values beyond 2^32 wrap; fmod keeps them exact
    assert_eq!(num(4_294_967_296.0).to_i32_js(), 0); // 2^32
    assert_eq!(num(4_294_967_297.0).to_u32_js(), 1); // 2^32 + 1
    assert_eq!(num(8_589_934_593.5).to_i32_js(), 1); // 2^33 + 1.5 truncated
    assert_eq!(num(9_223_372_036_854_776_000.0).to_i32_js(), 0); // ~2^63
    assert_eq!(num(-4_294_967_295.0).to_u32_js(), 1); // -(2^32 - 1)
}

#[test]
fn rust_to_js_conversions_split_lossless_and_checked() {
    // lossless From impls
    assert_eq!(JsNumber::from(-1_i8).get(), -1.0);
    assert_eq!(JsNumber::from(2_147_483_647_i32).get(), 2_147_483_647.0);
    assert_eq!(JsNumber::from(4_294_967_295_u32).get(), 4_294_967_295.0);
    assert_eq!(JsNumber::from(0.5_f32).get(), 0.5);
    // i64/u64 past 2^53 are checked or explicitly lossy
    assert_eq!(
        JsNumber::try_from_i64(9_007_199_254_740_992),
        Ok(JsNumber::new(9_007_199_254_740_992.0))
    );
    assert_eq!(
        JsNumber::try_from_i64(9_007_199_254_740_993),
        Err(ConversionError::OutOfRange)
    );
    assert_eq!(
        JsNumber::try_from_i64(i64::MIN),
        Err(ConversionError::OutOfRange)
    );
    assert_eq!(
        JsNumber::try_from_u64(9_007_199_254_740_993_u64),
        Err(ConversionError::OutOfRange)
    );
    assert_eq!(
        JsNumber::from_i64_lossy(i64::MAX).get(),
        9_223_372_036_854_776_000.0
    );
    assert_eq!(
        JsNumber::from_u64_lossy(u64::MAX).get(),
        1.8446744073709552e19
    );
}

#[test]
fn conversion_error_becomes_number_out_of_range_bffi_error() {
    let error = BffiError::from(ConversionError::OutOfRange);
    assert_eq!(error.code, bffi::bffi_core::ErrorCode::NumberOutOfRange);

    let error = BffiError::from(ConversionError::NotFinite);
    assert_eq!(error.code, bffi::bffi_core::ErrorCode::NumberOutOfRange);
    assert_eq!(error.message, "value is NaN or infinite");
}

#[test]
fn conversion_error_survives_as_the_source() {
    let error = BffiError::from(ConversionError::NotFinite);
    let recovered = error
        .source
        .as_ref()
        .and_then(|boxed| boxed.downcast_ref::<ConversionError>());
    assert_eq!(
        recovered,
        Some(&ConversionError::NotFinite),
        "the typed cause must not be lost in conversion"
    );

    // the std Error::source() chain exposes the same original error
    let as_dyn: &(dyn std::error::Error + 'static) = &error;
    let via_trait = as_dyn
        .source()
        .and_then(|source| source.downcast_ref::<ConversionError>());
    assert_eq!(via_trait, Some(&ConversionError::NotFinite));
}

#[test]
fn number_accessors_roundtrip() {
    assert_eq!(num(42.5).get(), 42.5);
    assert_eq!(f64::from(num(-0.5)), -0.5);
    assert_eq!(JsNumber::default().get(), 0.0);
    assert_eq!(format!("{:?}", num(1.5)), "JsNumber(1.5)");
}
