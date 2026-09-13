/**
 * The typed API factory: builds the JS-facing object from the loader
 * schema. The generic `ApiOf` mapping derives the TypeScript types
 * from the JSON literal itself, so the generated module (a `const`
 * object literal) ships exact signatures without hand-written types.
 */
import { dlopen, ptr } from "bun:ffi";
import { ErrorCode, type FfiLib, makeTakeError, sym } from "../runtime/error.ts";
import { makeReadBuffer } from "../runtime/buffer.ts";
import { assertSchema, buildDeclarations, type BuiltinFeatures, type FunctionJson, type ModuleJson, type TsName } from "./loader.ts";
import { wrapTask } from "../runtime/async.ts";
import { streamItemTs, wrapStream } from "../runtime/stream.ts";
import { isCompositeTs, jsToWire, tablesOf, wireToJs } from "./composite.ts";
import { decodeAt, encodeValue } from "../runtime/wire.ts";

const decoder = new TextDecoder();

/** The record table entry named `N` (`never` when absent). */
type NamedRecord<M extends ModuleJson, N extends string> = Extract<
  NonNullable<M["records"]>[number],
  { name: N }
>;

/** The enum table entry named `N` (`never` when absent). */
type NamedEnum<M extends ModuleJson, N extends string> = Extract<
  NonNullable<M["enums"]>[number],
  { name: N }
>;

/** The object shape of one record entry: fields keyed by name (the
 * module context threads through - a record field may reference
 * another composite of the same module). */
type RecordShape<R extends { fields: { name: string; ts: TsName }[] }, M extends ModuleJson> = {
  [F in R["fields"][number] as F["name"]]: TsOf<F["ts"], M>;
};

/** The string-literal union of one enum entry's variants. */
type EnumUnion<E extends { variants: { name: string }[] }> =
  E["variants"][number]["name"];

/**
 * The TypeScript type for one `ts` name in the schema. The Promise
 * cases are spelled out (a closed table, no template-literal infer):
 * the schema names them exactly. The B1 composites resolve against
 * the module's own `records`/`enums` tables: a record maps to its
 * field shape, an enum to the union of its variant names, and the
 * array forms wrap their element type. An unknown name falls back to
 * its own string literal (the validator rejects those upfront).
 */
