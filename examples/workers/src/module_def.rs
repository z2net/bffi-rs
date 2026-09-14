//! THE single descriptor aggregation of the workers example: one
//! explicit `ModuleDef` consumed by the `emit-json` binary (which
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline).

use bffi::{FunctionDef, ModuleDef};

/// The `#[bffi]` functions of this example, in declaration order.
pub const FUNCTIONS: &[FunctionDef] = &[
    crate::bffi_meta_bind_js_thread::FUNCTION,
    crate::bffi_meta_unbind_js_thread::FUNCTION,
    crate::bffi_meta_sum_to::FUNCTION,
    crate::bffi_meta_callback_register::FUNCTION,
    crate::bffi_meta_spawn_invoke_wait::FUNCTION,
    crate::bffi_meta_wait_code::FUNCTION,
    crate::bffi_meta_wait_value::FUNCTION,
    crate::bffi_meta_loop_pump::FUNCTION,
];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "workers",
    fns: FUNCTIONS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};
