/**
 * The loader-schema types (version 1, emitted by
 * `bffi_build::loader_json`) and the dlopen declaration builder that
 * turns the ABI view back into `bun:ffi` shapes.
 *
 * Canonical ABI names vs `bun:ffi` spellings differ in exactly two
 * places: `bool` crosses dlopen as `"u8"` (JS coerces `0`/`1`), and a
 * `ptr_len` parameter expands into the `("ptr", "u64")` pair.
 */

/** Schema version this loader accepts. */
export const SCHEMA_VERSION = 1;

/** The bun:ffi declaration spelling: the FFIType enum or one of its
 * runtime string names (`"u32"`, `"cstring"`, ...). */
export type FfiType = import("bun:ffi").FFITypeOrString;

/** The canonical ABI name of one parameter entry. */
export type AbiName =
  | "i8"
  | "i16"
  | "i32"
  | "u8"
  | "u16"
  | "u32"
  | "f32"
  | "f64"
  | "i64"
  | "u64"
  | "bool"
  | "cstring"
  | "ptr_len";

/** The out-slot name: a primitive width or the shared handle slot. */
export type OutName =
  | AbiName
  | "handle";

/** The return transport: a primitive width, a transient-buffer
 * handle, an async task handle, a stream handle, or the unit
 * return. */
export type RetAbiName =
  | OutName
  | "buffer"
  | "stream"
  | "task"
  | "void";

/** The TypeScript type name as written by `bffi-dts`. Named record/
 * enum references (`Sample`) and their arrays (`Sample[]`) resolve
 * against the module's `records`/`enums` tables; the flat array
 * forms (`number[]`, ...) are part of the B1 matrix. */
export type TsName =
  | "number"
  | "bigint"
  | "boolean"
  | "string"
  | "Uint8Array"
  | "string | null"
  | "Uint8Array | null"
  | "void"
  | "number[]"
  | "bigint[]"
  | "boolean[]"
  | "string[]"
  | `Promise<${string}>`
  | (string & {});

/** Every valid `abi` parameter name (the codegen/CLI validator
 * consumes this set). */
export const ABI_NAMES: ReadonlySet<string> = new Set([
  "i8",
  "i16",
  "i32",
  "u8",
  "u16",
  "u32",
  "f32",
  "f64",
  "i64",
  "u64",
  "bool",
  "cstring",
  "ptr_len",
]);

const OUT_NAMES_MUTABLE = new Set(ABI_NAMES);
OUT_NAMES_MUTABLE.add("handle");

/** Every valid out-slot name (primitive widths + the handle slot). */
export const OUT_NAMES: ReadonlySet<string> = OUT_NAMES_MUTABLE;

const RET_ABI_NAMES_MUTABLE = new Set(OUT_NAMES_MUTABLE);
RET_ABI_NAMES_MUTABLE.add("buffer");
RET_ABI_NAMES_MUTABLE.add("stream");
RET_ABI_NAMES_MUTABLE.add("task");
RET_ABI_NAMES_MUTABLE.add("void");

/** Every valid return-transport name. */
export const RET_ABI_NAMES: ReadonlySet<string> = RET_ABI_NAMES_MUTABLE;

/** The TypeScript type names the `bffi-dts` renderer emits. Named
 * record/enum references (and their `[]` forms) validate as TS
 * identifiers against the module tables instead of this closed set. */
export const TS_NAMES: ReadonlySet<string> = new Set([
  "number",
  "bigint",
  "boolean",
  "string",
  "Uint8Array",
  "string | null",
  "Uint8Array | null",
  "void",
  "number[]",
  "bigint[]",
  "boolean[]",
  "string[]",
  "Promise<void>",
  "Promise<number>",
  "Promise<bigint>",
  "Promise<boolean>",
  "Promise<string>",
  "Promise<Uint8Array>",
]);

/** Whether `ts` names a module composite (a record/enum table entry,
 * optionally as an array form). */
export function isNamedTs(
  ts: string,
  json: Pick<ModuleJson, "records" | "enums">,
): boolean {
  const name = ts.endsWith("[]") ? ts.slice(0, -2) : ts;
  if (!/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name)) {
    return false;
  }
  return (
    (json.records ?? []).some((record) => record.name === name)
    || (json.enums ?? []).some((enumeration) => enumeration.name === name)
  );
}

export interface ParamJson {
  name: string;
  ts: TsName;
  abi: AbiName;
}

export interface RetJson {
  ts: TsName;
  abi: RetAbiName;
}

