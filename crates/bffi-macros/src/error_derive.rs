//! The `#[derive(BffiError)]` expansion: typed domain error enums
//! with stable user codes.
//!
//! Each variant carries `#[bffi(code = 0x1001)]` (the reserved user
//! range `0x1000..=0xFFFF`, unique within the enum). The expansion
//! generates `Display` (the variant name), `std::error::Error`, and
//! `From<Self> for BffiError`: code = the user code (crosses in the
//! ABI status), variant = the enum variant name (JS `e.name`),
//! payload = the variant's named fields as one `TAG_RECORD` wire
//! record (JS `e.payload`; fields use the `BffiStreamItem` matrix).
//!
//! Rejections: `E013` (unsupported shape, missing/duplicate code,
//! out-of-range code), `E014` (unsupported field type).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;

use crate::derive::FieldKind;
use crate::support::util::extract_docs;

/// The `#[derive(BffiError)]` entry point (declared in `lib.rs`).
pub(crate) fn error(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item: syn::DeriveInput = match syn::parse(input) {
        Ok(item) => item,
        Err(err) => return err.to_compile_error().into(),
    };
    match expand(&item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// One validated variant: name, user code, named fields.
struct ErrorVariant {
    ident: syn::Ident,
    code: u32,
    fields: Vec<ErrorField>,
    docs: Vec<String>,
}

/// One named field of a variant: its ident, type and docs.
struct ErrorField {
    ident: syn::Ident,
    ty: syn::Type,
    docs: Vec<String>,
}

/// The enum expansion.
fn expand(item: &syn::DeriveInput) -> syn::Result<TokenStream2> {
    let name = &item.ident;
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            item.generics.span(),
            format!("bffi[E013]: error enum `{name}`: generics are not supported (B3 v1)"),
        ));
    }
    let syn::Data::Enum(syn::DataEnum { variants, .. }) = &item.data else {
        return Err(syn::Error::new(
            item.span(),
            format!("bffi[E013]: `{name}`: BffiError derives on enums only"),
        ));
    };

    let mut defs: Vec<ErrorVariant> = Vec::new();
    for variant in variants {
        if let syn::Fields::Unnamed(_) = &variant.fields {
            return Err(syn::Error::new(
                variant.span(),
                format!(
                    "bffi[E013]: error enum `{name}`: tuple variants are not supported; \
                     use named fields or a unit variant"
                ),
            ));
        }
        let code = parse_code(variant, name)?;
        for existing in &defs {
            if existing.code == code {
                return Err(syn::Error::new(
                    variant.span(),
                    format!(
                        "bffi[E013]: error enum `{name}`: duplicate code 0x{:04X} (also on `{}`)",
                        code, existing.ident
                    ),
                ));
            }
        }
        let mut fields = Vec::new();
        if let syn::Fields::Named(named) = &variant.fields {
            for field in &named.named {
                let Some(ident) = field.ident.as_ref() else {
                    continue;
                };
                fields.push(ErrorField {
                    ident: ident.clone(),
                    ty: field.ty.clone(),
                    docs: extract_docs(&field.attrs),
                });
            }
        }
        defs.push(ErrorVariant {
            ident: variant.ident.clone(),
            code,
            fields,
            docs: extract_docs(&variant.attrs),
        });
    }
    if defs.is_empty() {
        return Err(syn::Error::new(
            item.span(),
            format!("bffi[E013]: error enum `{name}`: at least one variant is required"),
        ));
    }

    let js_name = name.to_string();
    let enum_docs = extract_docs(&item.attrs);

    let display_arms = defs.iter().map(|def| {
        let ident = &def.ident;
        let vname = def.ident.to_string();
        quote! { Self::#ident { .. } => #vname, }
    });

    let from_arms = defs.iter().map(|def| {
        let ident = &def.ident;
        let vname = def.ident.to_string();
        let code_lit = def.code;
        if def.fields.is_empty() {
            quote! {
                #name::#ident {} => ::bffi::BffiError::new(
                    ::bffi::ErrorCode::DomainError,
                    #vname,
                )
                .with_user_code(#code_lit)
                .with_variant(#vname),
            }
        } else {
            let field_names = def.fields.iter().map(|f| &f.ident);
            let field_count = def.fields.len();
            let encodes = def.fields.iter().map(|f| {
                let ident = &f.ident;
                quote! {
                    ::bffi::BffiStreamItem::encode_into(&#ident, &mut __payload);
                }
            });
            quote! {
                #name::#ident { #(#field_names),* } => {
                    let mut __payload = ::std::vec::Vec::<u8>::new();
                    ::bffi::bffi_types::wire::encode_record_header(&mut __payload, #field_count);
                    #(#encodes)*
                    ::bffi::BffiError::new(
                        ::bffi::ErrorCode::DomainError,
                        #vname,
                    )
                    .with_user_code(#code_lit)
                    .with_variant(#vname)
                    .with_payload(__payload)
                }
            }
        }
    });

    // The descriptor table entry: variant name/code/fields. Field
    // types go through the same matrix as records; a rejection
    // carries the E014 code (the error-enum flavor of E010).
    let mut variant_defs = Vec::new();
    for def in &defs {
        let vname = def.ident.to_string();
        let code_lit = def.code;
        let docs = &def.docs;
        let mut field_defs = Vec::new();
        for field in &def.fields {
            let fname = field.ident.to_string();
            let fdocs = &field.docs;
            let ty_expr = FieldKind::classify(&field.ty, &field.ident)
                .map_err(|err| rewrite_code(err, "E010", "E014"))?
                .ts_expr();
            field_defs.push(quote! {
                ::bffi::dts::RecordFieldDef {
                    name: #fname,
                    docs: &[#(#fdocs),*],
                    ty: #ty_expr,
                }
            });
        }
        variant_defs.push(quote! {
            ::bffi::dts::ErrorVariantDef {
                name: #vname,
                docs: &[#(#docs),*],
                code: #code_lit,
                fields: &[#(#field_defs),*],
            }
        });
    }

    Ok(quote! {
        #[automatically_derived]
        impl #name {
            /// The descriptor table entry for `ModuleDef::errors`.
            pub const BFFI_ERROR_DEF: ::bffi::dts::ErrorDef =
                ::bffi::dts::ErrorDef {
                    js_name: #js_name,
                    docs: &[#(#enum_docs),*],
                    variants: &[#(#variant_defs),*],
                };
        }

        #[automatically_derived]
        impl ::std::fmt::Display for #name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let __name: &'static str = match self {
                    #(#display_arms)*
                };
                f.write_str(__name)
            }
        }

        #[automatically_derived]
        impl ::std::error::Error for #name {}

        #[automatically_derived]
        impl ::std::convert::From<#name> for ::bffi::BffiError {
            fn from(value: #name) -> Self {
                match value {
                    #(#from_arms)*
                }
            }
        }
    })
}

/// Rewrites the rejection code in a classification error message
/// (`E010` record wording becomes the error-enum flavor `E014`).
fn rewrite_code(mut err: syn::Error, from: &str, to: &str) -> syn::Error {
    let text = err.to_string();
    if let Some(rest) = text.split_once(from).map(|(_, r)| r) {
        err = syn::Error::new(err.span(), format!("bffi[{to}]:{rest}"));
    }
    err
}

/// Parses the required `#[bffi(code = <literal>)]` attribute and
/// validates the reserved user range `0x1000..=0xFFFF`.
fn parse_code(variant: &syn::Variant, enum_name: &syn::Ident) -> syn::Result<u32> {
    for attr in &variant.attrs {
        if !attr.path().is_ident("bffi") {
            continue;
        }
        let mut code: Option<u32> = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("code") {
                let value: syn::LitInt = meta.value()?.parse()?;
                code = Some(
                    value
                        .base10_parse::<u32>()
                        .map_err(|_| syn::Error::new(value.span(), "invalid code literal"))?,
                );
                Ok(())
            } else {
                Err(syn::Error::new(
                    meta.path.span(),
                    format!(
                        "unknown option `{}` (only `code` is supported)",
                        meta.path
                            .get_ident()
                            .map_or_else(String::new, ToString::to_string)
                    ),
                ))
            }
        })?;
        let Some(code) = code else {
            return Err(syn::Error::new(
                attr.span(),
                "expected `code = <literal>` on the bffi attribute",
            ));
        };
        if !(0x1000..=0xFFFF).contains(&code) {
            return Err(syn::Error::new(
                attr.span(),
                format!(
                    "bffi[E013]: error enum `{enum_name}`: code 0x{code:04X} is outside the \
                     reserved user range 0x1000..=0xFFFF"
                ),
            ));
        }
        return Ok(code);
    }
    Err(syn::Error::new(
        variant.span(),
        format!(
            "bffi[E013]: error enum `{enum_name}`: variant `{}` is missing \
             `#[bffi(code = 0x10XX)]`",
            variant.ident
        ),
    ))
}
