//! The parsed and validated models behind `#[bffi_class]` and
//! `#[bffi_impl]`.
//!
//! [`ClassModel::parse`] validates the struct (named, non-generic,
//! literal tag in the bffi-object range, exportable field types) and
//! [`ImplModel::parse`] validates the impl block (exactly one
//! constructor returning `Self`, `&self` methods over the accepted
//! matrix). Every violation is a spanned `E005`-`E008` diagnostic, so
//! the codegen stages can rely on the shape being valid.
//!
//! The boundary kinds and the parse helpers live in
//! `bffi_macro_support`; this crate keeps the `ItemStruct`/`ItemImpl`
//! parsing and its own `E005`-`E008` diagnostics.

use crate::class::errors::{
    attr_options, class_shape, field_type, impl_binding, method_shape, tag,
};
use crate::class::mapping::{self, RetKind};
use crate::support::paths::{PathCtx, is_crate_name};
use crate::support::util::{extract_docs, to_snake_case};
use proc_macro2::TokenStream;
use syn::spanned::Spanned;
use syn::{Fields, ItemImpl, ItemStruct, ReturnType, Visibility};

/// A getter-exported field type: primitives and 64-bit integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldTy {
    /// A small primitive (`i32`, `f64`, `bool`, ...).
    Prim(mapping::PrimTy),
    /// A 64-bit integer (`i64`/`u64`).
    BigInt(mapping::BigIntTy),
}

/// One exported field of the class.
pub(crate) struct FieldModel {
    /// Field name as written (a plain identifier: Rust field names).
    pub name: String,
    /// Field type kind.
    pub ty: FieldTy,
}

/// The validated `#[bffi_class]` struct.
pub(crate) struct ClassModel {
    /// The struct identifier (e.g. `Counter`).
    pub ident: syn::Ident,
    /// The snake_case JS name (e.g. `counter`).
    pub js_name: String,
    /// Doc lines, one leading space trimmed.
    pub docs: Vec<String>,
    /// The literal tag validated against `0x0100..=0x01FF`.
    pub tag: u16,
    /// Exported (`pub`, primitive-typed) fields, declaration order.
    pub fields: Vec<FieldModel>,
    /// The crate roots the generated code names (default: the direct
    /// dependencies; `crate = "..."`: the facade namespaces).
    pub paths: PathCtx,
}

/// One validated method parameter.
pub(crate) struct MethodParam {
    /// Parameter name as written (identifier or `_`).
    pub name: String,
    /// Boundary kind of the parameter type.
    pub kind: mapping::ShimKind,
}

/// The validated `#[bffi_constructor]` fn.
pub(crate) struct ConstructorModel {
    /// Constructor identifier (used for the plain Rust call only).
    pub ident: syn::Ident,
    /// Doc lines for the descriptor.
    pub docs: Vec<String>,
    /// Parameters in declaration order (no receiver).
    pub params: Vec<MethodParam>,
}

/// One validated `&self` method.
pub(crate) struct MethodModel {
    /// Method identifier.
    pub ident: syn::Ident,
    /// Doc lines for the descriptor.
    pub docs: Vec<String>,
    /// Parameters in declaration order (no receiver).
    pub params: Vec<MethodParam>,
    /// Return kind.
    pub ret: RetKind,
}

/// The validated `#[bffi_impl]` block.
pub(crate) struct ImplModel {
    /// The `Self` type identifier; must match a `#[bffi_class]` struct.
    pub type_ident: syn::Ident,
    /// The snake_case JS name of the type.
    pub js_name: String,
    /// The (exactly one) constructor.
    pub constructor: ConstructorModel,
    /// The `&self` methods, declaration order.
    pub methods: Vec<MethodModel>,
    /// The crate roots the generated code names (default: the direct
    /// dependencies; `crate = "..."`: the facade namespaces).
    pub paths: PathCtx,
}

