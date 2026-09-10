//! Integration tests for the build-time `.d.ts` helper
//! (`bffi::bffi_build::dts::write_to_file`): the bytes on disk are exactly
//! the renderer's, missing parents are created, existing files are
//! overwritten, repeated writes are byte-identical, and io errors
//! surface unchanged.
#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use bffi::bffi_dts::{AbiOut, AbiPrim, AbiSig, AbiType, FunctionDef, ModuleDef, ParamDef, TsType};

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

static ADD_FNS: &[FunctionDef] = &[FunctionDef {
    js_name: "add",
    export_name: "bffi_add",
    docs: &["Adds two numbers."],
    params: ADD_PARAMS,
    ret: TsType::Number,
    abi: AbiSig {
        params: &[AbiType::U32, AbiType::U32],
        out: Some(AbiOut::Prim(AbiPrim::U32)),
    },
}];

static PING_FNS: &[FunctionDef] = &[FunctionDef {
    js_name: "ping",
    export_name: "bffi_ping",
    docs: &[],
    params: &[],
    ret: TsType::Void,
    abi: AbiSig {
        params: &[],
        out: None,
    },
}];

fn add_module() -> ModuleDef {
    ModuleDef {
        name: "test",
        fns: ADD_FNS,
        classes: &[],
        records: &[],
        enums: &[],
    }
}

fn ping_module() -> ModuleDef {
    ModuleDef {
        name: "test",
        fns: PING_FNS,
        classes: &[],
        records: &[],
        enums: &[],
    }
}

/// A per-test scratch directory under the OS temp dir (unique per
/// process and test name, so parallel tests never collide).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bffi-build-dts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn written_bytes_are_exactly_the_render_output() {
    let dir = scratch("verbatim");
    let path = dir.join("api.d.ts");

    bffi::bffi_build::dts::write_to_file(&add_module(), &path).expect("write succeeds");
    let on_disk = std::fs::read(&path).expect("the file exists");
    assert_eq!(
        on_disk,
        bffi::bffi_dts::render(&add_module()).as_bytes(),
        "the bytes on disk must be the rendered bytes, unmodified"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_parent_directories_are_created() {
    let dir = scratch("parents");
    let path = dir.join("deeply/nested/js/api.d.ts");
    assert!(!path.parent().map(Path::exists).unwrap_or(true));

    bffi::bffi_build::dts::write_to_file(&add_module(), &path)
        .expect("write_to_file creates the missing parents");
    assert!(path.is_file(), "the file must exist after the write");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_file_is_overwritten() {
    let dir = scratch("overwrite");
    let path = dir.join("api.d.ts");

    bffi::bffi_build::dts::write_to_file(&add_module(), &path).expect("first write succeeds");
    bffi::bffi_build::dts::write_to_file(&ping_module(), &path).expect("second write succeeds");
    let on_disk = std::fs::read_to_string(&path).expect("the file exists");
    assert_eq!(
        on_disk,
        bffi::bffi_dts::render(&ping_module()),
        "the second write must fully replace the first content"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repeated_writes_are_byte_identical() {
    let dir = scratch("determinism");
    let first = dir.join("first/api.d.ts");
    let second = dir.join("second/api.d.ts");

    bffi::bffi_build::dts::write_to_file(&add_module(), &first).expect("first write succeeds");
    bffi::bffi_build::dts::write_to_file(&add_module(), &second).expect("second write succeeds");
    let left = std::fs::read(&first).expect("first file exists");
    let right = std::fs::read(&second).expect("second file exists");
    assert_eq!(
        left, right,
        "writing the same module twice must be byte-identical"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn io_errors_surface_unchanged() {
    let dir = scratch("error");
    let as_directory = dir.join("api.d.ts");
    std::fs::create_dir_all(&as_directory).expect("the scratch directory is created");

    let result = bffi::bffi_build::dts::write_to_file(&add_module(), &as_directory);
    assert!(
        result.is_err(),
        "writing over a directory must surface an io error"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
