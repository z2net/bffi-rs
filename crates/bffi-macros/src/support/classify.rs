//! Type classification: `syn::Type` -> boundary kinds.
//!
//! [`classify_param`] and [`classify_return`] turn a `syn::Type` into
//! the boundary kinds from [`crate::support::kind`], rejecting anything outside
//! the accepted set. Rejections are neutral: the classifiers return an
//! [`Unsupported`] carrying the span and the offending type, and each
//! proc-macro crate converts it into its own diagnostic (its own
//! E-code and exact help lines). The TypeScript mapping
//! ([`ts_prim`]/[`ts_type`]/[`ts_return`]) complements the
//! classifiers for the descriptor generators.

use crate::support::kind::{
    BigIntTy, BufferTy, KindPath, PrimTy, RetKind, SeqItem, ShimKind, TsKind,
};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::spanned::Spanned;

/// A type rejected by a classifier: the neutral form each proc-macro
/// crate maps onto its own diagnostic (E-code plus exact help and
/// note lines).
pub struct Unsupported<'a> {
    /// Span of the offending type.
    pub span: Span,
    /// The offending type.
    pub ty: &'a syn::Type,
}

// Manual impl: `syn::Type` only implements `Debug` behind the
// `extra-traits` feature, which this crate does not enable.
impl std::fmt::Debug for Unsupported<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Unsupported").finish_non_exhaustive()
    }
}

/// Kind of a plain path type: a small primitive or a 64-bit integer.
pub enum PathKind {
    /// A small primitive (`i32`, `f64`, `bool`, ...).
    Prim(PrimTy),
    /// A 64-bit integer (`i64`/`u64`).
    BigInt(BigIntTy),
}

/// Resolves a small-primitive name.
fn prim_from_str(name: &str) -> Option<PrimTy> {
    match name {
        "i8" => Some(PrimTy::I8),
        "i16" => Some(PrimTy::I16),
        "i32" => Some(PrimTy::I32),
        "u8" => Some(PrimTy::U8),
        "u16" => Some(PrimTy::U16),
        "u32" => Some(PrimTy::U32),
        "f32" => Some(PrimTy::F32),
        "f64" => Some(PrimTy::F64),
        "bool" => Some(PrimTy::Bool),
        _ => None,
    }
}

/// Resolves a 64-bit integer name.
fn bigint_from_str(name: &str) -> Option<BigIntTy> {
    match name {
        "i64" => Some(BigIntTy::I64),
        "u64" => Some(BigIntTy::U64),
        _ => None,
    }
}

/// Resolves the buffer payload behind `String` / `Vec<u8>` / `CopiedBuf`.
fn buffer_from_str(name: &str) -> Option<BufferTy> {
    match name {
        "String" => Some(BufferTy::String),
        "CopiedBuf" => Some(BufferTy::CopiedBuf),
        _ => None,
    }
}

/// The single-segment identifier of a plain path type, plus whether it
/// carries generic arguments. Non-path, qualified (`::x`/`std::x`)
/// and multi-segment paths are `None`.
pub fn path_ident(ty: &syn::Type) -> Option<(String, bool)> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() || path.path.leading_colon.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    let segment = &path.path.segments[0];
    let has_arguments = !segment.arguments.is_none();
    Some((segment.ident.to_string(), has_arguments))
}

/// Classifies `ty` when it is a plain, unqualified path over a single
/// generic-free segment (`u32`). Qualified paths (`std::u32`), paths
/// with arguments (`Vec<u8>`) and non-path types are `None`.
pub fn path_kind(ty: &syn::Type) -> Option<PathKind> {
    let (name, has_arguments) = path_ident(ty)?;
    if has_arguments {
        return None;
    }
    if let Some(prim) = prim_from_str(&name) {
        return Some(PathKind::Prim(prim));
    }
    bigint_from_str(&name).map(PathKind::BigInt)
}