export type TsOf<S extends TsName, M extends ModuleJson = ModuleJson> =
  S extends "Promise<void>"
    ? Promise<void>
    : S extends "Promise<number>"
      ? Promise<number>
      : S extends "Promise<bigint>"
        ? Promise<bigint>
        : S extends "Promise<boolean>"
          ? Promise<boolean>
          : S extends "Promise<string>"
            ? Promise<string>
             : S extends "Promise<Uint8Array>"
               ? Promise<Uint8Array>
               : S extends "Promise<number[]>"
                 ? Promise<number[]>
                 : S extends "Promise<bigint[]>"
                   ? Promise<bigint[]>
                   : S extends "Promise<boolean[]>"
                     ? Promise<boolean[]>
                     : S extends "Promise<string[]>"
                       ? Promise<string[]>
                         : S extends "Promise<Uint8Array[]>"
                           ? Promise<Uint8Array[]>
                           : S extends "Promise<string | null>"
                             ? Promise<string | null>
                             : S extends "Promise<Uint8Array | null>"
                               ? Promise<Uint8Array | null>
                               : S extends "Promise<number[] | null>"
                                 ? Promise<number[] | null>
                                 : S extends "Promise<bigint[] | null>"
                                   ? Promise<bigint[] | null>
                                   : S extends "Promise<boolean[] | null>"
                                     ? Promise<boolean[] | null>
                                     : S extends "Promise<string[] | null>"
                                       ? Promise<string[] | null>
                                       : S extends "Promise<Uint8Array[] | null>"
                                         ? Promise<Uint8Array[] | null>
                                         : S extends `Promise<${infer P} | null>`
                                           ? P extends TsName
                                             ? [NamedRecord<M, P>] extends [never]
                                               ? [NamedEnum<M, P>] extends [never]
                                                 ? S
                                                 : NamedEnum<M, P> extends infer E
                                                   ? E extends { variants: { name: string }[] }
                                                     ? Promise<EnumUnion<E> | null>
                                                     : S
                                                   : S
                                               : NamedRecord<M, P> extends infer Rec
                                                 ? Rec extends { fields: { name: string; ts: TsName }[] }
                                                   ? Promise<RecordShape<Rec, M> | null>
                                                   : S
                                                 : S
                                             : S
                                           : S extends `Promise<${infer P}>`
                           ? P extends TsName
                             ? [NamedRecord<M, P>] extends [never]
                               ? [NamedEnum<M, P>] extends [never]
                                 ? S
                                 : NamedEnum<M, P> extends infer E
                                   ? E extends { variants: { name: string }[] }
                                     ? Promise<EnumUnion<E>>
                                     : S
                                   : S
                               : NamedRecord<M, P> extends infer Rec
                                 ? Rec extends { fields: { name: string; ts: TsName }[] }
                                   ? Promise<RecordShape<Rec, M>>
                                   : S
                                 : S
                             : S
                            : S extends "string | null"
                 ? string | null
                 : S extends "Uint8Array | null"
                   ? Uint8Array | null
                   : S extends "number | null"
                     ? number | null
                     : S extends "bigint | null"
                       ? bigint | null
                       : S extends "boolean | null"
                         ? boolean | null
                         : S extends "number[]"
                    ? number[]
                    : S extends "bigint[]"
                      ? bigint[]
                      : S extends "boolean[]"
                        ? boolean[]
                  : S extends "string[]"
                    ? string[]
                    : S extends "Uint8Array[]"
                      ? Uint8Array[]
                      : S extends "number[] | null"
                        ? number[] | null
                        : S extends "bigint[] | null"
                          ? bigint[] | null
                          : S extends "boolean[] | null"
                            ? boolean[] | null
                            : S extends "string[] | null"
                              ? string[] | null
                              : S extends "Uint8Array[] | null"
                                ? Uint8Array[] | null
                                : S extends `${infer N} | null`
                                  ? N extends TsName
                                    ? [NamedRecord<M, N>] extends [never]
                                      ? [NamedEnum<M, N>] extends [never]
                                        ? S
                                        : NamedEnum<M, N> extends infer E
                                          ? E extends { variants: { name: string }[] }
                                            ? EnumUnion<E> | null
                                            : S
                                          : S
                                      : NamedRecord<M, N> extends infer Rec
                                        ? Rec extends { fields: { name: string; ts: TsName }[] }
                                          ? RecordShape<Rec, M> | null
                                          : S
                                        : S
                                    : S
                                  : S extends "AsyncIterableIterator<number>"
                            ? AsyncIterableIterator<number>
                            : S extends "AsyncIterableIterator<bigint>"
                              ? AsyncIterableIterator<bigint>
                              : S extends "AsyncIterableIterator<boolean>"
                                ? AsyncIterableIterator<boolean>
                                : S extends "AsyncIterableIterator<string>"
                                  ? AsyncIterableIterator<string>
                                  : S extends "AsyncIterableIterator<Uint8Array>"
                                    ? AsyncIterableIterator<Uint8Array>
                                    : S extends `AsyncIterableIterator<${infer N2} | Error>`
                                      ? N2 extends TsName
                                        ? AsyncIterableIterator<TsOf<N2, M> | Error>
                                        : S
                                      : S extends `AsyncIterableIterator<${infer N}>`
                                        ? N extends TsName
                                          ? AsyncIterableIterator<TsOf<N, M>>
                                          : S
                                        : S extends `${infer N}[]`
                            ? N extends TsName
                              ? TsOf<N, M>[]
                              : S
                            : [NamedRecord<M, S & string>] extends [never]
                              ? [NamedEnum<M, S & string>] extends [never]
                                ? S extends "void"
                                  ? void
                                  : S extends "number"
                                    ? number
                                    : S extends "bigint"
                                      ? bigint
                                      : S extends "boolean"
                                        ? boolean
                                        : S extends "string"
                                          ? string
                                          : S extends "Uint8Array"
                                            ? Uint8Array
                                            : S
                                : NamedEnum<M, S & string> extends infer E
                                  ? E extends { variants: { name: string }[] }
                                    ? EnumUnion<E>
                                    : S
                                  : S
                              : NamedRecord<M, S & string> extends infer Rec
                                ? Rec extends { fields: { name: string; ts: TsName }[] }
                                  ? RecordShape<Rec, M>
                                  : S
                                : S;

