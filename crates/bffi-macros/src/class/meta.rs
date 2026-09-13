//! Const descriptor codegen for `bffi-dts` `ClassDef`s.
//!
//! The two macro expansions split the metadata (they cannot write into
//! one module):
//!
//! - `#[bffi_class]` emits `bffi_meta_<name>::{TAG, DOCS, FIELDS}`;
//! - `#[bffi_impl]` emits `bffi_meta_<name>_impl::CLASS`, referencing
//!   the former by absolute path. The user aggregates
//!   `classes: &[bffi_meta_<name>_impl::CLASS]` into a `ModuleDef`.

use crate::class::mapping;
use crate::class::model::{ClassModel, FieldTy, ImplModel};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Renders the `bffi_meta_<name>` module for `#[bffi_class]`.
pub(crate) fn class_meta(model: &ClassModel) -> TokenStream {
    let dts = &model.paths.dts;
    let module = format_ident!("bffi_meta_{}", model.js_name);
    let tag = model.tag;
    let tag_doc = format!("The bffi-object range tag claimed by `{}`.", model.js_name);
    let docs_doc = format!("The doc lines of the `{}` class.", model.js_name);
    let fields_doc = format!(
        "The read-only field getters of the `{}` class.",
        model.js_name
    );
    let docs = model.docs.iter().map(|doc| quote! { #doc });
    let fields = model.fields.iter().map(|field| {
        let name = &field.name;
        let export = format!("bffi_{}_{}_get", model.js_name, field.name);
        let ts_ty = field_ts_kind(field.ty).tokens(&model.paths);
        let out = field_out(field.ty, &model.paths);
        quote! { #dts::FieldDef { js_name: #name, export_name: #export, docs: &[], ty: #ts_ty, out: #out } }
    });

    quote! {
        #[doc = "Metadata for the class descriptor (consumed by bffi-dts / bffi-build)."]
        pub mod #module {
            #[doc = #tag_doc]
            pub const TAG: u16 = #tag;

            #[doc = #docs_doc]
            pub const DOCS: &[&str] = &[#(#docs),*];

            #[doc = #fields_doc]
            pub const FIELDS: &[#dts::FieldDef] = &[#(#fields),*];
        }
    }
}

/// The getter's out-slot tokens of a field type: the exact width the
/// generated `bffi_<class>_<field>_get` shim writes.
fn field_out(ty: FieldTy, paths: &crate::support::paths::PathCtx) -> TokenStream {
    match ty {
        FieldTy::Prim(prim) => mapping::abi::prim_out(prim, paths),
        FieldTy::BigInt(big) => mapping::abi::bigint_out(big, paths),
    }
}

/// Renders the `bffi_meta_<name>_impl` module for `#[bffi_impl]`.
pub(crate) fn impl_meta(model: &ImplModel) -> TokenStream {
    let dts = &model.paths.dts;
    let module = format_ident!("bffi_meta_{}_impl", model.js_name);
    let class_doc = format!("The [`ClassDef`] descriptor for `{}`.", model.js_name);
    let base = format_ident!("bffi_meta_{}", model.js_name);
    let js_name = &model.js_name;
    let ctor = &model.constructor;
    let ctor_export = format!("bffi_{}_new", model.js_name);
    let release_export = format!("bffi_{}_release", model.js_name);
    let ctor_docs = ctor.docs.iter().map(|doc| quote! { #doc });
    let ctor_params = ctor.params.iter().map(|param| {
        let name = &param.name;
        let ty = mapping::ts_type(&param.kind).tokens(&model.paths);
        quote! { #dts::ParamDef { name: #name, ty: #ty } }
    });
    let ctor_abi = mapping::abi::abi_sig_task(
        ctor.params.iter().map(|param| param.kind.clone()),
        &model.paths,
    );
    let methods = model.methods.iter().map(|method| {
        let name = method.ident.to_string();
        let export = format!("bffi_{}_{}", model.js_name, method.ident);
        let params = method.params.iter().map(|param| {
            let name = &param.name;
            let ty = mapping::ts_type(&param.kind).tokens(&model.paths);
            quote! { #dts::ParamDef { name: #name, ty: #ty } }
        });
        let ret = mapping::ts_return(&method.ret).tokens(&model.paths);
        let abi = mapping::abi::abi_sig(
            method.params.iter().map(|param| param.kind.clone()),
            &method.ret,
            &model.paths,
        );
        let docs = method.docs.iter().map(|doc| quote! { #doc });
        quote! {
            #dts::MethodDef {
                js_name: #name,
                export_name: #export,
                docs: &[#(#docs),*],
                params: &[#(#params),*],
                ret: #ret,
                abi: #abi,
            }
        }
    });

    quote! {
        #[doc = #class_doc]
        pub mod #module {
            #[doc = "The constructor declaration (always rendered as `constructor`)."]
            pub const CONSTRUCTOR: #dts::MethodDef = #dts::MethodDef {
                js_name: "constructor",
                export_name: #ctor_export,
                docs: &[#(#ctor_docs),*],
                params: &[#(#ctor_params),*],
                ret: #dts::TsType::BigInt,
                abi: #ctor_abi,
            };

            #[doc = "The methods, in declaration order."]
            pub const METHODS: &[#dts::MethodDef] = &[#(#methods),*];

            #[doc = #class_doc]
            pub const CLASS: #dts::ClassDef = #dts::ClassDef {
                js_name: #js_name,
                // `super` = the shared expansion scope holding the
                // sibling `bffi_meta_<name>` module.
                release_export: #release_export,
                docs: &super::#base::DOCS,
                constructor: CONSTRUCTOR,
                fields: &super::#base::FIELDS,
                methods: METHODS,
            };
        }
    }
}

/// The TypeScript kind of a getter field type.
fn field_ts_kind(ty: FieldTy) -> mapping::TsKind {
    match ty {
        FieldTy::Prim(prim) => match prim {
            mapping::PrimTy::Bool => mapping::TsKind::Boolean,
            _ => mapping::TsKind::Number,
        },
        FieldTy::BigInt(_) => mapping::TsKind::BigInt,
    }
}
