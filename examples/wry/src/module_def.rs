//! THE single descriptor aggregation of the wry example: one
//! explicit `ModuleDef` (functions plus the `WebviewConfig` record)
//! consumed by the `emit-json` binary (which materializes
//! `.bffi/bffi.api.json` for the `@z2net/bffi` pipeline).

use bffi::{FunctionDef, ModuleDef, RecordDef};

/// The `#[bffi]` functions of this example, in declaration order.
pub const FUNCTIONS: &[FunctionDef] = &[
    crate::bffi_meta_webview_open::FUNCTION,
    crate::bffi_meta_webview_eval::FUNCTION,
    crate::bffi_meta_webview_close::FUNCTION,
    crate::bffi_meta_webview_bind_ipc::FUNCTION,
    crate::bffi_meta_webview_ipc_reply::FUNCTION,
    crate::bffi_meta_webview_poll_exit::FUNCTION,
    crate::bffi_meta_loop_pump::FUNCTION,
];

/// The record types of this example.
pub const RECORDS: &[RecordDef] = &[crate::WebviewConfig::BFFI_RECORD_DEF];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "wry",
    fns: FUNCTIONS,
    classes: &[],
    records: RECORDS,
    enums: &[],
    errors: &[],
};