/** Maps a params array onto a positional tuple type. */
export type ParamsOf<J extends readonly { ts: TsName }[], M extends ModuleJson = ModuleJson> = {
  [K in keyof J]: J[K] extends { ts: infer T } ? (T extends TsName ? TsOf<T, M> : never) : never;
};

/** The callable signature of one function/method descriptor. */
export type FnOf<J extends FunctionJson, M extends ModuleJson = ModuleJson> = (
  ...args: ParamsOf<J["params"], M>
) => TsOf<J["ret"]["ts"], M>;

/** The typed API object for a module literal. */
export type ApiOf<J extends ModuleJson> = {
  [F in J["functions"][number] as F["name"]]: FnOf<F, J>;
} & {
  [C in J["classes"][number] as C["name"]]: ClassOf<C, J>;
};

/** The typed class constructor + instance shape for one class. */
export type ClassOf<C extends ModuleJson["classes"][number], M extends ModuleJson = ModuleJson> = {
  new (
    ...args: ParamsOf<C["constructor"]["params"], M>
  ): {
    readonly [F in C["fields"][number] as F["name"]]: TsOf<F["ts"], M>;
  } & {
    [Me in C["methods"][number] as Me["name"]]: FnOf<Me, M>;
  } & {
    /** Releases the native handle early (also runs on GC). */
    release(): void;
  };
};

/**
 * Opens the native library at `libraryPath` with the dlopen
 * declarations derived from the schema, then builds the typed API.
 * Types come from the `const` literal in the generated module - pass
 * the schema through `as const` in codegen.
 */
export function createApi<J extends ModuleJson>(
  json: J,
  libraryPath: string,
  features: BuiltinFeatures = {},
): ApiOf<J> {
  assertSchema(json);
  const { symbols } = dlopen(libraryPath, buildDeclarations(json, features));
  return createApiFromLib(json, symbols as unknown as FfiLib);
}

/**
 * The pure factory over an already-loaded symbol table: the
 * testable half of [`createApi`] (mock libraries in unit tests).
 */
export function createApiFromLib<J extends ModuleJson>(json: J, lib: FfiLib): ApiOf<J> {
  assertSchema(json);
  const takeError = makeTakeError(lib);
  const readBuffer = makeReadBuffer(lib);

  /** Runs one export; throws the drained error on a non-zero status. */
  const call = (
    exportName: string,
    args: unknown[],
    outName: string | undefined,
  ): unknown => {
    const symbol = lib[exportName];
    if (typeof symbol !== "function") {
      throw new Error(`missing export: ${exportName}`);
    }
    const out = outName === undefined ? undefined : allocOut(outName);
    const full = out === undefined ? args : [...args, out];
    const status = Number(symbol(...full));
    if (status !== ErrorCode.Ok) {
      throw takeError(status) ?? new Error(`${exportName} failed: ${String(status)}`);
    }
    return readOut(outName, out);
  };

  const callFunction = (fn: FunctionJson): (...args: unknown[]) => unknown => {
    return (...jsArgs: unknown[]) => {
      const args = encodeArgs(fn.params, jsArgs, fn.name, json);
      const raw = call(fn.export, args, fn.out);
      return decodeReturn(fn, raw, readBuffer, lib, json);
    };
  };

  /** Class methods: the instance handle is prepended by the wrapper,
   * NOT counted in the descriptor's own parameter list. */
  const callMethod = (method: FunctionJson): ((handle: bigint, ...jsArgs: unknown[]) => unknown) => {
    return (handle, ...jsArgs) => {
      const args = encodeArgs(method.params, jsArgs, method.name, json);
      const raw = call(method.export, [handle, ...args], method.out);
      return decodeReturn(method, raw, readBuffer, lib, json);
    };
  };

  const api: Record<string, unknown> = {};
  for (const fn of json.functions) {
    api[fn.name] = callFunction(fn);
  }
  for (const cls of json.classes) {
    api[cls.name] = makeClass(cls, callFunction, callMethod, lib, takeError);
  }
  return api as ApiOf<J>;
}

