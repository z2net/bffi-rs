//! # bffi
//!
//! The public facade of the bffi-rs framework: one dependency giving
//! the whole stack (DESIGN §5), with [`unsafe_zero_copy`] as the
//! single zero-copy door (DESIGN §6.3: "zero-copy is allowed only
//! through `bffi::unsafe_zero_copy`").
//!
//! ## What is re-exported
//!
//! | Area | Names |
//! | ---- | ----- |
//! | core | [`Handle`], [`TypeTag`], [`Registry`], [`BffiError`], [`ErrorCode`], `catch_panic`/`run_extern_body`, the TLS last-error pair |
//! | errors | [`JsErrorName`], [`JsErrorShape`], the code -> JS-constructor mapping |
//! | types | [`JsNumber`], [`CopiedBuf`], `str_view`/`buf_view`, the string converters |
//! | objects | [`ObjectWrap`], [`ObjectError`], the tag helpers |
//! | callbacks | `register`/`invoke`/`revoke`, `bind_js_callback`, the JS-thread gate |
//! | dts | the full IR + `render` + `sanitize` |
//! | macros | `#[bffi]`, `#[bffi_async]`, `#[bffi_class]`, `#[bffi_impl]`, `#[bffi_constructor]`, `bffi_runtime_abi!` |
//! | event loop | `enqueue`/`marshal`/`run`/`stop`/`pump` |
//! | namespaces | [`core`], [`types`], [`dts`], [`object`], [`build`], [`r#async`] - the 1:1 re-export modules the macros emit by default |
//!
//! ## Macros: default facade paths, `crate = "..."` opt-outs
//!
//! The generated code of `#[bffi]` / `#[bffi_class]` /
//! `#[bffi_impl]` / `#[bffi_async]` names runtime paths that resolve
//! in one of two ways:
//!
//! - **Facade mode (the default)** - no attribute options needed:
//!   the expansion names `::bffi::core`, `::bffi::types`,
//!   `::bffi::dts`, `::bffi::object`, `::bffi::build` and
//!   `::bffi::r#async` - the namespaces re-exported below - so this
//!   crate alone suffices. `crate = "bffi"` (or any other facade
//!   crate re-exporting the same namespaces) redirects the roots;
//!   `crate = "bffi"` is therefore accepted and identical to the
//!   default.
//! - **Direct-dependencies mode (`crate = "direct"`)** - the
//!   expansion names the pre-merge roots `::bffi_core`,
//!   `::bffi_types`, ...; an escape hatch for in-workspace
//!   development against the historical crate names (not published
//!   on crates.io).
//!
//! Note that `bffi_runtime_abi!` keeps its `$crate`-relative paths in
//! `bffi-build` and always needs that crate as a direct dependency.
//!
//! ## Example
//!
//! ```
//! use bffi::{CopiedBuf, ErrorCode, Handle, ObjectWrap, TypeTag};
//!
//! const SESSION: TypeTag = TypeTag(0x0160);
//!
//! // Objects: wrap, read, release - through the facade types.
//! let wrap = ObjectWrap::<u32>::new(SESSION).expect("tag claimed once");
//! let handle = wrap.wrap(42).expect("room");
//! assert_eq!(*wrap.get(handle).expect("live"), 42);
//!
//! // Copy by default everywhere else:
//! let copied = CopiedBuf::from_slice(b"copy by default");
//! assert_eq!(copied.as_slice(), b"copy by default");
//! let _ok: ErrorCode = ErrorCode::Ok;
//! let _null: Handle = Handle::NULL;
//! ```

