//! The parsed and validated `#[bffi_stream]` function model: a
//! plain fn returning `impl Iterator<Item = T> + Send` whose spawn
//! shim registers the (item-encoded) iterator in the stream table
//! and writes the stream handle to the out-parameter.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;

use crate::support::codegen::{param_conversions, param_ident, shim_param};
use crate::support::kind::{ShimKind, TsKind};
use crate::support::paths::{self, PathCtx};
use crate::support::util::extract_docs;
use crate::support::{abi, classify};

/// A validated `#[bffi_stream]` function.
pub(crate) struct StreamFnModel {
    /// Function identifier.
    pub ident: syn::Ident,
    /// Doc lines, one leading space trimmed.
    pub docs: Vec<String>,
    /// Path context (facade default, `crate = "..."` redirect).
    pub paths: PathCtx,
    /// Parameters, in declaration order.
    pub params: Vec<(String, ShimKind)>,
    /// The classified item kind of the returned iterator.
    pub item: StreamItem,
}

/// The classified item kind of a stream.
pub(crate) enum StreamItem {
    /// `i8 i16 i32 u8 u16` items - one `i32` wire record each.
    Narrow,
    /// `u32 f32 f64` items - one `f64` wire record each.
    Wide,
    /// `i64` items.
    Int64,
    /// `bool` items.
    Bool,
    /// `String` items.
    Str,
    /// `Vec<u8>` items - one raw-bytes record each.
    Bytes,
    /// Nested `BffiRecord`/`BffiEnum` items.
    Record(syn::Path),
}

impl StreamItem {
    /// Classifies the `Item` type of the returned iterator or
    /// rejects it with `E012`.
    fn classify(ty: &syn::Type) -> syn::Result<Self> {
        let syn::Type::Path(syn::TypePath { qself: None, path }) = ty else {
            return Err(item_type_error(ty));
        };
        let Some(last) = path.segments.last() else {
            return Err(item_type_error(ty));
        };
        if path.segments.len() == 1 && last.arguments.is_none() {
            match last.ident.to_string().as_str() {
                "i8" | "i16" | "i32" | "u8" | "u16" => return Ok(Self::Narrow),
                "u32" | "f32" | "f64" => return Ok(Self::Wide),
                "i64" => return Ok(Self::Int64),
                "bool" => return Ok(Self::Bool),
                "String" => return Ok(Self::Str),
                "u64" => {
                    return Err(syn::Error::new(
                        ty.span(),
                        "bffi[E012]: u64 stream items are not supported yet (wire exactness); use i64",
                    ));
                }
                _ => return Ok(Self::Record(path.clone())),
            }
        }
        if path.segments.len() == 1 && last.ident == "Vec" {
            if let syn::PathArguments::AngleBracketed(args) = &last.arguments
                && args.args.len() == 1
                && let syn::GenericArgument::Type(inner) = &args.args[0]
                && classify::is_u8(inner)
            {
                return Ok(Self::Bytes);
            }
            return Err(item_type_error(ty));
        }
        Ok(Self::Record(path.clone()))
    }

