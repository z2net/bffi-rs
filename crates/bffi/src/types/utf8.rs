//! SIMD-accelerated UTF-8 validation (DESIGN §7, "UTF-8 validation").
//!
//! Three implementations of the same predicate, selected at runtime:
//!
//! - **scalar** - a classic DFA; the reference semantics every other path
//!   is cross-checked against in tests;
//! - **x86_64 SSSE3** - block validation with `_mm_alignr_epi8` shifted
//!   predicates and saturating-subtraction range checks, gated by
//!   `is_x86_feature_detected!`;
//! - **aarch64 NEON** - the same block algorithm on `vextq`/`vcleq`.
//!
//! # Unsafe policy
//!
//! The only `unsafe` here is the vector load from a `&[u8]` slice (always
//! in bounds: loads happen on full 16-byte chunks) and the unchecked UTF-8
//! constructor in [`super::string`], which relies on this validator's
//! `true` result (the zero-copy `str_view` validates through the checked
//! `std::str::from_utf8` and needs no unchecked step). Every block
//! carries a `SAFETY:` justification; public API remains safe.
//!
//! # Algorithm
//!
//! A byte is either a lead (`00-7F`, `C2-DF`, `E0-EF`, `F0-F4`) or a
//! continuation (`80-BF`). Whether position `i` must be a continuation is
//! fully determined by the bytes at `i-1`, `i-2`, `i-3`:
//!
//! ```text
//! required(i) = lead(i-1) | (lead3|lead4)(i-2) | lead4(i-3)
//! ```
//!
//! Special second bytes after `E0` (`A0-BF`), `ED` (`80-9F`), `F0`
//! (`90-BF`) and `F4` (`80-8F`) are checked as extra range violations.
//! All ranges are nibble-aligned, so plain vector compares suffice - no
//! lookup tables. Per 16-byte block the shifted views come from
//! `_mm_alignr_epi8` / `vextq_u8` against the previous block; at the end
//! a scalar DFA steps through the last three bulk bytes to carry the
//! continuation state across the boundary and validates the remaining
//! tail.

use std::sync::OnceLock;

/// Validates `bytes` as UTF-8, choosing the fastest implementation the
/// CPU supports. Semantics are identical to `std::str::from_utf8`.
pub(crate) fn validate(bytes: &[u8]) -> bool {
    static PATH: OnceLock<Path> = OnceLock::new();
    match PATH.get_or_init(Path::detect) {
        #[cfg(not(target_arch = "aarch64"))]
        Path::Scalar => scalar_dfa(bytes),
        #[cfg(target_arch = "x86_64")]
        Path::X86Ssse3 => x86::validate(bytes),
        #[cfg(target_arch = "aarch64")]
        Path::ArmNeon => arm::validate(bytes),
    }
}

#[derive(Clone, Copy, Debug)]
enum Path {
    /// Scalar fallback. Not reachable on aarch64, where NEON is
    /// mandatory - the variant is cfg-ed out there so it cannot be dead.
    #[cfg(not(target_arch = "aarch64"))]
    Scalar,
    #[cfg(target_arch = "x86_64")]
    X86Ssse3,
    #[cfg(target_arch = "aarch64")]
    ArmNeon,
}

impl Path {
    fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("ssse3") {
                return Self::X86Ssse3;
            }
            Self::Scalar
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::ArmNeon
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::Scalar
        }
    }
}