// The workspace restriction lints (expect/unwrap/panic) target production
// code; tests assert invariants and intentionally trigger panics.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// The merged library modules (pre-merge crate names; the facade
// re-exports below resolve through them). Feature-gated per slice.
#[cfg(feature = "async")]
#[path = "async/mod.rs"]
pub mod bffi_async;
#[cfg(feature = "build")]
#[path = "build/mod.rs"]
pub mod bffi_build;
#[cfg(feature = "callback")]
#[path = "callback/mod.rs"]
pub mod bffi_callback;
#[cfg(feature = "core")]
#[path = "core/mod.rs"]
pub mod bffi_core;
#[cfg(feature = "dts")]
#[path = "dts/mod.rs"]
pub mod bffi_dts;
#[cfg(feature = "error")]
#[path = "error/mod.rs"]
pub mod bffi_error;
#[cfg(feature = "event-loop")]
#[path = "event_loop/mod.rs"]
pub mod bffi_event_loop;
#[cfg(feature = "object")]
#[path = "object/mod.rs"]
pub mod bffi_object;
#[cfg(feature = "stream")]
#[path = "stream/mod.rs"]
pub mod bffi_stream;
#[cfg(feature = "types")]
#[path = "types/mod.rs"]
pub mod bffi_types;

// Every re-export below is gated on the feature that owns the
// module: a minimal build (`--no-default-features` plus the slices
// you need) must compile with exactly the enabled surface and no
// dangling paths.
#[cfg(feature = "async")]
pub use crate::bffi_async::{
    AsyncError, AsyncValue, Sleep, Timeout, TimeoutError, attach, cancel, pending_tasks, sleep,
    spawn,
};
#[cfg(feature = "build")]
pub use crate::bffi_build::BuildError;
#[cfg(feature = "callback")]
pub use crate::bffi_callback::{
    CallbackError, CallbackSig, JsCallbackInfo, Value, ValueType, bind_js_callback,
    ensure_js_thread, invoke, js_callback, register, revoke, set_js_thread,
};
#[cfg(feature = "core")]
pub use crate::bffi_core::{
    BffiError, ErrorCode, ErrorRich, Handle, MAX_GENERATION, MAX_INDEX, Registry, RegistryError,
    TableError, TypeTag, boundary, catch_panic, panic_message, run_extern_body, run_extern_body_or,
    set_last_error, take_last_error,
};
#[cfg(feature = "dts")]
pub use crate::bffi_dts::{
    AbiOut, AbiPrim, AbiSig, AbiType, ClassDef, EnumDef, EnumVariantDef, FieldDef, FunctionDef,
    MethodDef, ModuleDef, ParamDef, RecordDef, RecordFieldDef, TsType, render, sanitize,
};
#[cfg(feature = "error")]
pub use crate::bffi_error::{
    JsErrorExt, JsErrorName, JsErrorShape, js_error_name, take_last_error_shape,
};
#[cfg(feature = "event-loop")]
pub use crate::bffi_event_loop::{
    EventLoopError, Job, enqueue, executed_total, is_running, marshal, pending, pump, run, stop,
};
#[cfg(feature = "object")]
pub use crate::bffi_object::{ObjectError, ObjectWrap, TAG_MAX, TAG_MIN, tag_in_range};
#[cfg(feature = "stream")]
pub use crate::bffi_stream::{
    BffiStreamItem, Chunk, Ctx, StreamError, drop_stream, is_live, next_chunk,
    spawn as spawn_stream, spawn_push,
};
#[cfg(feature = "types")]
pub use crate::bffi_types::{
    BffiWire, ConversionError, CopiedBuf, JsNumber, buf_view, bytes_to_string, str_view,
    string_to_bytes,
};
// The attribute macros come with the optional `bffi-macros`
// dependency; the `#[bffi_async]` expansion additionally resolves
// `::bffi::r#async`, so enable the `async` feature to use it.
#[cfg(feature = "macros")]
pub use bffi_macros::{
    BffiEnum, BffiRecord, bffi, bffi_async, bffi_class, bffi_constructor, bffi_impl, bffi_stream,
};

