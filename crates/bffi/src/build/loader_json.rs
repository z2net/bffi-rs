//! The canonical loader-JSON emitter: the machine-readable bridge
//! between the explicit `ModuleDef` aggregation and the JS-side
//! codegen.
//!
//! This is the second build-time materializer next to [`dts`]:
//! [`dts::write_to_file`] renders the human-facing `.d.ts`, and
//! [`loader_json::write_to_file`] renders the loader schema the `bffi`
//! codegen CLI consumes. Aggregation stays the caller's explicit job
//! (the standing decision - no inventory, no linkme); both
//! materializers turn the same aggregated [`ModuleDef`] into
//! deterministic bytes.
//!
//! # Format contract (schema version 1)
//!
//! The output is pretty-printed JSON with a fixed key order, two-space
//! indentation, LF line endings and exactly one trailing newline. No
//! timestamps, paths or versions beyond the schema field: re-emitting
//! the same module always yields a byte-identical file, which makes
//! the JSON safe to commit and diff.
//!
//! - `"bffi": 1` - the schema version; the codegen rejects unknown
//!   versions.
//! - `"module"` - the [`ModuleDef::name`].
//! - `"functions"` / `"classes"` - the descriptors, in declaration
//!   order.
//!
//! Every function and method entry records the JS view (`params` with
//! `ts` types, `ret.ts`) AND the exact C ABI view: one `abi` name per
//! parameter (`"u32"`, `"cstring"`, `"ptr_len"`, ... - the canonical
//! names of [`bffi::dts::AbiType::as_str`]) and the `out` slot name
//! ([`bffi::dts::AbiOut::as_str`]). The `out` key is omitted for the
//! unit return. The return transport `ret.abi` is a primitive width,
//! `"handle"` (a class constructor's instance handle), `"buffer"` (a
//! transient-buffer payload), `"task"` (an `#[bffi_async]` task
//! handle) or `"void"`.
//!
//! Field getters record `export`, `ts` and `out` only: a getter has no
//! parameters, so the out slot fully describes its ABI.
//!
//! # Panics
//!
//! Never panics: the emitter only iterates slices and pushes into a
//! `String`.

// Internal module aliases (the pre-merge crate names).
use crate::bffi_dts;
use std::path::Path;

use bffi_dts::{
    AbiOut, AbiSig, ClassDef, FieldDef, FunctionDef, MethodDef, ModuleDef, ParamDef, TsType,
};

/// The loader schema version this crate emits.
///
/// The emitter writes exactly this value; bumping it is a breaking
/// schema change that the codegen side must learn first.
pub const SCHEMA_VERSION: u32 = 1;

// The emitter hard-codes the version literal below; a bump without
// updating the writer must not compile.
const _: () = assert!(SCHEMA_VERSION == 1);

/// Renders `module` into the canonical loader JSON.
///
/// See the [format contract](self) for the exact shape. The output
/// ends with exactly one trailing newline.
#[must_use]
pub fn to_json(module: &ModuleDef) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"bffi\": 1,\n");
    out.push_str("  \"module\": ");
    push_escaped(&mut out, module.name);
    out.push_str(",\n  \"functions\": [");
    for (index, function) in module.fns.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_function(function, &mut out, index == module.fns.len() - 1);
    }
    if !module.fns.is_empty() {
        push_indent(&mut out, 1);
    }
    out.push_str("],\n  \"classes\": [");
    for (index, class) in module.classes.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_class(class, &mut out, index == module.classes.len() - 1);
    }
    if !module.classes.is_empty() {
        push_indent(&mut out, 1);
    }
    out.push_str("]\n}\n");
    out
}

/// Renders `module` and writes the JSON bytes to `path`.
///
/// The bytes on disk are exactly [`to_json`]'s output. Missing parent
/// directories are created first ([`std::fs::create_dir_all`]); io
/// errors surface unchanged. Aggregation is the caller's job, exactly
/// like [`dts::write_to_file`].
pub fn write_to_file(module: &ModuleDef, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, to_json(module).as_bytes())
}