impl ClassModel {
    /// Parses `#[bffi_class(tag = 0x01xx)]` (optionally with
    /// `crate = "<name>"`) on a named struct.
    pub(crate) fn parse(attrs: &TokenStream, item: TokenStream) -> syn::Result<ClassModel> {
        let (tag_value, paths) = parse_class_args(attrs)?;
        let struc: ItemStruct = syn::parse2(item)
            .map_err(|err| class_shape(err.span(), "only `struct` declarations are supported"))?;
        if !struc.generics.params.is_empty() || struc.generics.where_clause.is_some() {
            return Err(class_shape(struc.generics.span(), "generic struct"));
        }
        let Fields::Named(fields) = &struc.fields else {
            return Err(class_shape(
                struc.fields.span(),
                "only named-field structs are supported (no tuple/unit structs)",
            ));
        };

        let mut exported = Vec::new();
        for field in &fields.named {
            let Visibility::Public(_) = &field.vis else {
                continue; // private fields are not exported, silently
            };
            let Some(ident) = field.ident.as_ref() else {
                continue; // named-field structs always carry idents
            };
            let name = ident.to_string();
            let Some(ty) = field_kind(&field.ty) else {
                return Err(field_type(field.ty.span(), &field.ty, &name));
            };
            exported.push(FieldModel { name, ty });
        }

        Ok(ClassModel {
            js_name: to_snake_case(&struc.ident.to_string()),
            ident: struc.ident,
            docs: extract_docs(&struc.attrs),
            tag: tag_value,
            fields: exported,
            paths,
        })
    }
}

/// The parsed `tag = <literal>` (required) and `crate = "<name>"`
/// (optional) attribute arguments.
struct ClassArgs {
    tag: Option<syn::LitInt>,
    paths: PathCtx,
}

impl syn::parse::Parse for ClassArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut tag: Option<syn::LitInt> = None;
        let mut paths = PathCtx::default();
        let mut crate_seen = false;
        while !input.is_empty() {
            // `parse_any`: `crate` is a Rust keyword, which the plain
            // `Ident` parse rejects.
            let key: syn::Ident = input.call(syn::ext::IdentExt::parse_any)?;
            input.parse::<syn::Token![=]>()?;
            match key.to_string().as_str() {
                "tag" => {
                    if tag.is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate `tag` option"));
                    }
                    tag = Some(input.parse()?);
                }
                "crate" => {
                    if crate_seen {
                        return Err(syn::Error::new(key.span(), "duplicate `crate` option"));
                    }
                    crate_seen = true;
                    let value: syn::LitStr = input.parse()?;
                    let name = value.value();
                    if !is_crate_name(&name) {
                        return Err(syn::Error::new(
                            value.span(),
                            format!("invalid crate name `{name}`"),
                        ));
                    }
                    paths = crate::support::paths::from_option(&name);
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown option `{other}`; only `tag` and `crate` are supported"),
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(Self { tag, paths })
    }
}

/// Parses and range-checks the class attribute arguments: `tag =
/// <literal>` is required and `crate = "<name>"` optional, in any
/// order.
fn parse_class_args(attrs: &TokenStream) -> syn::Result<(u16, PathCtx)> {
    if attrs.is_empty() {
        return Err(tag(attrs.span(), "missing `tag = ...`"));
    }
    let args: ClassArgs = syn::parse2(attrs.clone())
        .map_err(|err| tag(err.span(), format!("expected `tag = <literal>` ({})", err)))?;
    let Some(tag_lit) = args.tag else {
        return Err(tag(attrs.span(), "missing `tag = ...`"));
    };
    let value: u16 = tag_lit
        .base10_parse()
        .map_err(|_| tag(tag_lit.span(), "the tag must fit in u16"))?;
    if !(0x0100..=0x01FF).contains(&value) {
        return Err(tag(
            tag_lit.span(),
            format!("tag {value:#06x} is outside the bffi-object range 0x0100..=0x01FF"),
        ));
    }
    Ok((value, args.paths))
}

/// The getter kind of a field type: plain primitives and `i64`/`u64`.
fn field_kind(ty: &syn::Type) -> Option<FieldTy> {
    if let Some(kind) = mapping::path_kind(ty) {
        return Some(match kind {
            mapping::PathKind::Prim(prim) => FieldTy::Prim(prim),
            mapping::PathKind::BigInt(bigint) => FieldTy::BigInt(bigint),
        });
    }
    None
}

