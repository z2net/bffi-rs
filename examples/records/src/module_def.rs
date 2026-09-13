//! THE single descriptor aggregation of the records example: one
//! explicit `ModuleDef` (functions plus the B1 `records`/`enums`
//! tables) consumed by the `emit-json` binary (which materializes
//! `.bffi/bffi.api.json` for the `@z2net/bffi` pipeline).

use bffi::{EnumDef, ModuleDef, RecordDef};

/// The `#[bffi]` functions of this example, in declaration order.
pub const FUNCTIONS: &[bffi::FunctionDef] = &[
    crate::bffi_meta_make_sample::FUNCTION,
    crate::bffi_meta_recenter::FUNCTION,
    crate::bffi_meta_total_weight::FUNCTION,
    crate::bffi_meta_distances::FUNCTION,
    crate::bffi_meta_labels::FUNCTION,
    crate::bffi_meta_label_bytes::FUNCTION,
    crate::bffi_meta_heaviest::FUNCTION,
    crate::bffi_meta_classify::FUNCTION,
];

/// The record types of this example.
pub const RECORDS: &[RecordDef] = &[crate::Sample::BFFI_RECORD_DEF];

/// The enum types of this example.
pub const ENUMS: &[EnumDef] = &[crate::Axis::BFFI_ENUM_DEF];

/// The full module definition.
pub const MODULE: ModuleDef = ModuleDef {
    name: "records",
    fns: FUNCTIONS,
    classes: &[],
    records: RECORDS,
    enums: ENUMS,
    errors: &[],
};