/// Whether `ty` is the bare `str` path type (`str` parses as a
/// single-segment path, not a dedicated variant).
pub fn is_str_type(ty: &syn::Type) -> bool {
    matches!(path_ident(ty), Some((name, false)) if name == "str")
}

/// Whether `ty` is the bare `u8` path.
pub fn is_u8(ty: &syn::Type) -> bool {
    matches!(path_ident(ty), Some((name, false)) if name == "u8")
}

/// Whether `ty` is a slice of exactly `u8` (`[u8]`).
fn is_u8_slice(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Slice(slice) if is_u8(&slice.elem))
}

/// The generic argument list of a single-segment path type (empty for
/// argument-free paths).
pub fn generic_args(ty: &syn::Type) -> Vec<&syn::Type> {
    let syn::Type::Path(path) = ty else {
        return Vec::new();
    };
    let Some(segment) = path.path.segments.last() else {
        return Vec::new();
    };
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Vec::new();
    };
    args.args
        .iter()
        .filter_map(|arg| match arg {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect()
}

/// Names that look like bare user types but are rejected: they are
/// either unsized/unsupported scalars or owned-buffer channels with
/// dedicated rules. Their rejection texts are locked by the ui
/// goldens, so they must not fall into the `Record` classification.
const DENIED_BARE_NAMES: &[&str] = &[
    "str",
    "char",
    "i128",
    "u128",
    "isize",
    "usize",
    "String",
    "CopiedBuf",
];

/// The plain `syn::Path` of a type when it is argument-free on its
/// last segment (qualified paths included: `my_module::Point` is a
/// legitimate record reference). `None` for non-paths and paths whose
/// last segment carries generic arguments.
fn plain_path(ty: &syn::Type) -> Option<&syn::Path> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    let last = path.path.segments.last()?;
    if !last.arguments.is_none() {
        return None;
    }
    Some(&path.path)
}

/// Classifies the item of a `Vec<T>` sequence (or of a
/// `#[bffi_stream]`); `None` rejects.
pub(crate) fn seq_item(inner: &syn::Type) -> Option<SeqItem> {
    let syn::Type::Path(path) = inner else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    let last = path.path.segments.last()?;
    if !last.arguments.is_none() {
        return None;
    }
    let name = last.ident.to_string();
    match name.as_str() {
        "i8" | "i16" | "i32" | "u8" | "u16" => Some(SeqItem::Narrow),
        "u32" | "f32" | "f64" => Some(SeqItem::Wide),
        "i64" => Some(SeqItem::I64),
        "bool" => Some(SeqItem::Bool),
        "String" => Some(SeqItem::Str),
        "str" | "char" | "i128" | "u128" | "isize" | "usize" | "u64" | "CopiedBuf" => None,
        _ => Some(SeqItem::Record(KindPath(path.path.clone()))),
    }
}

/// Classifies a parameter type: plain primitives and `i64`/`u64` as
/// their kinds, `&str` (borrowed, not `mut`; lifetimes ignored) as
/// [`ShimKind::Str`], `&[u8]` (borrowed, not `mut`; lifetimes
/// ignored) as [`ShimKind::BufferView`], an owned record/enum type
/// as [`ShimKind::Record`] and a `Vec<T>` of a non-`u8` supported
/// item as [`ShimKind::Seq`]; everything else rejected.
pub fn classify_param(ty: &syn::Type) -> Result<ShimKind, Unsupported<'_>> {
    if let Some(kind) = path_kind(ty) {
        return Ok(match kind {
            PathKind::Prim(prim) => ShimKind::Prim(prim),
            PathKind::BigInt(bigint) => ShimKind::BigInt(bigint),
        });
    }
    if let syn::Type::Reference(reference) = ty
        && reference.mutability.is_none()
    {
        if is_str_type(&reference.elem) {
            return Ok(ShimKind::Str);
        }
        if is_u8_slice(&reference.elem) {
            return Ok(ShimKind::BufferView);
        }
    }
    if let Some((name, true)) = path_ident(ty)
        && name == "Vec"
    {
        let args = generic_args(ty);
        if let [inner] = args.as_slice()
            && !is_u8(inner)
            && let Some(item) = seq_item(inner)
        {
            return Ok(ShimKind::Seq(item));
        }
        // `Vec<u8>` params and unsupported items fall through to the
        // rejection below (owned byte buffers are return-only).
    }
    if let Some(path) = plain_path(ty) {
        if let Some(name) = path.segments.last().map(|seg| seg.ident.to_string())
            && DENIED_BARE_NAMES.contains(&name.as_str())
        {
            return Err(Unsupported {
                span: ty.span(),
                ty,
            });
        }
        return Ok(ShimKind::Record(KindPath(path.clone())));
    }
    Err(Unsupported {
        span: ty.span(),
        ty,
    })
}