export interface FunctionJson {
  name: string;
  export: string;
  docs: string[];
  params: ParamJson[];
  ret: RetJson;
  /** The out slot; absent for the unit return. */
  out?: OutName;
}

export interface FieldJson {
  name: string;
  export: string;
  docs: string[];
  ts: TsName;
  out: OutName;
}

export interface ClassJson {
  name: string;
  /** The generated release shim: `bffi_<name>_release`. */
  release: string;
  docs: string[];
  constructor: FunctionJson;
  fields: FieldJson[];
  methods: FunctionJson[];
}

/** One record field: its JS name, docs and TypeScript type (the
 * whole value crosses as one wire payload - fields carry no ABI of
 * their own). */
export interface RecordFieldJson {
  name: string;
  docs: string[];
  ts: TsName;
}

/** A `#[derive(BffiRecord)]` type of the module. */
export interface RecordJson {
  name: string;
  docs: string[];
  fields: RecordFieldJson[];
}

/** One unit-enum variant (wire-encoded as its name). */
export interface EnumVariantJson {
  name: string;
  docs: string[];
}

/** A `#[derive(BffiEnum)]` type of the module. */
export interface EnumJson {
  name: string;
  docs: string[];
  variants: EnumVariantJson[];
}

export interface ModuleJson {
  bffi: number;
  module: string;
  functions: FunctionJson[];
  classes: ClassJson[];
  /** The B1 record types (always present in freshly emitted JSON;
   * older JSON without the key reads as empty). */
  records?: RecordJson[];
  /** The B1 enum types. */
  enums?: EnumJson[];
}

/** Validates the schema header: unknown versions are rejected. */
export function assertSchema(json: ModuleJson): void {
  if (json.bffi !== SCHEMA_VERSION) {
    throw new Error(
      `unsupported bffi loader schema: ${String(json.bffi)} (expected ${SCHEMA_VERSION})`,
    );
  }
}

/** The `bun:ffi` argument spelling of one ABI parameter. The FFIType
 * enum members carry these literal names, so the literals type-check
 * contextually. */
function ffiArg(abi: AbiName): FfiType[] {
  switch (abi) {
    case "i8":
      return ["i8"];
    case "i16":
      return ["i16"];
    case "i32":
      return ["i32"];
    case "u8":
      return ["u8"];
    case "u16":
      return ["u16"];
    case "u32":
      return ["u32"];
    case "f32":
      return ["f32"];
    case "f64":
      return ["f64"];
    case "i64":
      return ["i64"];
    case "u64":
      return ["u64"];
    case "bool":
      return ["u8"];
    case "cstring":
      return ["cstring"];
    case "ptr_len":
      return ["ptr", "u64"];
  }
}

/** Which built-in export groups to append. A group missing from the
 * actual cdylib (the crate did not expand the matching `*_abi!()`)
 * MUST be disabled: `bun:ffi` throws at `dlopen` on any declared
 * symbol the library does not export. */
export interface BuiltinFeatures {
  /** `bffi_runtime_abi!()` exports (error drain + buffer pair).
   * Default: true. */
  runtime?: boolean;
  /** `bffi_async_abi!()` exports (attach/cancel). Default: false. */
  async?: boolean;
  /** `bffi_callback_abi!()` exports. Default: false. */
  callbacks?: boolean;
  /** `bffi_stream_abi!()` exports (next/drop). Default: false. */
  stream?: boolean;
}

const RUNTIME_DECLARATIONS: Record<string, { args: FfiType[]; returns: FfiType }> = {
  bffi_error_take_last: { args: [], returns: "u64" },
  bffi_error_name: { args: ["u64"], returns: "u32" },
  bffi_error_message_ptr: { args: ["u64"], returns: "ptr" },
  bffi_error_message_len: { args: ["u64"], returns: "u64" },
  bffi_error_cause_ptr: { args: ["u64"], returns: "ptr" },
  bffi_error_cause_len: { args: ["u64"], returns: "u64" },
  bffi_error_free: { args: ["u64"], returns: "u32" },
  bffi_error_user_code: { args: ["u64"], returns: "u32" },
  bffi_error_variant_ptr: { args: ["u64"], returns: "ptr" },
  bffi_error_variant_len: { args: ["u64"], returns: "u64" },
  bffi_error_payload_ptr: { args: ["u64"], returns: "ptr" },
  bffi_error_payload_len: { args: ["u64"], returns: "u64" },
  bffi_error_stack_ptr: { args: ["u64"], returns: "ptr" },
  bffi_error_stack_len: { args: ["u64"], returns: "u64" },
  bffi_buffer: { args: ["u64"], returns: "ptr" },
  bffi_buffer_length: { args: ["u64"], returns: "u64" },
  bffi_types_free: { args: ["u64"], returns: "u32" },
};