/// One DFA transition. `remaining` counts how many continuation bytes are
/// still expected; `lo`/`hi` bound the next byte when `remaining > 0`.
#[inline]
fn dfa_step(remaining: &mut u8, lo: &mut u8, hi: &mut u8, byte: u8) -> bool {
    if *remaining == 0 {
        match byte {
            0x00..=0x7F => {}
            0xC2..=0xDF => {
                *remaining = 1;
                *lo = 0x80;
                *hi = 0xBF;
            }
            0xE0 => {
                *remaining = 2;
                *lo = 0xA0;
                *hi = 0xBF;
            }
            0xE1..=0xEC | 0xEE..=0xEF => {
                *remaining = 2;
                *lo = 0x80;
                *hi = 0xBF;
            }
            0xED => {
                *remaining = 2;
                *lo = 0x80;
                *hi = 0x9F;
            }
            0xF0 => {
                *remaining = 3;
                *lo = 0x90;
                *hi = 0xBF;
            }
            0xF1..=0xF3 => {
                *remaining = 3;
                *lo = 0x80;
                *hi = 0xBF;
            }
            0xF4 => {
                *remaining = 3;
                *lo = 0x80;
                *hi = 0x8F;
            }
            _ => return false, // lone continuation, C0/C1, F5..FF
        }
    } else {
        if byte < *lo || byte > *hi {
            return false;
        }
        *remaining -= 1;
        *lo = 0x80;
        *hi = 0xBF;
    }
    true
}

/// Reference implementation: a plain scalar DFA.
///
/// On aarch64 the NEON path is unconditional, so this exists only for the
/// cross-validation unit tests.
#[cfg(any(not(target_arch = "aarch64"), test))]
pub(crate) fn scalar_dfa(bytes: &[u8]) -> bool {
    let (mut remaining, mut lo, mut hi) = (0_u8, 0x80_u8, 0xBF_u8);
    for &byte in bytes {
        if !dfa_step(&mut remaining, &mut lo, &mut hi, byte) {
            return false;
        }
    }
    remaining == 0
}

/// Carries the DFA state across the SIMD/tail boundary: steps through the
/// last up-to-three bytes of the validated bulk (the state at position
/// `p` depends only on bytes `p-3..p`).
fn boundary_state(bytes: &[u8], bulk: usize) -> (u8, u8, u8) {
    let (mut remaining, mut lo, mut hi) = (0_u8, 0x80_u8, 0xBF_u8);
    let start = bulk.saturating_sub(3);
    for &byte in &bytes[start..bulk] {
        dfa_step(&mut remaining, &mut lo, &mut hi, byte);
    }
    (remaining, lo, hi)
}

