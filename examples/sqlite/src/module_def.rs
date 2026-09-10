//! THE single descriptor aggregation of the sqlite example: one
//! explicit `ModuleDef` consumed by the `emit-json` binary (which
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline).

use bffi::{FunctionDef, ModuleDef};

/// The `#[bffi]` functions of this example, in declaration order.
pub const FUNCTIONS: &[FunctionDef] = &[
    crate::bffi_meta_sqlite_version::FUNCTION,
    crate::bffi_meta_open::FUNCTION,
    crate::bffi_meta_exec::FUNCTION,
    crate::bffi_meta_query::FUNCTION,
    crate::bffi_meta_close::FUNCTION,
];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "sqlite",
    fns: FUNCTIONS,
    classes: &[],
    records: &[],
    enums: &[],
};
