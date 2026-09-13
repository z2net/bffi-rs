//! THE single descriptor aggregation of the event-loop example: one
//! explicit `ModuleDef` consumed by the `emit-json` binary (which
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline).

use bffi::{FunctionDef, ModuleDef};

/// The `#[bffi]` functions of this example, in declaration order.
pub const FUNCTIONS: &[FunctionDef] = &[
    crate::bffi_meta_enqueue_job::FUNCTION,
    crate::bffi_meta_last_result::FUNCTION,
    crate::bffi_meta_loop_pump::FUNCTION,
    crate::bffi_meta_loop_pending::FUNCTION,
    crate::bffi_meta_loop_executed::FUNCTION,
    crate::bffi_meta_marshal_status::FUNCTION,
    crate::bffi_meta_loop_run::FUNCTION,
    crate::bffi_meta_loop_stop::FUNCTION,
];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "event-loop",
    fns: FUNCTIONS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};
