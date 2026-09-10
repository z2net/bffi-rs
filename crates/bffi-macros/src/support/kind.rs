//! The boundary kind model shared by the proc-macro crates.
//!
//! These enums are the normalized vocabulary both `#[bffi]` and
//! `#[bffi_class]`/`#[bffi_impl]` speak after parsing: every accepted
//! parameter and return type is classified into exactly one of them,
//! and the codegen layer consumes only these kinds. [`TsKind`] is the
//! typed bridge to the `bffi-dts` IR - its [`TsKind::tokens`] quote
//! the IR variants directly, no string round-trip.

use crate::support::paths::PathCtx;
use proc_macro2::TokenStream;
use quote::quote;

/// A `syn::Path` that compares and prints by its tokens: the kinds
/// below embed it, and `syn::Path` itself implements neither `Debug`
/// nor `Eq` without syn's `extra-traits` feature.
#[derive(Clone)]
pub struct KindPath(pub syn::Path);

impl std::fmt::Debug for KindPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", quote::ToTokens::to_token_stream(&self.0))
    }
}

impl PartialEq for KindPath {
    fn eq(&self, other: &Self) -> bool {
        quote::ToTokens::to_token_stream(&self.0).to_string()
            == quote::ToTokens::to_token_stream(&other.0).to_string()
    }
}

impl Eq for KindPath {}

/// A 64-bit integer crossing the boundary (`i64`/`u64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BigIntTy {
    /// `i64`
    I64,
    /// `u64`
    U64,
}

/// A small primitive accepted at the boundary: number-ish integers,
/// floats, or `bool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimTy {
    /// `i8`
    I8,
    /// `i16`
    I16,
    /// `i32`
    I32,
    /// `u8`
    U8,
    /// `u16`
    U16,
    /// `u32`
    U32,
    /// `f32`
    F32,
    /// `f64`
    F64,
    /// `bool`
    Bool,
}

/// The kind of one parameter (or return) at the boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShimKind {
    /// A small primitive (`number`-ish types and `bool`).
    Prim(PrimTy),
    /// A 64-bit integer (`i64`/`u64`).
    BigInt(BigIntTy),
    /// A borrowed `&str` copied across the boundary.
    Str,
    /// A borrowed `&[u8]` view: the shim receives a `(ptr, len)` pair
    /// valid for the duration of the call (bun:ffi TypedArray
    /// pointer), never taking ownership.
    BufferView,
    /// An owned `#[derive(BffiRecord)]`/`BffiEnum` type: crosses as a
    /// `(ptr, len)` wire payload the shim decodes (copy by default).
    Record(KindPath),
    /// An owned `Vec<T>` of a non-`u8` item: crosses as a `(ptr, len)`
    /// wire sequence the shim decodes item by item.
    Seq(SeqItem),
}

/// The item kind of a `Vec<T>` boundary sequence: which wire record
/// each item travels as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeqItem {
    /// `i8 i16 i32 u8 u16` items - one `i32` wire record each
    /// (narrowed back at decode).
    Narrow,
    /// `u32 f32 f64` items - one `f64` wire record each (exact for
    /// the whole range of those widths).
    Wide,
    /// `i64` items - one `i64` wire record each.
    I64,
    /// `bool` items.
    Bool,
    /// `String` items - one string record each.
    Str,
    /// Nested record/enum items - one complete nested record each.
    Record(KindPath),
}

/// An owned byte-carrying return type: stored in the `bffi-build`
/// transient-buffer table and handed to JS as a handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferTy {
    /// `String` - UTF-8 bytes; rendered as `string`.
    String,
    /// `Vec<u8>` - raw bytes; rendered as `Uint8Array`.
    ByteVec,
    /// `CopiedBuf` - raw bytes; rendered as `Uint8Array`.
    CopiedBuf,
}

/// The return side of a validated function or method.
// Not `Copy`: `Result` boxes its inner return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetKind {
    /// No return value (`()`).
    Unit,
    /// A small primitive.
    Prim(PrimTy),
    /// A 64-bit integer (`i64`/`u64`).
    BigInt(BigIntTy),
    /// An owned byte payload returned as a transient-buffer handle.
    Buffer(BufferTy),
    /// `Option` of a buffer payload: `None` writes the `0` handle.
    Nullable(BufferTy),
    /// `Result<T, E>`: `Ok` transports `T`, `Err` reports the domain
    /// error through the last-error channel (`ErrorCode::DomainError`).
    Result(Box<RetKind>),
    /// An owned `#[derive(BffiRecord)]`/`BffiEnum` value returned as
    /// a wire-encoded transient-buffer handle.
    Record(KindPath),
    /// An owned `Vec<T>` of a non-`u8` item, returned as a
    /// wire-encoded transient-buffer handle.
    Seq(SeqItem),
}

