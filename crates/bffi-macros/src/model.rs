//! The parsed and validated function model.
//!
//! [`FnModel::parse`] turns the annotated item into a normalized model
//! (identity, docs, params, return) that the shim and
//! descriptor generators in later stages consume. Every input outside
//! the P1 boundary rules is rejected here with a spanned error, so the
//! downstream stages can rely on the shape being valid.
//!
//! The boundary kind model lives in `crate::support::kind`; this
//! crate keeps the `ItemFn` parsing and its own `E001`-anchored shape
//! diagnostics.

use crate::errors::{attr_options, fn_shape, param_pattern};
use crate::mapping;
use crate::support::kind::{RetKind, ShimKind};
use crate::support::paths::{self, PathCtx, is_crate_name};
use crate::support::util::extract_docs;
use proc_macro2::TokenStream;
use syn::spanned::Spanned;
use syn::{FnArg, ItemFn, Pat, ReturnType};

/// One validated parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FnParam {
    /// Parameter name as written (identifier or `_`).
    pub name: String,
    /// Boundary kind of the parameter type.
    pub kind: ShimKind,
}

/// A validated `#[bffi]` function: everything later stages need to
/// generate the C ABI shim and the `bffi_meta` descriptor.
#[derive(Clone)]
pub(crate) struct FnModel {
    /// Function name.
    pub ident: syn::Ident,
    /// Doc-comment lines with exactly one leading space trimmed
    /// (`/// Adds.` becomes `Adds.`).
    pub docs: Vec<String>,
    /// Parameters in declaration order (receivers are rejected).
    pub params: Vec<FnParam>,
    /// Validated return type.
    pub ret: RetKind,
    /// The crate roots the generated code names (default: the direct
    /// dependencies; `crate = "..."`: the facade namespaces).
    pub paths: PathCtx,
}

impl FnModel {
    /// Parses and validates the annotated item against the P1
    /// boundary rules.
    ///
    /// `attrs` may carry at most one `crate = "<name>"` option (the
    /// facade-only mode); `item` must be a plain, non-generic,
    /// non-async, safe `fn` over the P1 type set. Rejections are
    /// spanned on the offending tokens and carry the documented help
    /// lines.
    pub(crate) fn parse(attrs: &TokenStream, item: TokenStream) -> syn::Result<FnModel> {
        let paths = parse_paths(attrs)?;
        let func: ItemFn = syn::parse2(item)?;
        validate_shape(&func.sig)?;

        let mut params = Vec::new();
        for arg in &func.sig.inputs {
            // Receivers are rejected by `validate_shape`, so every
            // remaining argument is a typed parameter.
            let FnArg::Typed(arg) = arg else { continue };
            // Only identifiers name a boundary parameter; `_` is
            // accepted and keeps the positional shim fallback. Anything
            // else (parenthesized, destructuring, renaming, ...) is
            // rejected so the descriptor never carries a non-identifier
            // name.
            let name = match &*arg.pat {
                Pat::Ident(pat) => pat.ident.to_string(),
                Pat::Wild(_) => "_".to_owned(),
                other => return Err(param_pattern(other.span())),
            };
            let kind = mapping::classify_param(&arg.ty, &name)?;
            params.push(FnParam { name, kind });
        }

        let ret = match &func.sig.output {
            ReturnType::Default => RetKind::Unit,
            ReturnType::Type(_, ty) => mapping::classify_return(ty)?,
        };

        Ok(FnModel {
            ident: func.sig.ident,
            docs: extract_docs(&func.attrs),
            params,
            ret,
            paths,
        })
    }
}

/// The parsed attribute options of `#[bffi]`: at most one
/// `crate = "<name>"`.
struct AttrPaths {
    crate_name: Option<String>,
}

impl syn::parse::Parse for AttrPaths {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut crate_name: Option<String> = None;
        while !input.is_empty() {
            // `parse_any`: `crate` is a Rust keyword, which the plain
            // `Ident` parse rejects.
            let key: syn::Ident = input.call(syn::ext::IdentExt::parse_any)?;
            if key != "crate" {
                return Err(syn::Error::new(key.span(), "unknown option"));
            }
            if crate_name.is_some() {
                return Err(syn::Error::new(key.span(), "duplicate `crate` option"));
            }
            input.parse::<syn::Token![=]>()?;
            let value: syn::LitStr = input.parse()?;
            crate_name = Some(value.value());
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(Self { crate_name })
    }
}

/// Resolves the attribute options into the path context: no attribute
/// selects the default facade roots (`::bffi::core`, ...);
/// `crate = "<name>"` redirects every generated path to
/// `::<name>::{core, types, dts, object, build}`, and the special
/// `crate = "direct"` selects the pre-merge dependency roots.
/// Anything else (unknown keys, non-literal or invalid values,
/// duplicates) is the `E004` rejection.
fn parse_paths(attrs: &TokenStream) -> syn::Result<PathCtx> {
    if attrs.is_empty() {
        return Ok(PathCtx::default());
    }
    let AttrPaths { crate_name } =
        syn::parse2(attrs.clone()).map_err(|_| attr_options(attrs.span()))?;
    match crate_name {
        Some(name) if is_crate_name(&name) => Ok(paths::from_option(&name)),
        _ => Err(attr_options(attrs.span())),
    }
}

/// Rejects every non-plain function shape, earliest violation first,
/// with the error spanned on the offending token.
fn validate_shape(sig: &syn::Signature) -> syn::Result<()> {
    if let Some(tokens) = &sig.asyncness {
        return Err(fn_shape(tokens.span(), "async function"));
    }
    if !sig.generics.params.is_empty() {
        return Err(fn_shape(sig.generics.params.span(), "generic function"));
    }
    if let Some(where_clause) = &sig.generics.where_clause {
        return Err(fn_shape(where_clause.span(), "generic function"));
    }
    if let Some(tokens) = &sig.unsafety {
        return Err(fn_shape(tokens.span(), "unsafe function"));
    }
    if let Some(FnArg::Receiver(recv)) = sig.inputs.first() {
        return Err(fn_shape(recv.span(), "method (self receiver)"));
    }
    if let Some(tokens) = &sig.variadic {
        return Err(fn_shape(tokens.span(), "variadic function"));
    }
    if let Some(tokens) = &sig.abi {
        return Err(fn_shape(tokens.span(), "extern abi function"));
    }
    if let Some(tokens) = &sig.constness {
        return Err(fn_shape(tokens.span(), "const function"));
    }
    Ok(())
}