    /// The `TsKind` of the stream descriptor's return type.
    fn ts_kind(&self) -> TsKind {
        match self {
            Self::Narrow | Self::Wide => TsKind::StreamNumber,
            Self::Int64 => TsKind::StreamBigInt,
            Self::Bool => TsKind::StreamBoolean,
            Self::Str => TsKind::StreamString,
            Self::Bytes => TsKind::StreamUint8Array,
            Self::Record(path) => {
                let name = path
                    .segments
                    .last()
                    .map(|seg| seg.ident.to_string())
                    .unwrap_or_default();
                TsKind::StreamExpr(quote! { ::bffi::dts::TsType::StreamRecord(#name) })
            }
        }
    }

    /// One item-encode statement inside the stream adapter (the item
    /// is bound to `#ident`, the record buffer to `rec`).
    fn encode_stmt(&self, wire: &TokenStream, ident: &syn::Ident) -> TokenStream {
        match self {
            Self::Narrow => quote! { #wire::encode_i32(&mut rec, #ident as i32); },
            Self::Wide => quote! { #wire::encode_f64(&mut rec, #ident as f64); },
            Self::Int64 => quote! { #wire::encode_i64(&mut rec, #ident); },
            Self::Bool => quote! { #wire::encode_bool(&mut rec, #ident); },
            Self::Str => quote! { #wire::encode_str(&mut rec, &#ident); },
            Self::Bytes => quote! { #wire::encode_bytes(&mut rec, &#ident); },
            Self::Record(path) => {
                quote! { #path::bffi_wire_encode(&#ident, &mut rec); }
            }
        }
    }
}

/// Parses and validates the annotated stream function.
pub(crate) fn parse(attrs: &TokenStream, item: TokenStream) -> syn::Result<StreamFnModel> {
    let paths = parse_paths(attrs)?;
    let func: syn::ItemFn = syn::parse2(item)?;

    if func.sig.asyncness.is_some() {
        return Err(crate::errors::fn_shape(
            func.sig.span(),
            "async function (`#[bffi_stream]` requires a plain fn)",
        ));
    }
    if !func.sig.generics.params.is_empty() || func.sig.generics.where_clause.is_some() {
        return Err(crate::errors::fn_shape(
            func.sig.generics.span(),
            "generic function",
        ));
    }

    let mut params = Vec::new();
    for (index, arg) in func.sig.inputs.iter().enumerate() {
        let syn::FnArg::Typed(pat) = arg else {
            return Err(crate::errors::fn_shape(
                arg.span(),
                "method receiver (`#[bffi_stream]` requires plain functions)",
            ));
        };
        let name = match pat.pat.as_ref() {
            syn::Pat::Ident(pat_ident) => pat_ident.ident.to_string(),
            _ => format!("__arg{index}"),
        };
        let kind = classify::classify_param(&pat.ty).map_err(|unsupported| {
            crate::errors::param_type(unsupported.span, unsupported.ty, &name)
        })?;
        params.push((name, kind));
    }

    // The return type must be `impl Iterator<Item = T> (+ Send)`.
    let syn::ReturnType::Type(_, ret_ty) = &func.sig.output else {
        return Err(stream_shape(
            func.sig.output.span(),
            "the fn must return `impl Iterator<Item = T> + Send`",
        ));
    };
    let item = parse_impl_iterator(ret_ty)?;

    Ok(StreamFnModel {
        ident: func.sig.ident.clone(),
        docs: extract_docs(&func.attrs),
        paths,
        params,
        item,
    })
}

/// Extracts the `Item` binding from `impl Iterator<Item = T> ...`.
fn parse_impl_iterator(ret_ty: &syn::Type) -> syn::Result<StreamItem> {
    let syn::Type::ImplTrait(impl_trait) = ret_ty else {
        return Err(stream_shape(
            ret_ty.span(),
            "the fn must return `impl Iterator<Item = T> + Send`",
        ));
    };
    for bound in &impl_trait.bounds {
        let syn::TypeParamBound::Trait(trait_bound) = bound else {
            continue;
        };
        let Some(last) = trait_bound.path.segments.last() else {
            continue;
        };
        if last.ident != "Iterator" {
            continue;
        }
        let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
            continue;
        };
        for arg in &args.args {
            let syn::GenericArgument::AssocType(assoc) = arg else {
                continue;
            };
            if assoc.ident != "Item" {
                continue;
            }
            return StreamItem::classify(&assoc.ty);
        }
    }
    Err(stream_shape(
        ret_ty.span(),
        "the `impl Iterator` binding must carry `Item = T`",
    ))
}

/// The spawn-shim expansion: the original fn unchanged, the shim
/// registering the item-encoded iterator, and the descriptor module.
pub(crate) fn expand(model: &StreamFnModel) -> TokenStream {
    let shim_ident = format_ident!("bffi_{}", model.ident);
    let shim_doc = format!(
        "Spawns the stream `{}` and writes the stream handle to `__ret` (generated by `#[bffi_stream]`).",
        model.ident
    );
    let paths = &model.paths;
    let core = &paths.core;
    let types = &paths.types;
    let wire = quote! { #types::wire };

    let mut shim_params: Vec<TokenStream> = Vec::new();
    let mut call_args: Vec<TokenStream> = Vec::new();
    for (index, (name, kind)) in model.params.iter().enumerate() {
        let param = shim_param(name, kind.clone(), index);
        shim_params.push(param);
        let ident = param_ident(name, index);
        call_args.push(match kind {
            ShimKind::Str | ShimKind::BufferView => {
                let view = format_ident!("{ident}_view");
                quote! { &#view }
            }
            _ => quote! { #ident },
        });
    }

    let conversions = param_conversions(
        &model.paths,
        model
            .params
            .iter()
            .map(|(name, kind)| (name.as_str(), kind.clone())),
    );

    let ident = &model.ident;
    let encode = model.item.encode_stmt(&wire, &format_ident!("__item"));
    let spawn = quote! {
        let __iter = #ident(#(#call_args,)*);
        let __encoded = __iter.map(|__item| {
            let mut rec = ::std::vec::Vec::<u8>::new();
            #encode
            rec
        });
        match ::bffi::bffi_stream::spawn(::std::boxed::Box::new(__encoded)) {
            ::std::result::Result::Ok(handle) => {
                // SAFETY: `__ret` is non-null (checked above) and valid
                // for one `u64` write per the bun:ffi out-parameter
                // contract.
                unsafe { ::std::ptr::write(__ret, handle.as_u64()); }
                #core::ErrorCode::Ok
            }
            ::std::result::Result::Err(error) => {
                let converted: #core::BffiError = error.into();
                let code = converted.code;
                #core::set_last_error(converted);
                code
            }
        }
    };

    let debug_shim = quote! {
        #[cfg(debug_assertions)]
        #[unsafe(no_mangle)]
        #[doc = #shim_doc]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn #shim_ident(#(#shim_params,)* __ret: *mut u64) -> #core::ErrorCode {
            #conversions
            #spawn
        }
    };
    let release_shim = quote! {
        #[cfg(not(debug_assertions))]
        #[unsafe(no_mangle)]
        #[doc = #shim_doc]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn #shim_ident(#(#shim_params,)* __ret: *mut u64) -> #core::ErrorCode {
            #core::boundary::run_extern_body(move || {
                #conversions
                #spawn
            })
        }
    };

    quote! { #debug_shim #release_shim }
}

/// The descriptor module: the stream-returning function definition.
pub(crate) fn stream_meta(model: &StreamFnModel) -> TokenStream {
    let module = format_ident!("bffi_meta_{}", model.ident);
    let js_name = model.ident.to_string();
    let export_name = format!("bffi_{}", model.ident);
    let module_doc = format!(
        "Metadata for the `{js_name}` stream descriptor (consumed by bffi-dts / bffi-build)."
    );
    let const_doc = format!("The `FunctionDef` descriptor for `{js_name}`.");

    let docs = model.docs.iter().map(|doc| quote! { #doc });
    let paths = &model.paths;
    let dts = &paths.dts;
    let params = model.params.iter().map(|(name, kind)| {
        let ty = classify::ts_type(kind).tokens(paths);
        quote! { #dts::ParamDef { name: #name, ty: #ty } }
    });
    let ret = model.item.ts_kind().tokens(paths);
    let param_shapes = model
        .params
        .iter()
        .map(|(_, kind)| abi::abi_param(kind.clone(), paths));
    let abi = quote! {
        #dts::AbiSig {
            params: &[#(#param_shapes),*],
            out: ::std::option::Option::Some(#dts::AbiOut::Handle),
        }
    };

    quote! {
        #[doc = #module_doc]
        pub mod #module {
            #[doc = #const_doc]
            pub const FUNCTION: #dts::FunctionDef = #dts::FunctionDef {
                js_name: #js_name,
                export_name: #export_name,
                docs: &[#(#docs),*],
                params: &[#(#params),*],
                ret: #ret,
                abi: #abi,
            };
        }
    }
}

/// The parsed `crate = "<name>"` attribute argument, shared with the
/// other macros.
fn parse_paths(attrs: &TokenStream) -> syn::Result<PathCtx> {
    if attrs.is_empty() {
        return Ok(PathCtx::default());
    }
    let text = attrs.to_string();
    let cleaned = text.replace(' ', "");
    let Some(rest) = cleaned.strip_prefix("crate=") else {
        return Err(crate::errors::attr_options(attrs.span()));
    };
    let name = rest.trim_matches('"');
    if !paths::is_crate_name(name) {
        return Err(crate::errors::attr_options(attrs.span()));
    }
    Ok(paths::from_option(name))
}

/// The `E012` stream-shape rejection.
fn stream_shape(span: proc_macro2::Span, what: &str) -> syn::Error {
    syn::Error::new(span, format!("bffi[E012]: {what}"))
}

/// The `E012` stream-item rejection.
fn item_type_error(ty: &syn::Type) -> syn::Error {
    let ty_text = quote::ToTokens::to_token_stream(ty).to_string();
    syn::Error::new(
        ty.span(),
        format!(
            "bffi[E012]: unsupported stream item type `{ty_text}`; \
             supported: i8|i16|i32|u8|u16|u32|f32|f64|i64|bool|String|Vec<u8>|nested records"
        ),
    )
}