/// Validates the tail (bytes after the last full 16-byte block) starting
/// from the DFA state at the bulk boundary.
fn validate_tail(bytes: &[u8], bulk: usize) -> bool {
    let (mut remaining, mut lo, mut hi) = boundary_state(bytes, bulk);
    for &byte in &bytes[bulk..] {
        if !dfa_step(&mut remaining, &mut lo, &mut hi, byte) {
            return false;
        }
    }
    remaining == 0
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use core::arch::x86_64::*;

    /// Unsigned range check `lo <= v[i] <= hi` for every lane, via
    /// saturating subtraction (each side saturates to zero exactly when
    /// the comparison holds).
    #[inline]
    #[target_feature(enable = "ssse3,sse2")]
    fn in_range(v: __m128i, lo: u8, hi: u8) -> __m128i {
        let lo_all = _mm_set1_epi8(lo as i8);
        let hi_all = _mm_set1_epi8(hi as i8);
        let ge_lo = _mm_cmpeq_epi8(_mm_subs_epu8(lo_all, v), _mm_setzero_si128());
        let le_hi = _mm_cmpeq_epi8(_mm_subs_epu8(v, hi_all), _mm_setzero_si128());
        _mm_and_si128(ge_lo, le_hi)
    }

    #[inline]
    #[target_feature(enable = "ssse3,sse2")]
    fn not(v: __m128i) -> __m128i {
        _mm_xor_si128(v, _mm_set1_epi8(-1))
    }

    /// SAFETY: requires `ssse3`; validated by runtime feature detection
    /// in [`super::validate`]. The tail (fewer than 16 bytes) is handled
    /// by the caller.
    #[target_feature(enable = "ssse3,sse2")]
    unsafe fn validate_blocks(bytes: &[u8]) -> bool {
        let mut error = _mm_setzero_si128();
        // Positions -3..0 of the first block are "no constraint": the zero
        // vector behaves like ASCII lead bytes.
        let mut prev = _mm_setzero_si128();
        let bulk = bytes.len() & !15;

        let mut offset = 0;
        while offset < bulk {
            // SAFETY: offset + 16 <= bulk <= bytes.len().
            let input = unsafe { _mm_loadu_si128(bytes.as_ptr().add(offset).cast::<__m128i>()) };
            // shifted by k: lane i holds bytes[offset + i - k]
            let s1 = _mm_alignr_epi8(input, prev, 15);
            let s2 = _mm_alignr_epi8(input, prev, 14);
            let s3 = _mm_alignr_epi8(input, prev, 13);

            let is_cont = in_range(input, 0x80, 0xBF);
            let is_ascii = in_range(input, 0x00, 0x7F);
            let any_lead = _mm_or_si128(
                _mm_or_si128(in_range(input, 0xC2, 0xDF), in_range(input, 0xE0, 0xEF)),
                in_range(input, 0xF0, 0xF4),
            );

            // Lead predicates re-computed on the shifted views: lane i of
            // `s1` is the byte at i-1, so its lead class constrains lane i.
            let any_lead1 = _mm_or_si128(
                _mm_or_si128(in_range(s1, 0xC2, 0xDF), in_range(s1, 0xE0, 0xEF)),
                in_range(s1, 0xF0, 0xF4),
            );
            let multi2 = _mm_or_si128(in_range(s2, 0xE0, 0xEF), in_range(s2, 0xF0, 0xF4));
            let lead4_3 = in_range(s3, 0xF0, 0xF4);

            // required(i) = lead(i-1) | (lead3|lead4)(i-2) | lead4(i-3)
            let required = _mm_or_si128(any_lead1, _mm_or_si128(multi2, lead4_3));

            // a required continuation must actually be a continuation
            error = _mm_or_si128(error, _mm_and_si128(required, not(is_cont)));
            // a start position must be a valid lead (rejects lone
            // continuations, C0/C1 and F5..FF)
            let valid_start = _mm_or_si128(is_ascii, any_lead);
            error = _mm_or_si128(error, _mm_and_si128(not(required), not(valid_start)));

            // special second bytes: s1 matching the exact lead value is
            // expressed with the same range check (lo == hi)
            error = _mm_or_si128(
                error,
                _mm_and_si128(in_range(s1, 0xE0, 0xE0), not(in_range(input, 0xA0, 0xBF))),
            );
            error = _mm_or_si128(
                error,
                _mm_and_si128(in_range(s1, 0xED, 0xED), not(in_range(input, 0x80, 0x9F))),
            );
            error = _mm_or_si128(
                error,
                _mm_and_si128(in_range(s1, 0xF0, 0xF0), not(in_range(input, 0x90, 0xBF))),
            );
            error = _mm_or_si128(
                error,
                _mm_and_si128(in_range(s1, 0xF4, 0xF4), not(in_range(input, 0x80, 0x8F))),
            );

            prev = input;
            offset += 16;
        }

        _mm_movemask_epi8(error) == 0
    }

    pub(crate) fn validate(bytes: &[u8]) -> bool {
        // SAFETY: the SSSE3 feature is detected by the caller.
        let blocks_ok = unsafe { validate_blocks(bytes) };
        if !blocks_ok {
            return false;
        }
        let bulk = bytes.len() & !15;
        super::validate_tail(bytes, bulk)
    }
}

#[cfg(target_arch = "aarch64")]
mod arm {
    use core::arch::aarch64::*;

    /// Unsigned range check `lo <= v[i] <= hi` for every lane.
    #[inline]
    #[target_feature(enable = "neon")]
    fn in_range(v: uint8x16_t, lo: u8, hi: u8) -> uint8x16_t {
        let ge_lo = vcleq_u8(vdupq_n_u8(lo), v);
        let le_hi = vcleq_u8(v, vdupq_n_u8(hi));
        vandq_u8(ge_lo, le_hi)
    }