/**
 * Encodes JS arguments onto the ABI: booleans coerce to `0`/`1`
 * (dlopen `"u8"`), an empty `Uint8Array` passes a null data pointer
 * with `len == 0` (bun:ffi rejects empty TypedArrays as pointers),
 * strings travel as cstrings verbatim, composite (record/enum/array)
 * parameters wire-encode into one `Uint8Array` crossing as a
 * borrowed `(ptr, len)` pair, everything else passes through
 * (numbers, bigints).
 */
function encodeArgs(
  params: readonly { name: string; ts: TsName; abi: string }[],
  jsArgs: readonly unknown[],
  fnName: string,
  json: ModuleJson,
): unknown[] {
  if (jsArgs.length !== params.length) {
    throw new Error(
      `${fnName}: expected ${String(params.length)} argument(s), got ${String(jsArgs.length)}`,
    );
  }
  const tables = tablesOf(json);
  const args: unknown[] = [];
  for (const [index, param] of params.entries()) {
    const value = jsArgs[index];
    switch (param.abi) {
      case "bool": {
        if (typeof value !== "boolean") {
          throw new TypeError(`${fnName}(${param.name}): expected boolean`);
        }
        args.push(value ? 1 : 0);
        break;
      }
      case "ptr_len": {
        if (isCompositeTs(param.ts, tables)) {
          const wire = jsToWire(tables, param.ts, value, `${fnName}(${param.name})`);
          const out: number[] = [];
          encodeValue(out, wire);
          const bytes = new Uint8Array(out);
          args.push(bytes.length > 0 ? ptr(bytes) : null);
          args.push(bytes.length);
          break;
        }
        if (!(value instanceof Uint8Array)) {
          throw new TypeError(`${fnName}(${param.name}): expected ${String(param.ts)}`);
        }
        args.push(value.length > 0 ? ptr(value) : null);
        args.push(value.length);
        break;
      }
      default:
        args.push(value);
        break;
    }
  }
  return args;
}

/** The TypedArray union an out slot allocates. */
type OutSlot =
  | Int8Array
  | Int16Array
  | Int32Array
  | Uint8Array
  | Uint16Array
  | Uint32Array
  | Float32Array
  | Float64Array
  | BigInt64Array
  | BigUint64Array;

/** Allocates the out slot TypedArray for an out name. */
function allocOut(out: string): OutSlot {
  switch (out) {
    case "i8":
      return new Int8Array(1);
    case "i16":
      return new Int16Array(1);
    case "i32":
      return new Int32Array(1);
    case "u8":
    case "bool":
      return new Uint8Array(1);
    case "u16":
      return new Uint16Array(1);
    case "u32":
      return new Uint32Array(1);
    case "f32":
      return new Float32Array(1);
    case "f64":
      return new Float64Array(1);
    case "i64":
      return new BigInt64Array(1);
    case "u64":
    case "handle":
      return new BigUint64Array(1);
    default:
      throw new Error(`unknown out slot: ${out}`);
  }
}

/** Reads the out slot back into the JS value it carries. */
function readOut(outName: string | undefined, out: OutSlot | undefined): unknown {
  if (outName === undefined || out === undefined) {
    return undefined;
  }
  const raw = out[0];
  if (outName === "bool") {
    return (raw ?? 0) !== 0;
  }
  if (outName === "handle" || outName === "u64" || outName === "i64") {
    return raw ?? 0n;
  }
  return raw ?? 0;
}

/**
 * Transports a successful raw return into the JS-visible value:
 * buffer handles become `Uint8Array`/`string` (or `null` when the
 * type is nullable and the payload is empty - the same
 * empty-vs-None limitation as the reference loader), task handles
 * wrap into promises, primitives pass through.
 */
