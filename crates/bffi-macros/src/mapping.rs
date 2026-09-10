//! Type classification and TypeScript kind mapping.
//!
//! The pure classification lives in `crate::support::classify`;
//! this module keeps the local signatures the model stage uses and
//! maps neutral rejections onto this crate's `E002`/`E003`
//! diagnostics (exact texts locked by the `tests/ui` goldens).

use crate::errors::{param_type, return_type};
use crate::support::classify as support;
use crate::support::kind::{RetKind, ShimKind};

pub(crate) use crate::support::abi;
pub(crate) use crate::support::classify::{ts_return, ts_type};

/// Classifies a parameter type: plain primitives and `i64`/`u64` as
/// their kinds, `&str` (borrowed, not `mut`; lifetimes ignored) as
/// [`ShimKind::Str`], everything else rejected.
pub(crate) fn classify_param(ty: &syn::Type, name: &str) -> syn::Result<ShimKind> {
    support::classify_param(ty).map_err(|u| param_type(u.span, u.ty, name))
}

/// Classifies a return type: the plain primitives plus `i64`/`u64`,
/// the empty tuple, the owned byte payloads (`String` / `Vec<u8>` /
/// `CopiedBuf`), `Option` of a payload, and `Result<T, E>` over any of
/// those. Everything else is rejected.
pub(crate) fn classify_return(ty: &syn::Type) -> syn::Result<RetKind> {
    support::classify_return(ty).map_err(|u| return_type(u.span, u.ty))
}

