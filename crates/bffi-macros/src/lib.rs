//! # bffi-macros
//!
//! The `#[bffi]` attribute macro of the bffi-rs framework: native
//! bindings for [Bun](https://bun.sh) that target `bun:ffi` and a
//! thin C ABI layer (see
//! [DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)).
//!
//! ## Mission
//!
//! - A single ABI generator: one macro owns everything that crosses
//!   the FFI boundary - no hand-written shims, no drift between the
//!   Rust signature and the generated C ABI.
//! - Each `#[bffi]` expansion produces exactly three artifacts: the
//!   original item unchanged, an `extern "C"` shim that enforces the
//!   boundary policy (copy by default, panics converted into JS
//!   errors), and a `bffi_meta` const descriptor consumed by
//!   `bffi-dts` for TypeScript `.d.ts` generation.
//! - The macro runs in the user crate: the expansion lands wherever
//!   the attribute is used.
//!
//! The full contract - syntax, expansion, build-profile policy, the
//! P1 type matrix, diagnostics, and user-crate requirements - is
//! documented on [`bffi`].
//!
//! ## The contract at a glance
//!
//! - **Syntax.** The attribute takes one optional option,
//!   `crate = "<name>"` (redirect the generated paths; `"direct"`
//!   selects the pre-merge roots); any other option is rejected with
//!   `E004`.
//! - **Expansion.** On a plain `fn add(a: u32, b: u32) -> u32` the
//!   macro emits the function unchanged, a `bffi_add` C ABI shim, and
//!   a `bffi_meta_add` module holding the const
//!   `::bffi::dts::FunctionDef`.
//! - **Build-profile policy.** Debug builds run the body bare (a
//!   panic escapes and aborts, DESIGN §6.5); release builds run it
//!   inside `::bffi::core::boundary::run_extern_body`, which turns a
//!   panic into `ErrorCode::Panic` plus a stored last error.
//! - **Type matrix (P1).** `i8|i16|i32|u8|u16|u32|f32|f64` ->
//!   `number`, `i64|u64` -> `bigint`, `bool` -> `boolean`, `&str` ->
//!   `string` (parameter only), `()` -> `void` (return only).
//!   Everything else is rejected at compile time.
//! - **Diagnostics.** Stable codes `E001` (shape), `E002` (parameter
//!   type), `E003` (return type), `E004` (attribute options), each
//!   with `help:`/`note:` lines pointing at DESIGN.md.
//! - **User-crate requirements.** The expansion names the facade
//!   namespaces `::bffi::{core, types, dts, build}` by default, so a
//!   dependency on the `bffi` facade suffices, and function names
//!   must be unique - the `no_mangle` shims collide at link time
//!   otherwise. With `crate = "<name>"` the expansion names
//!   `::<name>::{core, types, dts, build}` instead; with
//!   `crate = "direct"` the pre-merge roots `::bffi_core`, ... .

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

#[allow(unused_extern_crates)]
extern crate proc_macro;

mod async_fn;
mod class;
mod derive;
mod errors;
mod mapping;
mod meta;
mod model;
mod shim;
mod stream_fn;
mod support;

use proc_macro::TokenStream;