/// THE single zero-copy door (DESIGN §6.3). Zero-copy is allowed only
/// through `bffi::unsafe_zero_copy`; everything else in this facade
/// copies by default.
///
/// The constructors are safe - they take `&[u8]` - but the returned
/// views borrow their input: the borrow checker keeps them from
/// outliving the FFI call. Never store a view in Rust state, and
/// assume JS may mutate the aliased memory at any time. The genuinely
/// unsafe `(ptr, len) -> &[u8]` step at the ABI lives in
/// `bffi-build`/the generated shims, not here.
///
/// ```
/// let text = bffi::str_view(b"hello").expect("valid utf-8");
/// assert_eq!(text.as_str(), "hello");
///
/// let bytes = [1_u8, 2, 3];
/// let view = bffi::buf_view(&bytes);
/// assert_eq!(view.as_slice(), [1, 2, 3]);
///
/// // The view types themselves are reachable only through the door:
/// let typed: bffi::unsafe_zero_copy::ZeroCopyStr<'_> = text;
/// let _buf: bffi::unsafe_zero_copy::ZeroCopyBuf<'_> = view;
/// ```
#[cfg(feature = "types")]
pub mod unsafe_zero_copy {
    // The view TYPES are exported only through this module - the
    // module name is the warning label. The constructor functions
    // (`str_view`/`buf_view`) are also available at the facade root
    // alongside the copying converters; they are re-exported here as
    // well because the `#[bffi_async]` shims name the
    // `::bffi::types::unsafe_zero_copy::` path (the `types`
    // namespace re-exports it).
    pub use crate::bffi_types::unsafe_zero_copy::{ZeroCopyBuf, ZeroCopyStr, buf_view, str_view};
}

/// Namespaced re-export of [`bffi-core`]: `bffi::core::*` mirrors
/// `bffi::core::*` 1:1. These are the paths the `crate = "bffi"`
/// macros emit for the core roots (`::bffi::core::ErrorCode`, ...).
///
/// [`bffi-core`]: https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/core
#[cfg(feature = "core")]
pub mod core {
    pub use crate::bffi_core::*;
}

/// Namespaced re-export of [`bffi-types`]: `bffi::types::*` mirrors
/// `bffi::types::*` 1:1, including `unsafe_zero_copy`.
///
/// [`bffi-types`]: https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/types
#[cfg(feature = "types")]
pub mod types {
    pub use crate::bffi_types::*;
}

/// Namespaced re-export of [`bffi-dts`]: `bffi::dts::*` mirrors
/// `bffi::dts::*` 1:1. The descriptor consts the macros emit resolve
/// here in facade-only mode (`::bffi::dts::FunctionDef`, ...).
///
/// [`bffi-dts`]: https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/dts
#[cfg(feature = "dts")]
pub mod dts {
    pub use crate::bffi_dts::*;
}

/// Namespaced re-export of [`bffi-object`]: `bffi::object::*` mirrors
/// `bffi::object::*` 1:1. The class shims resolve here in facade-only
/// mode (`::bffi::object::ObjectWrap`, ...).
///
/// [`bffi-object`]: https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/object
#[cfg(feature = "object")]
pub mod object {
    pub use crate::bffi_object::*;
}

/// Namespaced re-export of the async slice: `bffi::r#async::*`
/// mirrors the pre-merge `bffi-async` crate 1:1. The `#[bffi_async]`
/// shims resolve here (`::bffi::r#async::spawn`, ...); the module
/// name is a raw identifier because `async` is a keyword.
#[cfg(feature = "async")]
pub mod r#async {
    pub use crate::bffi_async::*;
}

/// Namespaced re-export of [`bffi-build`]: `bffi::build::*` mirrors
/// `bffi::build::*` 1:1. The buffer-return shims resolve here in
/// facade-only mode (`::bffi::build::runtime::store_bytes`, ...).
///
/// [`bffi-build`]: https://github.com/z2net/bffi-rs/blob/main/crates/bffi/src/build
#[cfg(feature = "build")]
pub mod build {
    pub use crate::bffi_build::*;
}