/// The TypeScript type of an accepted boundary item, as a
/// `<dts>::TsType` variant token stream (the `dts` root comes from
/// the [`PathCtx`]).
// Not `Copy`: `Expr` carries a token stream.
// Not `Copy`: `Expr` carries a token stream.
#[derive(Clone, Debug)]
pub enum TsKind {
    /// `number`
    Number,
    /// `bigint`
    BigInt,
    /// `boolean`
    Boolean,
    /// `string`
    String,
    /// `Uint8Array`
    Uint8Array,
    /// `string | null` (`Option<String>` returns).
    NullableString,
    /// `Uint8Array | null` (`Option<Vec<u8>>` / `Option<CopiedBuf>`
    /// returns).
    NullableUint8Array,
    /// `void`
    Void,
    /// `Promise<void>` (`#[bffi_async]` returns of `()`).
    PromiseVoid,
    /// `Promise<number>` (`#[bffi_async]` returns of the number-ish
    /// primitives).
    PromiseNumber,
    /// `Promise<bigint>` (`#[bffi_async]` returns of `i64`/`u64`).
    PromiseBigInt,
    /// `Promise<boolean>` (`#[bffi_async]` returns of `bool`).
    PromiseBoolean,
    /// `Promise<string>` (`#[bffi_async]` returns of `String`).
    PromiseString,
    /// `Promise<Uint8Array>` (`#[bffi_async]` returns of `Vec<u8>` /
    /// `CopiedBuf`).
    PromiseUint8Array,
    /// A pre-quoted `TsType` expression: the B1 named composites
    /// (`#path::BFFI_TS_TYPE` of a record/enum) carry their exact IR
    /// tokens with no path context of their own.
    Expr(TokenStream),
    /// `number[]` (a `Vec` sequence of number-ish items).
    NumberArray,
    /// `bigint[]` (`Vec<i64>`).
    BigIntArray,
    /// `boolean[]` (`Vec<bool>`).
    BooleanArray,
    /// `string[]` (`Vec<String>`).
    StringArray,
    /// `<Name>[]` (a `Vec` sequence of a named record/enum).
    RecordArray(String),
}

/// Manual: `Expr` compares by its token text, the array wrappers by
/// payload, everything else by variant (all fieldless).
impl PartialEq for TsKind {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Expr(a), Self::Expr(b)) => a.to_string() == b.to_string(),
            (Self::RecordArray(a), Self::RecordArray(b)) => a == b,
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }
}

impl TsKind {
    /// The `<dts>::TsType` variant tokens for this kind, rooted at
    /// `ctx`'s `dts` crate path.
    pub fn tokens(&self, ctx: &PathCtx) -> TokenStream {
        let dts = &ctx.dts;
        match self {
            TsKind::Number => quote! { #dts::TsType::Number },
            TsKind::BigInt => quote! { #dts::TsType::BigInt },
            TsKind::Boolean => quote! { #dts::TsType::Boolean },
            TsKind::String => quote! { #dts::TsType::String },
            TsKind::Uint8Array => quote! { #dts::TsType::Uint8Array },
            TsKind::NullableString => quote! { #dts::TsType::NullableString },
            TsKind::NullableUint8Array => quote! { #dts::TsType::NullableUint8Array },
            TsKind::Void => quote! { #dts::TsType::Void },
            TsKind::PromiseVoid => quote! { #dts::TsType::PromiseVoid },
            TsKind::PromiseNumber => quote! { #dts::TsType::PromiseNumber },
            TsKind::PromiseBigInt => quote! { #dts::TsType::PromiseBigInt },
            TsKind::PromiseBoolean => quote! { #dts::TsType::PromiseBoolean },
            TsKind::PromiseString => quote! { #dts::TsType::PromiseString },
            TsKind::PromiseUint8Array => quote! { #dts::TsType::PromiseUint8Array },
            TsKind::Expr(tokens) => tokens.clone(),
            TsKind::NumberArray => quote! { #dts::TsType::NumberArray },
            TsKind::BigIntArray => quote! { #dts::TsType::BigIntArray },
            TsKind::BooleanArray => quote! { #dts::TsType::BooleanArray },
            TsKind::StringArray => quote! { #dts::TsType::StringArray },
            TsKind::RecordArray(name) => quote! { #dts::TsType::RecordArray(#name) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TsKind;
    use crate::support::paths::PathCtx;

    #[test]
    fn ts_kind_tokens_quote_the_ir_variant() {
        let ctx = PathCtx::default();
        assert_eq!(
            TsKind::Number.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: Number"
        );
        assert_eq!(
            TsKind::Uint8Array.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: Uint8Array"
        );
        assert_eq!(
            TsKind::NullableString.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: NullableString"
        );
        assert_eq!(
            TsKind::NullableUint8Array.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: NullableUint8Array"
        );
        assert_eq!(
            TsKind::Void.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: Void"
        );
    }

    #[test]
    fn ts_kind_tokens_follow_the_path_context() {
        let ctx = PathCtx::from_attr("bffi");
        assert_eq!(
            TsKind::Number.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: Number"
        );
        assert_eq!(
            TsKind::BigInt.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: BigInt"
        );
    }
}
