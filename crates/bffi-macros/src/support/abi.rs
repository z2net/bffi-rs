//! ABI-signature token generators for the descriptor emitters.
//!
//! Every helper here quotes a `bffi-dts` IR variant naming the exact
//! C ABI shape behind a validated kind (CALLING-CONVENTION.md §3/§4):
//! one [`AbiType`](bffi_dts) per JS-visible parameter, one out-slot
//! ([`AbiOut`]) for the return. The mapping is the single source the
//! descriptor emitters of both proc-macro crates share, so the
//! recorded signature cannot drift from the generated shims - the
//! table-driven tests at the bottom pin the two sides together.
//!
//! The `bffi-dts` variants are quoted through the
//! [`PathCtx`](crate::support::paths::PathCtx) `dts` root, exactly like the
//! [`TsKind`] tokens: this crate holds no runtime dependency on the
//! descriptor IR.

use crate::support::kind::{BigIntTy, PrimTy, RetKind, ShimKind};
use crate::support::paths::PathCtx;
use proc_macro2::TokenStream;
use quote::quote;

/// The `AbiPrim` variant ident of a small boundary primitive.
fn prim_variant(prim: PrimTy) -> TokenStream {
    match prim {
        PrimTy::I8 => quote! { I8 },
        PrimTy::I16 => quote! { I16 },
        PrimTy::I32 => quote! { I32 },
        PrimTy::U8 => quote! { U8 },
        PrimTy::U16 => quote! { U16 },
        PrimTy::U32 => quote! { U32 },
        PrimTy::F32 => quote! { F32 },
        PrimTy::F64 => quote! { F64 },
        PrimTy::Bool => quote! { Bool },
    }
}

/// The `AbiPrim` variant ident of a 64-bit boundary integer.
fn bigint_variant(big: BigIntTy) -> TokenStream {
    match big {
        BigIntTy::I64 => quote! { I64 },
        BigIntTy::U64 => quote! { U64 },
    }
}

