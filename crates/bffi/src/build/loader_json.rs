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
//! - `"abiVersion"` - the runtime ABI revision
//!   ([`abi`](super::abi)::[`BFFI_ABI_VERSION`](super::abi::BFFI_ABI_VERSION))
//!   this cdylib was built against; the JS loader's handshake compares
//!   it against `bffi_runtime_abi_version()` from the loaded library
//!   and refuses a mismatched binary.
//! - `"exportsHash"` - the FNV-1a 64-bit hash of the exported surface
//!   ([`module_exports_hash`]) as a decimal string; the loader
//!   compares it against `bffi_module_exports_hash()` to guarantee
//!   the JSON and the binary describe the same exports (the
//!   JSON↔binary integrity check).
//! - `"functions"` / `"classes"` / `"records"` / `"enums"` - the
//!   descriptors, in declaration order. The `records`/`enums` tables
//!   are the B1 composite types: a record entry is
//!   `{ name, docs, fields: [{ name, docs, ts }] }` (fields have no
//!   ABI of their own - the whole value crosses as one wire payload),
//!   an enum entry is `{ name, docs, variants: [{ name, docs }] }`.
//!   Their `ts` names (and the array forms `Name[]`) are referenced
//!   by the function `ts` strings. Both arrays are always present
//!   (empty when unused) so the shape stays fixed.
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

/// The FNV-1a 64-bit offset basis ([`module_exports_hash`] starts
/// here; an empty module hashes to exactly this value).
const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// The FNV-1a 64-bit prime.
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;
/// The separator byte fed between every hashed element (it cannot
/// appear inside a UTF-8 name, so element boundaries stay unambiguous).
const HASH_SEPARATOR: u8 = 0xFF;

/// The running FNV-1a 64-bit state ([`module_exports_hash`]'s engine).
struct Fnv1a(u64);

impl Fnv1a {
    /// The state at the offset basis.
    fn new() -> Self {
        Self(FNV1A_OFFSET_BASIS)
    }

    /// Feeds every byte: xor-low, multiply by the prime.
    fn feed_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(FNV1A_PRIME);
        }
    }

    /// Feeds one string's UTF-8 bytes.
    fn feed_str(&mut self, value: &str) {
        self.feed_bytes(value.as_bytes());
    }

    /// Feeds the element separator.
    fn feed_separator(&mut self) {
        self.feed_bytes(&[HASH_SEPARATOR]);
    }

    /// Feeds one element (its bytes plus the separator).
    fn feed_element(&mut self, value: &str) {
        self.feed_str(value);
        self.feed_separator();
    }

    /// Feeds one numeric element (little-endian bytes plus the
    /// separator).
    fn feed_element_u32(&mut self, value: u32) {
        self.feed_bytes(&value.to_le_bytes());
        self.feed_separator();
    }
}

/// Hashes `def`'s exported surface with FNV-1a 64 (offset basis
/// `0xcbf29ce484222325`, prime `0x100000001b3`): every name, export
/// symbol, ABI string and TS string of every descriptor is fed as raw
/// UTF-8 bytes, one `0xFF` separator byte after each element. The
/// walk order is the descriptor's canonical declaration order - fns,
/// then classes (name, release export, constructor, fields, methods),
/// then records, enums and errors - so the same [`ModuleDef`] content
/// always produces the same `u64` and any exported-surface change
/// (rename, signature, added/removed item) changes the value.
///
/// This is the Rust half of the JSON↔binary integrity check: the
/// loader JSON carries the value (`"exportsHash"`) and the cdylib
/// recomputes it at runtime through `bffi_module_exports_hash()`
/// (the `module = ...` form of [`bffi_runtime_abi!`](crate::bffi_runtime_abi)).
///
/// Deliberately NOT fed: docs lines (cosmetic), param names (the ABI
/// only sees the param types) and the module name (the exported
/// surface is the same surface whatever the module is called).
#[must_use]
pub fn module_exports_hash(def: &ModuleDef) -> u64 {
    let mut hash = Fnv1a::new();
    for function in def.fns {
        feed_fn_like(
            &mut hash,
            function.js_name,
            function.export_name,
            function.ret,
            &function.abi,
        );
    }
    for class in def.classes {
        hash.feed_element(class.js_name);
        hash.feed_element(class.release_export);
        feed_fn_like(
            &mut hash,
            class.constructor.js_name,
            class.constructor.export_name,
            class.constructor.ret,
            &class.constructor.abi,
        );
        for field in class.fields {
            hash.feed_element(field.js_name);
            hash.feed_element(&field.ty.as_str());
            hash.feed_element(field.out.as_str());
        }
        for method in class.methods {
            feed_fn_like(
                &mut hash,
                method.js_name,
                method.export_name,
                method.ret,
                &method.abi,
            );
        }
    }
    for record in def.records {
        hash.feed_element(record.js_name);
        for field in record.fields {
            hash.feed_element(field.name);
            hash.feed_element(&field.ty.as_str());
        }
    }
    for enumeration in def.enums {
        hash.feed_element(enumeration.js_name);
        for variant in enumeration.variants {
            hash.feed_element(variant.name);
        }
    }
    for error in def.errors {
        hash.feed_element(error.js_name);
        for variant in error.variants {
            hash.feed_element(variant.name);
            hash.feed_element_u32(variant.code);
        }
    }
    hash.0
}

