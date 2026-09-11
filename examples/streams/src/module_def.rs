//! THE single descriptor aggregation of the streams example: one
//! explicit `ModuleDef` (functions plus the B1 `records`/`enums`
//! tables) consumed by the `emit-json` binary (which materializes
//! `.bffi/bffi.api.json` for the `@z2net/bffi` pipeline).

use bffi::{EnumDef, ModuleDef, RecordDef};

/// The `#[bffi_stream]` functions of this example, in declaration
/// order.
pub const FUNCTIONS: &[bffi::FunctionDef] = &[
    crate::bffi_meta_numbers::FUNCTION,
    crate::bffi_meta_samples::FUNCTION,
    crate::bffi_meta_fizzbuzz::FUNCTION,
];

/// The record types of this example.
pub const RECORDS: &[RecordDef] = &[crate::Sample::BFFI_RECORD_DEF];

/// The enum types of this example.
pub const ENUMS: &[EnumDef] = &[crate::Axis::BFFI_ENUM_DEF];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "streams",
    fns: FUNCTIONS,
    classes: &[],
    records: RECORDS,
    enums: ENUMS,
};