/// The `AbiOut::Prim(..)` tokens of a small primitive slot.
pub fn prim_out(prim: PrimTy, ctx: &PathCtx) -> TokenStream {
    let dts = &ctx.dts;
    let variant = prim_variant(prim);
    quote! { #dts::AbiOut::Prim(#dts::AbiPrim::#variant) }
}

/// The `AbiOut::Prim(..)` tokens of a 64-bit integer slot.
pub fn bigint_out(big: BigIntTy, ctx: &PathCtx) -> TokenStream {
    let dts = &ctx.dts;
    let variant = bigint_variant(big);
    quote! { #dts::AbiOut::Prim(#dts::AbiPrim::#variant) }
}

/// The `AbiType` tokens of one accepted parameter kind.
///
/// A borrowed `&[u8]` records as ONE `PtrLen` entry: the `(ptr, len)`
/// dlopen pair is the loader's expansion, not the descriptor's
/// (CALLING-CONVENTION.md §3).
pub fn abi_param(kind: ShimKind, ctx: &PathCtx) -> TokenStream {
    let dts = &ctx.dts;
    match kind {
        ShimKind::Prim(prim) => {
            let variant = prim_variant(prim);
            quote! { #dts::AbiType::#variant }
        }
        ShimKind::BigInt(big) => {
            let variant = bigint_variant(big);
            quote! { #dts::AbiType::#variant }
        }
        ShimKind::Str => quote! { #dts::AbiType::Cstring },
        // Records and sequences cross as one borrowed `(ptr, len)`
        // wire payload, exactly like a `&[u8]` view.
        ShimKind::BufferView | ShimKind::Record(_) | ShimKind::Seq(_) => {
            quote! { #dts::AbiType::PtrLen }
        }
    }
}

/// The out-slot tokens of an accepted return kind: `None` for the
/// unit return, the primitive width for scalar returns, the shared
/// `u64` handle slot for every byte-carrying payload (`Result`
/// unwraps to its inner shape).
pub fn abi_out(ret: &RetKind, ctx: &PathCtx) -> Option<TokenStream> {
    let dts = &ctx.dts;
    match ret {
        RetKind::Unit => None,
        RetKind::Prim(prim) => Some(prim_out(*prim, ctx)),
        RetKind::BigInt(big) => Some(bigint_out(*big, ctx)),
        // Records and sequences travel as one wire-encoded
        // transient-buffer handle.
        RetKind::Buffer(_) | RetKind::Nullable(_) | RetKind::Record(_) | RetKind::Seq(_) => {
            Some(quote! { #dts::AbiOut::Handle })
        }
        RetKind::Result(inner) => abi_out(inner, ctx),
    }
}

/// The `AbiSig` tokens of a validated model: one [`abi_param`] entry
/// per parameter, in declaration order, plus the [`abi_out`] slot.
pub fn abi_sig(
    params: impl IntoIterator<Item = ShimKind>,
    ret: &RetKind,
    ctx: &PathCtx,
) -> TokenStream {
    let dts = &ctx.dts;
    let params = params.into_iter().map(|kind| abi_param(kind, ctx));
    match abi_out(ret, ctx) {
        Some(out) => quote! {
            #dts::AbiSig {
                params: &[#(#params),*],
                out: ::std::option::Option::Some(#out),
            }
        },
        None => quote! {
            #dts::AbiSig {
                params: &[#(#params),*],
                out: ::std::option::Option::None,
            }
        },
    }
}

/// The `AbiSig` tokens of an `#[bffi_async]` spawn shim: the regular
/// parameter shapes plus the FIXED [`AbiOut::Handle`] slot - the
/// spawn shim writes the task handle into `__ret: *mut u64` for
/// every return kind, unit included (CALLING-CONVENTION.md §4).
pub fn abi_sig_task(params: impl IntoIterator<Item = ShimKind>, ctx: &PathCtx) -> TokenStream {
    let dts = &ctx.dts;
    let params = params.into_iter().map(|kind| abi_param(kind, ctx));
    quote! {
        #dts::AbiSig {
            params: &[#(#params),*],
            out: ::std::option::Option::Some(#dts::AbiOut::Handle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{abi_out, abi_param, abi_sig, abi_sig_task, bigint_out, prim_out};
    use crate::support::codegen::{out_param, shim_param};
    use crate::support::kind::{BigIntTy, BufferTy, PrimTy, RetKind, ShimKind};
    use crate::support::paths::PathCtx;
    use proc_macro2::TokenStream;
    use quote::quote;

    /// The shim's Rust out-type for a small primitive: the exact
    /// token `out_param` emits as the pointee of `__ret`.
    fn prim_pointee(prim: PrimTy) -> TokenStream {
        match prim {
            PrimTy::I8 => quote! { i8 },
            PrimTy::I16 => quote! { i16 },
            PrimTy::I32 => quote! { i32 },
            PrimTy::U8 => quote! { u8 },
            PrimTy::U16 => quote! { u16 },
            PrimTy::U32 => quote! { u32 },
            PrimTy::F32 => quote! { f32 },
            PrimTy::F64 => quote! { f64 },
            PrimTy::Bool => quote! { bool },
        }
    }

    /// EQUIVALENCE (descriptor ABI <-> shim transport): for every
    /// small primitive, the recorded `AbiPrim` variant names the
    /// exact width the generated shim writes into `__ret`. The
    /// explicit table pins BOTH sides; changing either one without
    /// the other breaks this test.
    #[test]
    fn prim_slots_match_the_shim_out_widths() {
        let cases = [
            (PrimTy::I8, "I8"),
            (PrimTy::I16, "I16"),
            (PrimTy::I32, "I32"),
            (PrimTy::U8, "U8"),
            (PrimTy::U16, "U16"),
            (PrimTy::U32, "U32"),
            (PrimTy::F32, "F32"),
            (PrimTy::F64, "F64"),
            (PrimTy::Bool, "Bool"),
        ];
        let ctx = PathCtx::default();
        for (prim, variant) in cases {
            let out = prim_out(prim, &ctx).to_string();
            assert!(
                out.contains(&format!("AbiPrim :: {variant}")),
                "`{prim:?}` must record the `{variant}` slot, got `{out}`"
            );
            // The shim writes the same width the descriptor records:
            // `i8`..`f64` keep their Rust names, `Bool` is the one
            // byte the C ABI gives `bool`.
            let pointee = prim_pointee(prim).to_string();
            let shim = out_param(&RetKind::Prim(prim))[0].to_string();
            assert!(
                shim.contains(&format!("* mut {pointee}")),
                "shim out for `{prim:?}` must be `*mut {pointee}`, got `{shim}`"
            );
            // The parameter side records the same variant.
            let param = abi_param(ShimKind::Prim(prim), &ctx).to_string();
            assert!(
                param.contains(&format!("AbiType :: {variant}")),
                "`{prim:?}` parameter must record `{variant}`, got `{param}`"
            );
            // And the shim takes the parameter at the same width.
            let shim_param_tokens = shim_param("x", ShimKind::Prim(prim), 0).to_string();
            assert!(
                shim_param_tokens.contains(&format!("x : {pointee}")),
                "shim param for `{prim:?}` must be `x: {pointee}`"
            );
        }
    }

    /// EQUIVALENCE: 64-bit integers record `I64`/`U64` on both sides.
    #[test]
    fn bigint_slots_match_the_shim_out_widths() {
        let cases = [(BigIntTy::I64, "I64", "i64"), (BigIntTy::U64, "U64", "u64")];
        let ctx = PathCtx::default();
        for (big, variant, rust_ty) in cases {
            let out = bigint_out(big, &ctx).to_string();
            assert!(out.contains(&format!("AbiPrim :: {variant}")));
            let shim = out_param(&RetKind::BigInt(big))[0].to_string();
            assert!(shim.contains(&format!("* mut {rust_ty}")));
            let param = abi_param(ShimKind::BigInt(big), &ctx).to_string();
            assert!(param.contains(&format!("AbiType :: {variant}")));
        }
    }

    /// EQUIVALENCE: every byte-carrying return shares the `u64`
    /// handle slot the shim writes (`__ret: *mut u64`), and the unit
    /// return has no slot on either side.
    #[test]
    fn handle_and_unit_returns_match_the_shim() {
        let ctx = PathCtx::default();
        let returns = [
            RetKind::Buffer(BufferTy::String),
            RetKind::Buffer(BufferTy::ByteVec),
            RetKind::Buffer(BufferTy::CopiedBuf),
            RetKind::Nullable(BufferTy::String),
            RetKind::Nullable(BufferTy::CopiedBuf),
            RetKind::Result(Box::new(RetKind::Buffer(BufferTy::String))),
            RetKind::Result(Box::new(RetKind::Unit)),
        ];
        for ret in &returns {
            let is_unit = match ret {
                RetKind::Result(inner) => matches!(inner.as_ref(), RetKind::Unit),
                _ => false,
            };
            let out = abi_out(ret, &ctx);
            let shim = out_param(ret);
            if is_unit {
                assert!(out.is_none(), "unit must record no slot");
                assert!(shim.is_empty(), "unit must have no out-parameter");
            } else {
                assert!(
                    out.expect("handle return must record a slot")
                        .to_string()
                        .contains("AbiOut :: Handle"),
                    "byte-carrying returns must record the handle slot"
                );
                assert_eq!(shim.len(), 1);
                assert!(shim[0].to_string().contains("* mut u64"));
            }
        }
    }

    /// EQUIVALENCE: borrowed parameters record the `Cstring`/`PtrLen`
    /// shapes the shim declares (cstring pointer; `(ptr, len)` pair).
    #[test]
    fn borrowed_params_match_the_shim_shapes() {
        let ctx = PathCtx::default();
        let cstring = abi_param(ShimKind::Str, &ctx).to_string();
        assert!(cstring.contains("AbiType :: Cstring"));
        let shim = shim_param("name", ShimKind::Str, 0).to_string();
        assert!(shim.contains("name_ptr : * const :: std :: os :: raw :: c_char"));

        let ptr_len = abi_param(ShimKind::BufferView, &ctx).to_string();
        assert!(ptr_len.contains("AbiType :: PtrLen"));
        let shim = shim_param("data", ShimKind::BufferView, 0).to_string();
        assert!(shim.contains("data_ptr : * const u8"));
        assert!(shim.contains("data_len : u64"));
    }

    #[test]
    fn abi_sig_quotes_params_in_order_and_the_out_slot() {
        let ctx = PathCtx::default();
        let tokens = abi_sig(
            [ShimKind::Prim(PrimTy::U32), ShimKind::Str],
            &RetKind::Prim(PrimTy::F64),
            &ctx,
        )
        .to_string();
        assert!(tokens.contains("AbiSig"));
        assert!(tokens.contains("params : & ["));
        assert!(tokens.contains("AbiType :: U32"));
        assert!(tokens.contains("AbiType :: Cstring"));
        assert!(tokens.contains("Some"));
        assert!(tokens.contains("AbiOut :: Prim"));
        assert!(tokens.contains("AbiPrim :: F64"));

        let unit = abi_sig([], &RetKind::Unit, &ctx).to_string();
        assert!(unit.contains("None"));
        assert!(unit.contains("params : & ["));
    }

    #[test]
    fn facade_context_redirects_the_dts_root() {
        let ctx = PathCtx::from_attr("bffi");
        let tokens = abi_sig([ShimKind::Str], &RetKind::Unit, &ctx).to_string();
        assert!(tokens.contains(":: bffi :: dts :: AbiSig"));
        assert!(tokens.contains(":: bffi :: dts :: AbiType :: Cstring"));
    }

    /// The spawn shape records the handle slot even for the unit
    /// return: the task handle always travels through `__ret`.
    #[test]
    fn task_signature_always_records_the_handle_slot() {
        let ctx = PathCtx::default();
        for ret in [RetKind::Unit, RetKind::Prim(PrimTy::U32)] {
            let sig = match ret {
                RetKind::Unit => abi_sig_task([], &ctx).to_string(),
                _ => abi_sig_task([ShimKind::Prim(PrimTy::U32)], &ctx).to_string(),
            };
            assert!(sig.contains("Some"));
            assert!(sig.contains("AbiOut :: Handle"));
        }
    }
}