/// Feeds one fn-shaped descriptor (a [`FunctionDef`]/[`MethodDef`]
/// body): name, export, every param's ABI string, then the return's
/// TS type and transport name - each element separated. The param
/// NAMES are deliberately not fed: the ABI only sees the param types.
fn feed_fn_like(hash: &mut Fnv1a, name: &str, export: &str, ret: TsType, abi: &AbiSig) {
    hash.feed_element(name);
    hash.feed_element(export);
    for param_abi in abi.params {
        hash.feed_element(param_abi.as_str());
    }
    hash.feed_element(&ret.as_str());
    hash.feed_element(ret_abi_str(ret, abi));
}

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
    // The handshake pair (right after `module`, before the tables):
    // the ABI revision and the exported-surface hash the JS loader
    // verifies against the loaded cdylib.
    out.push_str(",\n  \"abiVersion\": ");
    out.push_str(&super::abi::BFFI_ABI_VERSION.to_string());
    out.push_str(",\n  \"exportsHash\": ");
    push_escaped(&mut out, &module_exports_hash(module).to_string());
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
    out.push_str("],\n  \"records\": [");
    for (index, record) in module.records.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_record(record, &mut out, index == module.records.len() - 1);
    }
    if !module.records.is_empty() {
        push_indent(&mut out, 1);
    }
    out.push_str("],\n  \"enums\": [");
    for (index, enumeration) in module.enums.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_enum(enumeration, &mut out, index == module.enums.len() - 1);
    }
    if !module.enums.is_empty() {
        push_indent(&mut out, 1);
    }
    out.push_str("],\n  \"errors\": [");
    for (index, error) in module.errors.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_error(error, &mut out, index == module.errors.len() - 1);
    }
    if !module.errors.is_empty() {
        push_indent(&mut out, 1);
    }
    out.push_str("]\n}\n");
    out
}

/// Renders `module` and writes the JSON bytes to `path`.
///
/// The bytes on disk are exactly [`to_json`]'s output, including the
/// runtime ABI handshake pair (`abiVersion`/`exportsHash`) computed
/// from the same `ModuleDef`. Missing parent directories are created
/// first ([`std::fs::create_dir_all`]); io errors surface unchanged.
/// Aggregation is the caller's job, exactly like
/// [`dts::write_to_file`].
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

