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
    fields: Vec<(syn::Ident, syn::Type)>,
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
                fields.push((ident.clone(), field.ty.clone()));
            }
        }
        defs.push(ErrorVariant {
            ident: variant.ident.clone(),
            code,
            fields,
        });
    }
    if defs.is_empty() {
        return Err(syn::Error::new(
            item.span(),
            format!("bffi[E013]: error enum `{name}`: at least one variant is required"),
        ));
    }

    let js_name = name.to_string();

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
                Self::#ident {} => ::bffi::BffiError::new(
                    ::bffi::ErrorCode::DomainError,
                    #vname,
                )
                .with_user_code(#code_lit)
                .with_variant(#vname),
            }
        } else {
            let field_names = def.fields.iter().map(|(ident, _)| ident);
            let field_count = def.fields.len();
            let encodes = def.fields.iter().map(|(ident, _)| {
                quote! {
                    ::bffi::BffiStreamItem::encode_into(value.#ident, &mut __payload);
                }
            });
            quote! {
                Self::#ident { #(#field_names):* } => {
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

    Ok(quote! {
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

        #[allow(dead_code)]
        const _: &str = #js_name;
    })
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
