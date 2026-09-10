//! Integration tests for the canonical loader-JSON emitter
//! (`bffi::bffi_build::loader_json`): deterministic bytes, a committed
//! golden file covering the whole descriptor matrix, JSON escaping,
//! and the `write_to_file` materialization (parents created,
//! overwrites byte-identical).
//!
//! The fixture module below intentionally covers every `AbiType`
//! parameter shape, every out slot, buffer/task/handle returns,
//! nullability and a full class - the same matrix the `bffi` codegen
//! must parse (the emitter and the codegen agree on this file).

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use bffi::bffi_dts::{
    AbiOut, AbiPrim, AbiSig, AbiType, ClassDef, FieldDef, FunctionDef, MethodDef, ModuleDef,
    ParamDef, TsType,
};

const UNIT_ABI: AbiSig = AbiSig {
    params: &[],
    out: None,
};
const HANDLE_ABI: AbiSig = AbiSig {
    params: &[],
    out: Some(AbiOut::Handle),
};

static ADD_ABI: AbiSig = AbiSig {
    params: &[AbiType::U32, AbiType::U32],
    out: Some(AbiOut::Prim(AbiPrim::U32)),
};

/// The full-matrix fixture module.
fn fixture_module() -> ModuleDef {
    static FNS: &[FunctionDef] = &[
        FunctionDef {
            js_name: "add",
            export_name: "bffi_add",
            docs: &["Adds two numbers."],
            params: &[
                ParamDef {
                    name: "a",
                    ty: TsType::Number,
                },
                ParamDef {
                    name: "b",
                    ty: TsType::Number,
                },
            ],
            ret: TsType::Number,
            abi: ADD_ABI,
        },
        FunctionDef {
            js_name: "shout",
            export_name: "bffi_shout",
            docs: &["Shouts.", "Loudly."],
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
            js_name: "byte_sum",
            export_name: "bffi_byte_sum",
            docs: &[],
            params: &[ParamDef {
                name: "data",
                ty: TsType::Uint8Array,
            }],
            ret: TsType::Number,
            abi: AbiSig {
                params: &[AbiType::PtrLen],
                out: Some(AbiOut::Prim(AbiPrim::U32)),
            },
        },
        FunctionDef {
            js_name: "is_even",
            export_name: "bffi_is_even",
            docs: &[],
            params: &[ParamDef {
                name: "x",
                ty: TsType::BigInt,
            }],
            ret: TsType::Boolean,
            abi: AbiSig {
                params: &[AbiType::U64],
                out: Some(AbiOut::Prim(AbiPrim::Bool)),
            },
        },
        FunctionDef {
            js_name: "mirror_i64",
            export_name: "bffi_mirror_i64",
            docs: &[],
            params: &[ParamDef {
                name: "x",
                ty: TsType::BigInt,
            }],
            ret: TsType::BigInt,
            abi: AbiSig {
                params: &[AbiType::I64],
                out: Some(AbiOut::Prim(AbiPrim::I64)),
            },
        },
        FunctionDef {
            js_name: "mirror_u64",
            export_name: "bffi_mirror_u64",
            docs: &[],
            params: &[ParamDef {
                name: "x",
                ty: TsType::BigInt,
            }],
            ret: TsType::BigInt,
            abi: AbiSig {
                params: &[AbiType::U64],
                out: Some(AbiOut::Prim(AbiPrim::U64)),
            },
        },
        FunctionDef {
            js_name: "maybe_name",
            export_name: "bffi_maybe_name",
            docs: &[],
            params: &[],
            ret: TsType::NullableString,
            abi: HANDLE_ABI,
        },
        FunctionDef {
            js_name: "touch",
            export_name: "bffi_touch",
            docs: &[],
            params: &[ParamDef {
                name: "flag",
                ty: TsType::Boolean,
            }],
            ret: TsType::Void,
            abi: AbiSig {
                params: &[AbiType::Bool],
                out: None,
            },
        },
        FunctionDef {
            js_name: "compute",
            export_name: "bffi_compute",
            docs: &["Async compute."],
            params: &[ParamDef {
                name: "x",
                ty: TsType::Number,
            }],
            ret: TsType::PromiseNumber,
            abi: AbiSig {
                params: &[AbiType::U32],
                out: Some(AbiOut::Handle),
            },
        },
    ];
    static CLASSES: &[ClassDef] = &[ClassDef {
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
        methods: &[MethodDef {
            js_name: "increment",
            export_name: "bffi_counter_increment",
            docs: &["Adds one."],
            params: &[],
            ret: TsType::Number,
            abi: AbiSig {
                params: &[],
                out: Some(AbiOut::Prim(AbiPrim::U32)),
            },
        }],
    }];
    ModuleDef {
        name: "matrix",
        fns: FNS,
        classes: CLASSES,
        records: &[],
        enums: &[],
    }
}

#[test]
fn rendering_is_deterministic() {
    let module = fixture_module();
    assert_eq!(
        bffi::bffi_build::loader_json::to_json(&module),
        bffi::bffi_build::loader_json::to_json(&module),
        "re-emitting the same module must be byte-identical"
    );
}

#[test]
fn golden_matches_the_committed_file() {
    let rendered = bffi::bffi_build::loader_json::to_json(&fixture_module());
    assert!(!rendered.contains('\r'), "the emitter must be LF-only");
    assert!(rendered.ends_with("}\n"), "exactly one trailing newline");
    let golden = include_str!("loader_json/golden.json");
    assert_eq!(rendered, golden.replace("\r\n", "\n"), "golden drift");
}

