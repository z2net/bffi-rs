//! The B1 derive macros: `#[derive(BffiRecord)]` and
//! `#[derive(BffiEnum)]`.
//!
//! A derived record gains `BFFI_TS_TYPE` (`TsType::Record("Name")`),
//! the `BFFI_RECORD_DEF` descriptor table entry, and the wire
//! codec pair `bffi_wire_encode` / `bffi_wire_decode` over the
//! value-level helpers of `bffi::types::wire`. `Option<T>` fields
//! encode `None` as the `TAG_UNIT` byte and `Some(v)` as the inner
//! value record. `Vec<T>` fields ride the shared sequence matrix
//! (`TAG_SEQ` framing, the same item kinds as function-level
//! sequences). A derived enum
//! gains the same shape with `TsType::Enum("Name")`: a unit-only
//! enum encodes as its variant name string; an enum with payload
//! variants wraps every variant in a kind envelope (a `TAG_RECORD`
//! whose first field is the variant name, followed by the payload
//! fields positionally through the same `FieldKind` matrix - tuple
//! fields are named `_0`..).
//!
//! The expansions name the facade paths (`::bffi::types::wire`,
//! `::bffi::dts`, `::bffi::core`) unconditionally: the published
//! integration is `bffi` alone (the `crate = "direct"` escape hatch
//! of the attribute macros does not extend to derives).
//!
//! Rejections carry the `E009`-`E011` codes:
//!
//! | Code  | Meaning                                               |
//! | ----- | ----------------------------------------------------- |
//! | `E009` | unsupported record shape (tuple/unit struct, generics) |
//! | `E010` | unsupported record/variant field type                 |
//! | `E011` | unsupported enum shape (generics)                     |

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;

use crate::support::classify::seq_item;
use crate::support::kind::{SeqItem, TsKind};
use crate::support::paths::PathCtx;
use crate::support::util::extract_docs;

