//! `#[bffi_async]`: async function parsing and spawn-shim codegen.
//!
//! An `async fn name(params) -> R` annotated with `#[bffi_async]`
//! emits, exactly like `#[bffi]`, the original item plus a spawn shim
//! `bffi_<name>(params..., __ret: *mut u64) -> ErrorCode`: the shim
//! converts the (synchronous) parameters, moves them into the future,
//! spawns it on the built-in executor and writes the task handle to
//! `__ret`. JavaScript turns the handle into a `Promise` with
//! `wrapTask` (see examples/async).
//!
//! Borrowed parameters (`&str`, `&[u8]`) are rejected: they cannot
//! cross the spawn boundary (the borrow must outlive the future, but
//! the cstring/view only lives for the shim call). The owned shapes -
//! `String` and `Vec<u8>` - are accepted and moved in.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;

use crate::errors;
use crate::support;
use crate::support::classify::{generic_args, is_u8, path_ident, path_kind, ts_prim, ts_promise};
use crate::support::kind::{BigIntTy, PrimTy, RetKind, TsKind};
use crate::support::paths::{self, PathCtx, is_crate_name};

/// One validated async parameter kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsyncParam {
    /// A small primitive.
    Prim(PrimTy),
    /// A 64-bit integer.
    BigInt(BigIntTy),
    /// An owned `String` (moved into the future).
    OwnedStr,
    /// An owned `Vec<u8>` (moved into the future).
    OwnedBytes,
}

impl AsyncParam {
    /// The TypeScript kind of the parameter.
    fn ts_kind(self) -> TsKind {
        match self {
            Self::Prim(prim) => ts_prim(prim),
            Self::BigInt(_) => TsKind::BigInt,
            Self::OwnedStr => TsKind::String,
            Self::OwnedBytes => TsKind::Uint8Array,
        }
    }
}

/// A validated `#[bffi_async]` function.
pub(crate) struct AsyncFnModel {
    /// Function identifier.
    pub ident: syn::Ident,
    /// Doc lines, one leading space trimmed.
    pub docs: Vec<String>,
    /// Path context (default or facade-only mode).
    pub paths: PathCtx,
    /// Parameters, in declaration order.
    pub params: Vec<(String, AsyncParam)>,
    /// The validated return kind.
    pub ret: RetKind,
}

/// The parsed `crate = "<name>"` attribute argument, shared with the
/// sync macro: absent selects the default facade roots, `"direct"`
/// the pre-merge roots, any other valid name that facade's
/// namespaces.
fn parse_paths(attrs: &TokenStream) -> syn::Result<PathCtx> {
    if attrs.is_empty() {
        return Ok(PathCtx::default());
    }
    let text = attrs.to_string();
    let cleaned = text.replace(' ', "");
    let Some(rest) = cleaned.strip_prefix("crate=") else {
        return Err(errors::attr_options(attrs.span()));
    };
    let name = rest.trim_matches('"');
    if !is_crate_name(name) {
        return Err(errors::attr_options(attrs.span()));
    }
    Ok(paths::from_option(name))
}

impl AsyncFnModel {
    /// Parses and validates the annotated async function.
    pub(crate) fn parse(attrs: &TokenStream, item: TokenStream) -> syn::Result<AsyncFnModel> {
        let paths = parse_paths(attrs)?;
        let func: syn::ItemFn = syn::parse2(item)?;

        if func.sig.asyncness.is_none() {
            return Err(errors::fn_shape(
                func.sig.span(),
                "sync function (`#[bffi_async]` requires `async fn`; use `#[bffi]` for sync functions)",
            ));
        }
        if !func.sig.generics.params.is_empty() || func.sig.generics.where_clause.is_some() {
            return Err(errors::fn_shape(
                func.sig.generics.span(),
                "generic async function",
            ));
        }
        if func.sig.unsafety.is_some() {
            return Err(errors::fn_shape(func.sig.span(), "unsafe async function"));
        }
        if func.sig.variadic.is_some() {
            return Err(errors::fn_shape(func.sig.span(), "variadic async function"));
        }
        if func.sig.abi.is_some() {
            return Err(errors::fn_shape(func.sig.span(), "extern async function"));
        }
        if func.sig.constness.is_some() {
            return Err(errors::fn_shape(func.sig.span(), "const async function"));
        }

        let mut params = Vec::new();
        for arg in &func.sig.inputs {
            let syn::FnArg::Typed(arg) = arg else {
                return Err(errors::fn_shape(
                    arg.span(),
                    "async fn with a `self` receiver",
                ));
            };
            let name = match &*arg.pat {
                syn::Pat::Ident(pat) => pat.ident.to_string(),
                syn::Pat::Wild(_) => "_".to_owned(),
                other => {
                    return Err(errors::fn_shape(
                        other.span(),
                        "non-identifier parameter pattern (use `name: Type` or `_`)",
                    ));
                }
            };
            let kind = classify_async_param(&arg.ty, &name)?;
            params.push((name, kind));
        }

        let ret = match &func.sig.output {
            syn::ReturnType::Default => RetKind::Unit,
            syn::ReturnType::Type(_, ty) => {
                let ret = support::classify::classify_return(ty)
                    .map_err(|unsupported| errors::return_type(unsupported.span, unsupported.ty))?;
                if matches!(
                    ret,
                    RetKind::Nullable(_) | RetKind::NullableRecord(_) | RetKind::NullableSeq(_)
                ) {
                    return Err(errors::async_nullable_return(ty.span(), ty));
                }
                ret
            }
        };

        let docs = crate::support::util::extract_docs(&func.attrs);

        Ok(Self {
            ident: func.sig.ident.clone(),
            docs,
            paths,
            params,
            ret,
        })
    }
}

