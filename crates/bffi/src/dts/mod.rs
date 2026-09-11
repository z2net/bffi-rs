//! # bffi-dts
//!
//! TypeScript `.d.ts` generation for the bffi-rs framework: native
//! bindings for [Bun](https://bun.sh). Types are generated from day
//! one so every exported symbol ships with an accurate declaration
//! file (see the "TS types" decision in
//! [DESIGN.md](https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)).
//!
//! ## Mission
//!
//! - A static intermediate representation (IR) of declarations.
//!   Descriptors are `&'static` values so the `#[bffi]` macro
//!   (a later stage) can emit them as constants.
//! - A deterministic renderer: the output depends only on the IR.
//!   Re-rendering the same IR always produces a byte-identical file.
//! - Exported symbols follow the `export_name = "bffi_" + js_name`
//!   convention, so declarations match the C ABI surface exactly.
//!
//! ## Render guarantees
//!
//! The renderer is pure and never panics:
//!
//! - a fixed two-line header; no timestamps or versions embedded;
//! - LF line endings and a trailing newline;
//! - output is a function of the IR alone.
//!
//! ## Quick start
//!
//! ```rust
//! use bffi::dts::{
//!     AbiOut, AbiPrim, AbiSig, AbiType, FunctionDef, ModuleDef, ParamDef, TsType,
//! };
//!
//! static PARAMS: &[ParamDef] = &[
//!     ParamDef { name: "a", ty: TsType::Number },
//!     ParamDef { name: "b", ty: TsType::Number },
//! ];
//! static ABI_PARAMS: &[AbiType] = &[AbiType::U32, AbiType::U32];
//! static DOCS: &[&str] = &["Adds two numbers."];
//! static FNS: &[FunctionDef] = &[FunctionDef {
//!     js_name: "add",
//!     export_name: "bffi_add",
//!     docs: DOCS,
//!     params: PARAMS,
//!     ret: TsType::Number,
//!     abi: AbiSig {
//!         params: ABI_PARAMS,
//!         out: Some(AbiOut::Prim(AbiPrim::U32)),
//!     },
//! }];
//!
//! let module = ModuleDef { name: "math", fns: FNS, classes: &[], records: &[], enums: &[], errors: &[] };
//! let dts = bffi::dts::render(&module);
//! assert!(dts.contains("/** Adds two numbers. */"));
//! assert!(dts.contains("export function add(a: number, b: number): number;"));
//! ```

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// Internal module aliases (the pre-merge crate names).
pub mod ident;
pub mod ir;
pub mod render;

pub use ident::sanitize;
pub use ir::{
    AbiOut, AbiPrim, AbiSig, AbiType, ClassDef, EnumDef, EnumVariantDef, ErrorDef, ErrorVariantDef,
    FieldDef, FunctionDef, MethodDef, ModuleDef, ParamDef, RecordDef, RecordFieldDef, TsType,
};
pub use render::render;
