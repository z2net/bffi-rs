//! Acceptance tests for the generated `bffi_meta_<name>` descriptors
//! (kanboard 6.1): every annotated function exposes a const
//! [`::bffi_dts::FunctionDef`] that matches the signature and renders
//! through `bffi-dts` into the expected TypeScript declarations.

#![allow(clippy::expect_used, clippy::unwrap_used)]
// The probe namespace modules mirror the facade namespaces; the
// generated descriptors carry the docs.
#![allow(missing_docs)]

use bffi::{AbiOut, AbiPrim, AbiSig, AbiType, FunctionDef, ModuleDef, ParamDef, TsType};

#[bffi_macros::bffi]
/// Adds two numbers.
fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[bffi_macros::bffi]
/// Handles a name.
fn greet(who: &str) -> u32 {
    who.len() as u32
}

#[bffi_macros::bffi]
/// Builds a greeting.
fn build_greeting(who: &str) -> String {
    format!("hello {who}")
}

#[bffi_macros::bffi]
/// Reads a payload.
fn payload() -> Option<Vec<u8>> {
    None
}

#[bffi_macros::bffi]
/// Finds a name.
fn find_name(hit: bool) -> Option<String> {
    if hit { Some("ada".to_owned()) } else { None }
}

#[bffi_macros::bffi]
/// Sums the bytes.
fn byte_sum(data: &[u8]) -> u32 {
    data.iter().map(|byte| u32::from(*byte)).sum()
}

// Facade-only mode without the facade: `crate = "bffi_macros_probe"`
// makes the generated shim and descriptor resolve through
// `::bffi_macros_probe::{core, types, dts, build}`; the descriptor
// VALUES are unaffected by the crate option. The `extern crate self`
// alias puts THIS crate into the extern prelude under the probe name,
// so the absolute paths resolve to the re-export modules below.
extern crate self as bffi_macros_probe;

pub mod core {
    pub use bffi::core::*;
}
pub mod types {
    pub use bffi::types::*;
}
pub mod dts {
    pub use bffi::dts::*;
}
pub mod build {
    pub use bffi::build::*;
}

#[bffi_macros::bffi(crate = "bffi_macros_probe")]
/// Doubles through the probe namespaces.
fn doubled(x: u32) -> u32 {
    x * 2
}

#[test]
fn descriptor_matches_the_expected_literal() {
    assert_eq!(
        bffi_meta_add::FUNCTION,
        FunctionDef {
            js_name: "add",
            export_name: "bffi_add",
            docs: &["Adds two numbers."],
            params: &[
                ParamDef {
                    name: "a",
                    ty: TsType::Number
                },
                ParamDef {
                    name: "b",
                    ty: TsType::Number
                },
            ],
            ret: TsType::Number,
            abi: AbiSig {
                params: &[AbiType::U32, AbiType::U32],
                out: Some(AbiOut::Prim(AbiPrim::U32)),
            },
        }
    );
    assert_eq!(
        bffi_meta_greet::FUNCTION,
        FunctionDef {
            js_name: "greet",
            export_name: "bffi_greet",
            docs: &["Handles a name."],
            params: &[ParamDef {
                name: "who",
                ty: TsType::String
            }],
            ret: TsType::Number,
            abi: AbiSig {
                params: &[AbiType::Cstring],
                out: Some(AbiOut::Prim(AbiPrim::U32)),
            },
        }
    );
    // Buffer payloads: String -> `string`, Vec<u8> -> `Uint8Array`;
    // the ABI return travels as the shared u64 handle slot.
    assert_eq!(
        bffi_meta_build_greeting::FUNCTION,
        FunctionDef {
            js_name: "build_greeting",
            export_name: "bffi_build_greeting",
            docs: &["Builds a greeting."],
            params: &[ParamDef {
                name: "who",
                ty: TsType::String
            }],
            ret: TsType::String,
            abi: AbiSig {
                params: &[AbiType::Cstring],
                out: Some(AbiOut::Handle),
            },
        }
    );
    // `Option` returns render honest `| null` types; `docs` contain
    // only the author lines (no auto-generated notes).
    assert_eq!(
        bffi_meta_payload::FUNCTION.ret,
        TsType::NullableUint8Array,
        "Option<Vec<u8>> renders as `Uint8Array | null`"
    );
    assert_eq!(
        bffi_meta_payload::FUNCTION.docs,
        &["Reads a payload."],
        "docs contain only the author line"
    );
    assert_eq!(
        bffi_meta_payload::FUNCTION.abi,
        AbiSig {
            params: &[],
            out: Some(AbiOut::Handle),
        },
        "the Option payload returns through the handle slot"
    );
    assert_eq!(
        bffi_meta_find_name::FUNCTION,
        FunctionDef {
            js_name: "find_name",
            export_name: "bffi_find_name",
            docs: &["Finds a name."],
            params: &[ParamDef {
                name: "hit",
                ty: TsType::Boolean
            }],
            ret: TsType::NullableString,
            abi: AbiSig {
                params: &[AbiType::Bool],
                out: Some(AbiOut::Handle),
            },
        }
    );
    // A borrowed `&[u8]` parameter renders as ONE `Uint8Array`
    // ParamDef: the `(ptr, len)` C pair is ABI-level only - and the
    // ABI signature records it as ONE `PtrLen` entry.
    assert_eq!(
        bffi_meta_byte_sum::FUNCTION,
        FunctionDef {
            js_name: "byte_sum",
            export_name: "bffi_byte_sum",
            docs: &["Sums the bytes."],
            params: &[ParamDef {
                name: "data",
                ty: TsType::Uint8Array
            }],
            ret: TsType::Number,
            abi: AbiSig {
                params: &[AbiType::PtrLen],
                out: Some(AbiOut::Prim(AbiPrim::U32)),
            },
        }
    );
}

#[test]
fn crate_option_descriptor_values_are_unaffected() {
    assert_eq!(
        bffi_meta_doubled::FUNCTION,
        FunctionDef {
            js_name: "doubled",
            export_name: "bffi_doubled",
            docs: &["Doubles through the probe namespaces."],
            params: &[ParamDef {
                name: "x",
                ty: TsType::Number
            }],
            ret: TsType::Number,
            abi: AbiSig {
                params: &[AbiType::U32],
                out: Some(AbiOut::Prim(AbiPrim::U32)),
            },
        }
    );
}

#[test]
fn descriptors_render_through_bffi_dts() {
    static FNS: &[FunctionDef] = &[
        bffi_meta_add::FUNCTION,
        bffi_meta_greet::FUNCTION,
        bffi_meta_build_greeting::FUNCTION,
        bffi_meta_payload::FUNCTION,
        bffi_meta_find_name::FUNCTION,
        bffi_meta_byte_sum::FUNCTION,
        bffi_meta_doubled::FUNCTION,
    ];
    let module = ModuleDef {
        name: "math",
        fns: FNS,
        classes: &[],
        records: &[],
        enums: &[],
    };
    let rendered = bffi::render(&module);
    assert!(rendered.contains("/** Adds two numbers. */"));
    assert!(rendered.contains("export function add(a: number, b: number): number;"));
    assert!(rendered.contains("export function greet(who: string): number;"));
    assert!(rendered.contains("export function build_greeting(who: string): string;"));
    assert!(rendered.contains("export function payload(): Uint8Array | null;"));
    assert!(rendered.contains("export function find_name(hit: boolean): string | null;"));
    assert!(rendered.contains("export function byte_sum(data: Uint8Array): number;"));
    assert!(rendered.contains("export function doubled(x: number): number;"));
}