/// Regenerates the committed golden (like `TRYBUILD=overwrite`):
///
/// ```sh
/// BFFI_LOADER_JSON_OVERWRITE=1 cargo test -p bffi --test build-loader_json write_golden
/// ```
///
/// Re-run the plain suite afterwards and review the diff by hand.
#[test]
fn write_golden() {
    let rendered = bffi::bffi_build::loader_json::to_json(&fixture_module());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/loader_json/golden.json");
    if std::env::var_os("BFFI_LOADER_JSON_OVERWRITE").is_some() {
        std::fs::create_dir_all(path.parent().expect("parent exists")).expect("mkdir");
        std::fs::write(&path, rendered).expect("write golden");
    }
    assert!(path.exists(), "the golden file is committed");
}

#[test]
fn golden_files_contain_no_carriage_returns() {
    assert!(!include_str!("loader_json/golden.json").contains('\r'));
}

#[test]
fn output_is_valid_json_with_every_matrix_shape() {
    let rendered = bffi::bffi_build::loader_json::to_json(&fixture_module());
    let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
    assert_eq!(parsed["bffi"].as_u64(), Some(1));
    assert_eq!(parsed["module"], "matrix");
    let functions = parsed["functions"].as_array().expect("functions array");
    assert_eq!(functions.len(), 9);
    let classes = parsed["classes"].as_array().expect("classes array");
    assert_eq!(classes.len(), 1);

    // The full AbiType parameter matrix appears in the fixture.
    let mut param_abis = std::collections::BTreeSet::new();
    for function in functions {
        for param in function["params"].as_array().expect("params") {
            param_abis.insert(param["abi"].as_str().expect("abi").to_owned());
        }
    }
    for shape in ["u32", "cstring", "ptr_len", "u64", "i64", "bool"] {
        assert!(
            param_abis.contains(shape),
            "the matrix must cover `{shape}`"
        );
    }

    // The return transports: prim widths, buffer, task, void, handle.
    let mut ret_abis = std::collections::BTreeSet::new();
    for function in functions {
        ret_abis.insert(function["ret"]["abi"].as_str().expect("abi").to_owned());
    }
    for transport in ["u32", "u64", "buffer", "task", "void"] {
        assert!(
            ret_abis.contains(transport),
            "the matrix must cover `{transport}`"
        );
    }
    let constructor = &classes[0]["constructor"];
    assert_eq!(constructor["ret"]["abi"], "handle");
    assert_eq!(constructor["out"], "handle");

    // The unit return omits the out key entirely.
    let touch = functions
        .iter()
        .find(|f| f["name"] == "touch")
        .expect("touch in the fixture");
    assert!(touch.get("out").is_none(), "unit returns have no out key");

    // Field getters carry export + out, no params.
    let field = &classes[0]["fields"][0];
    assert_eq!(field["export"], "bffi_counter_value_get");
    assert_eq!(field["out"], "u32");
}

#[test]
fn strings_survive_escaping() {
    static DOCS: &[&str] = &["quote \" backslash \\ newline \n cr \r tab \t control \u{1} ok"];
    static FNS: &[FunctionDef] = &[FunctionDef {
        js_name: "doc_ed",
        export_name: "bffi_doc_ed",
        docs: DOCS,
        params: &[],
        ret: TsType::Void,
        abi: UNIT_ABI,
    }];
    let module = ModuleDef {
        name: "esc",
        fns: FNS,
        classes: &[],
        records: &[],
        enums: &[],
    };
    let rendered = bffi::bffi_build::loader_json::to_json(&module);
    let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
    assert_eq!(
        parsed["functions"][0]["docs"][0],
        "quote \" backslash \\ newline \n cr \r tab \t control \u{1} ok",
        "every escaped character round-trips through the parser"
    );
}

#[test]
fn write_to_file_persists_the_bytes_verbatim() {
    let dir = std::env::temp_dir().join(format!("bffi-loader-json-{}", std::process::id()));
    let path = dir.join("nested/js/bffi.api.json");
    let module = fixture_module();
    bffi::bffi_build::loader_json::write_to_file(&module, &path).expect("write succeeds");
    let from_disk = std::fs::read_to_string(&path).expect("the written file exists");
    assert_eq!(bffi::bffi_build::loader_json::to_json(&module), from_disk);
    // A second write is byte-identical (overwrite, no drift).
    bffi::bffi_build::loader_json::write_to_file(&module, &path).expect("rewrite succeeds");
    assert_eq!(from_disk, std::fs::read_to_string(&path).expect("reread"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_to_file_surfaces_io_errors() {
    let dir = std::env::temp_dir().join(format!("bffi-loader-json-io-{}", std::process::id()));
    // A FILE where `create_dir_all` must build a directory: the
    // materializer surfaces the io error unchanged.
    let blocked = dir.join("blocked");
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(&blocked, b"not a directory").expect("create the blocking file");
    let target = blocked.join("nested/out.json");
    let result = bffi::bffi_build::loader_json::write_to_file(&fixture_module(), &target);
    assert!(result.is_err(), "a file in the parent chain must error");
    let _ = std::fs::remove_dir_all(&dir);
}