    #[target_feature(enable = "neon")]
    fn not(v: uint8x16_t) -> uint8x16_t {
        vmvnq_u8(v)
    }

    /// SAFETY: NEON is mandatory on aarch64; the tail (fewer than 16
    /// bytes) is handled by the caller.
    #[target_feature(enable = "neon")]
    unsafe fn validate_blocks(bytes: &[u8]) -> bool {
        let mut error = vdupq_n_u8(0);
        let mut prev = vdupq_n_u8(0);
        let bulk = bytes.len() & !15;

        let mut offset = 0;
        while offset < bulk {
            // SAFETY: offset + 16 <= bulk <= bytes.len().
            let input = unsafe { vld1q_u8(bytes.as_ptr().add(offset)) };
            let s1 = vextq_u8(prev, input, 15);
            let s2 = vextq_u8(prev, input, 14);
            let s3 = vextq_u8(prev, input, 13);

            let is_cont = in_range(input, 0x80, 0xBF);
            let is_ascii = in_range(input, 0x00, 0x7F);
            let any_lead = vorrq_u8(
                vorrq_u8(in_range(input, 0xC2, 0xDF), in_range(input, 0xE0, 0xEF)),
                in_range(input, 0xF0, 0xF4),
            );

            // Lead predicates re-computed on the shifted views: lane i of
            // `s1` is the byte at i-1, so its lead class constrains lane i.
            let any_lead1 = vorrq_u8(
                vorrq_u8(in_range(s1, 0xC2, 0xDF), in_range(s1, 0xE0, 0xEF)),
                in_range(s1, 0xF0, 0xF4),
            );
            let multi2 = vorrq_u8(in_range(s2, 0xE0, 0xEF), in_range(s2, 0xF0, 0xF4));
            let lead4_3 = in_range(s3, 0xF0, 0xF4);

            // required(i) = lead(i-1) | (lead3|lead4)(i-2) | lead4(i-3)
            let required = vorrq_u8(any_lead1, vorrq_u8(multi2, lead4_3));

            error = vorrq_u8(error, vandq_u8(required, not(is_cont)));
            let valid_start = vorrq_u8(is_ascii, any_lead);
            error = vorrq_u8(error, vandq_u8(not(required), not(valid_start)));

            // special second bytes
            error = vorrq_u8(
                error,
                vandq_u8(
                    vceqq_u8(s1, vdupq_n_u8(0xE0)),
                    not(in_range(input, 0xA0, 0xBF)),
                ),
            );
            error = vorrq_u8(
                error,
                vandq_u8(
                    vceqq_u8(s1, vdupq_n_u8(0xED)),
                    not(in_range(input, 0x80, 0x9F)),
                ),
            );
            error = vorrq_u8(
                error,
                vandq_u8(
                    vceqq_u8(s1, vdupq_n_u8(0xF0)),
                    not(in_range(input, 0x90, 0xBF)),
                ),
            );
            error = vorrq_u8(
                error,
                vandq_u8(
                    vceqq_u8(s1, vdupq_n_u8(0xF4)),
                    not(in_range(input, 0x80, 0x8F)),
                ),
            );

            prev = input;
            offset += 16;
        }

        vmaxvq_u8(error) == 0
    }

