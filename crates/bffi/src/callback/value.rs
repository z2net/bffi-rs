//! Callback values and callback signatures.
//!
//! The signature IR of the callback layer: [`Value`] is a dynamically
//! typed payload crossing the boundary, [`ValueType`] is its static
//! classification, and [`CallbackSig`] validates that a slice of
//! [`Value`]s matches a declared callback signature (arity first, then
//! per-element types).
//!
//! The matrix mirrors the framework-wide wire codec (`bffi::types::
//! wire`) plus the async-layer `AsyncValue` precedent: `Unit` (the
//! `void` return / absent value), `Str` (UTF-8, crossing a JS-bound
//! call as a `cstring`) and `Bytes` (raw bytes) extend the original
//! primitive four.

// Internal module aliases (the pre-merge crate names).
use crate::bffi_types;
use bffi_types::CopiedBuf;
use bffi_types::wire;

/// The static classification of a [`Value`] payload crossing the
/// callback boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueType {
    /// No value (`void` return, the `TAG_UNIT` wire record).
    Unit,
    /// 32-bit signed integer.
    I32,
    /// 64-bit signed integer.
    I64,
    /// 64-bit IEEE-754 floating-point number.
    F64,
    /// Boolean.
    Bool,
    /// A UTF-8 string.
    Str,
    /// Raw bytes.
    Bytes,
}

impl ValueType {
    /// The wire tag of this value type (`bffi::types::wire`, the
    /// framework-wide codec table).
    #[must_use]
    pub const fn wire_tag(self) -> u8 {
        match self {
            Self::Unit => wire::TAG_UNIT,
            Self::I32 => wire::TAG_I32,
            Self::I64 => wire::TAG_I64,
            Self::F64 => wire::TAG_F64,
            Self::Bool => wire::TAG_BOOL,
            Self::Str => wire::TAG_STR,
            Self::Bytes => wire::TAG_BYTES,
        }
    }

    /// Parses a wire tag into this type; `None` for tags outside the
    /// callback value matrix.
    #[must_use]
    pub const fn from_wire_tag(tag: u8) -> Option<Self> {
        match tag {
            wire::TAG_UNIT => Some(Self::Unit),
            wire::TAG_I32 => Some(Self::I32),
            wire::TAG_I64 => Some(Self::I64),
            wire::TAG_F64 => Some(Self::F64),
            wire::TAG_BOOL => Some(Self::Bool),
            wire::TAG_STR => Some(Self::Str),
            wire::TAG_BYTES => Some(Self::Bytes),
            _ => None,
        }
    }
}

/// A dynamically typed payload passed to or from a callback.
///
/// Every variant pairs its payload with an implicit [`ValueType`]
/// available through [`Value::ty`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// No value (`void` return of a JS-bound call).
    Unit,
    /// A 32-bit signed integer.
    I32(i32),
    /// A 64-bit signed integer.
    I64(i64),
    /// A 64-bit IEEE-754 floating-point number.
    F64(f64),
    /// A boolean.
    Bool(bool),
    /// A UTF-8 string (copied into the payload).
    Str(String),
    /// Raw bytes (copied into the payload).
    Bytes(CopiedBuf),
}

impl Value {
    /// The static [`ValueType`] of this value.
    #[must_use]
    pub const fn ty(&self) -> ValueType {
        match self {
            Self::Unit => ValueType::Unit,
            Self::I32(_) => ValueType::I32,
            Self::I64(_) => ValueType::I64,
            Self::F64(_) => ValueType::F64,
            Self::Bool(_) => ValueType::Bool,
            Self::Str(_) => ValueType::Str,
            Self::Bytes(_) => ValueType::Bytes,
        }
    }

    /// The wire bytes of this value: one `[tag][payload]` record in
    /// the framework-wide codec (`bffi::types::wire`).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    /// Encodes this value into an existing buffer (the multi-argument
    /// form of [`Value::encode`]).
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::Unit => out.push(wire::TAG_UNIT),
            Self::I32(v) => {
                out.push(wire::TAG_I32);
                wire::push_i32_le(out, *v);
            }
            Self::I64(v) => {
                out.push(wire::TAG_I64);
                wire::push_i64_le(out, *v);
            }
            Self::F64(v) => {
                out.push(wire::TAG_F64);
                wire::push_f64_le(out, *v);
            }
            Self::Bool(v) => {
                out.push(wire::TAG_BOOL);
                wire::push_bool(out, *v);
            }
            Self::Str(text) => {
                out.push(wire::TAG_STR);
                wire::push_u32_le(out, text.len() as u32);
                out.extend_from_slice(text.as_bytes());
            }
            Self::Bytes(bytes) => {
                out.push(wire::TAG_BYTES);
                wire::push_u32_le(out, bytes.as_slice().len() as u32);
                out.extend_from_slice(bytes.as_slice());
            }
        }
    }
}

/// The declared signature of a callback: a return type plus an ordered
/// list of parameter types.
///
/// Use [`CallbackSig::matches`] to check that a slice of [`Value`]
/// arguments satisfies the signature before invoking the callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackSig {
    ret: ValueType,
    params: Vec<ValueType>,
}

impl CallbackSig {
    /// Builds a signature returning `ret` and taking `params` (in
    /// declaration order).
    pub fn new(ret: ValueType, params: &[ValueType]) -> Self {
        Self {
            ret,
            params: params.to_vec(),
        }
    }