/// Classifies a return type: the plain primitives plus `i64`/`u64`,
/// the empty tuple, the owned byte payloads (`String` / `Vec<u8>` /
/// `CopiedBuf`), `Option` of a payload, `Result<T, E>` over any of
/// those, an owned record/enum type and a `Vec<T>` sequence of a
/// supported non-`u8` item. Everything else is rejected.
pub fn classify_return(ty: &syn::Type) -> Result<RetKind, Unsupported<'_>> {
    if let syn::Type::Tuple(tuple) = ty
        && tuple.elems.is_empty()
    {
        return Ok(RetKind::Unit);
    }
    if let Some(kind) = path_kind(ty) {
        return Ok(match kind {
            PathKind::Prim(prim) => RetKind::Prim(prim),
            PathKind::BigInt(bigint) => RetKind::BigInt(bigint),
        });
    }
    if let Some((name, false)) = path_ident(ty)
        && let Some(buffer) = buffer_from_str(&name)
    {
        return Ok(RetKind::Buffer(buffer));
    }
    if let Some((name, true)) = path_ident(ty) {
        let args = generic_args(ty);
        match (name.as_str(), args.as_slice()) {
            ("Vec", [inner]) if is_u8(inner) => return Ok(RetKind::Buffer(BufferTy::ByteVec)),
            ("Vec", [inner]) => {
                if let Some(item) = seq_item(inner) {
                    return Ok(RetKind::Seq(item));
                }
            }
            ("Option", [inner]) => {
                if let Some(buffer) = classify_buffer_only(inner) {
                    return Ok(RetKind::Nullable(buffer));
                }
            }
            ("Result", [ok, err]) => {
                let inner = classify_return(ok)?;
                // Shape-check only: the trait obligations on `E`
                // (`Error + Send + Sync + 'static`) surface as a
                // regular trait-bound error in the expansion, where
                // rustc names the exact missing impl.
                if path_ident(err).is_none() {
                    return Err(Unsupported {
                        span: ty.span(),
                        ty,
                    });
                }
                return Ok(RetKind::Result(Box::new(inner)));
            }
            _ => {}
        }
    }
    if let Some(path) = plain_path(ty) {
        if let Some(name) = path.segments.last().map(|seg| seg.ident.to_string())
            && DENIED_BARE_NAMES.contains(&name.as_str())
        {
            return Err(Unsupported {
                span: ty.span(),
                ty,
            });
        }
        return Ok(RetKind::Record(KindPath(path.clone())));
    }
    Err(Unsupported {
        span: ty.span(),
        ty,
    })
}

/// Classifies `ty` when it is exactly a buffer payload; `None` when it
/// is another type (the caller rejects with the help line).
fn classify_buffer_only(ty: &syn::Type) -> Option<BufferTy> {
    if let Some((name, false)) = path_ident(ty) {
        return buffer_from_str(&name);
    }
    if let Some((name, true)) = path_ident(ty)
        && name == "Vec"
    {
        let args = generic_args(ty);
        if args.len() == 1 && is_u8(args[0]) {
            return Some(BufferTy::ByteVec);
        }
    }
    None
}