    pub(crate) fn validate(bytes: &[u8]) -> bool {
        // SAFETY: NEON is mandatory on aarch64.
        let blocks_ok = unsafe { validate_blocks(bytes) };
        if !blocks_ok {
            return false;
        }
        let bulk = bytes.len() & !15;
        super::validate_tail(bytes, bulk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All valid boundary code points of the four UTF-8 ranges.
    fn interesting_valid() -> Vec<String> {
        [
            0, 1, 0x7F, 0x80, 0x7FF, 0x800, 0xFFFD, 0xFFFF, 0x10000, 0x1F600, 0x10FFFF,
        ]
        .into_iter()
        .map(|cp| char::from_u32(cp).unwrap().to_string())
        .collect()
    }

    /// Every rejection category: lone/truncated continuations, overlong
    /// encodings, surrogates, out-of-range leads.
    fn interesting_invalid() -> Vec<Vec<u8>> {
        vec![
            vec![0x80],
            vec![0xBF],
            vec![0xC0, 0x80],             // overlong NUL
            vec![0xC1, 0xBF],             // overlong
            vec![0xC3],                   // truncated 2-byte
            vec![0xE0, 0x80, 0x80],       // overlong 3-byte
            vec![0xE0, 0x9F, 0xBF],       // overlong boundary
            vec![0xED, 0xA0, 0x80],       // surrogate D800
            vec![0xED, 0xBF, 0xBF],       // surrogate DFFF
            vec![0xF0, 0x80, 0x80, 0x80], // overlong 4-byte
            vec![0xF0, 0x8F, 0xBF, 0xBF], // overlong boundary
            vec![0xF4, 0x90, 0x80, 0x80], // beyond U+10FFFF
            vec![0xF5, 0x80, 0x80, 0x80], // invalid lead
            vec![0xFF],                   // invalid byte
            vec![0xC3, 0xA9, 0x80],       // valid then lone continuation
            vec![0xE2, 0x82],             // truncated at 2 of 3
            vec![0xF0, 0x9F],             // truncated at 2 of 4
        ]
    }

    #[test]
    fn scalar_and_public_agree_on_valid_inputs() {
        for text in interesting_valid() {
            for repeat in 0..20_u8 {
                let bytes = text.repeat(repeat as usize + 1).into_bytes();
                assert!(scalar_dfa(&bytes), "scalar: {bytes:?}");
                assert!(validate(&bytes), "dispatch: {bytes:?}");
                assert_eq!(
                    std::str::from_utf8(&bytes).is_ok(),
                    validate(&bytes),
                    "std equivalence: {bytes:?}"
                );
            }
        }
    }

    #[test]
    fn scalar_and_public_agree_on_invalid_inputs() {
        for bytes in interesting_invalid() {
            assert!(!scalar_dfa(&bytes), "scalar must reject: {bytes:?}");
            assert!(!validate(&bytes), "dispatch must reject: {bytes:?}");
            assert!(std::str::from_utf8(&bytes).is_err(), "std: {bytes:?}");
        }
    }

    #[test]
    fn block_boundary_lengths_are_equivalent_to_std() {
        // Around the 16-byte SIMD block edge: every length in 0..=80 for a
        // mixed ASCII/multibyte payload.
        let unit = "a\u{e9}\u{1f680}".as_bytes().to_vec();
        for pad in 0..=80_u8 {
            let mut bytes = vec![b'x'; pad as usize];
            bytes.extend_from_slice(&unit);
            assert_eq!(
                validate(&bytes),
                std::str::from_utf8(&bytes).is_ok(),
                "len {}",
                bytes.len()
            );
        }
    }

    #[test]
    fn deterministic_random_sweep_matches_std() {
        // Deterministic LCG so failures are reproducible without a fuzz
        // dependency.
        let mut state: u64 = 0x2026_0904_DEAD_BEEF;
        let mut buffer = Vec::with_capacity(96);
        for _ in 0..10_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let len = (state >> 33) as usize % 96;
            buffer.clear();
            for _ in 0..len {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                // bias towards the interesting high ranges
                let byte = ((state >> 33) as u8)
                    | match (state >> 40) % 4 {
                        0 => 0x00,
                        1 => 0x40,
                        2 => 0x80,
                        _ => 0xC0,
                    };
                buffer.push(byte);
            }
            assert_eq!(
                validate(&buffer),
                std::str::from_utf8(&buffer).is_ok(),
                "mismatch on {buffer:?}"
            );
        }
    }
}
