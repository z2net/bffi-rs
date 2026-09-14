//! THE single descriptor aggregation of the callbacks example: one
//! explicit `ModuleDef` consumed by the `emit-json` binary (which
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline).

use bffi::{FunctionDef, ModuleDef};

/// The `#[bffi]` functions of this example, in declaration order.
pub const FUNCTIONS: &[FunctionDef] = &[
    crate::bffi_meta_callback_register::FUNCTION,
    crate::bffi_meta_callback_invoke_status::FUNCTION,
    crate::bffi_meta_callback_ptr::FUNCTION,
    crate::bffi_meta_bind_js_thread::FUNCTION,
    crate::bffi_meta_marshal_invoke::FUNCTION,
    crate::bffi_meta_last_invoked::FUNCTION,
    crate::bffi_meta_loop_run::FUNCTION,
    crate::bffi_meta_loop_stop::FUNCTION,
];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "callbacks",
    fns: FUNCTIONS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};