function decodeReturn(
  fn: { ret: { ts: TsName; abi: string }; name: string },
  raw: unknown,
  readBuffer: (handle: bigint) => Uint8Array,
  lib: FfiLib,
  json: ModuleJson,
): unknown {
  if (fn.ret.abi === "void") {
    return undefined;
  }
  if (fn.ret.abi === "task") {
    if (typeof raw !== "bigint") {
      throw new TypeError(`${fn.name}: expected a task handle, got ${typeof raw}`);
    }
    return wrapTask(lib, raw, fn.ret.ts, json);
  }
  if (fn.ret.abi === "stream") {
    if (typeof raw !== "bigint") {
      throw new TypeError(`${fn.name}: expected a stream handle, got ${typeof raw}`);
    }
    const itemTs = streamItemTs(fn.ret.ts);
    if (itemTs === null) {
      throw new Error(
        `${fn.name}: stream return type must be AsyncIterableIterator<T>, got ${fn.ret.ts}`,
      );
    }
    return wrapStream(lib, raw, itemTs, json);
  }
  if (fn.ret.abi === "buffer") {
    if (typeof raw !== "bigint") {
      throw new TypeError(`${fn.name}: expected a buffer handle, got ${typeof raw}`);
    }
    // A `0` handle is the documented null marker of the nullable
    // (`| null`) return forms.
    if (raw === 0n && fn.ret.ts.endsWith(" | null")) {
      return null;
    }
    const bytes = readBuffer(raw);
    const tables = tablesOf(json);
    if (isCompositeTs(fn.ret.ts, tables)) {
      const ts = fn.ret.ts.endsWith(" | null") ? fn.ret.ts.slice(0, -" | null".length) : fn.ret.ts;
      return wireToJs(tables, ts, decodeAt(bytes, 0).value, `${fn.name}()`);
    }
    if (fn.ret.ts === "string") {
      return decoder.decode(bytes);
    }
    if (fn.ret.ts === "string | null") {
      return bytes.length === 0 ? null : decoder.decode(bytes);
    }
    if (fn.ret.ts === "Uint8Array | null") {
      return bytes.length === 0 ? null : bytes;
    }
    return bytes;
  }
  // `handle` (the constructor's instance handle) and every primitive
  // width pass through as-is.
  return raw;
}

interface NativeInstance {
  handle: bigint;
}

/**
 * Builds the class wrapper: the constructor allocates the instance
 * handle, every member prepends it, and a FinalizationRegistry runs
 * the release shim on GC (the documented JS contract); `release()`
 * unregisters and releases early.
 */
function makeClass(
  cls: ModuleJson["classes"][number],
  callFunction: (fn: FunctionJson) => (...args: unknown[]) => unknown,
  callMethod: (method: FunctionJson) => (handle: bigint, ...jsArgs: unknown[]) => unknown,
  lib: FfiLib,
  takeError: (status?: number) => Error | null,
): new (...args: unknown[]) => unknown {
  const ctor = callFunction(cls.constructor);
  // A GC finalizer may race an explicit `release()`; the duplicate
  // release is `InvalidHandle` (4) and must stay silent - every other
  // status is a real error.
  const release = (handle: bigint): void => {
    const status = Number(sym(lib, cls.release)(handle));
    if (status === ErrorCode.InvalidHandle) {
      return;
    }
    if (status !== ErrorCode.Ok) {
      throw takeError(status) ?? new Error(`${cls.release} failed: ${String(status)}`);
    }
  };
  const finalizers = new FinalizationRegistry((handle: bigint) => {
    release(handle);
  });

  const Wrapper = class {
    handle: bigint;
    constructor(...args: unknown[]) {
      this.handle = (ctor(...args) as bigint) ?? 0n;
      finalizers.register(this, this.handle);
    }
    release(): void {
      finalizers.unregister(this);
      release(this.handle);
    }
  };

  for (const field of cls.fields) {
    Object.defineProperty(Wrapper.prototype, field.name, {
      get(this: NativeInstance) {
        const out = allocOut(field.out);
        const status = Number(sym(lib, field.export)(this.handle, out));
        if (status !== ErrorCode.Ok) {
          throw takeError(status) ?? new Error(`${field.export} failed: ${String(status)}`);
        }
        return readOut(field.out, out);
      },
    });
  }
  for (const method of cls.methods) {
    const bound = callMethod(method);
    Object.defineProperty(Wrapper.prototype, method.name, {
      value: function (this: NativeInstance, ...args: unknown[]) {
        return bound(this.handle, ...args);
      },
      writable: true,
      configurable: true,
    });
  }
  return Wrapper as new (...args: unknown[]) => unknown;
}