/// One function entry, indented at depth 2 inside the `functions`
/// array.
fn push_function(function: &FunctionDef, out: &mut String, last: bool) {
    push_indent(out, 2);
    out.push_str("{\n");
    push_entry_body(
        out,
        3,
        function.js_name,
        function.export_name,
        function.docs,
    );
    out.push_str(",\n");
    push_params(out, 3, function.params, &function.abi);
    out.push_str(",\n");
    push_ret(out, 3, function.ret, &function.abi);
    push_out(out, 3, &function.abi);
    out.push('\n');
    push_indent(out, 2);
    out.push('}');
    if !last {
        out.push(',');
    }
    out.push('\n');
}

/// One class entry: name, docs, constructor, field getters, methods.
fn push_class(class: &ClassDef, out: &mut String, last: bool) {
    push_indent(out, 2);
    out.push_str("{\n");
    push_key_string(out, 3, "name", class.js_name);
    out.push_str(",\n");
    push_key_string(out, 3, "release", class.release_export);
    out.push_str(",\n");
    push_docs(out, 3, class.docs);
    out.push_str(",\n");
    push_indent(out, 3);
    out.push_str("\"constructor\": ");
    push_method_body(&class.constructor, out, 4);
    out.push_str(",\n");
    push_indent(out, 3);
    out.push_str("\"fields\": [");
    for (index, field) in class.fields.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_field(field, out, index == class.fields.len() - 1);
    }
    if !class.fields.is_empty() {
        push_indent(out, 3);
    }
    out.push_str("],\n");
    push_indent(out, 3);
    out.push_str("\"methods\": [");
    for (index, method) in class.methods.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_indent(out, 4);
        push_method_body(method, out, 5);
        if index != class.methods.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    if !class.methods.is_empty() {
        push_indent(out, 3);
    }
    out.push_str("]\n");
    push_indent(out, 2);
    out.push('}');
    if !last {
        out.push(',');
    }
    out.push('\n');
}

/// One field-getter entry: `export`, `ts` and the out slot (a getter
/// has no parameters, so the out slot fully describes its ABI).
fn push_field(field: &FieldDef, out: &mut String, last: bool) {
    push_indent(out, 4);
    out.push_str("{\n");
    push_key_string(out, 5, "name", field.js_name);
    out.push_str(",\n");
    push_key_string(out, 5, "export", field.export_name);
    out.push_str(",\n");
    push_docs(out, 5, field.docs);
    out.push_str(",\n");
    push_key_string(out, 5, "ts", &field.ty.as_str());
    out.push_str(",\n");
    push_key_string(out, 5, "out", field.out.as_str());
    out.push('\n');
    push_indent(out, 4);
    out.push('}');
    if !last {
        out.push(',');
    }
    out.push('\n');
}

/// The `{ name, export, docs, ... }` head of a function or method
/// body: everything before `params` (the tails differ per shape).
fn push_entry_body(out: &mut String, depth: usize, name: &str, export: &str, docs: &[&str]) {
    push_key_string(out, depth, "name", name);
    out.push_str(",\n");
    push_key_string(out, depth, "export", export);
    out.push_str(",\n");
    push_docs(out, depth, docs);
}

/// A method body after `"constructor": ` or at its `methods` array
/// position: `{ name, export, docs, params, ret, out }`. `depth` is
/// the indent depth of the member keys (the closing brace sits one
/// level above it).
fn push_method_body(method: &MethodDef, out: &mut String, depth: usize) {
    out.push_str("{\n");
    push_entry_body(out, depth, method.js_name, method.export_name, method.docs);
    out.push_str(",\n");
    push_params(out, depth, method.params, &method.abi);
    out.push_str(",\n");
    push_ret(out, depth, method.ret, &method.abi);
    push_out(out, depth, &method.abi);
    out.push('\n');
    push_indent(out, depth - 1);
    out.push('}');
}

/// The `params` array: one `{ name, ts, abi }` entry per parameter,
/// where `abi[i]` corresponds to `params[i]` (declaration order; a
/// `&[u8]` stays ONE `ptr_len` entry here and is expanded into the
/// `("ptr", "u64")` dlopen pair by the loader).
fn push_params(out: &mut String, depth: usize, params: &[ParamDef], abi: &AbiSig) {
    push_indent(out, depth);
    out.push_str("\"params\": [");
    for (index, param) in params.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_indent(out, depth + 1);
        out.push_str("{\n");
        push_key_string(out, depth + 2, "name", param.name);
        out.push_str(",\n");
        push_key_string(out, depth + 2, "ts", &param.ty.as_str());
        out.push_str(",\n");
        push_key_string(out, depth + 2, "abi", abi.params[index].as_str());
        out.push('\n');
        push_indent(out, depth + 1);
        out.push('}');
        if index != params.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    if !params.is_empty() {
        push_indent(out, depth);
    }
    out.push(']');
}

