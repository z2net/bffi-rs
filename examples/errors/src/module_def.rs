//! THE single descriptor aggregation of the errors example: one
//! explicit `ModuleDef` consumed by the `emit-json` binary (which
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline).

use bffi::{ModuleDef, RecordDef};

/// The `#[bffi]` functions of this example, in declaration order.
pub const FUNCTIONS: &[bffi::FunctionDef] = &[
    crate::bffi_meta_find_user::FUNCTION,
    crate::bffi_meta_validate_age::FUNCTION,
    crate::bffi_meta_ad_hoc::FUNCTION,
    crate::bffi_meta_framework_error::FUNCTION,
];

/// The record types of this example.
pub const RECORDS: &[RecordDef] = &[crate::User::BFFI_RECORD_DEF];

/// The derived error enums of this example.
pub const ERRORS: &[bffi::ErrorDef] = &[crate::UsersError::BFFI_ERROR_DEF];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "errors",
    fns: FUNCTIONS,
    classes: &[],
    records: RECORDS,
    enums: &[],
    errors: ERRORS,
};
