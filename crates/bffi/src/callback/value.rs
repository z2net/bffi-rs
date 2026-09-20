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
//! primitive four; `U64` rides the exact `TAG_U64` carrier, and
//! `Wire` declares a pre-encoded composite (a `TAG_RECORD` /
//! `TAG_SEQ` payload) so callbacks accept sequences and records.

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
    /// The exact unsigned 64-bit integer (`TAG_U64` carrier; JS sees
    /// a non-negative `bigint` even above `i64::MAX`).
    U64,
    /// 64-bit IEEE-754 floating-point number.
    F64,
    /// Boolean.
    Bool,
    /// A UTF-8 string.
    Str,
    /// Raw bytes.
    Bytes,
    /// A pre-encoded composite payload (a complete `TAG_RECORD` /
    /// `TAG_SEQ` record). Declared in signatures through
    /// [`ValueType::Wire`].
    Wire,
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
            Self::U64 => wire::TAG_U64,
            Self::F64 => wire::TAG_F64,
            Self::Bool => wire::TAG_BOOL,
            Self::Str => wire::TAG_STR,
            Self::Bytes => wire::TAG_BYTES,
            Self::Wire => wire::TAG_WIRE,
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
            wire::TAG_U64 => Some(Self::U64),
            wire::TAG_F64 => Some(Self::F64),
            wire::TAG_BOOL => Some(Self::Bool),
            wire::TAG_STR => Some(Self::Str),
            wire::TAG_BYTES => Some(Self::Bytes),
            wire::TAG_WIRE => Some(Self::Wire),
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
    /// The exact unsigned 64-bit integer.
    U64(u64),
    /// A 64-bit IEEE-754 floating-point number.
    F64(f64),
    /// A boolean.
    Bool(bool),
    /// A UTF-8 string (copied into the payload).
    Str(String),
    /// Raw bytes (copied into the payload).
    Bytes(CopiedBuf),
    /// A pre-encoded composite payload: the bytes already carry their
    /// leading tag (`TAG_RECORD` / `TAG_SEQ`) and re-emit verbatim.
    /// Declared in signatures through [`ValueType::Wire`].
    Wire(Vec<u8>),
}

impl Value {
    /// The static [`ValueType`] of this value.
    #[must_use]
    pub const fn ty(&self) -> ValueType {
        match self {
            Self::Unit => ValueType::Unit,
            Self::I32(_) => ValueType::I32,
            Self::I64(_) => ValueType::I64,
            Self::U64(_) => ValueType::U64,
            Self::F64(_) => ValueType::F64,
            Self::Bool(_) => ValueType::Bool,
            Self::Str(_) => ValueType::Str,
            Self::Bytes(_) => ValueType::Bytes,
            Self::Wire(_) => ValueType::Wire,
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
            Self::U64(v) => wire::encode_u64(out, *v),
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
            // The bytes are already a complete `[tag][payload]`
            // record - append verbatim.
            Self::Wire(bytes) => out.extend_from_slice(bytes),
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
    /// parameter type at the same position - with one exact-carrier
    /// allowance: a non-negative `I64` argument satisfies a declared
    /// [`ValueType::U64`] and an in-range `U64` argument satisfies a
    /// declared [`ValueType::I64`] (the JS encoder picks the tag by
    /// value; the value-level mirror of
    /// `wire::decode_u64_lenient`).
    #[must_use]
    pub fn matches(&self, args: &[Value]) -> bool {
        args.len() == self.params.len()
            && args
                .iter()
                .zip(self.params.iter())
                .all(|(arg, param)| Self::arg_matches(arg, *param))
    }

    /// The exact-carrier match of one argument against one declared
    /// parameter type.
    fn arg_matches(arg: &Value, param: ValueType) -> bool {
        match (arg, param) {
            (Value::I64(v), ValueType::U64) => *v >= 0,
            (Value::U64(v), ValueType::I64) => *v <= i64::MAX as u64,
            (arg, param) => arg.ty() == param,
        }
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
    fn u64_value_round_trips_through_the_exact_carrier() {
        assert_eq!(ValueType::U64.wire_tag(), wire::TAG_U64);
        assert_eq!(
            ValueType::from_wire_tag(wire::TAG_U64),
            Some(ValueType::U64)
        );
        assert_eq!(Value::U64(u64::MAX).ty(), ValueType::U64);
        assert_eq!(Value::U64(u64::MAX).encode(), {
            let mut out = Vec::new();
            wire::encode_u64(&mut out, u64::MAX);
            out
        });
    }

    #[test]
    fn wire_value_carries_a_preencoded_composite() {
        assert_eq!(ValueType::Wire.wire_tag(), wire::TAG_WIRE);
        assert_eq!(
            ValueType::from_wire_tag(wire::TAG_WIRE),
            Some(ValueType::Wire)
        );
        let record = {
            let mut out = vec![wire::TAG_RECORD];
            wire::push_u32_le(&mut out, 1);
            wire::encode_bool(&mut out, true);
            out
        };
        let value = Value::Wire(record.clone());
        assert_eq!(value.ty(), ValueType::Wire);
        assert_eq!(value.encode(), record, "a Wire value re-emits verbatim");
    }

    #[test]
    fn wire_signature_matches_wire_arguments_only() {
        let sig = CallbackSig::new(ValueType::Unit, &[ValueType::Wire]);
        let record = {
            let mut out = vec![wire::TAG_RECORD];
            wire::push_u32_le(&mut out, 0);
            out
        };
        assert!(sig.matches(&[Value::Wire(record)]));
        assert!(!sig.matches(&[Value::I32(1)]));
    }

    #[test]
    fn exact_i64_u64_args_coerce_across_the_signed_view() {
        // The JS encoder picks the tag by value (a bigint below
        // `i64::MAX` rides `TAG_I64`), so exact-carrier declarations
        // accept the signed view within range - the value-level
        // mirror of `wire::decode_u64_lenient`.
        let unsigned = CallbackSig::new(ValueType::Unit, &[ValueType::U64]);
        assert!(
            unsigned.matches(&[Value::I64(5)]),
            "a non-negative i64 satisfies a declared u64 param"
        );
        assert!(
            !unsigned.matches(&[Value::I64(-1)]),
            "a negative i64 cannot ride a u64 param"
        );
        let signed = CallbackSig::new(ValueType::Unit, &[ValueType::I64]);
        assert!(
            signed.matches(&[Value::U64(5)]),
            "an in-range u64 satisfies a declared i64 param"
        );
        assert!(
            !signed.matches(&[Value::U64(u64::MAX)]),
            "an out-of-range u64 cannot ride an i64 param"
        );
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