impl ImplModel {
    /// Parses an `impl Type` block under `#[bffi_impl]` (optionally
    /// with `crate = "<name>"`).
    pub(crate) fn parse(attrs: &TokenStream, item: TokenStream) -> syn::Result<ImplModel> {
        let paths = parse_impl_paths(attrs)?;
        let imp: ItemImpl = syn::parse2(item)
            .map_err(|err| impl_binding(err.span(), "only `impl Type` blocks are supported"))?;
        if !imp.generics.params.is_empty() || imp.generics.where_clause.is_some() {
            return Err(class_shape(imp.generics.span(), "generic impl"));
        }
        let syn::Type::Path(path) = imp.self_ty.as_ref() else {
            return Err(impl_binding(
                imp.self_ty.span(),
                "the impl target must be a plain path",
            ));
        };
        let Some(segment) = path.path.segments.last() else {
            return Err(impl_binding(
                imp.self_ty.span(),
                "the impl target must be a plain path",
            ));
        };
        let type_ident = segment.ident.clone();

        let mut constructor: Option<ConstructorModel> = None;
        let mut methods = Vec::new();
        for item in &imp.items {
            let syn::ImplItem::Fn(method) = item else {
                continue; // consts/types pass through untouched
            };
            let has_marker = method.attrs.iter().any(|attr| {
                // Accept both `#[bffi_constructor]` and the
                // qualified `#[bffi_class::bffi_constructor]`.
                attr.path().is_ident("bffi_constructor")
                    || attr
                        .path()
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == "bffi_constructor")
            });
            if has_marker {
                if constructor.is_some() {
                    return Err(impl_binding(
                        method.sig.ident.span(),
                        "multiple `#[bffi_constructor]` fns (exactly one is allowed)",
                    ));
                }
                let model = parse_constructor(method)?;
                constructor = Some(model);
                continue;
            }
            let model = parse_method(method)?;
            methods.push(model);
        }
        let Some(constructor) = constructor else {
            return Err(impl_binding(
                imp.self_ty.span(),
                "no `#[bffi_constructor]` fn found (JS needs a constructor to create instances)",
            ));
        };

        Ok(ImplModel {
            js_name: to_snake_case(&type_ident.to_string()),
            type_ident,
            constructor,
            methods,
            paths,
        })
    }
}

/// The parsed `crate = "<name>"` attribute argument of
/// `#[bffi_impl]`: at most one, optional.
struct ImplPaths {
    paths: PathCtx,
}

impl syn::parse::Parse for ImplPaths {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut paths = PathCtx::default();
        let mut crate_seen = false;
        while !input.is_empty() {
            // `parse_any`: `crate` is a Rust keyword, which the plain
            // `Ident` parse rejects.
            let key: syn::Ident = input.call(syn::ext::IdentExt::parse_any)?;
            if key != "crate" {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown option `{key}`; only `crate` is supported"),
                ));
            }
            if crate_seen {
                return Err(syn::Error::new(key.span(), "duplicate `crate` option"));
            }
            crate_seen = true;
            input.parse::<syn::Token![=]>()?;
            let value: syn::LitStr = input.parse()?;
            let name = value.value();
            if !is_crate_name(&name) {
                return Err(syn::Error::new(
                    value.span(),
                    format!("invalid crate name `{name}`"),
                ));
            }
            paths = crate::support::paths::from_option(&name);
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(Self { paths })
    }
}

/// Resolves the `#[bffi_impl]` attribute options into the path
/// context. No attribute selects the default facade roots
/// (`::bffi::core`, ...); anything unparsable or invalid is the
/// `E006` rejection.
fn parse_impl_paths(attrs: &TokenStream) -> syn::Result<PathCtx> {
    if attrs.is_empty() {
        return Ok(PathCtx::default());
    }
    let ImplPaths { paths } = syn::parse2(attrs.clone()).map_err(|_| attr_options(attrs.span()))?;
    Ok(paths)
}