    /// The declared return type.
    #[must_use]
    pub fn ret(&self) -> ValueType {
        self.ret
    }

    /// The declared parameter types, in declaration order.
    #[must_use]
    pub fn params(&self) -> &[ValueType] {
        &self.params
    }

    /// Whether `args` satisfies this signature: the arity must agree
    /// and every element's [`Value::ty`] must equal the declared
    /// parameter type at the same position.
    #[must_use]
    pub fn matches(&self, args: &[Value]) -> bool {
        args.len() == self.params.len()
            && args
                .iter()
                .zip(self.params.iter())
                .all(|(arg, param)| arg.ty() == *param)
    }
}

#[cfg(test)]
mod tests {
    use super::{CallbackSig, CopiedBuf, Value, ValueType, wire};

    #[test]
    fn value_matches_when_arity_and_types_align() {
        let sig = CallbackSig::new(ValueType::Bool, &[ValueType::I32, ValueType::F64]);
        let args = [Value::I32(1), Value::F64(2.0)];
        assert!(sig.matches(&args));
    }

    #[test]
    fn value_rejects_arity_mismatch() {
        let sig = CallbackSig::new(ValueType::Bool, &[ValueType::I32, ValueType::F64]);
        let shorter = [Value::I32(1)];
        let longer = [Value::I32(1), Value::F64(2.0), Value::Bool(true)];
        assert!(
            !sig.matches(&shorter),
            "fewer args than params must not match"
        );
        assert!(
            !sig.matches(&longer),
            "more args than params must not match"
        );
    }

    #[test]
    fn value_rejects_type_mismatch() {
        let sig = CallbackSig::new(ValueType::Bool, &[ValueType::I32]);
        let args = [Value::I64(1)];
        assert!(
            !sig.matches(&args),
            "I64 where I32 is declared must not match"
        );
    }

    #[test]
    fn value_empty_params_match_empty_args() {
        let sig = CallbackSig::new(ValueType::Bool, &[]);
        assert!(sig.matches(&[]));
    }

    #[test]
    fn value_ty_reports_the_variant() {
        assert_eq!(Value::Unit.ty(), ValueType::Unit);
        assert_eq!(Value::I32(1).ty(), ValueType::I32);
        assert_eq!(Value::I64(2).ty(), ValueType::I64);
        assert_eq!(Value::F64(3.0).ty(), ValueType::F64);
        assert_eq!(Value::Bool(true).ty(), ValueType::Bool);
        assert_eq!(Value::Str("s".to_owned()).ty(), ValueType::Str);
        assert_eq!(
            Value::Bytes(CopiedBuf::from_slice(&[0])).ty(),
            ValueType::Bytes
        );
    }

    #[test]
    fn value_wire_tags_round_trip_through_the_codec_table() {
        for tag in [
            ValueType::Unit,
            ValueType::I32,
            ValueType::I64,
            ValueType::F64,
            ValueType::Bool,
            ValueType::Str,
            ValueType::Bytes,
        ] {
            assert_eq!(
                ValueType::from_wire_tag(tag.wire_tag()),
                Some(tag),
                "{tag:?} must round-trip through its wire tag"
            );
        }
        assert_eq!(ValueType::from_wire_tag(0xFF), None);
    }

    #[test]
    fn extended_values_encode_the_framework_wire_layout() {
        assert_eq!(Value::Unit.encode(), vec![wire::TAG_UNIT]);
        assert_eq!(Value::Str("héllo".to_owned()).encode(), {
            let mut out = vec![wire::TAG_STR];
            wire::push_u32_le(&mut out, 6);
            out.extend_from_slice("héllo".as_bytes());
            out
        });
        assert_eq!(Value::Bytes(CopiedBuf::from_slice(&[9, 8])).encode(), {
            let mut out = vec![wire::TAG_BYTES];
            wire::push_u32_le(&mut out, 2);
            out.extend_from_slice(&[9, 8]);
            out
        });
    }

    #[test]
    fn sig_accessors_roundtrip() {
        let params = [
            ValueType::I32,
            ValueType::I64,
            ValueType::F64,
            ValueType::Bool,
        ];
        let sig = CallbackSig::new(ValueType::F64, &params);
        assert_eq!(sig.ret(), ValueType::F64);
        assert_eq!(sig.params(), &params);
    }

    #[test]
    fn sig_types_are_clone_and_comparable() {
        fn assert_clone<T: Clone>() {}
        fn assert_partial_eq<T: PartialEq>() {}

        assert_clone::<Value>();
        assert_clone::<ValueType>();
        assert_clone::<CallbackSig>();
        assert_partial_eq::<CallbackSig>();

        let params = [ValueType::I32, ValueType::Bool];
        let first = CallbackSig::new(ValueType::I64, &params);
        let second = CallbackSig::new(ValueType::I64, &params);
        assert_eq!(first, second, "identical signatures must compare equal");

        let different_ret = CallbackSig::new(ValueType::F64, &params);
        assert_ne!(
            first, different_ret,
            "different return types must not be equal"
        );

        let different_params = [ValueType::I32];
        let third = CallbackSig::new(ValueType::I64, &different_params);
        assert_ne!(first, third, "different parameter lists must not be equal");
    }
}
