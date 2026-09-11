//! THE single descriptor aggregation of the async example: one
//! explicit `ModuleDef` consumed by the `emit-json` binary (which
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline).

use bffi::{FunctionDef, ModuleDef};

/// The `#[bffi]` / `#[bffi_async]` functions of this example, in
/// declaration order.
pub const FUNCTIONS: &[FunctionDef] = &[
    crate::bffi_meta_double_async::FUNCTION,
    crate::bffi_meta_shout_async::FUNCTION,
    crate::bffi_meta_report_async::FUNCTION,
    crate::bffi_meta_ticks_async::FUNCTION,
    crate::bffi_meta_fail_async::FUNCTION,
    crate::bffi_meta_panic_async::FUNCTION,
    crate::bffi_meta_timed_async::FUNCTION,
    crate::bffi_meta_spawn_slow::FUNCTION,
    crate::bffi_meta_cancel_task::FUNCTION,
    crate::bffi_meta_async_pending::FUNCTION,
    crate::bffi_meta_loop_pump::FUNCTION,
];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "async",
    fns: FUNCTIONS,
    classes: &[],
    records: &[crate::Report::BFFI_RECORD_DEF],
    enums: &[],
    errors: &[],
};