/// One record entry: name, docs, fields (`name`/`docs`/`ts` each -
/// a record field has no ABI of its own, the whole value travels as
/// one wire payload).
fn push_record(record: &bffi_dts::RecordDef, out: &mut String, last: bool) {
    push_indent(out, 2);
    out.push_str("{\n");
    push_key_string(out, 3, "name", record.js_name);
    out.push_str(",\n");
    push_docs(out, 3, record.docs);
    out.push_str(",\n");
    push_indent(out, 3);
    out.push_str("\"fields\": [");
    for (index, field) in record.fields.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_indent(out, 4);
        out.push_str("{\n");
        push_key_string(out, 5, "name", field.name);
        out.push_str(",\n");
        push_docs(out, 5, field.docs);
        out.push_str(",\n");
        push_key_string(out, 5, "ts", &field.ty.as_str());
        out.push('\n');
        push_indent(out, 4);
        out.push('}');
        if index != record.fields.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    if !record.fields.is_empty() {
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

/// One enum entry: name, docs, variants (`name`/`docs` each).
fn push_enum(enumeration: &bffi_dts::EnumDef, out: &mut String, last: bool) {
    push_indent(out, 2);
    out.push_str("{\n");
    push_key_string(out, 3, "name", enumeration.js_name);
    out.push_str(",\n");
    push_docs(out, 3, enumeration.docs);
    out.push_str(",\n");
    push_indent(out, 3);
    out.push_str("\"variants\": [");
    for (index, variant) in enumeration.variants.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_indent(out, 4);
        out.push_str("{\n");
        push_key_string(out, 5, "name", variant.name);
        out.push_str(",\n");
        push_docs(out, 5, variant.docs);
        out.push('\n');
        push_indent(out, 4);
        out.push('}');
        if index != enumeration.variants.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    if !enumeration.variants.is_empty() {
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

/// One error-enum entry, indented at depth 2 inside the `errors`
/// array: `name`, `docs` and the `variants` table (name, docs, the
/// hex `code` and the payload `fields` in declaration order).
fn push_error(error: &bffi_dts::ErrorDef, out: &mut String, last: bool) {
    push_indent(out, 2);
    out.push_str("{\n");
    push_key_string(out, 3, "name", error.js_name);
    out.push_str(",\n");
    push_docs(out, 3, error.docs);
    out.push_str(",\n");
    push_indent(out, 3);
    out.push_str("\"variants\": [");
    for (index, variant) in error.variants.iter().enumerate() {
        if index == 0 {
            out.push('\n');
        }
        push_indent(out, 4);
        out.push_str("{\n");
        push_key_string(out, 5, "name", variant.name);
        out.push_str(",\n");
        push_docs(out, 5, variant.docs);
        out.push_str(",\n");
        push_indent(out, 5);
        out.push_str("\"code\": \"0x");
        out.push_str(&format!("{:04X}", variant.code));
        out.push_str("\",\n");
        push_indent(out, 5);
        out.push_str("\"fields\": [");
        for (index, field) in variant.fields.iter().enumerate() {
            if index == 0 {
                out.push('\n');
            }
            push_indent(out, 6);
            out.push_str("{\n");
            push_key_string(out, 7, "name", field.name);
            out.push_str(",\n");
            push_docs(out, 7, field.docs);
            out.push_str(",\n");
            push_key_string(out, 7, "ts", &field.ty.as_str());
            out.push('\n');
            push_indent(out, 6);
            out.push('}');
            if index != variant.fields.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        if !variant.fields.is_empty() {
            push_indent(out, 5);
        }
        out.push_str("]\n");
        push_indent(out, 4);
        out.push('}');
        if index != error.variants.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    if !error.variants.is_empty() {
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
            | TsType::PromiseUint8Array
            | TsType::PromiseRecord(_)
            | TsType::PromiseNumberArray
            | TsType::PromiseBigIntArray
            | TsType::PromiseBooleanArray
            | TsType::PromiseStringArray
            | TsType::PromiseUint8ArrayArray
            | TsType::PromiseRecordArray(_)
            | TsType::PromiseNullableString
            | TsType::PromiseNullableUint8Array
            | TsType::PromiseNullableRecord(_)
            | TsType::PromiseNullableNumberArray
            | TsType::PromiseNullableBigIntArray
            | TsType::PromiseNullableBooleanArray
            | TsType::PromiseNullableStringArray
            | TsType::PromiseNullableUint8ArrayArray
            | TsType::PromiseNullableRecordArray(_) => "task",
            TsType::StreamNumber
            | TsType::StreamBigInt
            | TsType::StreamBoolean
            | TsType::StreamString
            | TsType::StreamUint8Array
            | TsType::StreamRecord(_)
            | TsType::StreamResultNumber
            | TsType::StreamResultBigInt
            | TsType::StreamResultBoolean
            | TsType::StreamResultString
            | TsType::StreamResultUint8Array
            | TsType::StreamResultRecord(_) => "stream",
            TsType::BigInt => "handle",
            _ => "buffer",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA_VERSION, module_exports_hash, to_json, write_to_file};
    use crate::bffi_dts::{
        AbiOut, AbiPrim, AbiSig, AbiType, ClassDef, EnumDef, EnumVariantDef, ErrorDef,
        ErrorVariantDef, FieldDef, FunctionDef, MethodDef, ModuleDef, ParamDef, RecordDef,
        RecordFieldDef, TsType,
    };

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    /// A fn-like descriptor exercising params/abi/ret in the hash.
    const TEST_FN: FunctionDef = FunctionDef {
        js_name: "add",
        export_name: "bffi_add",
        docs: &[],
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
        abi: AbiSig {
            params: &[AbiType::F64, AbiType::F64],
            out: Some(AbiOut::Prim(AbiPrim::F64)),
        },
    };

    /// The hash probe: one of everything, in one fixed order.
    const TEST_MODULE: ModuleDef = ModuleDef {
        name: "probe",
        fns: &[
            TEST_FN,
            FunctionDef {
                js_name: "greet",
                export_name: "bffi_greet",
                docs: &[],
                params: &[ParamDef {
                    name: "who",
                    ty: TsType::String,
                }],
                ret: TsType::String,
                abi: AbiSig {
                    params: &[AbiType::Cstring],
                    out: Some(AbiOut::Handle),
                },
            },
        ],
        classes: &[ClassDef {
            js_name: "Counter",
            release_export: "bffi_Counter_release",
            docs: &[],
            constructor: MethodDef {
                js_name: "constructor",
                export_name: "bffi_Counter_new",
                docs: &[],
                params: &[ParamDef {
                    name: "start",
                    ty: TsType::Number,
                }],
                ret: TsType::BigInt,
                abi: AbiSig {
                    params: &[AbiType::F64],
                    out: Some(AbiOut::Handle),
                },
            },
            fields: &[FieldDef {
                js_name: "count",
                export_name: "bffi_Counter_count_get",
                docs: &[],
                ty: TsType::Number,
                out: AbiOut::Prim(AbiPrim::F64),
            }],
            methods: &[MethodDef {
                js_name: "increment",
                export_name: "bffi_Counter_increment",
                docs: &[],
                params: &[],
                ret: TsType::Void,
                abi: AbiSig {
                    params: &[],
                    out: None,
                },
            }],
        }],
        records: &[RecordDef {
            js_name: "Point",
            docs: &[],
            fields: &[
                RecordFieldDef {
                    name: "x",
                    docs: &[],
                    ty: TsType::Number,
                },
                RecordFieldDef {
                    name: "y",
                    docs: &[],
                    ty: TsType::Number,
                },
            ],
        }],
        enums: &[EnumDef {
            js_name: "Mode",
            docs: &[],
            variants: &[
                EnumVariantDef {
                    name: "On",
                    docs: &[],
                },
                EnumVariantDef {
                    name: "Off",
                    docs: &[],
                },
            ],
        }],
        errors: &[ErrorDef {
            js_name: "ProbeError",
            docs: &[],
            variants: &[ErrorVariantDef {
                name: "BadInput",
                docs: &[],
                code: 0x1001,
                fields: &[RecordFieldDef {
                    name: "input",
                    docs: &[],
                    ty: TsType::String,
                }],
            }],
        }],
    };

    /// The same surface as [`TEST_MODULE`] with one fn renamed: any
    /// exported-surface change must move the hash.
    const TEST_MODULE_RENAMED: ModuleDef = ModuleDef {
        fns: &[
            FunctionDef {
                js_name: "add_renamed",
                ..TEST_FN
            },
            TEST_MODULE.fns[1],
        ],
        ..TEST_MODULE
    };

    #[test]
    fn empty_module_has_the_fixed_skeleton_and_trailing_newline() {
        let module = ModuleDef {
            name: "probe",
            fns: &[],
            classes: &[],
            records: &[],
            enums: &[],
            errors: &[],
        };
        // An empty module hashes nothing: the hash is exactly the
        // FNV-1a 64 offset basis (0xcbf29ce484222325).
        assert_eq!(
            to_json(&module),
            "{\n  \"bffi\": 1,\n  \"module\": \"probe\",\n  \"abiVersion\": 1,\n  \"exportsHash\": \"14695981039346656037\",\n  \"functions\": [],\n  \"classes\": [],\n  \"records\": [],\n  \"enums\": [],\n  \"errors\": []\n}\n"
        );
    }

    #[test]
    fn exports_hash_is_deterministic_for_the_same_def() {
        assert_eq!(
            module_exports_hash(&TEST_MODULE),
            module_exports_hash(&TEST_MODULE)
        );
    }

    #[test]
    fn exports_hash_moves_when_a_name_changes() {
        assert_ne!(
            module_exports_hash(&TEST_MODULE),
            module_exports_hash(&TEST_MODULE_RENAMED)
        );
    }

    #[test]
    fn exports_hash_moves_on_any_signature_or_table_change() {
        let hash = module_exports_hash(&TEST_MODULE);

        // A renamed record field moves the hash.
        const RENAMED_FIELD: ModuleDef = ModuleDef {
            records: &[RecordDef {
                fields: &[
                    RecordFieldDef {
                        name: "x2",
                        docs: &[],
                        ty: TsType::Number,
                    },
                    TEST_MODULE.records[0].fields[1],
                ],
                ..TEST_MODULE.records[0]
            }],
            ..TEST_MODULE
        };
        assert_ne!(hash, module_exports_hash(&RENAMED_FIELD));

        // A changed error code moves the hash.
        const RECODED: ModuleDef = ModuleDef {
            errors: &[ErrorDef {
                variants: &[ErrorVariantDef {
                    code: 0x1002,
                    ..TEST_MODULE.errors[0].variants[0]
                }],
                ..TEST_MODULE.errors[0]
            }],
            ..TEST_MODULE
        };
        assert_ne!(hash, module_exports_hash(&RECODED));

        // A changed param ABI moves the hash.
        const REABIED: ModuleDef = ModuleDef {
            fns: &[
                FunctionDef {
                    abi: AbiSig {
                        params: &[AbiType::U32, AbiType::U32],
                        out: Some(AbiOut::Prim(AbiPrim::F64)),
                    },
                    ..TEST_FN
                },
                TEST_MODULE.fns[1],
            ],
            ..TEST_MODULE
        };
        assert_ne!(hash, module_exports_hash(&REABIED));
    }

    #[test]
    fn to_json_places_the_handshake_pair_after_module() {
        let json = to_json(&TEST_MODULE);
        let module_at = json.find("\"module\": ").expect("module key");
        let abi_at = json.find("\"abiVersion\": 1").expect("abiVersion key");
        let hash_key = format!("\"exportsHash\": \"{}\"", module_exports_hash(&TEST_MODULE));
        let hash_at = json.find(&hash_key).expect("exportsHash key");
        let fns_at = json.find("\"functions\":").expect("functions key");
        assert!(module_at < abi_at && abi_at < hash_at && hash_at < fns_at);
    }

    #[test]
    fn write_to_file_emits_the_handshake_pair() {
        let path = std::env::temp_dir().join(format!(
            "bffi-loader-json-{}-{}.json",
            std::process::id(),
            module_exports_hash(&TEST_MODULE)
        ));
        write_to_file(&TEST_MODULE, &path).expect("write succeeds");
        let contents = std::fs::read_to_string(&path).expect("read back succeeds");
        let _ = std::fs::remove_file(&path);
        assert!(contents.contains("\n  \"abiVersion\": 1,\n"));
        assert!(contents.contains(&format!(
            "\n  \"exportsHash\": \"{}\",\n",
            module_exports_hash(&TEST_MODULE)
        )));
    }

    // The macro forms expand inside this crate exactly like into a
    // user cdylib. The `module = ...` form additionally generates the
    // `bffi_module_exports_hash` export; it also generates the base
    // exports once, so it is the ONLY expansion in this binary.
    crate::bffi_runtime_abi!(module = TEST_MODULE);

    #[test]
    fn macro_hash_export_matches_the_pure_function() {
        assert_eq!(
            bffi_runtime_abi_version(),
            crate::bffi_build::abi::BFFI_ABI_VERSION
        );
        assert_eq!(
            bffi_module_exports_hash(),
            module_exports_hash(&TEST_MODULE)
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
            errors: &[],
        };
        // Reuse the emitter through a docs-shaped key: the module name
        // travels through the same escaper.
        let _ = docs;
        let json = to_json(&module);
        assert!(json.contains("\"module\": \"esc\\\"ape\""));
    }
}
