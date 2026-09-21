//! The `bffi_impl_wire!` explicit instantiation macro.
//!
//! A proc-macro cannot see through a generic struct's
//! monomorphization sites, and the wire identity is a const
//! `&'static str` - so a concrete instantiation registers under an
//! explicit alias:
//!
//! ```ignore
//! bffi_impl_wire! {
//!     /// A pair of unsigned 32-bit values.
//!     Pair<u32> as PairU32 {
//!         /// The first value.
//!         first: u32,
//!         second: u32,
//!     }
//! }
//! ```
//!
//! The expansion emits the `pub type PairU32 = Pair<u32>;` alias
//! (the name `#[bffi]` parameters and the TS surface reference), the
//! `BFFI_TS_TYPE`/`BFFI_RECORD_DEF` consts and the `BffiWire` impl
//! for the concrete instantiation - the exact record-shape tokens of
//! [`crate::derive`], so the two paths cannot drift. Field types ride
//! the same matrix (E010 for unsupported ones); the declared types
//! must mirror the instantiation (the generated `Self { .. }`
//! construction and the `self.field` encodes are checked against the
//! real struct at compile time).
//!
//! Two shape rules keep the generated impls legal Rust: the target
//! must be a path type over a LOCAL generic struct (the orphan rule
//! rejects foreign types - E016), and the alias identifies the
//! instantiation, so one generic struct may register under several
//! wire names (`Pair<u32> as PairU32`, `Pair<f64> as PairF64`, ...).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;

use crate::derive::{FieldKind, record_shape_tokens};
use crate::support::diagnostics::{DESIGN_NOTE, MacroDiagnostic};
use crate::support::util::extract_docs;

/// The parsed macro input: outer docs, the target type, the `as
/// Alias` identifier and the braced field list.
struct Input {
    docs: Vec<syn::Attribute>,
    self_ty: syn::Type,
    alias: syn::Ident,
    fields: Vec<(syn::Ident, syn::Type, Vec<syn::Attribute>)>,
}

impl Parse for Input {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let docs = input.call(syn::Attribute::parse_outer)?;
        let self_ty: syn::Type = input.parse()?;
        let _ = input.parse::<syn::Token![as]>()?;
        let alias: syn::Ident = input.parse()?;
        let content;
        syn::braced!(content in input);
        let mut fields = Vec::new();
        while !content.is_empty() {
            let attrs = content.call(syn::Attribute::parse_outer)?;
            let ident: syn::Ident = content.parse()?;
            let _ = content.parse::<syn::Token![:]>()?;
            let ty: syn::Type = content.parse()?;
            if !content.is_empty() {
                let _ = content.parse::<syn::Token![,]>()?;
            }
            fields.push((ident, ty, attrs));
        }
        Ok(Input {
            docs,
            self_ty,
            alias,
            fields,
        })
    }
}

/// The `bffi_impl_wire!` entry point (declared in `lib.rs`, which
/// owns the `proc_macro` shim; this module holds the expansion).
pub(crate) fn expand(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let parsed = syn::parse_macro_input!(input as Input);
    match expand_input(&parsed) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// The expansion: the alias item plus the shared record-shape impls.
fn expand_input(input: &Input) -> syn::Result<TokenStream2> {
    if !matches!(input.self_ty, syn::Type::Path(_)) {
        let ty_text = quote::ToTokens::to_token_stream(&input.self_ty).to_string();
        return Err(MacroDiagnostic::new(
            "E016",
            format!("unsupported instantiation target `{ty_text}`"),
        )
        .with_help(
            "use a path type over a local generic struct: `Pair<u32> as PairU32 { first: u32, .. }`",
        )
        .with_note("the target must be local to your crate: the generated `BffiWire` impl would break the orphan rule otherwise")
        .with_note(DESIGN_NOTE)
        .to_compile_error(input.self_ty.span()));
    }

    let mut fields = Vec::new();
    for (ident, ty, attrs) in &input.fields {
        let kind = FieldKind::classify(ty, ident)?;
        fields.push((ident.clone(), kind, extract_docs(attrs)));
    }

    let alias = &input.alias;
    let self_ty = &input.self_ty;
    let docs = extract_docs(&input.docs);
    let js_name = alias.to_string();
    let impls = record_shape_tokens(&quote! { #self_ty }, &js_name, &docs, &fields);

    Ok(quote! {
        #(#[doc = #docs])*
        pub type #alias = #self_ty;

        #impls
    })
}