/// The `#[derive(BffiRecord)]` entry point (declared in `lib.rs`,
/// which owns the `proc_macro_derive` shims; this module holds the
/// expansion).
pub(crate) fn record(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item: syn::DeriveInput = syn::parse_macro_input!(input as syn::DeriveInput);
    match expand_record(&item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// The `#[derive(BffiEnum)]` entry point.
pub(crate) fn enumeration(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item: syn::DeriveInput = syn::parse_macro_input!(input as syn::DeriveInput);
    match expand_enum(&item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// The record expansion.
fn expand_record(item: &syn::DeriveInput) -> syn::Result<TokenStream2> {
    let name = &item.ident;
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(shape_error(
            item.generics.span(),
            &name.to_string(),
            "generic records",
        ));
    }
    let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(named),
        ..
    }) = &item.data
    else {
        return Err(shape_error(
            item.span(),
            &name.to_string(),
            "tuple/unit structs",
        ));
    };

    let mut fields = Vec::new();
    for field in &named.named {
        // Unreachable for named fields; the shape match above
        // guarantees the ident.
        let Some(ident) = field.ident.as_ref() else {
            continue;
        };
        let kind = FieldKind::classify(&field.ty, ident)?;
        fields.push((ident.clone(), kind, extract_docs(&field.attrs)));
    }

    let self_ty = quote! { #name };
    let docs = extract_docs(&item.attrs);
    Ok(record_shape_tokens(
        &self_ty,
        &name.to_string(),
        &docs,
        &fields,
    ))
}

/// The shared record-shape expansion: the descriptor consts plus the
/// `BffiWire` impl over `self_ty`. The derive passes the plain struct
/// name; [`crate::instantiation`] passes a concrete generic
/// instantiation (`Pair<u32>`), which resolves through the public
/// alias the macro emits beside the impls.
pub(crate) fn record_shape_tokens(
    self_ty: &TokenStream2,
    js_name: &str,
    docs: &[String],
    fields: &[(syn::Ident, FieldKind, Vec<String>)],
) -> TokenStream2 {
    let mut field_defs = Vec::new();
    let mut encode_stmts = Vec::new();
    let mut decode_stmts = Vec::new();
    let mut construct_fields = Vec::new();

    for (ident, kind, field_docs) in fields {
        let field_name = ident.to_string();
        let ty_expr = kind.ts_expr();

        field_defs.push(quote! {
            ::bffi::dts::RecordFieldDef {
                name: #field_name,
                docs: &[#(#field_docs),*],
                ty: #ty_expr,
            }
        });
        encode_stmts.push(kind.encode_stmt(ident));
        decode_stmts.push(kind.decode_stmt(ident));
        construct_fields.push(kind.construct_field(ident));
    }

    let field_count = fields.len();

    quote! {
        #[automatically_derived]
        impl #self_ty {
            /// The TS-facing reference to this record.
            pub const BFFI_TS_TYPE: ::bffi::dts::TsType =
                ::bffi::dts::TsType::Record(#js_name);

            /// The descriptor table entry for `ModuleDef::records`.
            pub const BFFI_RECORD_DEF: ::bffi::dts::RecordDef =
                ::bffi::dts::RecordDef {
                    js_name: #js_name,
                    docs: &[#(#docs),*],
                    fields: &[#(#field_defs),*],
                };
        }

        #[automatically_derived]
        impl ::bffi::types::wire::BffiWire for #self_ty {
            /// Appends this value as one complete wire record.
            fn bffi_wire_encode(&self, out: &mut ::std::vec::Vec<u8>) {
                use ::bffi::types::wire as __w;
                __w::encode_record_header(out, #field_count);
                #(#encode_stmts)*
            }

            /// Decodes one wire record at `offset`; returns the value
            /// and the offset past it.
            fn bffi_wire_decode(
                bytes: &[u8],
                offset: usize,
            ) -> ::core::result::Result<(Self, usize), ::bffi::core::BffiError> {
                use ::bffi::types::wire as __w;
                let (field_count, __off) = __w::decode_record_header(bytes, offset)?;
                if field_count != #field_count {
                    return ::core::result::Result::Err(
                        ::bffi::core::BffiError::new(
                            ::bffi::core::ErrorCode::InvalidArgument,
                            "wire: record field count mismatch",
                        ),
                    );
                }
                #(#decode_stmts)*
                ::core::result::Result::Ok((
                    Self { #(#construct_fields,)* },
                    __off,
                ))
            }
        }
    }
}

/// One supported field type of a record.
pub(crate) enum FieldKind {
    /// `i8 i16 i32 u8 u16` - wire `i32`, TS `number`.
    NarrowInt,
    /// `u32 f32 f64` - wire `f64`, TS `number`.
    WideNumber,
    /// `i64` - wire `i64`, TS `bigint`.
    Int64,
    /// `u64` - wire `u64` (exact, TAG 10), TS `bigint`.
    UInt64,
    /// `bool`.
    Bool,
    /// `String`.
    Str,
    /// `Vec<u8>`.
    Bytes,
    /// A `Vec<T>` of a supported sequence item: rides the shared
    /// `TAG_SEQ` framing (the same item matrix as function-level
    /// sequences).
    Seq(SeqItem),
    /// A nested `BffiRecord`/`BffiEnum` type.
    Named(syn::Path),
    /// `Option<inner>` over one of the kinds above: `None` rides the
    /// `TAG_UNIT` byte, `Some` the inner value record as-is.
    Opt(Box<Self>),
}

impl FieldKind {
    /// Classifies one field type or rejects it with `E010`.
    pub(crate) fn classify(ty: &syn::Type, field: &syn::Ident) -> syn::Result<Self> {
        let syn::Type::Path(syn::TypePath { qself: None, path }) = ty else {
            return Err(field_type_error(ty, field));
        };
        let Some(last) = path.segments.last() else {
            return Err(field_type_error(ty, field));
        };
        if path.segments.len() != 1 {
            // A qualified path (`foo::Bar`) is a nested type.
            return Ok(Self::Named(path.clone()));
        }
        if last.arguments.is_none() {
            match last.ident.to_string().as_str() {
                "i8" | "i16" | "i32" | "u8" | "u16" => return Ok(Self::NarrowInt),
                "u32" | "f32" | "f64" => return Ok(Self::WideNumber),
                "i64" => return Ok(Self::Int64),
                "u64" => return Ok(Self::UInt64),
                "bool" => return Ok(Self::Bool),
                "String" => return Ok(Self::Str),
                _ => return Ok(Self::Named(path.clone())),
            }
        }
        if last.ident == "Vec" {
            if let syn::PathArguments::AngleBracketed(args) = &last.arguments
                && args.args.len() == 1
                && let syn::GenericArgument::Type(inner) = &args.args[0]
            {
                if is_u8(inner) {
                    return Ok(Self::Bytes);
                }
                // A sequence field rides the shared item matrix;
                // consulted only inside the `Vec` arm so bare-name
                // fields keep their `Named` classification.
                if let Some(item) = seq_item(inner) {
                    return Ok(Self::Seq(item));
                }
            }
            let ty_text = quote::ToTokens::to_token_stream(ty).to_string();
            return Err(syn::Error::new(
                ty.span(),
                format!(
                    "bffi[E010]: unsupported field type `{ty_text}` on `{field}`; supported \
                     sequence items: i8|i16|i32|u8|u16|u32|f32|f64|i64|u64|bool|String|\
                     Vec<u8>|records (B1 v1)"
                ),
            ));
        }
        if last.ident == "Option" {
            return Self::classify_option(ty, last, field);
        }
        Err(field_type_error(ty, field))
    }

    /// Classifies `Option<inner>`: the inner type through the same
    /// matrix; `Option<Option<T>>` is an `E010` rejection (v1).
    fn classify_option(
        ty: &syn::Type,
        segment: &syn::PathSegment,
        field: &syn::Ident,
    ) -> syn::Result<Self> {
        let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
            return Err(field_type_error(ty, field));
        };
        if args.args.len() != 1 {
            return Err(field_type_error(ty, field));
        }
        let inner = match &args.args[0] {
            syn::GenericArgument::Type(inner) => inner,
            _ => return Err(field_type_error(ty, field)),
        };
        if is_option(inner) {
            let ty_text = quote::ToTokens::to_token_stream(ty).to_string();
            return Err(syn::Error::new(
                ty.span(),
                format!(
                    "bffi[E010]: nested `{ty_text}` on `{field}`; `Option<Option<T>>` is \
                     not supported (v1)"
                ),
            ));
        }
        Ok(Self::Opt(Box::new(Self::classify(inner, field)?)))
    }

    /// The `TsType` expression of the field.
    pub(crate) fn ts_expr(&self) -> TokenStream2 {
        match self {
            Self::NarrowInt | Self::WideNumber => {
                quote! { ::bffi::dts::TsType::Number }
            }
            Self::Int64 => quote! { ::bffi::dts::TsType::BigInt },
            Self::UInt64 => quote! { ::bffi::dts::TsType::BigInt },
            Self::Bool => quote! { ::bffi::dts::TsType::Boolean },
            Self::Str => quote! { ::bffi::dts::TsType::String },
            Self::Bytes => quote! { ::bffi::dts::TsType::Uint8Array },
            Self::Seq(item) => seq_array_kind(item).tokens(&PathCtx::default()),
            Self::Named(path) => quote! { #path::BFFI_TS_TYPE },
            Self::Opt(inner) => Self::nullable_expr(inner),
        }
    }

    /// The nullable `TsType` expression of an `Option` field's inner
    /// kind: the flat `| null` flavors of the IR, routed through the
    /// shared [`TsKind`] bridge.
    fn nullable_expr(inner: &Self) -> TokenStream2 {
        let kind = match inner {
            Self::NarrowInt | Self::WideNumber => TsKind::NullableNumber,
            Self::Int64 | Self::UInt64 => TsKind::NullableBigInt,
            Self::Bool => TsKind::NullableBoolean,
            Self::Str => TsKind::NullableString,
            Self::Bytes => TsKind::NullableUint8Array,
            Self::Seq(item) => match item {
                SeqItem::Narrow | SeqItem::Wide => TsKind::NullableNumberArray,
                SeqItem::I64 | SeqItem::U64 => TsKind::NullableBigIntArray,
                SeqItem::Bool => TsKind::NullableBooleanArray,
                SeqItem::Str => TsKind::NullableStringArray,
                SeqItem::Bytes => TsKind::NullableUint8ArrayArray,
                SeqItem::Record(path) => TsKind::NullableRecordArray(
                    path.0
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string())
                        .unwrap_or_default(),
                ),
            },
            Self::Named(path) => TsKind::NullableRecord(
                path.segments
                    .last()
                    .map(|segment| segment.ident.to_string())
                    .unwrap_or_default(),
            ),
            // Unreachable: `classify` rejects nested options before
            // an `Opt` can wrap one; the arm exists for
            // exhaustiveness.
            Self::Opt(_) => TsKind::Void,
        };
        kind.tokens(&PathCtx::default())
    }

    /// One encode statement for the field.
    fn encode_stmt(&self, ident: &syn::Ident) -> TokenStream2 {
        self.encode_access(quote! { self.#ident })
    }

    /// One encode statement over any access expression of this kind's
    /// type; the `Option` wrapper branches on the value.
    fn encode_access(&self, access: TokenStream2) -> TokenStream2 {
        match self {
            Self::NarrowInt => quote! { __w::encode_i32(out, i32::from(#access)); },
            Self::WideNumber => quote! { __w::encode_f64(out, #access as f64); },
            Self::Int64 => quote! { __w::encode_i64(out, #access); },
            Self::UInt64 => quote! { __w::encode_u64(out, #access); },
            Self::Bool => quote! { __w::encode_bool(out, #access); },
            Self::Str => quote! { __w::encode_str(out, &#access); },
            Self::Bytes => quote! { __w::encode_bytes(out, &#access); },
            Self::Seq(item) => {
                let push = seq_item_encode_stmt(item);
                quote! {
                    __w::encode_seq_header(out, #access.len());
                    for __item in &#access {
                        #push
                    }
                }
            }
            Self::Named(path) => {
                // Fully-qualified through the trait: the expansion never
                // depends on `BffiWire` being in scope, and a non-derived
                // nested type fails with the trait bound (E0277).
                quote! { <#path as ::bffi::types::wire::BffiWire>::bffi_wire_encode(&#access, out); }
            }
            Self::Opt(inner) => {
                let some = Self::encode_access_ref(inner);
                quote! {
                    match &#access {
                        ::core::option::Option::None => out.push(__w::TAG_UNIT),
                        ::core::option::Option::Some(__v) => { #some }
                    }
                }
            }
        }
    }

    /// The encode statement of an `Option`'s inner kind over the
    /// borrowed binding `__v` (the `Some` arm of [`Self::encode_access`]).
    fn encode_access_ref(inner: &Self) -> TokenStream2 {
        match inner {
            Self::NarrowInt => quote! { __w::encode_i32(out, i32::from(*__v)); },
            Self::WideNumber => quote! { __w::encode_f64(out, *__v as f64); },
            Self::Int64 => quote! { __w::encode_i64(out, *__v); },
            Self::UInt64 => quote! { __w::encode_u64(out, *__v); },
            Self::Bool => quote! { __w::encode_bool(out, *__v); },
            Self::Str => quote! { __w::encode_str(out, __v); },
            Self::Bytes => quote! { __w::encode_bytes(out, __v); },
            Self::Seq(item) => {
                let push = seq_item_encode_stmt(item);
                quote! {
                    __w::encode_seq_header(out, __v.len());
                    for __item in __v.iter() {
                        #push
                    }
                }
            }
            Self::Named(path) => {
                // Fully-qualified through the trait (see `encode_access`).
                quote! { <#path as ::bffi::types::wire::BffiWire>::bffi_wire_encode(__v, out); }
            }
            // Unreachable: `classify` rejects nested options before
            // an `Opt` can wrap one; the arm exists for
            // exhaustiveness.
            Self::Opt(_) => quote! {},
        }
    }

    /// One decode statement for the field.
    fn decode_stmt(&self, ident: &syn::Ident) -> TokenStream2 {
        match self {
            Self::Opt(inner) => {
                let value = syn::Ident::new("__v", ident.span());
                let some = inner.decode_stmt(&value);
                quote! {
                    let (#ident, __off) = match bytes.get(__off) {
                        ::core::option::Option::Some(&__w::TAG_UNIT) => {
                            (::core::option::Option::None, __off + 1)
                        }
                        ::core::option::Option::Some(_) => {
                            #some
                            (::core::option::Option::Some(#value), __off)
                        }
                        ::core::option::Option::None => {
                            return ::core::result::Result::Err(
                                ::bffi::core::BffiError::new(
                                    ::bffi::core::ErrorCode::InvalidArgument,
                                    "wire: truncated optional field",
                                ),
                            );
                        }
                    };
                }
            }
            // The wire carries i32/f64; the construction site narrows
            // back to the exact field width.
            Self::NarrowInt => quote! {
                let (#ident, __off) = __w::decode_i32(bytes, __off)?;
            },
            // The JS encoder picks i32/f64 by value: decode both.
            Self::WideNumber => quote! {
                let (#ident, __off) = __w::decode_number(bytes, __off)?;
            },
            Self::Int64 => quote! {
                let (#ident, __off) = __w::decode_i64(bytes, __off)?;
            },
            Self::UInt64 => quote! {
                let (#ident, __off) = __w::decode_u64_lenient(bytes, __off)?;
            },
            Self::Bool => quote! {
                let (#ident, __off) = __w::decode_bool(bytes, __off)?;
            },
            Self::Str => quote! {
                let (__v, __off) = __w::decode_str(bytes, __off)?;
                let #ident = ::std::string::String::from(__v);
            },
            Self::Bytes => quote! {
                let (__v, __off) = __w::decode_bytes(bytes, __off)?;
                let #ident = ::std::vec::Vec::from(__v);
            },
            Self::Seq(item) => {
                let item_decode = seq_field_item_decode(item, ident);
                quote! {
                    let (seq_count, mut seq_off) = __w::decode_seq_header(bytes, __off)?;
                    // A hostile count is bounded by the header check
                    // (MAX_WIRE_PAYLOAD); the reservation only pre-sizes
                    // a sane fraction of it, like the shim decode does.
                    let mut #ident =
                        ::std::vec::Vec::with_capacity((seq_count as usize).min(4096));
                    for _ in 0..seq_count {
                        #item_decode
                    }
                    let __off = seq_off;
                }
            }
            Self::Named(path) => quote! {
                // Fully-qualified through the trait (see `encode_access`).
                let (#ident, __off) =
                    <#path as ::bffi::types::wire::BffiWire>::bffi_wire_decode(bytes, __off)?;
            },
        }
    }

    /// The construction field for `Self { ... }` (narrows the decoded
    /// wire value back to the exact field width where needed).
    fn construct_field(&self, ident: &syn::Ident) -> TokenStream2 {
        match self {
            Self::NarrowInt | Self::WideNumber => quote! { #ident: #ident as _ },
            Self::Opt(inner) if matches!(**inner, Self::NarrowInt | Self::WideNumber) => {
                quote! { #ident: #ident.map(|__v| __v as _) }
            }
            _ => quote! { #ident },
        }
    }
}

/// The array `TsKind` of one sequence item kind (the field-level
/// wrapper around the shared matrix).
fn seq_array_kind(item: &SeqItem) -> TsKind {
    match item {
        SeqItem::Narrow | SeqItem::Wide => TsKind::NumberArray,
        SeqItem::I64 | SeqItem::U64 => TsKind::BigIntArray,
        SeqItem::Bool => TsKind::BooleanArray,
        SeqItem::Str => TsKind::StringArray,
        SeqItem::Bytes => TsKind::Uint8ArrayArray,
        SeqItem::Record(path) => TsKind::RecordArray(
            path.0
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default(),
        ),
    }
}

/// The encode statement for one sequence item (bound to `__item`, a
/// shared reference) writing into `out`.
fn seq_item_encode_stmt(item: &SeqItem) -> TokenStream2 {
    match item {
        SeqItem::Narrow => quote! { __w::encode_i32(out, *__item as i32); },
        SeqItem::Wide => quote! { __w::encode_f64(out, *__item as f64); },
        SeqItem::I64 => quote! { __w::encode_i64(out, *__item); },
        SeqItem::U64 => quote! { __w::encode_u64(out, *__item); },
        SeqItem::Bool => quote! { __w::encode_bool(out, *__item); },
        SeqItem::Str => quote! { __w::encode_str(out, __item); },
        SeqItem::Bytes => quote! { __w::encode_bytes(out, __item); },
        SeqItem::Record(path) => {
            let path = &path.0;
            quote! { <#path as ::bffi::types::wire::BffiWire>::bffi_wire_encode(__item, out); }
        }
    }
}

/// The decode statement for one sequence item inside the field loop:
/// advances `seq_off` over `bytes` and pushes the value into `#ident`.
fn seq_field_item_decode(item: &SeqItem, ident: &syn::Ident) -> TokenStream2 {
    let decode = match item {
        SeqItem::Narrow => quote! { __w::decode_i32(bytes, seq_off)? },
        SeqItem::Wide => quote! { __w::decode_number(bytes, seq_off)? },
        SeqItem::I64 => quote! { __w::decode_i64(bytes, seq_off)? },
        SeqItem::U64 => quote! { __w::decode_u64_lenient(bytes, seq_off)? },
        SeqItem::Bool => quote! { __w::decode_bool(bytes, seq_off)? },
        SeqItem::Str => quote! { __w::decode_str(bytes, seq_off)? },
        SeqItem::Bytes => quote! { __w::decode_bytes(bytes, seq_off)? },
        SeqItem::Record(path) => {
            let path = &path.0;
            quote! { <#path as ::bffi::types::wire::BffiWire>::bffi_wire_decode(bytes, seq_off)? }
        }
    };
    let push = match item {
        SeqItem::Narrow | SeqItem::Wide => quote! { #ident.push(value as _) },
        SeqItem::I64 | SeqItem::U64 | SeqItem::Bool | SeqItem::Record(_) => {
            quote! { #ident.push(value) }
        }
        SeqItem::Str => quote! { #ident.push(::std::string::String::from(value)) },
        SeqItem::Bytes => quote! { #ident.push(::std::vec::Vec::from(value)) },
    };
    quote! {
        let (value, next) = #decode;
        seq_off = next;
        #push;
    }
}

/// Whether the type is syntactically a single-segment `Option<_>`.
fn is_option(ty: &syn::Type) -> bool {
    let syn::Type::Path(syn::TypePath { qself: None, path }) = ty else {
        return false;
    };
    path.segments.len() == 1
        && path.segments[0].ident == "Option"
        && !path.segments[0].arguments.is_none()
}

/// Whether the type is literally `u8`.
fn is_u8(ty: &syn::Type) -> bool {
    let syn::Type::Path(syn::TypePath { qself: None, path }) = ty else {
        return false;
    };
    path.segments.len() == 1 && path.segments[0].ident == "u8"
}

/// The enum expansion.
///
/// Unit-only enums keep the byte-identical variant-name string shape.
/// An enum with payload variants wraps EVERY variant in a kind
/// envelope: a `TAG_RECORD` whose first field is the variant name,
/// followed by the payload fields positionally (the same
/// `FieldKind` matrix as records; tuple fields are named `_0`..).
fn expand_enum(item: &syn::DeriveInput) -> syn::Result<TokenStream2> {
    let name = &item.ident;
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(enum_shape_error(
            item.generics.span(),
            &name.to_string(),
            "generic enums",
        ));
    }
    let syn::Data::Enum(syn::DataEnum { variants, .. }) = &item.data else {
        return Err(enum_shape_error(
            item.span(),
            &name.to_string(),
            "non-enum items",
        ));
    };

    let mut variant_defs = Vec::new();
    let mut has_payload = false;
    // The unit-only shape (below) needs the bare name list; the
    // mixed shape carries per-variant codegen.
    let mut variant_names: Vec<String> = Vec::new();
    let mut encode_arms = Vec::new();
    let mut decode_arms = Vec::new();

    for variant in variants {
        let vname = variant.ident.to_string();
        let docs = extract_docs(&variant.attrs);
        // One payload field: its wire name, its binding ident and
        // the classified kind.
        let mut fields: Vec<(String, syn::Ident, FieldKind, Vec<String>)> = Vec::new();
        match &variant.fields {
            syn::Fields::Named(named) => {
                for field in &named.named {
                    let Some(ident) = field.ident.as_ref() else {
                        continue;
                    };
                    let kind = FieldKind::classify(&field.ty, ident)?;
                    fields.push((
                        ident.to_string(),
                        ident.clone(),
                        kind,
                        extract_docs(&field.attrs),
                    ));
                }
            }
            syn::Fields::Unnamed(unnamed) => {
                for (index, field) in unnamed.unnamed.iter().enumerate() {
                    let ident = syn::Ident::new(&format!("_{index}"), field.span());
                    let kind = FieldKind::classify(&field.ty, &ident)?;
                    fields.push((ident.to_string(), ident, kind, Vec::new()));
                }
            }
            syn::Fields::Unit => {}
        }
        has_payload |= !fields.is_empty();

        let field_defs = fields.iter().map(|(fname, _, kind, fdocs)| {
            let ty_expr = kind.ts_expr();
            quote! {
                ::bffi::dts::RecordFieldDef {
                    name: #fname,
                    docs: &[#(#fdocs),*],
                    ty: #ty_expr,
                }
            }
        });
        variant_defs.push(quote! {
            ::bffi::dts::EnumVariantDef {
                name: #vname,
                docs: &[#(#docs),*],
                fields: &[#(#field_defs),*],
            }
        });
        variant_names.push(vname.clone());

        // Only the mixed shape needs match arms; keep building them
        // unconditionally (the unit-only path ignores them).
        let ident = &variant.ident;
        let count = 1 + fields.len();
        let pattern = match &variant.fields {
            syn::Fields::Named(_) => {
                let idents = fields.iter().map(|(_, ident, _, _)| ident);
                quote! { { #(#idents),* } }
            }
            syn::Fields::Unnamed(_) => {
                let idents = fields.iter().map(|(_, ident, _, _)| ident);
                quote! { (#(#idents),*) }
            }
            syn::Fields::Unit => quote! {},
        };
        let field_encodes = fields.iter().map(|(_, ident, kind, _)| {
            // Matching on `&Self` binds each field by reference; the
            // place expression `(*#ident)` lets every kind encode as
            // if it held the value.
            kind.encode_access(quote! { (*#ident) })
        });
        encode_arms.push(quote! {
            Self::#ident #pattern => {
                __w::encode_record_header(out, #count);
                __w::encode_str(out, #vname);
                #(#field_encodes)*
            }
        });

        let field_decodes = fields
            .iter()
            .map(|(_, ident, kind, _)| kind.decode_stmt(ident));
        // The construction of one decoded field: the value-only form
        // for tuple variants, the `name:`-prefixed form for named
        // ones (narrow widths narrow back from the wire carrier).
        let named_fields = matches!(&variant.fields, syn::Fields::Named(_));
        let constructed = fields.iter().map(|(_, ident, kind, _)| {
            let value = match kind {
                FieldKind::NarrowInt | FieldKind::WideNumber => quote! { #ident as _ },
                FieldKind::Opt(inner)
                    if matches!(**inner, FieldKind::NarrowInt | FieldKind::WideNumber) =>
                {
                    quote! { #ident.map(|__v| __v as _) }
                }
                _ => quote! { #ident },
            };
            if named_fields {
                quote! { #ident: #value }
            } else {
                value
            }
        });
        let construct = match &variant.fields {
            syn::Fields::Named(_) => quote! { Self::#ident { #(#constructed),* } },
            syn::Fields::Unnamed(_) => quote! { Self::#ident(#(#constructed),*) },
            syn::Fields::Unit => quote! { Self::#ident },
        };
        decode_arms.push(quote! {
            #vname => {
                if count != #count {
                    return ::core::result::Result::Err(
                        ::bffi::core::BffiError::new(
                            ::bffi::core::ErrorCode::InvalidArgument,
                            "wire: variant field count mismatch",
                        ),
                    );
                }
                #(#field_decodes)*
                ::core::result::Result::Ok((#construct, __off))
            }
        });
    }

    let js_name = name.to_string();
    let docs = extract_docs(&item.attrs);

    let wire_impl = if has_payload {
        quote! {
            #[automatically_derived]
            impl ::bffi::types::wire::BffiWire for #name {
                /// Appends this value as one complete wire record (the
                /// kind envelope: variant name + positional payload).
                fn bffi_wire_encode(&self, out: &mut ::std::vec::Vec<u8>) {
                    use ::bffi::types::wire as __w;
                    match self {
                        #(#encode_arms)*
                    }
                }

                /// Decodes one wire record at `offset`; returns the value
                /// and the offset past it.
                fn bffi_wire_decode(
                    bytes: &[u8],
                    offset: usize,
                ) -> ::core::result::Result<(Self, usize), ::bffi::core::BffiError> {
                    use ::bffi::types::wire as __w;
                    let (count, __off) = __w::decode_record_header(bytes, offset)?;
                    let (__kind, __off) = __w::decode_str(bytes, __off)?;
                    match __kind {
                        #(#decode_arms)*
                        // The kind string is arbitrary input: unknown
                        // names are an error, not a panic.
                        _ => ::core::result::Result::Err(
                            ::bffi::core::BffiError::new(
                                ::bffi::core::ErrorCode::InvalidArgument,
                                "wire: unknown variant",
                            ),
                        ),
                    }
                }
            }
        }
    } else {
        let match_encode_arms = variants.iter().map(|variant| {
            let ident = &variant.ident;
            let vname = variant.ident.to_string();
            quote! { Self::#ident => #vname, }
        });
        let match_decode_arms = variants.iter().map(|variant| {
            let ident = &variant.ident;
            let vname = variant.ident.to_string();
            quote! { #vname => Self::#ident, }
        });
        let variant_list = quote! { &[#(#variant_names),*] };
        quote! {
            #[automatically_derived]
            impl ::bffi::types::wire::BffiWire for #name {
                /// Appends this value as one complete wire record (its
                /// variant name).
                fn bffi_wire_encode(&self, out: &mut ::std::vec::Vec<u8>) {
                    use ::bffi::types::wire as __w;
                    let __name = match self { #(#match_encode_arms)* };
                    __w::encode_str(out, __name);
                }

                /// Decodes one wire record at `offset`; returns the value
                /// and the offset past it.
                fn bffi_wire_decode(
                    bytes: &[u8],
                    offset: usize,
                ) -> ::core::result::Result<(Self, usize), ::bffi::core::BffiError> {
                    use ::bffi::types::wire as __w;
                    let (name, __off) = __w::decode_variant(bytes, offset, #variant_list)?;
                    let value = match name {
                        #(#match_decode_arms)*
                        // decode_variant guarantees the name is one of the
                        // declared variants; the arm exists for exhaustiveness.
                        _ => return ::core::result::Result::Err(
                            ::bffi::core::BffiError::new(
                                ::bffi::core::ErrorCode::InvalidArgument,
                                "wire: unknown variant",
                            ),
                        ),
                    };
                    ::core::result::Result::Ok((value, __off))
                }
            }
        }
    };

    Ok(quote! {
        #[automatically_derived]
        impl #name {
            /// The TS-facing reference to this enum.
            pub const BFFI_TS_TYPE: ::bffi::dts::TsType =
                ::bffi::dts::TsType::Enum(#js_name);

            /// The descriptor table entry for `ModuleDef::enums`.
            pub const BFFI_ENUM_DEF: ::bffi::dts::EnumDef =
                ::bffi::dts::EnumDef {
                    js_name: #js_name,
                    docs: &[#(#docs),*],
                    variants: &[#(#variant_defs),*],
                };
        }

        #wire_impl
    })
}

/// The `E009` record-shape rejection.
fn shape_error(span: proc_macro2::Span, name: &str, what: &str) -> syn::Error {
    syn::Error::new(
        span,
        format!("bffi[E009]: record `{name}`: {what} are not supported (B1 v1)"),
    )
}

/// The `E010` field-type rejection.
fn field_type_error(ty: &syn::Type, field: &syn::Ident) -> syn::Error {
    let ty_text = quote::ToTokens::to_token_stream(ty).to_string();
    syn::Error::new(
        ty.span(),
        format!(
            "bffi[E010]: unsupported field type `{ty_text}` on `{field}`; \
                 supported: i8|i16|i32|u8|u16|u32|f32|f64|i64|u64|bool|String|Vec<u8>|\
                 Vec<T> (supported items)|Option<T>|nested records"
        ),
    )
}

/// The `E011` enum-shape rejection.
fn enum_shape_error(span: proc_macro2::Span, name: &str, what: &str) -> syn::Error {
    syn::Error::new(
        span,
        format!("bffi[E011]: enum `{name}`: {what} are not supported (B1 v1)"),
    )
}
