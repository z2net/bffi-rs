//! The value a completed task delivers to JavaScript.
//!
//! Every variant encodes into a transient-buffer payload
//! (`[tag: u8][payload]`) so the resolve callback keeps ONE shape:
//! a `u64` handle JS reads through the `bffi_buffer` pair and frees
//! with `bffi_types_free`. Exactness is preserved for `i64`/`u64`
//! (no `f64` narrowing).
//!
//! The wire layout is the framework-wide codec (`bffi::types::wire`);
//! this enum owns only the async-specific conversions.

// Internal module aliases (the pre-merge crate names).
use crate::bffi_types;
use bffi_types::CopiedBuf;
use bffi_types::wire;

/// The output value of a spawned task.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AsyncValue {
    /// No value.
    Unit,
    /// A 32-bit signed integer.
    I32(i32),
    /// A 64-bit signed integer.
    I64(i64),
    /// A double.
    F64(f64),
    /// A boolean.
    Bool(bool),
    /// A UTF-8 string (copied into the payload).
    Str(String),
    /// Raw bytes (copied into the payload).
    Bytes(CopiedBuf),
    /// A pre-encoded wire record (a named composite or a sequence):
    /// the bytes already carry their leading tag (`TAG_RECORD` /
    /// `TAG_SEQ`), so the payload is byte-identical to the value
    /// channel the sync shims produce.
    Wire(Vec<u8>),
}

impl AsyncValue {
    /// Encodes the value into the transient-buffer payload:
    /// `[tag][payload...]`. `Str` payloads are UTF-8 with a `u32`
    /// length prefix; `Bytes` payloads are raw bytes with a `u32`
    /// length prefix.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Unit => out.push(wire::TAG_UNIT),
            Self::I32(v) => {
                out.push(wire::TAG_I32);
                wire::push_i32_le(&mut out, *v);
            }
            Self::I64(v) => {
                out.push(wire::TAG_I64);
                wire::push_i64_le(&mut out, *v);
            }
            Self::F64(v) => {
                out.push(wire::TAG_F64);
                wire::push_f64_le(&mut out, *v);
            }
            Self::Bool(v) => {
                out.push(wire::TAG_BOOL);
                wire::push_bool(&mut out, *v);
            }
            Self::Str(text) => {
                out.push(wire::TAG_STR);
                wire::push_u32_le(&mut out, text.len() as u32);
                out.extend_from_slice(text.as_bytes());
            }
            Self::Bytes(bytes) => {
                out.push(wire::TAG_BYTES);
                wire::push_u32_le(&mut out, bytes.as_slice().len() as u32);
                out.extend_from_slice(bytes.as_slice());
            }
            // The bytes are already a complete `[tag][payload]`
            // record - append verbatim.
            Self::Wire(bytes) => out.extend_from_slice(bytes.as_slice()),
        }
        out
    }
}

/// Lossless conversions feeding the `#[bffi_async]` codegen: an
/// async fn's return value is turned into an [`AsyncValue`] with
/// `.into()` at the spawn boundary.
///
/// `u8`/`u16` widen into [`AsyncValue::I32`]; `u32` widens into
/// [`AsyncValue::I64`] (its range exceeds `i32`).
macro_rules! impl_from_into_i32 {
    ($($ty:ty),* $(,)?) => {$(
        impl From<$ty> for AsyncValue {
            fn from(value: $ty) -> Self {
                Self::I32(i32::from(value))
            }
        }
    )*};
}

impl_from_into_i32!(i8, i16, i32, u8, u16);

macro_rules! impl_from_into_i64 {
    ($($ty:ty),* $(,)?) => {$(
        impl From<$ty> for AsyncValue {
            fn from(value: $ty) -> Self {
                Self::I64(i64::from(value))
            }
        }
    )*};
}

impl_from_into_i64!(u32);

impl From<i64> for AsyncValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<u64> for AsyncValue {
    fn from(value: u64) -> Self {
        Self::I64(value as i64)
    }
}

impl From<f64> for AsyncValue {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl From<f32> for AsyncValue {
    fn from(value: f32) -> Self {
        Self::F64(f64::from(value))
    }
}

impl From<bool> for AsyncValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<String> for AsyncValue {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<&'static str> for AsyncValue {
    fn from(value: &'static str) -> Self {
        Self::Str(value.to_owned())
    }
}

impl From<Vec<u8>> for AsyncValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(CopiedBuf::from_vec(value))
    }
}

impl From<CopiedBuf> for AsyncValue {
    fn from(value: CopiedBuf) -> Self {
        Self::Bytes(value)
    }
}

impl From<()> for AsyncValue {
    fn from((): ()) -> Self {
        Self::Unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bffi_types::wire::{TAG_BOOL, TAG_BYTES, TAG_I32, TAG_I64, TAG_STR, TAG_UNIT};

    #[test]
    fn unit_encodes_as_a_single_tag() {
        assert_eq!(AsyncValue::Unit.encode(), vec![TAG_UNIT]);
    }

    #[test]
    fn primitives_encode_little_endian_after_the_tag() {
        assert_eq!(
            AsyncValue::I32(-2).encode(),
            vec![TAG_I32, 0xFE, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            AsyncValue::I64(1).encode(),
            vec![TAG_I64, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(AsyncValue::Bool(true).encode(), vec![TAG_BOOL, 1]);
    }

    #[test]
    fn strings_and_bytes_carry_a_u32_length_prefix() {
        let encoded = AsyncValue::Str("hey".to_owned()).encode();
        assert_eq!(encoded, vec![TAG_STR, 3, 0, 0, 0, b'h', b'e', b'y']);

        let encoded = AsyncValue::Bytes(CopiedBuf::from_slice(&[9, 8])).encode();
        assert_eq!(encoded, vec![TAG_BYTES, 2, 0, 0, 0, 9, 8]);
    }
}
