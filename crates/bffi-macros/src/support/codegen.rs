//! Token generators for the shim transport layer.
//!
//! Every helper here emits the same tokens for both proc-macro
//! consumers: the out-parameter plumbing, the return-transport tail,
//! the borrowed-parameter conversions (`&str` cstrings, `&[u8]`
//! ptr+len views), and the small Rust-type renderers. The generated
//! code names the runtime crates through a
//! [`PathCtx`](crate::support::paths::PathCtx): the default context emits the
//! direct-dependency absolute paths (`::bffi_core`, `::bffi_types`,
//! `::bffi_build`), the `crate = "..."` context emits the facade
//! namespaces.

use crate::support::kind::{BigIntTy, BufferTy, PrimTy, RetKind, SeqItem, ShimKind};
use crate::support::paths::PathCtx;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

/// Declares one shim parameter.
///
/// Primitives and bigints keep their Rust type; a `&str` parameter
/// arrives as a NUL-terminated cstring pointer per the `bun:ffi`
/// convention (DESIGN §6.3); a `&[u8]` parameter arrives as a
/// `(ptr, len)` pair per the borrowed-buffer convention
/// (CALLING-CONVENTION.md §3).
pub fn shim_param(name: &str, kind: ShimKind, index: usize) -> TokenStream {
    let name = param_ident(name, index);
    match kind {
        ShimKind::Str => {
            let ptr = format_ident!("{name}_ptr");
            quote! { #ptr: *const ::std::os::raw::c_char }
        }
        // Records and sequences cross as one borrowed `(ptr, len)`
        // wire payload (copy by default: the shim decodes into owned
        // data before the call).
        ShimKind::BufferView | ShimKind::Record(_) | ShimKind::Seq(_) => {
            let ptr = format_ident!("{name}_ptr");
            let len = format_ident!("{name}_len");
            quote! { #ptr: *const u8, #len: u64 }
        }
        ShimKind::Prim(prim) => {
            let ty = prim_ty(prim);
            quote! { #name: #ty }
        }
        ShimKind::BigInt(big) => {
            let ty = bigint_ty(big);
            quote! { #name: #ty }
        }
    }
}

/// Declares the out-parameter that carries the return value across the
/// C ABI (`()` returns have none; buffer payloads travel as `u64`
/// handles; `Result` carries its inner type).
pub fn out_param(ret: &RetKind) -> Vec<TokenStream> {
    match ret {
        RetKind::Unit => Vec::new(),
        RetKind::Prim(prim) => {
            let ty = prim_ty(*prim);
            vec![quote! { __ret: *mut #ty }]
        }
        RetKind::BigInt(big) => {
            let ty = bigint_ty(*big);
            vec![quote! { __ret: *mut #ty }]
        }
        RetKind::Buffer(_) | RetKind::Nullable(_) => vec![quote! { __ret: *mut u64 }],
        // Records and sequences (plain or nullable) travel as one
        // wire-encoded transient-buffer handle.
        RetKind::Record(_)
        | RetKind::Seq(_)
        | RetKind::NullableRecord(_)
        | RetKind::NullableSeq(_) => vec![quote! { __ret: *mut u64 }],
        RetKind::Result(inner) => out_param(inner),
    }
}

/// Whether the return shape needs the `__ret` out-parameter.
pub fn has_out(ret: &RetKind) -> bool {
    match ret {
        RetKind::Unit => false,
        RetKind::Result(inner) => has_out(inner),
        _ => true,
    }
}

/// Generates the return-transport tail from the call expression: the
/// final expression of the shim body, the exported `u32` status
/// (a user code replaces the framework code when present).
pub fn ret_body(ctx: &PathCtx, ret: &RetKind, call: TokenStream) -> TokenStream {
    let core = &ctx.core;
    match ret {
        RetKind::Unit => quote! {
            #call;
            #core::ErrorCode::Ok.as_u32()
        },
        RetKind::Result(inner) => {
            let ok_tail = value_tail(ctx, inner);
            quote! {
                match #call {
                    ::std::result::Result::Ok(__value) => { #ok_tail }
                    ::std::result::Result::Err(__err) => {
                        // `E: Into<BffiError>`: derived error enums
                        // carry their user code (crossing in the
                        // status) and payload; plain Display errors
                        // convert to a DomainError.
                        let __converted: #core::BffiError =
                            ::std::convert::Into::into(__err);
                        let __status = __converted.status_u32();
                        #core::set_last_error(__converted);
                        return __status;
                    }
                }
            }
        }
        other => {
            let tail = value_tail(ctx, other);
            quote! {
                let __value = #call;
                #tail
            }
        }
    }
}

/// Consumes an already-bound `__value` and yields the transport tail
/// for `ret`'s shape.
pub fn value_tail(ctx: &PathCtx, ret: &RetKind) -> TokenStream {
    let core = &ctx.core;
    match ret {
        RetKind::Unit => quote! { #core::ErrorCode::Ok.as_u32() },
        RetKind::Prim(_) | RetKind::BigInt(_) => quote! {
            // SAFETY: `__ret` is non-null (checked above) and valid for one
            // `T` write per the bun:ffi out-parameter contract.
            unsafe { ::std::ptr::write(__ret, __value); }
            #core::ErrorCode::Ok.as_u32()
        },
        RetKind::Buffer(ty) => {
            let conv = buffer_conv(ctx, *ty);
            let build = &ctx.build;
            quote! {
                #conv
                match #build::runtime::store_bytes(__bytes) {
                    ::std::result::Result::Ok(__handle) => {
                        // SAFETY: `__ret` is non-null (checked above) and valid
                        // for one `u64` write per the bun:ffi out-parameter contract.
                        unsafe { ::std::ptr::write(__ret, __handle.as_u64()); }
                        #core::ErrorCode::Ok.as_u32()
                    }
                    ::std::result::Result::Err(__e) => {
                        #core::set_last_error(#core::BffiError::from(__e));
                        #core::ErrorCode::TableFull.as_u32()
                    }
                }
            }
        }
        RetKind::Nullable(ty) => {
            let some_tail = value_tail(ctx, &RetKind::Buffer(*ty));
            nullable_tail(ctx, some_tail)
        }
        RetKind::NullableRecord(path) => {
            let some_tail = value_tail(ctx, &RetKind::Record(path.clone()));
            nullable_tail(ctx, some_tail)
        }
        RetKind::NullableSeq(item) => {
            let some_tail = value_tail(ctx, &RetKind::Seq(item.clone()));
            nullable_tail(ctx, some_tail)
        }
        RetKind::Record(_) | RetKind::Seq(_) => {
            let build = &ctx.build;
            let types = &ctx.types;
            let encode = wire_encode_value(ctx, ret, format_ident!("__value"));
            quote! {
                #[allow(unused_imports)]
                use #types::wire::BffiWire as _;
                let mut __buf = ::std::vec::Vec::<u8>::new();
                #encode
                match #build::runtime::store_bytes(#types::CopiedBuf::from_vec(__buf)) {
                    ::std::result::Result::Ok(__handle) => {
                        // SAFETY: `__ret` is non-null (checked above) and valid
                        // for one `u64` write per the bun:ffi out-parameter contract.
                        unsafe { ::std::ptr::write(__ret, __handle.as_u64()); }
                        #core::ErrorCode::Ok.as_u32()
                    }
                    ::std::result::Result::Err(__e) => {
                        #core::set_last_error(#core::BffiError::from(__e));
                        #core::ErrorCode::TableFull.as_u32()
                    }
                }
            }
        }
        // Unreachable: `ret_body` unwraps `Result` first.
        RetKind::Result(_) => quote! { #core::ErrorCode::Ok.as_u32() },
    }
}

/// Wraps an already-built `Some(__value)` transport tail into the
/// `Option` match: `None` writes the documented `0` null handle.
fn nullable_tail(ctx: &PathCtx, some_tail: TokenStream) -> TokenStream {
    let core = &ctx.core;
    quote! {
        match __value {
            ::std::option::Option::Some(__value) => { #some_tail }
            ::std::option::Option::None => {
                // SAFETY: `__ret` is non-null (checked above) and valid for
                // one `u64` write; `0` is the documented null handle.
                unsafe { ::std::ptr::write(__ret, 0_u64); }
                #core::ErrorCode::Ok.as_u32()
            }
        }
    }
}

/// Appends the wire encoding of a bound `value` (by the given ident)
/// for a record/seq return into `__buf`.
fn wire_encode_value(ctx: &PathCtx, ret: &RetKind, value: syn::Ident) -> TokenStream {
    let types = &ctx.types;
    let wire = quote! { #types::wire };
    match ret {
        RetKind::Record(path) => {
            let path = &path.0;
            quote! { #path::bffi_wire_encode(&#value, &mut __buf); }
        }
        RetKind::Seq(item) => {
            let push = seq_item_encode(&wire, item);
            quote! {
                #wire::encode_seq_header(&mut __buf, #value.len());
                for __item in &#value {
                    #push
                }
            }
        }
        _ => TokenStream::new(),
    }
}

/// One item-encode statement inside a sequence loop (the item is
/// bound to `__item`).
pub(crate) fn seq_item_encode(wire: &TokenStream, item: &SeqItem) -> TokenStream {
    match item {
        SeqItem::Narrow => quote! { #wire::encode_i32(&mut __buf, *__item as i32); },
        SeqItem::Wide => quote! { #wire::encode_f64(&mut __buf, *__item as f64); },
        SeqItem::I64 => quote! { #wire::encode_i64(&mut __buf, *__item); },
        SeqItem::U64 => quote! { #wire::encode_u64(&mut __buf, *__item); },
        SeqItem::Bool => quote! { #wire::encode_bool(&mut __buf, *__item); },
        SeqItem::Str => quote! { #wire::encode_str(&mut __buf, __item); },
        SeqItem::Bytes => quote! { #wire::encode_bytes(&mut __buf, __item); },
        SeqItem::Record(path) => {
            let path = &path.0;
            quote! { #path::bffi_wire_encode(__item, &mut __buf); }
        }
    }
}

/// Converts a bound `__value` into `__bytes: CopiedBuf`.
pub fn buffer_conv(ctx: &PathCtx, ty: BufferTy) -> TokenStream {
    let types = &ctx.types;
    match ty {
        BufferTy::String => quote! {
            let __bytes = #types::CopiedBuf::from_vec(__value.into_bytes());
        },
        BufferTy::ByteVec => quote! {
            let __bytes = #types::CopiedBuf::from_vec(__value);
        },
        BufferTy::CopiedBuf => quote! {
            let __bytes = __value;
        },
    }
}

/// The borrowed-parameter conversion preamble: one null-checked
/// conversion block per `&str` (cstring -> UTF-8 view) or `&[u8]`
/// (ptr+len -> byte view) parameter, in order.
pub fn param_conversions<'a, I>(ctx: &PathCtx, params: I) -> TokenStream
where
    I: IntoIterator<Item = (&'a str, ShimKind)>,
{
    let core = &ctx.core;
    let types = &ctx.types;
    let mut body = TokenStream::new();
    for (index, (name, kind)) in params.into_iter().enumerate() {
        let name = param_ident(name, index);
        match kind {
            ShimKind::Str => {
                let ptr = format_ident!("{name}_ptr");
                let bytes = format_ident!("{name}_bytes");
                let view = format_ident!("{name}_view");
                body.extend(quote! {
                    if #ptr.is_null() {
                        let error = #core::BffiError::new(
                            #core::ErrorCode::NullPointer,
                            "string argument pointer is null",
                        );
                        #core::set_last_error(error);
                        return #core::ErrorCode::NullPointer.as_u32();
                    }
                    // SAFETY: bun:ffi hands out NUL-terminated cstrings for `&str`
                    // parameters (DESIGN.md §6.3); the pointer is null-checked above.
                    let #bytes = unsafe { ::std::ffi::CStr::from_ptr(#ptr) }.to_bytes();
                    let #view = match #types::unsafe_zero_copy::str_view(#bytes) {
                        ::std::result::Result::Ok(v) => v,
                        ::std::result::Result::Err(e) => {
                            #core::set_last_error(e);
                            return #core::ErrorCode::InvalidUtf8.as_u32();
                        }
                    };
                });
            }
            ShimKind::BufferView => {
                let ptr = format_ident!("{name}_ptr");
                let len = format_ident!("{name}_len");
                let view = format_ident!("{name}_view");
                body.extend(quote! {
                    if #len > 0_u64 && #ptr.is_null() {
                        let error = #core::BffiError::new(
                            #core::ErrorCode::NullPointer,
                            "buffer argument pointer is null",
                        );
                        #core::set_last_error(error);
                        return #core::ErrorCode::NullPointer.as_u32();
                    }
                    // SAFETY: bun:ffi keeps the TypedArray pointer valid
                    // for the duration of the call (CALLING-CONVENTION.md
                    // §3). `len == 0` takes the empty-slice branch, so
                    // `from_raw_parts` never sees a null pointer (`len ==
                    // 0` permits one per the ABI contract); otherwise the
                    // pointer is non-null (checked above) and valid for
                    // exactly `len` bytes.
                    let #view = if #len == 0_u64 {
                        #types::buf_view(&[])
                    } else {
                        #types::buf_view(unsafe {
                            ::std::slice::from_raw_parts(#ptr, #len as usize)
                        })
                    };
                });
            }
            ShimKind::Record(path) => {
                let ptr = format_ident!("{name}_ptr");
                let len = format_ident!("{name}_len");
                let slice = format_ident!("{name}_wire");
                let path = &path.0;
                body.extend(quote! {
                    #[allow(unused_imports)]
                    use #types::wire::BffiWire as _;
                    if #len == 0_u64 || #ptr.is_null() {
                        let error = #core::BffiError::new(
                            #core::ErrorCode::NullPointer,
                            "record argument payload is null",
                        );
                        #core::set_last_error(error);
                        return #core::ErrorCode::NullPointer.as_u32();
                    }
                    // SAFETY: bun:ffi keeps the TypedArray pointer valid
                    // for the duration of the call; the wire decode below
                    // only reads and copies into owned data.
                    let #slice = unsafe {
                        ::std::slice::from_raw_parts(#ptr, #len as usize)
                    };
                    let #name = match #path::bffi_wire_decode(#slice, 0) {
                        ::std::result::Result::Ok((value, _)) => value,
                        ::std::result::Result::Err(error) => {
                            let code = error.status_u32();
                            #core::set_last_error(error);
                            return code;
                        }
                    };
                });
            }
            ShimKind::Seq(item) => {
                let ptr = format_ident!("{name}_ptr");
                let len = format_ident!("{name}_len");
                let slice = format_ident!("{name}_wire");
                let slice_ref = slice.clone();
                let decode = seq_item_decode(ctx, &item, &slice_ref);
                body.extend(quote! {
                    #[allow(unused_imports)]
                    use #types::wire::BffiWire as _;
                    if #len == 0_u64 || #ptr.is_null() {
                        let error = #core::BffiError::new(
                            #core::ErrorCode::NullPointer,
                            "sequence argument payload is null",
                        );
                        #core::set_last_error(error);
                        return #core::ErrorCode::NullPointer.as_u32();
                    }
                    // SAFETY: bun:ffi keeps the TypedArray pointer valid
                    // for the duration of the call; the wire decode below
                    // only reads and copies into owned data.
                    let #slice = unsafe {
                        ::std::slice::from_raw_parts(#ptr, #len as usize)
                    };
                    let #name = match (|| -> ::std::result::Result<
                        ::std::vec::Vec<_>,
                        #core::BffiError,
                    > {
                        let (count, mut offset) =
                            #types::wire::decode_seq_header(#slice, 0)?;
                        let mut items = ::std::vec::Vec::with_capacity(
                            (count as usize).min(4096),
                        );
                        for _ in 0..count {
                            #decode
                        }
                        ::std::result::Result::Ok(items)
                    })() {
                        ::std::result::Result::Ok(value) => value,
                        ::std::result::Result::Err(error) => {
                            let code = error.status_u32();
                            #core::set_last_error(error);
                            return code;
                        }
                    };
                });
            }
            ShimKind::Prim(_) | ShimKind::BigInt(_) => {}
        }
    }
    body
}

/// Sanitizes a parameter name into a usable shim identifier.
///
/// Parameter names mirror the source pattern and are not guaranteed to
/// be plain identifiers (`_` stays as written; raw keyword idents
/// carry their `r#` prefix), so unusable names deterministically fall
/// back to the positional `__arg<index>`.
pub fn param_ident(name: &str, index: usize) -> Ident {
    syn::parse_str::<Ident>(name).unwrap_or_else(|_| format_ident!("__arg{index}"))
}

/// One item-decode statement inside the sequence loop (the payload is
/// bound to `slice`, the running offset to `offset`, the destination
/// to `items`).
fn seq_item_decode(ctx: &PathCtx, item: &SeqItem, slice: &Ident) -> TokenStream {
    let types = &ctx.types;
    let wire = quote! { #types::wire };
    match item {
        SeqItem::Narrow => quote! {
            let (value, next) = #wire::decode_i32(#slice, offset)?;
            offset = next;
            items.push(value as _);
        },
        SeqItem::Wide => quote! {
            let (value, next) = #wire::decode_number(#slice, offset)?;
            offset = next;
            items.push(value as _);
        },
        SeqItem::I64 => quote! {
            let (value, next) = #wire::decode_i64(#slice, offset)?;
            offset = next;
            items.push(value as _);
        },
        SeqItem::U64 => quote! {
            let (value, next) = #wire::decode_u64(#slice, offset)?;
            offset = next;
            items.push(value);
        },
        SeqItem::Bool => quote! {
            let (value, next) = #wire::decode_bool(#slice, offset)?;
            offset = next;
            items.push(value);
        },
        SeqItem::Str => quote! {
            let (value, next) = #wire::decode_str(#slice, offset)?;
            offset = next;
            items.push(::std::string::String::from(value));
        },
        SeqItem::Bytes => quote! {
            let (value, next) = #wire::decode_bytes(#slice, offset)?;
            offset = next;
            items.push(::std::vec::Vec::from(value));
        },
        SeqItem::Record(path) => {
            let path = &path.0;
            quote! {
                let (value, next) = #path::bffi_wire_decode(#slice, offset)?;
                offset = next;
                items.push(value);
            }
        }
    }
}

/// The Rust primitive type of a small boundary primitive.
pub fn prim_ty(prim: PrimTy) -> TokenStream {
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

/// The Rust type of a 64-bit boundary integer.
pub fn bigint_ty(big: BigIntTy) -> TokenStream {
    match big {
        BigIntTy::I64 => quote! { i64 },
        BigIntTy::U64 => quote! { u64 },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bigint_ty, buffer_conv, has_out, out_param, param_conversions, param_ident, prim_ty,
        ret_body, shim_param, value_tail,
    };
    use crate::support::kind::{BigIntTy, BufferTy, PrimTy, RetKind, ShimKind};
    use crate::support::paths::PathCtx;
    use quote::quote;

    #[test]
    fn wildcard_pattern_name_falls_back_to_positional() {
        assert_eq!(param_ident("_", 2).to_string(), "__arg2");
    }

    #[test]
    fn valid_ident_passes_through() {
        assert_eq!(param_ident("phrase", 0).to_string(), "phrase");
    }

    #[test]
    fn keyword_falls_back_to_positional() {
        assert_eq!(param_ident("fn", 1).to_string(), "__arg1");
    }

    #[test]
    fn primitive_types_render_as_written() {
        assert_eq!(prim_ty(PrimTy::U32).to_string(), "u32");
        assert_eq!(prim_ty(PrimTy::Bool).to_string(), "bool");
        assert_eq!(bigint_ty(BigIntTy::I64).to_string(), "i64");
        assert_eq!(bigint_ty(BigIntTy::U64).to_string(), "u64");
    }

    #[test]
    fn unit_has_no_out_param_anything_else_does() {
        assert!(!has_out(&RetKind::Unit));
        // `Result<(), E>` carries no value, so no out-parameter either
        // (the err channel travels through the last-error slot).
        assert!(!has_out(&RetKind::Result(Box::new(RetKind::Unit))));
        assert!(has_out(&RetKind::Prim(PrimTy::U32)));
        assert!(has_out(&RetKind::BigInt(BigIntTy::U64)));
        assert!(has_out(&RetKind::Buffer(BufferTy::String)));
        assert!(has_out(&RetKind::Nullable(BufferTy::ByteVec)));
        assert!(has_out(&RetKind::Result(Box::new(RetKind::Prim(
            PrimTy::U32
        )))));
    }

    #[test]
    fn out_param_traverses_result_to_the_inner_type() {
        let tokens = out_param(&RetKind::Result(Box::new(RetKind::Prim(PrimTy::U32))));
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0].to_string(),
            quote! { __ret : * mut u32 }.to_string()
        );
        assert_eq!(
            out_param(&RetKind::Buffer(BufferTy::CopiedBuf))[0].to_string(),
            quote! { __ret : * mut u64 }.to_string()
        );
        assert!(out_param(&RetKind::Unit).is_empty());
    }

    #[test]
    fn unit_ret_body_evaluates_and_returns_ok() {
        let call = quote! { touch(7) };
        let tokens = ret_body(&PathCtx::default(), &RetKind::Unit, call).to_string();
        assert!(tokens.contains("touch (7)"));
        assert!(tokens.contains("ErrorCode :: Ok"));
    }

    #[test]
    fn result_ret_body_maps_err_to_the_domain_error_channel() {
        let tokens = ret_body(
            &PathCtx::default(),
            &RetKind::Result(Box::new(RetKind::Unit)),
            quote! { f() },
        )
        .to_string();
        assert!(tokens.contains("Result :: Ok"));
        assert!(tokens.contains("Result :: Err"));
        assert!(
            tokens.contains("Into :: into"),
            "the Err goes through the E: Into<BffiError> contract"
        );
        assert!(
            tokens.contains("status_u32"),
            "a derived user code replaces the framework status"
        );
    }

    #[test]
    fn nullable_none_tail_writes_the_zero_handle() {
        let tokens =
            value_tail(&PathCtx::default(), &RetKind::Nullable(BufferTy::String)).to_string();
        assert!(tokens.contains("Option :: Some"));
        assert!(tokens.contains("Option :: None"));
        assert!(tokens.contains("0_u64"));
    }

    #[test]
    fn buffer_conv_walks_the_payloads() {
        let ctx = PathCtx::default();
        assert!(
            buffer_conv(&ctx, BufferTy::String)
                .to_string()
                .contains("into_bytes")
        );
        assert!(
            buffer_conv(&ctx, BufferTy::ByteVec)
                .to_string()
                .contains("from_vec")
        );
        assert_eq!(
            buffer_conv(&ctx, BufferTy::CopiedBuf).to_string(),
            quote! { let __bytes = __value ; }.to_string()
        );
    }

    #[test]
    fn default_context_emits_the_facade_paths() {
        let tokens =
            value_tail(&PathCtx::default(), &RetKind::Buffer(BufferTy::ByteVec)).to_string();
        assert!(tokens.contains(":: bffi :: build :: runtime :: store_bytes"));
        assert!(tokens.contains(":: bffi :: types :: CopiedBuf :: from_vec"));
        assert!(tokens.contains(":: bffi :: core :: ErrorCode :: TableFull"));
    }

    #[test]
    fn direct_context_emits_the_pre_merge_paths() {
        let tokens =
            value_tail(&PathCtx::direct(), &RetKind::Buffer(BufferTy::ByteVec)).to_string();
        assert!(tokens.contains(":: bffi_build :: runtime :: store_bytes"));
        assert!(tokens.contains(":: bffi_types :: CopiedBuf :: from_vec"));
        assert!(tokens.contains(":: bffi_core :: ErrorCode :: TableFull"));
    }

    #[test]
    fn facade_context_emits_the_namespaced_paths() {
        let ctx = PathCtx::from_attr("bffi");
        let tokens = value_tail(&ctx, &RetKind::Buffer(BufferTy::ByteVec)).to_string();
        assert!(tokens.contains(":: bffi :: build :: runtime :: store_bytes"));
        assert!(tokens.contains(":: bffi :: types :: CopiedBuf :: from_vec"));
        assert!(tokens.contains(":: bffi :: core :: ErrorCode :: TableFull"));
    }

    #[test]
    fn param_conversions_skip_non_borrowed_params_in_order() {
        let tokens = param_conversions(
            &PathCtx::default(),
            [
                ("a", ShimKind::Prim(PrimTy::U32)),
                ("b", ShimKind::Str),
                ("c", ShimKind::BigInt(BigIntTy::U64)),
            ],
        )
        .to_string();
        assert!(tokens.contains("b_ptr"));
        assert!(tokens.contains("b_view"));
        assert!(!tokens.contains("a_ptr"));
        assert!(!tokens.contains("c_ptr"));
        assert!(tokens.contains("str_view"));
        assert!(tokens.contains("InvalidUtf8"));
    }

    #[test]
    fn buffer_view_shim_param_expands_to_the_ptr_len_pair() {
        let tokens = shim_param("data", ShimKind::BufferView, 0).to_string();
        assert!(tokens.contains("data_ptr : * const u8"));
        assert!(tokens.contains("data_len : u64"));
    }

    #[test]
    fn buffer_view_conversion_null_checks_and_builds_the_view() {
        let tokens =
            param_conversions(&PathCtx::default(), [("data", ShimKind::BufferView)]).to_string();
        assert!(tokens.contains("data_len > 0_u64 && data_ptr . is_null ()"));
        assert!(tokens.contains("buffer argument pointer is null"));
        assert!(tokens.contains("NullPointer"));
        assert!(tokens.contains("from_raw_parts (data_ptr , data_len as usize)"));
        assert!(
            tokens.contains("data_len == 0_u64"),
            "the empty view must not feed a null pointer to `from_raw_parts`"
        );
        assert!(tokens.contains("buf_view"));
        assert!(tokens.contains("data_view"));
    }
}
