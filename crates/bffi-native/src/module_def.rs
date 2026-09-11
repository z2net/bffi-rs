//! THE single descriptor aggregation of the reference library: one
//! explicit `ModuleDef` consumed by the `emit-json` binary (which
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline and for the `packages/native` main package).

use bffi::{FunctionDef, ModuleDef};

/// The `#[bffi]` functions of this library, in declaration order.
pub const FUNCTIONS: &[FunctionDef] = &[
    crate::bffi_meta_add::FUNCTION,
    crate::bffi_meta_shout::FUNCTION,
    crate::bffi_meta_version::FUNCTION,
];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "native",
    fns: FUNCTIONS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};