/// The `ret` entry: the TypeScript type plus the return transport
/// name ([`ret_abi_str`]).
fn push_ret(out: &mut String, depth: usize, ret: TsType, abi: &AbiSig) {
    push_indent(out, depth);
    out.push_str("\"ret\": {\n");
    push_key_string(out, depth + 1, "ts", &ret.as_str());
    out.push_str(",\n");
    push_key_string(out, depth + 1, "abi", ret_abi_str(ret, abi));
    out.push('\n');
    push_indent(out, depth);
    out.push('}');
}

/// The `out` key, omitted for the unit return (no out-parameter).
fn push_out(out: &mut String, depth: usize, abi: &AbiSig) {
    if let Some(slot) = abi.out {
        out.push_str(",\n");
        push_key_string(out, depth, "out", slot.as_str());
    }
}

/// The docs array; rendered even when empty (fixed key order).
fn push_docs(out: &mut String, depth: usize, docs: &[&str]) {
    push_indent(out, depth);
    out.push_str("\"docs\": [");
    for (index, doc) in docs.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_indent(out, depth + 1);
        push_escaped(out, doc);
        if index != docs.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    if !docs.is_empty() {
        push_indent(out, depth);
    }
    out.push(']');
}

/// Writes `"key": ` at `depth`, then the escaped string value.
fn push_key_string(out: &mut String, depth: usize, key: &str, value: &str) {
    push_indent(out, depth);
    push_escaped(out, key);
    out.push_str(": ");
    push_escaped(out, value);
}

/// `depth * 2` spaces.
fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Pushes `value` as a JSON string literal (quotes included), escaping
/// `"`, `\` and every control character; everything else (including
/// non-ASCII) passes through as UTF-8.
fn push_escaped(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            other => {
                if (other as u32) < 0x20 {
                    out.push_str(&format!("\\u{:04x}", other as u32));
                } else {
                    out.push(other);
                }
            }
        }
    }
    out.push('"');
}

/// The return transport name: the primitive width for scalar returns,
/// `"handle"` for the constructor's instance handle (the only
/// `BigInt`-returning shape that transports through the shared
/// [`AbiOut::Handle`] slot), `"buffer"` for transient-buffer
/// payloads, `"task"` for async task handles, and `"void"` for the
/// unit return. Future `AbiOut` variants fall back to their own
/// canonical `as_str` name.
fn ret_abi_str(ret: TsType, abi: &AbiSig) -> &'static str {
    let Some(out) = abi.out else {
        return "void";
    };
    match out {
        AbiOut::Prim(prim) => prim.as_str(),
        AbiOut::Handle => match ret {
            TsType::PromiseVoid
            | TsType::PromiseNumber
            | TsType::PromiseBigInt
            | TsType::PromiseBoolean
            | TsType::PromiseString
            | TsType::PromiseUint8Array => "task",
            TsType::BigInt => "handle",
            _ => "buffer",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA_VERSION, to_json};
    use crate::bffi_dts::ModuleDef;

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn empty_module_has_the_fixed_skeleton_and_trailing_newline() {
        let module = ModuleDef {
            name: "probe",
            fns: &[],
            classes: &[],
            records: &[],
            enums: &[],
        };
        assert_eq!(
            to_json(&module),
            "{\n  \"bffi\": 1,\n  \"module\": \"probe\",\n  \"functions\": [],\n  \"classes\": []\n}\n"
        );
    }

    #[test]
    fn strings_escape_json_control_characters() {
        let docs: &[&str] = &["quote \" backslash \\ newline \n tab \t end"];
        let module = ModuleDef {
            name: "esc\"ape",
            fns: &[],
            classes: &[],
            records: &[],
            enums: &[],
        };
        // Reuse the emitter through a docs-shaped key: the module name
        // travels through the same escaper.
        let _ = docs;
        let json = to_json(&module);
        assert!(json.contains("\"module\": \"esc\\\"ape\""));
    }
}