/// Marks a function for bffi-rs code generation.
///
/// # Syntax
///
/// The attribute takes at most one option:
///
/// - `#[bffi]` - the generated code names the `bffi` facade
///   namespaces (`::bffi::core`, `::bffi::types`, `::bffi::dts`,
///   `::bffi::build`, `::bffi::r#async`), so `bffi` alone suffices
///   as a dependency;
/// - `#[bffi(crate = "<name>")]` - redirects every generated path to
///   another facade re-exporting the same namespaces;
///   `crate = "bffi"` is accepted and identical to the default;
/// - `#[bffi(crate = "direct")]` - names the pre-merge direct
///   dependencies (`::bffi_core`, `::bffi_types`, `::bffi_dts`,
///   `::bffi_build`); an in-workspace escape hatch (the pre-merge
///   crates are not published).
///
/// Any other option (`#[bffi(rename = "x")]`, a non-literal or
/// invalid `crate` value) is rejected with `E004`.
///
/// # Expansion
///
/// On a validated function the macro emits three artifacts, in order:
///
/// ```text
/// #[bffi]
/// /// Adds two numbers.
/// fn add(a: u32, b: u32) -> u32 { a + b }
///
/// // 1. The original item, unchanged (docs stay on it).
/// fn add(a: u32, b: u32) -> u32 { a + b }
///
/// // 2. The C ABI shim - the symbol the JS side links against,
/// //    emitted twice under complementary cfgs (see below):
/// #[unsafe(no_mangle)]
/// pub extern "C" fn bffi_add(a: u32, b: u32, __ret: *mut u32)
///     -> ::bffi::core::ErrorCode { /* ... */ }
///
/// // 3. The const descriptor consumed by bffi-dts / bffi-build:
/// pub mod bffi_meta_add {
///     pub const FUNCTION: ::bffi::dts::FunctionDef = ::bffi::dts::FunctionDef {
///         js_name: "add",
///         export_name: "bffi_add",
///         docs: &["Adds two numbers."],
///         params: &[/* ParamDef { name: "a", ty: TsType::Number }, ... */],
///         ret: ::bffi::dts::TsType::Number,
///     };
/// }
/// ```
///
/// The shim copies inputs (never zero-copy), validates every pointer,
/// and transports the result through an out-parameter: the return
/// value is written through `__ret: *mut T` appended after the
/// parameters; `()` returns have no out-parameter. A `&str` parameter
/// arrives as `*const c_char` (NUL-terminated cstring per the
/// `bun:ffi` convention, DESIGN §6.3); the shim null-checks it,
/// converts with `CStr::from_ptr`, and validates UTF-8 through
/// `bffi::types::unsafe_zero_copy::str_view`. Every failure stores a
/// `BffiError` via `::bffi::core::set_last_error` and returns the
/// matching `ErrorCode`; success returns `ErrorCode::Ok`.
///
/// # Build-profile policy (DESIGN §6.5)
///
/// The two cfg variants of the shim differ only in how the body runs:
///
/// - `#[cfg(debug_assertions)]` - the body runs bare. A panic escapes
///   and aborts the process, keeping stack traces intact while
///   debugging.
/// - `#[cfg(not(debug_assertions))]` - the body runs inside
///   `::bffi::core::boundary::run_extern_body`. A panic unwinds no
///   further: it becomes `ErrorCode::Panic` plus a stored thread-local
///   last error.
///
/// # Type matrix (P1)
///
/// | Rust type                                     | Param | Return | TsType    |
/// | --------------------------------------------- | ----- | ------ | --------- |
/// | `i8` `i16` `i32` `u8` `u16` `u32` `f32` `f64` | yes   | yes    | `number`  |
/// | `i64` `u64`                                   | yes   | yes    | `bigint`  |
/// | `bool`                                        | yes   | yes    | `boolean` |
/// | `&str` (borrowed, not `mut`; lifetimes ok)    | yes   | -      | `string`  |
/// | `()`                                          | -     | yes    | `void`    |
///
/// Everything else is rejected at compile time. Buffers, `Option`,
/// structs and `Result` arrive with `bffi-build` (P2).
///
/// # Diagnostics
///
/// Rejections carry stable codes (locked by the golden `.stderr` files
/// in `tests/ui`; do not renumber):
///
/// | Code   | Meaning                                                          |
/// | ------ | ---------------------------------------------------------------- |
/// | `E001` | unsupported function shape (async/generic/unsafe/self/variadic/extern/const/non-ident param patterns) |
/// | `E002` | unsupported parameter type                                       |
/// | `E003` | unsupported return type                                          |
/// | `E004` | unknown option; only `crate = "..."` is supported                |
///
/// Format - `bffi[<code>]: <message>` first line, then `  = help: `
/// and `  = note: ` lines ending with the DESIGN.md URL:
///
/// ```text
/// error: bffi[E002]: unsupported type `Vec < u8 >` for parameter `data`
///   = help: supported in P1: i8|i16|i32|i64|u8|u16|u32|u64|f32|f64|bool|&str|()
///   = note: buffers, Option, structs and Result arrive with bffi-build (P2)
///   = note: boundary rules: DESIGN.md (https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)
/// ```
///
/// These codes are the compile-time counterpart of the runtime
/// `BffiError` scheme (code + message + source in `bffi-core`);
/// mapping runtime errors to JS is `bffi-error`'s job - the macro
/// never depends on it. Items that are not plain functions fail with a
/// plain `syn` parse error, outside the E-code scheme.
///
/// # Requirements on the user crate
///
/// - Default: a dependency on the `bffi` facade - the expansion
///   names `::bffi::core`, `::bffi::types`, `::bffi::dts` and
///   `::bffi::build`, which the facade re-exports.
/// - `crate = "direct"`: dependencies on the pre-merge
///   `bffi-core`, `bffi-types`, `bffi-dts` (and `bffi-build` for
///   buffer and `Result` returns) - the expansion names
///   `::bffi_core`, `::bffi_types`, and `::bffi_dts` at the call
///   site. The pre-merge crates are not published; this mode exists
///   for in-workspace development.
/// - Function names must be unique: each shim is `#[unsafe(no_mangle)]`,
///   so two `#[bffi]` functions with the same name collide at link
///   time as duplicate symbols.
/// - The user crate must use edition 2024: the generated shims carry
///   `#[unsafe(no_mangle)]`, which the 2024 edition gates.
///
/// Inputs outside these rules produce a spanned compile error with the
/// documented help lines; see
/// [DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md).
#[proc_macro_attribute]
pub fn bffi(attrs: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = proc_macro2::TokenStream::from(attrs);
    let item = proc_macro2::TokenStream::from(item);
    // The item is parsed twice; this first parse gives the generator
    // the typed item.
    match syn::parse2::<syn::ItemFn>(item.clone()) {
        Err(err) => err.to_compile_error().into(),
        Ok(func) => match model::FnModel::parse(&attrs, item) {
            Ok(model_) => {
                let shim = shim::expand(&model_);
                let meta = meta::expand(&model_);
                quote::quote! { #func #shim #meta }.into()
            }
            Err(err) => err.to_compile_error().into(),
        },
    }
}

/// Marks an async function for bffi-rs spawn-shim generation.
///
/// # Syntax
///
/// `#[bffi_async]` (optionally with `crate = "<name>"` to redirect
/// the generated paths, or `crate = "direct"` for the pre-merge
/// roots) on a plain `async fn` whose parameters are the owned matrix
/// (primitives, `i64`/`u64`, `bool`, `String`, `Vec<u8>`) and whose
/// return is the P2 matrix (`()`, primitives, `i64`/`u64`,
/// `String`, `Vec<u8>`, `CopiedBuf`, `Result<T, E>`). Borrowed
/// parameters (`&str`, `&[u8]`) cannot cross the spawn boundary and
/// are rejected with `E002`; `Option` async returns are rejected with
/// `E003`.
///
/// # Expansion
///
/// The original async fn unchanged plus a spawn shim
/// `bffi_<name>(params..., __ret: *mut u64) -> ErrorCode`: the shim
/// moves the parameters into the future, spawns it on the built-in
/// executor and writes the task handle to `__ret`. JavaScript turns
/// the handle into a `Promise` with the loader's `wrapTask`; the
/// descriptor return type is `Promise<T>`.
///
/// Resolutions are delivered while the JS thread drains the event
/// loop (`pump()` / `run()`).
#[proc_macro_attribute]
pub fn bffi_async(attrs: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = proc_macro2::TokenStream::from(attrs);
    let item = proc_macro2::TokenStream::from(item);
    let item2 = item.clone();
    match async_fn::AsyncFnModel::parse(&attrs, item) {
        Ok(model) => {
            let shim = async_fn::expand(&model);
            let meta = async_fn::async_meta(&model);
            quote::quote! { #item2 #shim #meta }.into()
        }
        Err(err) => err.to_compile_error().into(),
    }
}

/// Declares a native class over a named struct: read-only getters for
/// `pub` primitive fields, the `bffi_<name>_release` destructor export,
/// and the `bffi_meta_<name>` metadata module.
///
/// Syntax: `#[bffi_class(tag = 0x01xx)]` with a literal tag in the
/// bffi-object range (`0x0100..=0x01FF`); one tag = one type per
/// process. An optional `crate = "<name>"` redirects the generated
/// paths (default: the `bffi` facade namespaces; `"direct"` selects
/// the pre-merge roots).
#[proc_macro_attribute]
pub fn bffi_class(attrs: TokenStream, item: TokenStream) -> TokenStream {
    class::bffi_class(attrs.into(), item.into()).into()
}

/// Marks the `impl` block of a `#[bffi_class]` struct: generates the
/// constructor and `&self`-method shims plus the
/// `bffi_meta_<name>_impl::CLASS` descriptor.
///
/// Exactly one `#[bffi_constructor] pub fn new(...) -> Self` is
/// required; every other `fn` must take `&self` (methods with `&mut
/// self`/`self` cannot be served through the `Arc<T>` ownership
/// model). An optional `crate = "<name>"` redirects the generated
/// paths (default: the `bffi` facade namespaces; `"direct"` selects
/// the pre-merge roots) - use the same value as on the matching
/// `#[bffi_class]`.
#[proc_macro_attribute]
pub fn bffi_impl(attrs: TokenStream, item: TokenStream) -> TokenStream {
    class::bffi_impl(attrs.into(), item.into()).into()
}

/// Marks the constructor inside a `#[bffi_impl]` block. A pure marker:
/// the impl macro reads the attribute; this macro echoes the item
/// unchanged so the annotation can also stand alone.
#[proc_macro_attribute]
pub fn bffi_constructor(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    class::bffi_constructor(_attrs.into(), item.into()).into()
}

/// Marks a named struct as a boundary-crossing record type (B1):
/// generates the `BFFI_RECORD_DEF` descriptor entry and the
/// `bffi_wire_encode` / `bffi_wire_decode` pair over the shared wire
/// codec. See `derive` module docs for the supported field matrix and
/// the `E009`/`E010` rejections.
#[proc_macro_derive(BffiRecord)]
pub fn bffi_record_derive(input: TokenStream) -> TokenStream {
    derive::record(input)
}

/// Marks a unit enum as a boundary-crossing choice type (B1): the
/// value encodes as its variant name, the TS side sees a union of
/// string literals. `E011` rejects data-carrying variants and
/// generics.
#[proc_macro_derive(BffiEnum)]
pub fn bffi_enum_derive(input: TokenStream) -> TokenStream {
    derive::enumeration(input)
}

/// Marks a plain fn returning `impl Iterator<Item = T> + Send` as a
/// stream export (B2): the spawn shim registers the item-encoded
/// iterator in the stream table and returns its handle; JavaScript
/// pulls chunks through the generic `bffi_stream_next` export (the
/// user crate generates it with `bffi::bffi_stream_abi!()`) and
/// iterates with `for await`. Item matrix as in `Vec<T>` sequences
/// plus `Vec<u8>` items; rejections carry `E012`.
#[proc_macro_attribute]
pub fn bffi_stream(attrs: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = proc_macro2::TokenStream::from(attrs);
    let item = proc_macro2::TokenStream::from(item);
    let item2 = item.clone();
    match stream_fn::parse(&attrs, item) {
        Ok(model) => {
            let shim = stream_fn::expand(&model);
            let meta = stream_fn::stream_meta(&model);
            quote::quote! { #item2 #shim #meta }.into()
        }
        Err(err) => err.to_compile_error().into(),
    }
}