const ASYNC_DECLARATIONS: Record<string, { args: FfiType[]; returns: FfiType }> = {
  bffi_async_attach: {
    args: ["u64", "u64", "u64", "pointer"],
    returns: "u32",
  },
  bffi_async_cancel: { args: ["u64"], returns: "u32" },
};

const CALLBACK_DECLARATIONS: Record<string, { args: FfiType[]; returns: FfiType }> = {
  bffi_callback_set_thread: { args: [], returns: "u32" },
  bffi_callback_bind: {
    args: ["u8", "ptr", "u64", "u64", "pointer"],
    returns: "u32",
  },
  bffi_callback_invoke: {
    args: ["u64", "ptr", "u64", "pointer"],
    returns: "u32",
  },
  bffi_callback_revoke: { args: ["u64"], returns: "u32" },
};

const STREAM_DECLARATIONS: Record<string, { args: FfiType[]; returns: FfiType }> = {
  bffi_stream_next: {
    args: ["u64", "u32", "pointer"],
    returns: "u32",
  },
  bffi_stream_drop: { args: ["u64"], returns: "u32" },
};

/** Built-in declarations selected by [`BuiltinFeatures`]. */
export function builtinDeclarations(
  features: BuiltinFeatures = {},
): Record<string, { args: FfiType[]; returns: FfiType }> {
  const out: Record<string, { args: FfiType[]; returns: FfiType }> = {};
  if (features.runtime ?? true) {
    Object.assign(out, RUNTIME_DECLARATIONS);
  }
  if (features.async) {
    Object.assign(out, ASYNC_DECLARATIONS);
  }
  if (features.callbacks) {
    Object.assign(out, CALLBACK_DECLARATIONS);
  }
  if (features.stream) {
    Object.assign(out, STREAM_DECLARATIONS);
  }
  return out;
}

/**
 * Builds the `dlopen` declarations object for a module: every export
 * keyed by symbol name, `args` derived from the ABI parameter view
 * (the receiver handle of class members prepended by the caller) and
 * the trailing out-parameter, `returns` always the `u32` ErrorCode -
 * plus the built-in export groups selected by `features`.
 */
export function buildDeclarations(
  json: ModuleJson,
  features: BuiltinFeatures = {},
): Record<string, {
  args: FfiType[];
  returns: FfiType;
}> {
  assertSchema(json);
  const declarations: Record<string, { args: FfiType[]; returns: FfiType }> = {};
  const add = (
    exportName: string,
    abi: { params: { abi: AbiName }[] },
    out: OutName | undefined,
  ): void => {
    if (exportName in declarations) {
      throw new Error(`duplicate export symbol: ${exportName}`);
    }
    const args = abi.params.flatMap((param) => ffiArg(param.abi));
    if (out !== undefined) {
      args.push("pointer");
    }
    declarations[exportName] = { args, returns: "u32" };
  };
  for (const fn of json.functions) {
    add(fn.export, fn, fn.out);
  }
  for (const cls of json.classes) {
    // Class members receive the instance handle first: one `u64`
    // argument prepended by the caller (CALLING-CONVENTION.md §7).
    declarations[cls.release] = { args: ["u64"], returns: "u32" };
    declarations[cls.constructor.export] = {
      args: cls.constructor.params.flatMap((param) => ffiArg(param.abi)).concat(
        cls.constructor.out === undefined ? [] : ["pointer"],
      ),
      returns: "u32",
    };
    for (const field of cls.fields) {
      if (field.export in declarations) {
        throw new Error(`duplicate export symbol: ${field.export}`);
      }
      declarations[field.export] = { args: ["u64", "pointer"], returns: "u32" };
    }
    for (const method of cls.methods) {
      if (method.export in declarations) {
        throw new Error(`duplicate export symbol: ${method.export}`);
      }
      const args = ["u64" as FfiType].concat(method.params.flatMap((param) => ffiArg(param.abi)));
      if (method.out !== undefined) {
        args.push("pointer");
      }
      declarations[method.export] = { args, returns: "u32" };
    }
  }
  for (const [name, declaration] of Object.entries(builtinDeclarations(features))) {
    if (!(name in declarations)) {
      declarations[name] = declaration;
    }
  }
  return declarations;
}
