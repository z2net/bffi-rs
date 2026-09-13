//! Golden tests for the deterministic `.d.ts` renderer.
//!
//! The files under `tests/golden/` are byte-normative: LF line endings
//! and exactly one trailing newline, per the format contract on
//! [`bffi::bffi_dts::render`]. Git may normalize working-tree files to CRLF
//! on Windows checkouts (`core.autocrlf`), so every textual comparison
//! normalizes `\r\n` back to `\n` first; the committed bytes are still
//! guarded by [`golden_files_contain_no_carriage_returns`], and the
//! repo-root `.gitattributes` forces `eol=lf` for `*.d.ts`.
//!
//! Module fixtures are declared as `static` descriptor arrays, proving
//! the const-descriptor pattern the `#[bffi]` macro (a later stage)
//! will emit.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bffi::bffi_dts::{
    AbiOut, AbiPrim, AbiSig, AbiType, ClassDef, FieldDef, FunctionDef, MethodDef, ModuleDef,
    ParamDef, TsType, render::render,
};

const U32_ABI: AbiSig = AbiSig {
    params: &[],
    out: Some(AbiOut::Prim(AbiPrim::U32)),
};
const UNIT_ABI: AbiSig = AbiSig {
    params: &[],
    out: None,
};
const HANDLE_ABI: AbiSig = AbiSig {
    params: &[],
    out: Some(AbiOut::Handle),
};

static ADD_PARAMS: &[ParamDef] = &[
    ParamDef {
        name: "a",
        ty: TsType::Number,
    },
    ParamDef {
        name: "b",
        ty: TsType::Number,
    },
];
static ADD_ABI: AbiSig = AbiSig {
    params: &[AbiType::U32, AbiType::U32],
    out: Some(AbiOut::Prim(AbiPrim::U32)),
};

static MATH_FNS: &[FunctionDef] = &[
    FunctionDef {
        js_name: "add",
        export_name: "bffi_add",
        docs: &["Adds two numbers."],
        params: ADD_PARAMS,
        ret: TsType::Number,
        abi: ADD_ABI,
    },
    FunctionDef {
        js_name: "find_name",
        export_name: "bffi_find_name",
        docs: &["Finds a name."],
        params: &[],
        ret: TsType::NullableString,
        abi: HANDLE_ABI,
    },
    FunctionDef {
        js_name: "read_payload",
        export_name: "bffi_read_payload",
        docs: &["Reads a payload."],
        params: &[],
        ret: TsType::NullableUint8Array,
        abi: HANDLE_ABI,
    },
];

static MATH: ModuleDef = ModuleDef {
    name: "math",
    fns: MATH_FNS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};

static KITCHEN_FNS: &[FunctionDef] = &[
    FunctionDef {
        js_name: "add",
        export_name: "bffi_add",
        docs: &["Adds two numbers."],
        params: ADD_PARAMS,
        ret: TsType::Number,
        abi: ADD_ABI,
    },
    FunctionDef {
        js_name: "class",
        export_name: "bffi_class",
        docs: &["Stores a value.", "Returns nothing."],
        params: &[
            ParamDef {
                name: "delete",
                ty: TsType::BigInt,
            },
            ParamDef {
                name: "data",
                ty: TsType::Uint8Array,
            },
            ParamDef {
                name: "flag",
                ty: TsType::Boolean,
            },
        ],
        ret: TsType::Void,
        abi: AbiSig {
            params: &[AbiType::U64, AbiType::PtrLen, AbiType::Bool],
            out: None,
        },
    },
    FunctionDef {
        js_name: "greet",
        export_name: "bffi_greet",
        docs: &["Greets."],
        params: &[ParamDef {
            name: "name",
            ty: TsType::String,
        }],
        ret: TsType::String,
        abi: AbiSig {
            params: &[AbiType::Cstring],
            out: Some(AbiOut::Handle),
        },
    },
    FunctionDef {
        js_name: "noop",
        export_name: "bffi_noop",
        docs: &[],
        params: &[],
        ret: TsType::Void,
        abi: UNIT_ABI,
    },
];

static KITCHEN: ModuleDef = ModuleDef {
    name: "kitchen",
    fns: KITCHEN_FNS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};

static EMPTY: ModuleDef = ModuleDef {
    name: "empty",
    fns: &[],
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};

/// A class fixture exercising the constructor, a field getter, a
/// method, a reserved-word method name, and multi-line JSDoc.
static COUNTER_CLASS: &[ClassDef] = &[ClassDef {
    js_name: "counter",
    release_export: "bffi_counter_release",
    docs: &["A native counter."],
    constructor: MethodDef {
        js_name: "constructor",
        export_name: "bffi_counter_new",
        docs: &["Creates a counter."],
        params: &[ParamDef {
            name: "start",
            ty: TsType::Number,
        }],
        ret: TsType::BigInt,
        abi: AbiSig {
            params: &[AbiType::U32],
            out: Some(AbiOut::Handle),
        },
    },
    fields: &[FieldDef {
        js_name: "value",
        export_name: "bffi_counter_value_get",
        docs: &["The current value."],
        ty: TsType::Number,
        out: AbiOut::Prim(AbiPrim::U32),
    }],
    methods: &[
        MethodDef {
            js_name: "increment",
            export_name: "bffi_counter_increment",
            docs: &["Adds one and returns the new value."],
            params: &[],
            ret: TsType::Number,
            abi: U32_ABI,
        },
        MethodDef {
            js_name: "class",
            export_name: "bffi_counter_class",
            docs: &[],
            params: &[],
            ret: TsType::Void,
            abi: UNIT_ABI,
        },
    ],
}];