/// TypeScript kind of a small primitive: every numeric type is
/// [`TsKind::Number`], `bool` is [`TsKind::Boolean`].
pub fn ts_prim(prim: PrimTy) -> TsKind {
    match prim {
        PrimTy::I8
        | PrimTy::I16
        | PrimTy::I32
        | PrimTy::U8
        | PrimTy::U16
        | PrimTy::U32
        | PrimTy::F32
        | PrimTy::F64 => TsKind::Number,
        PrimTy::Bool => TsKind::Boolean,
    }
}

/// The descriptor-context form of a user path: descriptor consts live
/// in a sibling `bffi_meta_*` module, where bare names of the
/// annotated module do not resolve - relative paths gain a `super::`
/// anchor (leading-`::` / `crate::` / `self::` / `super::` paths pass
/// through unchanged).
fn descriptor_path(path: &syn::Path) -> TokenStream {
    if path.leading_colon.is_some() {
        return quote! { #path };
    }
    if let Some(first) = path.segments.first()
        && matches!(first.ident.to_string().as_str(), "crate" | "self" | "super")
    {
        return quote! { #path };
    }
    quote! { super:: #path }
}

/// TypeScript kind of an accepted parameter kind.
pub fn ts_type(kind: &ShimKind) -> TsKind {
    match kind {
        ShimKind::Prim(prim) => ts_prim(*prim),
        ShimKind::BigInt(_) => TsKind::BigInt,
        ShimKind::Str => TsKind::String,
        // The descriptor sees ONE `Uint8Array` parameter: the
        // `(ptr, len)` C pair is ABI-level only.
        ShimKind::BufferView => TsKind::Uint8Array,
        // Records/enums carry their own exact `TsType` through the
        // derive's `BFFI_TS_TYPE` (descriptor-anchored: see
        // [`descriptor_path`]).
        ShimKind::Record(path) => {
            let p = descriptor_path(&path.0);
            TsKind::Expr(quote! { #p::BFFI_TS_TYPE })
        }
        ShimKind::Seq(item) => ts_seq_item(item),
    }
}

/// The `TsKind` of one sequence item kind (the array wrapper).
fn ts_seq_item(item: &SeqItem) -> TsKind {
    match item {
        SeqItem::Narrow | SeqItem::Wide => TsKind::NumberArray,
        SeqItem::I64 => TsKind::BigIntArray,
        SeqItem::Bool => TsKind::BooleanArray,
        SeqItem::Str => TsKind::StringArray,
        SeqItem::Record(path) => {
            let name = path
                .0
                .segments
                .last()
                .map(|seg| seg.ident.to_string())
                .unwrap_or_default();
            TsKind::RecordArray(name)
        }
    }
}

/// TypeScript kind of an accepted return type. `Nullable` payloads
/// map to the dedicated `NullableString` / `NullableUint8Array`
/// kinds, whose `as_str` carries the `| null` contract.
pub fn ts_return(ret: &RetKind) -> TsKind {
    match ret {
        RetKind::Unit => TsKind::Void,
        RetKind::Prim(prim) => ts_prim(*prim),
        RetKind::BigInt(_) => TsKind::BigInt,
        RetKind::Buffer(BufferTy::String) => TsKind::String,
        RetKind::Buffer(_) => TsKind::Uint8Array,
        RetKind::Nullable(BufferTy::String) => TsKind::NullableString,
        RetKind::Nullable(_) => TsKind::NullableUint8Array,
        RetKind::Result(inner) => ts_return(inner),
        RetKind::Record(path) => {
            let p = descriptor_path(&path.0);
            TsKind::Expr(quote! { #p::BFFI_TS_TYPE })
        }
        RetKind::Seq(item) => ts_seq_item(item),
    }
}

/// TypeScript kind of an `#[bffi_async]` return: the `Promise*`
/// family wrapping the plain mapping. `Nullable` async returns are
/// rejected at classification time (v1 scope) and never reach this
/// mapping.
pub fn ts_promise(ret: &RetKind) -> TsKind {
    match ret {
        RetKind::Unit => TsKind::PromiseVoid,
        RetKind::Prim(prim) => match ts_prim(*prim) {
            TsKind::Boolean => TsKind::PromiseBoolean,
            _ => TsKind::PromiseNumber,
        },
        RetKind::BigInt(_) => TsKind::PromiseBigInt,
        RetKind::Buffer(BufferTy::String) => TsKind::PromiseString,
        RetKind::Buffer(_) => TsKind::PromiseUint8Array,
        RetKind::Result(inner) => ts_promise(inner),
        RetKind::Nullable(inner) => ts_promise(&RetKind::Buffer(*inner)),
        // Async records/sequences arrive with the next B1 slice; the
        // async model rejects them before descriptors are emitted, so
        // these arms are defensive only.
        RetKind::Record(_) | RetKind::Seq(_) => TsKind::Void,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PathKind, classify_param, classify_return, is_str_type, is_u8, path_ident, path_kind,
        ts_prim, ts_return, ts_type,
    };
    use crate::support::kind::{BigIntTy, BufferTy, PrimTy, RetKind, SeqItem, ShimKind, TsKind};

    /// Parses a type source, panicking in tests only (allowed by the
    /// crate-root `cfg_attr(test)` escape hatch).
    fn ty(src: &str) -> syn::Type {
        syn::parse_str(src).expect("valid type source")
    }

    #[test]
    fn accepted_primitives_map_to_ts_kinds() {
        let cases = [
            (PrimTy::I8, TsKind::Number),
            (PrimTy::I16, TsKind::Number),
            (PrimTy::I32, TsKind::Number),
            (PrimTy::U8, TsKind::Number),
            (PrimTy::U16, TsKind::Number),
            (PrimTy::U32, TsKind::Number),
            (PrimTy::F32, TsKind::Number),
            (PrimTy::F64, TsKind::Number),
            (PrimTy::Bool, TsKind::Boolean),
        ];
        for (prim, expected) in cases {
            assert_eq!(ts_prim(prim), expected);
        }
    }

    #[test]
    fn accepted_param_types_map_to_ts_kinds() {
        let cases = [
            ("i8", TsKind::Number),
            ("i32", TsKind::Number),
            ("f64", TsKind::Number),
            ("bool", TsKind::Boolean),
            ("i64", TsKind::BigInt),
            ("u64", TsKind::BigInt),
            ("&str", TsKind::String),
            ("&[u8]", TsKind::Uint8Array),
        ];
        for (src, expected) in cases {
            let kind = classify_param(&ty(src)).expect("accepted");
            assert_eq!(ts_type(&kind), expected, "param type `{src}`");
        }
    }

    #[test]
    fn str_params_accept_lifetimes() {
        for src in ["&str", "&'a str"] {
            let kind = classify_param(&ty(src)).expect("accepted");
            assert_eq!(ts_type(&kind), TsKind::String, "param type `{src}`");
        }
    }

    #[test]
    fn buffer_view_params_accept_lifetimes_and_classify_distinctly() {
        for src in ["&[u8]", "&'a [u8]"] {
            let kind = classify_param(&ty(src)).expect("accepted");
            assert_eq!(kind, ShimKind::BufferView, "param type `{src}`");
            assert_eq!(ts_type(&kind), TsKind::Uint8Array, "param type `{src}`");
        }
    }

    #[test]
    fn bigints_classify_distinctly() {
        assert_eq!(
            classify_param(&ty("i64")).expect("accepted"),
            ShimKind::BigInt(BigIntTy::I64)
        );
        assert_eq!(
            classify_param(&ty("u64")).expect("accepted"),
            ShimKind::BigInt(BigIntTy::U64)
        );
    }

    #[test]
    fn buffer_returns_classify_with_their_ts_kinds() {
        let cases = [
            ("String", RetKind::Buffer(BufferTy::String), TsKind::String),
            (
                "Vec<u8>",
                RetKind::Buffer(BufferTy::ByteVec),
                TsKind::Uint8Array,
            ),
            (
                "CopiedBuf",
                RetKind::Buffer(BufferTy::CopiedBuf),
                TsKind::Uint8Array,
            ),
        ];
        for (src, expected, ts) in cases {
            let ret = classify_return(&ty(src)).expect("accepted");
            assert_eq!(ret, expected, "return type `{src}`");
            assert_eq!(ts_return(&ret), ts, "ts kind for `{src}`");
        }
    }

    #[test]
    fn option_buffer_returns_classify_nullable() {
        let cases = [
            ("Option<String>", TsKind::NullableString),
            ("Option<Vec<u8>>", TsKind::NullableUint8Array),
            ("Option<CopiedBuf>", TsKind::NullableUint8Array),
        ];
        for (src, ts) in cases {
            let ret = classify_return(&ty(src)).expect("accepted");
            assert!(
                matches!(ret, RetKind::Nullable(_)),
                "`{src}` must classify as Nullable"
            );
            assert_eq!(ts_return(&ret), ts, "ts kind for `{src}`");
        }
    }

    #[test]
    fn result_returns_wrap_any_supported_inner() {
        for src in [
            "Result<u32, MyError>",
            "Result<(), MyError>",
            "Result<String, MyError>",
            "Result<Option<CopiedBuf>, MyError>",
        ] {
            let ret = classify_return(&ty(src)).expect("accepted");
            assert!(
                matches!(ret, RetKind::Result(_)),
                "`{src}` must classify as Result"
            );
        }
        assert_eq!(
            ts_return(&classify_return(&ty("Result<u32, MyError>")).expect("accepted")),
            TsKind::Number
        );
        assert_eq!(
            ts_return(
                &classify_return(&ty("Result<Option<CopiedBuf>, MyError>")).expect("accepted")
            ),
            TsKind::NullableUint8Array
        );
    }

    #[test]
    fn vec_of_unsupported_items_and_single_arg_result_are_rejected() {
        assert!(classify_return(&ty("Vec<char>")).is_err());
        assert!(classify_return(&ty("Result<u32>")).is_err());
    }

    #[test]
    fn unit_return_maps_to_void() {
        let ret = classify_return(&ty("()")).expect("accepted");
        assert_eq!(ts_return(&ret), TsKind::Void);
    }

    #[test]
    fn unsupported_return_types_are_rejected_neutrally() {
        let parsed = ty("char");
        let err = classify_return(&parsed).expect_err("rejected");
        assert!(std::ptr::eq(err.ty, &parsed), "carries the offending type");
        assert!(classify_return(&ty("&str")).is_err());
        assert!(
            classify_return(&ty("std::string::String")).is_err(),
            "qualified paths stay rejected"
        );
        for src in ["Option<i32>", "Option<u64>", "Option<bool>"] {
            assert!(
                classify_return(&ty(src)).is_err(),
                "`{src}` is not a buffer"
            );
        }
    }

    #[test]
    fn rejected_param_types_fail_classification() {
        let cases = [
            "&mut str",
            "&mut [u8]",
            "&[i32]",
            "&u32",
            "str",
            "i128",
            "usize",
            "char",
            "String",
            "Vec<u8>",
            "*const u8",
        ];
        for src in cases {
            let parsed = ty(src);
            let result = classify_param(&parsed);
            assert!(result.is_err(), "`{src}` must be rejected");
        }
    }

    #[test]
    fn path_helpers_respect_their_contracts() {
        assert_eq!(path_ident(&ty("u32")), Some(("u32".to_owned(), false)));
        assert_eq!(path_ident(&ty("Vec<u8>")), Some(("Vec".to_owned(), true)));
        assert_eq!(path_ident(&ty("std::string::String")), None);
        assert_eq!(path_ident(&ty("()")), None);
        assert!(matches!(
            path_kind(&ty("bool")),
            Some(PathKind::Prim(PrimTy::Bool))
        ));
        assert!(matches!(
            path_kind(&ty("u64")),
            Some(PathKind::BigInt(BigIntTy::U64))
        ));
        assert!(path_kind(&ty("Vec<u8>")).is_none());
        assert!(is_str_type(&ty("str")));
        assert!(!is_str_type(&ty("u8")));
        assert!(is_u8(&ty("u8")));
        assert!(!is_u8(&ty("i8")));
    }

    #[test]
    fn bare_named_types_classify_as_records() {
        for src in ["Point", "JobStatus", "my_module::Point"] {
            let kind = classify_param(&ty(src)).expect("accepted");
            assert!(
                matches!(kind, ShimKind::Record(_)),
                "`{src}` must classify as Record"
            );
            let ret = classify_return(&ty(src)).expect("accepted");
            assert!(
                matches!(ret, RetKind::Record(_)),
                "`{src}` must classify as a record return"
            );
        }
    }

    #[test]
    fn owned_buffers_stay_return_only_params() {
        // `String`/`CopiedBuf`/`Vec<u8>` params keep their rejection:
        // the owned-buffer channel is return-only.
        for src in ["String", "CopiedBuf", "Vec<u8>"] {
            assert!(
                classify_param(&ty(src)).is_err(),
                "`{src}` stays a rejected param"
            );
        }
    }

    #[test]
    fn vec_sequences_classify_with_their_item_kinds() {
        let point: syn::Path = syn::parse_str("Point").expect("path");
        let cases: &[(&str, SeqItem)] = &[
            ("Vec<i32>", SeqItem::Narrow),
            ("Vec<u16>", SeqItem::Narrow),
            ("Vec<u32>", SeqItem::Wide),
            ("Vec<f64>", SeqItem::Wide),
            ("Vec<i64>", SeqItem::I64),
            ("Vec<bool>", SeqItem::Bool),
            ("Vec<String>", SeqItem::Str),
            (
                "Vec<Point>",
                SeqItem::Record(crate::support::kind::KindPath(point)),
            ),
        ];
        for (src, expected_item) in cases {
            let kind = classify_param(&ty(src)).expect("accepted");
            let ShimKind::Seq(item) = kind else {
                panic!("`{src}` must classify as Seq");
            };
            assert_eq!(&item, expected_item, "item kind for `{src}`");
            let ret = classify_return(&ty(src)).expect("accepted");
            let RetKind::Seq(ret_item) = ret else {
                panic!("`{src}` must classify as a seq return");
            };
            assert_eq!(&ret_item, expected_item, "ret item for `{src}`");
        }
    }

    #[test]
    fn vec_of_unsupported_items_is_rejected() {
        for src in ["Vec<char>", "Vec<u64>", "Vec<Option<u32>>", "Vec<Vec<u8>>"] {
            assert!(
                classify_param(&ty(src)).is_err(),
                "`{src}` param must be rejected"
            );
            assert!(
                classify_return(&ty(src)).is_err(),
                "`{src}` return must be rejected"
            );
        }
    }

    #[test]
    fn record_ts_kinds_are_pre_quoted_expressions() {
        let kind = classify_param(&ty("Point")).expect("accepted");
        let TsKind::Expr(tokens) = ts_type(&kind) else {
            panic!("record params must map to TsKind::Expr");
        };
        let text = tokens.to_string().replace(' ', "");
        assert!(text.contains("Point::BFFI_TS_TYPE"), "got: {text}");

        let ret = classify_return(&ty("Vec<f64>")).expect("accepted");
        assert_eq!(ts_return(&ret), TsKind::NumberArray);
    }
}