/// Parses one `#[bffi_constructor]` fn: no receiver, returns `Self`,
/// parameters over the accepted matrix.
fn parse_constructor(method: &syn::ImplItemFn) -> syn::Result<ConstructorModel> {
    if method.sig.asyncness.is_some() {
        return Err(method_shape(method.sig.span(), "async constructor"));
    }
    if !method.sig.generics.params.is_empty() {
        return Err(method_shape(
            method.sig.generics.span(),
            "generic constructor",
        ));
    }
    if let Some(syn::FnArg::Receiver(receiver)) = method.sig.inputs.first() {
        return Err(method_shape(
            receiver.span(),
            "constructor with a `self` receiver",
        ));
    }
    match &method.sig.output {
        ReturnType::Type(_, ty) => {
            if !matches!(
                ty.as_ref(),
                syn::Type::Path(p)
                    if p.qself.is_none()
                        && p.path.segments.len() == 1
                        && p.path.segments[0].ident == "Self"
            ) {
                return Err(method_shape(ty.span(), "constructor must return `Self`"));
            }
        }
        ReturnType::Default => {
            return Err(method_shape(
                method.sig.span(),
                "constructor must return `Self`",
            ));
        }
    }

    let mut params = Vec::new();
    // Constructors have no receiver (checked above): every input is a
    // parameter.
    for arg in &method.sig.inputs {
        let syn::FnArg::Typed(arg) = arg else {
            continue;
        };
        let name = match &*arg.pat {
            syn::Pat::Ident(pat) => pat.ident.to_string(),
            syn::Pat::Wild(_) => "_".to_owned(),
            other => {
                return Err(method_shape(
                    other.span(),
                    "non-identifier parameter pattern (use `name: Type` or `_`)",
                ));
            }
        };
        let kind = mapping::classify_param(&arg.ty, &name)?;
        params.push(MethodParam { name, kind });
    }

    Ok(ConstructorModel {
        ident: method.sig.ident.clone(),
        docs: extract_docs(&method.attrs),
        params,
    })
}

/// Parses one `&self` method over the accepted matrix.
fn parse_method(method: &syn::ImplItemFn) -> syn::Result<MethodModel> {
    if method.sig.asyncness.is_some() {
        return Err(method_shape(method.sig.ident.span(), "async method"));
    }
    if !method.sig.generics.params.is_empty() {
        return Err(method_shape(method.sig.generics.span(), "generic method"));
    }
    if method.sig.unsafety.is_some() {
        return Err(method_shape(method.sig.ident.span(), "unsafe method"));
    }
    let Some(syn::FnArg::Receiver(receiver)) = method.sig.inputs.first() else {
        return Err(method_shape(
            method.sig.span(),
            "associated fn without a `self` receiver (constructors need `#[bffi_constructor]`)",
        ));
    };
    if receiver.reference.is_none() || receiver.mutability.is_some() {
        return Err(method_shape(
            receiver.span(),
            "methods take `&self` only (`&mut self` and by-value `self` cannot be served through Arc<T>)",
        ));
    }

    let mut params = Vec::new();
    for arg in method.sig.inputs.iter().skip(1) {
        let syn::FnArg::Typed(arg) = arg else {
            continue;
        };
        let name = match &*arg.pat {
            syn::Pat::Ident(pat) => pat.ident.to_string(),
            syn::Pat::Wild(_) => "_".to_owned(),
            other => {
                return Err(method_shape(
                    other.span(),
                    "non-identifier parameter pattern (use `name: Type` or `_`)",
                ));
            }
        };
        let kind = mapping::classify_param(&arg.ty, &name)?;
        params.push(MethodParam { name, kind });
    }
    let ret = match &method.sig.output {
        ReturnType::Default => RetKind::Unit,
        ReturnType::Type(_, ty) => mapping::classify_return(ty)?,
    };

    Ok(MethodModel {
        ident: method.sig.ident.clone(),
        docs: extract_docs(&method.attrs),
        params,
        ret,
    })
}