/// Classifies an owned async parameter: primitives, bigints, `bool`,
/// `String` and `Vec<u8>`. Borrowed parameters (`&str`, `&[u8]`) are
/// rejected with the async-specific help.
fn classify_async_param(ty: &syn::Type, name: &str) -> syn::Result<AsyncParam> {
    if let Some(kind) = path_kind(ty) {
        return Ok(match kind {
            support::classify::PathKind::Prim(prim) => AsyncParam::Prim(prim),
            support::classify::PathKind::BigInt(big) => AsyncParam::BigInt(big),
        });
    }
    if let Some((single, false)) = path_ident(ty) {
        match single.as_str() {
            "String" => return Ok(AsyncParam::OwnedStr),
            "Vec" => {
                let args = generic_args(ty);
                if args.len() == 1 && is_u8(args[0]) {
                    return Ok(AsyncParam::OwnedBytes);
                }
            }
            _ => {}
        }
    }
    Err(errors::async_param_type(ty.span(), ty, name))
}

/// The async spawn shim: parameter declarations, the conversion
/// preamble and the future-tail codegen.
pub(crate) fn expand(model: &AsyncFnModel) -> TokenStream {
    let shim_ident = format_ident!("bffi_{}", model.ident);
    let shim_doc = format!(
        "Spawns the async task `{}` and writes the task handle to `__ret` (generated by `#[bffi_async]`).",
        model.ident
    );
    let paths = &model.paths;
    let core = &paths.core;
    let types = &paths.types;
    let async_root = &paths.r#async;

    let mut shim_params: Vec<TokenStream> = Vec::new();
    let mut conversions = TokenStream::new();
    let mut call_args: Vec<TokenStream> = Vec::new();
    for (index, (name, kind)) in model.params.iter().enumerate() {
        let ident = support::codegen::param_ident(name, index);
        match kind {
            AsyncParam::Prim(prim) => {
                let ty = support::codegen::prim_ty(*prim);
                shim_params.push(quote! { #ident: #ty });
                call_args.push(quote! { #ident });
            }
            AsyncParam::BigInt(big) => {
                let ty = support::codegen::bigint_ty(*big);
                shim_params.push(quote! { #ident: #ty });
                call_args.push(quote! { #ident });
            }
            AsyncParam::OwnedStr => {
                let ptr = format_ident!("{ident}_ptr");
                shim_params.push(quote! { #ptr: *const ::std::os::raw::c_char });
                conversions.extend(quote! {
                    if #ptr.is_null() {
                        let error = #core::BffiError::new(
                            #core::ErrorCode::NullPointer,
                            "string argument pointer is null",
                        );
                        #core::set_last_error(error);
                        return #core::ErrorCode::NullPointer;
                    }
                    // SAFETY: bun:ffi hands out NUL-terminated cstrings for
                    // string parameters (DESIGN.md 6.3); null-checked above.
                    let #ident = unsafe { ::std::ffi::CStr::from_ptr(#ptr) }
                        .to_bytes();
                    let #ident = match #types::unsafe_zero_copy::str_view(#ident) {
                        Ok(v) => v.as_str().to_owned(),
                        Err(error) => {
                            #core::set_last_error(error);
                            return #core::ErrorCode::InvalidUtf8;
                        }
                    };
                });
                call_args.push(quote! { #ident });
            }
            AsyncParam::OwnedBytes => {
                let ptr = format_ident!("{ident}_ptr");
                let len = format_ident!("{ident}_len");
                shim_params.push(quote! { #ptr: *const u8, #len: u64 });
                conversions.extend(quote! {
                    if #len > 0 && #ptr.is_null() {
                        let error = #core::BffiError::new(
                            #core::ErrorCode::NullPointer,
                            "buffer argument pointer is null",
                        );
                        #core::set_last_error(error);
                        return #core::ErrorCode::NullPointer;
                    }
                    // SAFETY: bun:ffi keeps the TypedArray pointer valid
                    // for the duration of the call; `len == 0` permits a
                    // null pointer (an empty slice is valid).
                    let #ident =
                        unsafe { ::std::slice::from_raw_parts(#ptr, #len as usize) }
                            .to_vec();
                });
                call_args.push(quote! { #ident });
            }
        }
    }

    let ident = &model.ident;
    let call = quote! { #ident(#(#call_args,)*) };
    let tail = async_tail(paths, &model.ret, call);
    let future = quote! {
        let __fut = async move {
            #tail
        };
        match #async_root::spawn(Box::pin(__fut)) {
            Ok(handle) => {
                // SAFETY: `__ret` is non-null (checked above) and valid
                // for one `u64` write per the bun:ffi out-parameter
                // contract.
                unsafe { ::std::ptr::write(__ret, handle.as_u64()); }
                #core::ErrorCode::Ok
            }
            Err(error) => {
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
            // The owned-parameter conversions run BEFORE the spawn:
            // they copy the cstring/(ptr, len) views into owned data
            // that outlives the call (and may early-return an error).
            #conversions
            #future
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
                #future
            })
        }
    };

    quote! { #debug_shim #release_shim }
}

/// The boundary kind of an owned async parameter at the ABI level:
/// `String` crosses as a cstring pointer, `Vec<u8>` as a `(ptr, len)`
/// view (the shim copies both into the future before the spawn).
fn abi_kind(kind: AsyncParam) -> crate::support::kind::ShimKind {
    match kind {
        AsyncParam::Prim(prim) => crate::support::kind::ShimKind::Prim(prim),
        AsyncParam::BigInt(big) => crate::support::kind::ShimKind::BigInt(big),
        AsyncParam::OwnedStr => crate::support::kind::ShimKind::Str,
        AsyncParam::OwnedBytes => crate::support::kind::ShimKind::BufferView,
    }
}

/// The descriptor module: the promise-returning function definition.
pub(crate) fn async_meta(model: &AsyncFnModel) -> TokenStream {
    let module = format_ident!("bffi_meta_{}", model.ident);
    let js_name = model.ident.to_string();
    let export_name = format!("bffi_{}", model.ident);
    let module_doc = format!(
        "Metadata for the `{js_name}` async task descriptor (consumed by bffi-dts / bffi-build)."
    );
    let const_doc = format!("The `FunctionDef` descriptor for `{js_name}`.");

    let docs = model.docs.iter().map(|doc| quote! { #doc });
    let paths = &model.paths;
    let dts = &paths.dts;
    let params = model.params.iter().map(|(name, kind)| {
        let ty = kind.ts_kind().tokens(paths);
        quote! { #dts::ParamDef { name: #name, ty: #ty } }
    });
    let ret = ts_promise(&model.ret).tokens(paths);
    let abi =
        support::abi::abi_sig_task(model.params.iter().map(|(_, kind)| abi_kind(*kind)), paths);

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

/// The future body for a return kind: awaits the call expression and
/// converts the output into `Ok(AsyncValue)`; `Result` errors become
/// `Err(BffiError)` through the domain channel.
fn async_tail(paths: &PathCtx, ret: &RetKind, call: TokenStream) -> TokenStream {
    let core = &paths.core;
    let async_root = &paths.r#async;
    match ret {
        RetKind::Unit => quote! {
            #call.await;
            ::std::result::Result::Ok(#async_root::AsyncValue::Unit)
        },
        RetKind::Result(inner) => {
            let ok_tail = async_value_from(paths, inner);
            quote! {
                match #call.await {
                    ::std::result::Result::Ok(__value) => {
                        ::std::result::Result::Ok(#ok_tail)
                    }
                    ::std::result::Result::Err(__err) => {
                        ::std::result::Result::Err(#core::BffiError::with_source(
                            #core::ErrorCode::DomainError,
                            ::std::string::ToString::to_string(&__err),
                            ::std::boxed::Box::new(__err),
                        ))
                    }
                }
            }
        }
        other => {
            let conversion = async_value_from(paths, other);
            quote! {
                let __value = #call.await;
                ::std::result::Result::Ok(#conversion)
            }
        }
    }
}

/// The `AsyncValue` conversion expression for an already-bound
/// `__value`.
fn async_value_from(paths: &PathCtx, ret: &RetKind) -> TokenStream {
    let async_root = &paths.r#async;
    match ret {
        RetKind::Unit => quote! { #async_root::AsyncValue::Unit },
        _ => quote! { #async_root::AsyncValue::from(__value) },
    }
}