#[cfg(test)]
mod tests {
    use super::{RetKind, ShimKind, classify_param, classify_return, ts_return, ts_type};
    use crate::model::FnModel;
    use crate::support::classify::ts_prim;
    use crate::support::kind::{BigIntTy, BufferTy, PrimTy, TsKind};
    use quote::quote;

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
    fn ts_kind_tokens_quote_the_ir_variant() {
        let ctx = crate::support::paths::PathCtx::default();
        assert_eq!(
            TsKind::Number.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: Number"
        );
        assert_eq!(
            TsKind::Uint8Array.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: Uint8Array"
        );
        assert_eq!(
            TsKind::NullableString.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: NullableString"
        );
        assert_eq!(
            TsKind::NullableUint8Array.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: NullableUint8Array"
        );
        assert_eq!(
            TsKind::Void.tokens(&ctx).to_string(),
            ":: bffi :: dts :: TsType :: Void"
        );
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
        ];
        for (src, expected) in cases {
            let kind = classify_param(&ty(src), "x").expect("accepted");
            assert_eq!(ts_type(&kind), expected, "param type `{src}`");
        }
    }

    #[test]
    fn str_params_accept_lifetimes() {
        for src in ["&str", "&'a str"] {
            let kind = classify_param(&ty(src), "x").expect("accepted");
            assert_eq!(ts_type(&kind), TsKind::String, "param type `{src}`");
        }
    }

    #[test]
    fn buffer_view_params_map_to_uint8array() {
        for src in ["&[u8]", "&'a [u8]"] {
            let kind = classify_param(&ty(src), "x").expect("accepted");
            assert_eq!(kind, ShimKind::BufferView, "param type `{src}`");
            assert_eq!(ts_type(&kind), TsKind::Uint8Array, "param type `{src}`");
        }
    }

    #[test]
    fn bigints_classify_distinctly() {
        assert_eq!(
            classify_param(&ty("i64"), "x").expect("accepted"),
            ShimKind::BigInt(BigIntTy::I64)
        );
        assert_eq!(
            classify_param(&ty("u64"), "x").expect("accepted"),
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
    fn option_over_primitives_is_rejected() {
        for src in ["Option<i32>", "Option<u64>", "Option<bool>"] {
            let err = classify_return(&ty(src)).expect_err("rejected");
            assert!(
                err.to_string().contains("bffi[E003]"),
                "`{src}` must carry E003"
            );
        }
    }

    #[test]
    fn vec_of_unsupported_items_and_single_arg_result_are_rejected() {
        // `Vec<char>` items stay outside the sequence matrix; a
        // single-argument `Result` is a shape error.
        assert!(classify_return(&ty("Vec<char>")).is_err());
        assert!(classify_return(&ty("Result<u32>")).is_err());
    }

    #[test]
    fn unit_return_maps_to_void() {
        let ret = classify_return(&ty("()")).expect("accepted");
        assert_eq!(ts_return(&ret), TsKind::Void);
    }

    #[test]
    fn unsupported_param_type_is_rejected_with_documented_message() {
        let err = classify_param(&ty("Vec<u8>"), "data").expect_err("rejected");
        let text = err.to_string();
        assert!(text.starts_with("bffi[E002]: unsupported type `Vec < u8 >` for parameter `data`"));
        assert!(text.contains(
            "  = help: supported: i8|i16|i32|i64|u8|u16|u32|u64|f32|f64|bool|&str|&[u8]|()"
        ));
        assert!(text.contains("DESIGN.md"));
    }

    #[test]
    fn unsupported_return_type_is_rejected() {
        assert!(classify_return(&ty("char")).is_err());
        assert!(classify_return(&ty("&str")).is_err());
        assert!(
            classify_return(&ty("std::string::String")).is_err(),
            "qualified paths stay rejected"
        );
    }

    #[test]
    fn rejected_param_types_fail_classification() {
        let cases = [
            ("&mut str", "mutability would break the copy guarantee"),
            ("&mut [u8]", "mutability would break the copy guarantee"),
            ("&[i32]", "only byte slices may be borrowed"),
            ("&u32", "only `&str` and `&[u8]` may be borrowed"),
            ("str", "bare `str` is unsized"),
            ("i128", "integer width outside the matrix"),
            ("usize", "integer width outside the matrix"),
            ("char", "not in the matrix"),
            ("String", "owned strings are return-only"),
        ];
        for (src, why) in cases {
            let result = classify_param(&ty(src), "x");
            assert!(result.is_err(), "`{src}` must be rejected: {why}");
            // Rendering contract: type rejections carry the E002 code.
            let text = result.expect_err("checked above").to_string();
            assert!(text.contains("bffi[E002]"), "`{src}` must carry E002");
        }
    }

    #[test]
    fn happy_path_model_parse() {
        let item = quote! {
            #[doc = " Adds."]
            fn add(a: u32, b: u32) -> u32 {
                a + b
            }
        };
        let attrs = proc_macro2::TokenStream::new();
        let model = FnModel::parse(&attrs, item).expect("accepted");
        assert_eq!(model.ident, "add");
        assert_eq!(model.docs, ["Adds."]);
        assert_eq!(model.params.len(), 2);
        assert_eq!(model.params[0].name, "a");
        assert_eq!(model.params[0].kind, ShimKind::Prim(PrimTy::U32));
        assert_eq!(model.params[1].name, "b");
        assert_eq!(model.params[1].kind, ShimKind::Prim(PrimTy::U32));
        assert_eq!(model.ret, RetKind::Prim(PrimTy::U32));
    }

    #[test]
    fn attribute_options_are_rejected() {
        let item = quote! { fn f(x: u32) {} };
        let result = FnModel::parse(&quote! { rename = "x" }, item);
        assert!(result.is_err());
        let err = result.err().expect("rejected");
        assert!(
            err.to_string()
                .starts_with("bffi[E004]: unknown option; only `crate = \"...\"` is supported")
        );
    }

    #[test]
    fn no_attribute_selects_the_default_paths() {
        let item = quote! { fn f(x: u32) -> u32 { x } };
        let model = FnModel::parse(&proc_macro2::TokenStream::new(), item).expect("accepted");
        assert_eq!(
            model.paths.core.to_string(),
            ":: bffi :: core",
            "no attribute selects the facade roots"
        );
    }

    #[test]
    fn crate_option_redirects_the_generated_paths() {
        let item = quote! { fn f(x: u32) -> u32 { x } };
        let model = FnModel::parse(&quote! { crate = "bffi" }, item).expect("accepted");
        assert_eq!(model.paths.core.to_string(), ":: bffi :: core");
        assert_eq!(model.paths.types.to_string(), ":: bffi :: types");
        assert_eq!(model.paths.dts.to_string(), ":: bffi :: dts");
        assert_eq!(model.paths.build.to_string(), ":: bffi :: build");
    }

    #[test]
    fn crate_direct_selects_the_pre_merge_roots() {
        let item = quote! { fn f(x: u32) -> u32 { x } };
        let model = FnModel::parse(&quote! { crate = "direct" }, item).expect("accepted");
        assert_eq!(model.paths.core.to_string(), ":: bffi_core");
        assert_eq!(model.paths.types.to_string(), ":: bffi_types");
        assert_eq!(model.paths.dts.to_string(), ":: bffi_dts");
        assert_eq!(model.paths.build.to_string(), ":: bffi_build");
    }

    #[test]
    fn non_literal_crate_value_is_rejected_with_e004() {
        let item = quote! { fn f(x: u32) {} };
        let result = FnModel::parse(&quote! { crate = bffi }, item);
        let err = result.err().expect("rejected");
        assert!(err.to_string().starts_with("bffi[E004]:"));
    }

    #[test]
    fn invalid_crate_name_is_rejected_with_e004() {
        for name in ["", "1bad", "core", "std", "has space"] {
            let item = quote! { fn f(x: u32) {} };
            let src = format!(r#"crate = "{name}""#);
            let attr: proc_macro2::TokenStream = syn::parse_str(&src).expect("attribute source");
            let result = FnModel::parse(&attr, item);
            assert!(result.is_err(), "crate = {name:?} must be rejected");
            let err = result.err().expect("rejected");
            assert!(
                err.to_string().starts_with("bffi[E004]:"),
                "crate = {name:?} must carry E004"
            );
        }
    }

    #[test]
    fn duplicate_and_unknown_options_are_rejected_with_e004() {
        let item = quote! { fn f(x: u32) {} };
        let result = FnModel::parse(&quote! { crate = "a", crate = "b" }, item);
        assert!(
            result
                .err()
                .expect("rejected")
                .to_string()
                .starts_with("bffi[E004]:")
        );

        let item = quote! { fn f(x: u32) {} };
        let result = FnModel::parse(&quote! { crate = "a", rename = "x" }, item);
        assert!(
            result
                .err()
                .expect("rejected")
                .to_string()
                .starts_with("bffi[E004]:")
        );
    }
}