static SHAPES: ModuleDef = ModuleDef {
    name: "shapes",
    fns: &[],
    classes: COUNTER_CLASS,
    records: &[],
    enums: &[],
    errors: &[],
};

static B1_XY_PARAMS: &[ParamDef] = &[
    ParamDef {
        name: "x",
        ty: TsType::Number,
    },
    ParamDef {
        name: "y",
        ty: TsType::Number,
    },
];

static B1_FNS: &[FunctionDef] = &[
    FunctionDef {
        js_name: "make_point",
        export_name: "bffi_make_point",
        docs: &["Builds a point."],
        params: B1_XY_PARAMS,
        ret: TsType::Record("Point"),
        abi: HANDLE_ABI,
    },
    FunctionDef {
        js_name: "all_points",
        export_name: "bffi_all_points",
        docs: &["All the points."],
        params: &[],
        ret: TsType::RecordArray("Point"),
        abi: HANDLE_ABI,
    },
];

static B1: ModuleDef = ModuleDef {
    name: "b1",
    fns: B1_FNS,
    classes: &[],
    records: &[bffi::bffi_dts::RecordDef {
        js_name: "Point",
        docs: &["A point in 2D space."],
        fields: &[
            bffi::bffi_dts::RecordFieldDef {
                name: "x",
                docs: &["The x coordinate."],
                ty: TsType::Number,
            },
            bffi::bffi_dts::RecordFieldDef {
                name: "y",
                docs: &[],
                ty: TsType::Number,
            },
        ],
    }],
    enums: &[bffi::bffi_dts::EnumDef {
        js_name: "JobStatus",
        docs: &["The status of a job."],
        variants: &[
            bffi::bffi_dts::EnumVariantDef {
                name: "Idle",
                docs: &[],
            },
            bffi::bffi_dts::EnumVariantDef {
                name: "Running",
                docs: &[],
            },
            bffi::bffi_dts::EnumVariantDef {
                name: "Done",
                docs: &[],
            },
        ],
    }],
    errors: &[],
};

/// Normalizes CRLF line endings to LF, undoing any `core.autocrlf`
/// normalization `include_str!` picked up from the working tree.
fn normalize_lf(contents: &str) -> String {
    contents.replace("\r\n", "\n")
}

#[test]
fn golden_math_matches() {
    let expected = normalize_lf(include_str!("golden/math.d.ts"));
    assert_eq!(render(&MATH), expected);
}

#[test]
fn golden_kitchen_matches() {
    let expected = normalize_lf(include_str!("golden/kitchen.d.ts"));
    assert_eq!(render(&KITCHEN), expected);
}

#[test]
fn golden_empty_matches() {
    let expected = normalize_lf(include_str!("golden/empty.d.ts"));
    assert_eq!(render(&EMPTY), expected);
}

#[test]
fn golden_classes_match() {
    let expected = normalize_lf(include_str!("golden/classes.d.ts"));
    assert_eq!(render(&SHAPES), expected);
}

#[test]
fn golden_b1_matches() {
    let expected = normalize_lf(include_str!("golden/b1.d.ts"));
    assert_eq!(render(&B1), expected);
}

static STREAM_FNS: &[FunctionDef] = &[
    FunctionDef {
        js_name: "numbers",
        export_name: "bffi_numbers",
        docs: &["A numeric sequence."],
        params: &[],
        ret: TsType::StreamNumber,
        abi: HANDLE_ABI,
    },
    FunctionDef {
        js_name: "samples",
        export_name: "bffi_samples",
        docs: &["Record streams carry their composite item type."],
        params: &[],
        ret: TsType::StreamRecord("Sample"),
        abi: HANDLE_ABI,
    },
];

static STREAMS: ModuleDef = ModuleDef {
    name: "streams",
    fns: STREAM_FNS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};

#[test]
fn golden_streams_match() {
    let expected = normalize_lf(include_str!("golden/streams.d.ts"));
    assert_eq!(render(&STREAMS), expected);
}

#[test]
fn render_is_deterministic() {
    for module in [&MATH, &KITCHEN, &EMPTY, &SHAPES, &B1, &STREAMS] {
        let first = render(module);
        let second = render(module);
        assert_eq!(first, second, "re-rendering {module:?} must be identical");
    }
}

#[test]
fn golden_files_contain_no_carriage_returns() {
    for contents in [
        include_str!("golden/math.d.ts"),
        include_str!("golden/kitchen.d.ts"),
        include_str!("golden/empty.d.ts"),
        include_str!("golden/classes.d.ts"),
        include_str!("golden/b1.d.ts"),
    ] {
        assert!(!contents.contains('\r'), "golden file must be LF-only");
    }
}
