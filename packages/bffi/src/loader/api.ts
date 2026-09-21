/**
 * The typed API factory: builds the JS-facing object from the loader
 * schema. The generic `ApiOf` mapping derives the TypeScript types
 * from the JSON literal itself, so the generated module (a `const`
 * object literal) ships exact signatures without hand-written types.
 */
import { dlopen, ptr } from "bun:ffi";
import { ErrorCode, symOptional, type FfiLib, makeTakeError, sym } from "../runtime/error.ts";
import { makeReadBuffer } from "../runtime/buffer.ts";
import { assertSchema, BFFI_ABI_VERSION, buildDeclarations, type BuiltinFeatures, type FunctionJson, type ModuleJson, type TsName } from "./loader.ts";
import { wrapTask } from "../runtime/async.ts";
import { streamItemTs, wrapStream } from "../runtime/stream.ts";
import { isCompositeTs, jsToWire, tablesOf, wireToJs } from "./composite.ts";
import { decodeAt, encodeValue } from "../runtime/wire.ts";
import { makeLibDisposer } from "../runtime/dispose.ts";
import type { ApiOf } from "./api-types.ts";

const decoder = new TextDecoder();


/**
 * Opens the native library at `libraryPath` with the dlopen
 * declarations derived from the schema, then builds the typed API.
 * Types come from the `const` literal in the generated module - pass
 * the schema through `as const` in codegen.
 *
 * This is the explicit low-level API: dlopen on a raw binary path is
 * a trust decision. The pipeline (`bffi()`) resolves platform
 * packages by default and gates raw paths behind `trust: "explicit"`.
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
 * The loader handshake: the manifest's `abiVersion` (absent reads as
 * 1 for legacy manifests) and the binary's `bffi_runtime_abi_version()`
 * must both match [`BFFI_ABI_VERSION`], and a manifest `exportsHash`,
 * when present, must match the binary's `bffi_module_exports_hash()`.
 * Runs BEFORE any user wrapper is built - a stale manifest/binary
 * pair fails loudly instead of silently calling the wrong ABI.
 */
function runHandshake(json: ModuleJson, lib: FfiLib): void {
  const manifestVersion = json.abiVersion ?? 1;
  if (manifestVersion !== BFFI_ABI_VERSION) {
    throw new Error(
      `ABI version mismatch: manifest ${String(manifestVersion)} != runtime ${String(BFFI_ABI_VERSION)} - ` +
        "rebuild the module (bun bffi build) and regenerate api.gen.ts",
    );
  }
  const binaryVersion = lib.bffi_runtime_abi_version;
  if (typeof binaryVersion === "function") {
    const reported = Number(binaryVersion());
    if (reported !== BFFI_ABI_VERSION) {
      throw new Error(
        `ABI version mismatch: binary ${String(reported)} != runtime ${String(BFFI_ABI_VERSION)} - ` +
          "rebuild the module (bun bffi build)",
      );
    }
  }
  const exportsHash = json.exportsHash;
  if (exportsHash === undefined) {
    return; // legacy manifest: no hash check
  }
  const binaryHash = lib.bffi_module_exports_hash;
  if (typeof binaryHash !== "function") {
    throw new Error(
      "binary predates the ABI handshake exports - rebuild with the current bffi version",
    );
  }
  if (BigInt(exportsHash) !== binaryHash()) {
    throw new Error(
      "exports hash mismatch: the loader manifest does not describe this binary " +
        "(stale .bffi/bffi.api.json vs the built cdylib)",
    );
  }
}

/**
 * Registers the CALLING thread as a JS thread when the module ships
 * the callback surface (multi-isolate: every JS isolate that loads a
 * module binds itself - the manual `setJsThread()` ritual is gone;
 * idempotent, so an explicit later call is still fine). Modules
 * without the callback exports skip silently.
 */
function autoBindJsThread(lib: FfiLib): void {
  const setThread = symOptional(lib, "bffi_callback_set_thread");
  if (setThread === undefined) {
    return;
  }
  const status = Number(setThread());
  if (status !== ErrorCode.Ok) {
    throw new Error(`bffi_callback_set_thread failed: ${String(status)}`);
  }
}

/**
 * The pure factory over an already-loaded symbol table: the
 * testable half of [`createApi`] (mock libraries in unit tests).
 *
 * This is the explicit low-level API (no dlopen, no trust gate); the
 * ABI handshake still runs against the manifest and the symbol table.
 */
export function createApiFromLib<J extends ModuleJson>(json: J, lib: FfiLib): ApiOf<J> {
  assertSchema(json);
  runHandshake(json, lib);
  autoBindJsThread(lib);
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
  // The Api carries the JS-side dispose surface: `using api = ...`
  // retires every JSCallback trampoline created against this library.
  return Object.assign(api, makeLibDisposer(lib)) as unknown as ApiOf<J>;
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
    /** Rejects `null` for a parameter whose TS type is not nullable. */
    const rejectNull = (): void => {
      if (!param.ts.endsWith("| null")) {
        throw new TypeError(`${fnName}(${param.name}): expected ${String(param.ts)}`);
      }
    };
    switch (param.abi) {
      case "bool": {
        if (typeof value !== "boolean") {
          throw new TypeError(`${fnName}(${param.name}): expected boolean`);
        }
        args.push(value ? 1 : 0);
        break;
      }
      case "cstring": {
        if (value === null) {
          rejectNull();
          args.push(null);
          break;
        }
        args.push(value);
        break;
      }
      case "ptr_len": {
        if (value === null) {
          rejectNull();
          args.push(null, 0);
          break;
        }
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
      case "opt_number": {
        if (value === null) {
          rejectNull();
          args.push(0, 0);
          break;
        }
        if (typeof value === "boolean") {
          args.push(value ? 1 : 0, 1);
          break;
        }
        if (typeof value !== "number" || !Number.isFinite(value)) {
          throw new TypeError(`${fnName}(${param.name}): expected ${String(param.ts)}`);
        }
        args.push(value, 1);
        break;
      }
      case "opt_i64":
      case "opt_u64": {
        if (value === null) {
          rejectNull();
          args.push(0n, 0);
          break;
        }
        if (typeof value !== "bigint") {
          throw new TypeError(`${fnName}(${param.name}): expected ${String(param.ts)}`);
        }
        args.push(value, 1);
        break;
      }
      case "opt_ptr_len": {
        if (value === null) {
          rejectNull();
          args.push(null, 0, 0);
          break;
        }
        if (!(value instanceof Uint8Array)) {
          throw new TypeError(`${fnName}(${param.name}): expected ${String(param.ts)}`);
        }
        args.push(value.length > 0 ? ptr(value) : null, value.length, 1);
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
 * Builds the early-release wrapper of one class: calls the release
 * export and treats a duplicate release (`InvalidHandle`, raced with
 * the GC finalizer) as success - every other non-zero status throws
 * the drained error.
 */
export function makeRelease(
  lib: FfiLib,
  exportName: string,
  takeError: (status?: number) => Error | null,
): (handle: bigint) => void {
  const releaseExport = sym(lib, exportName);
  return (handle: bigint): void => {
    const status = Number(releaseExport(handle));
    if (status === ErrorCode.InvalidHandle) {
      return;
    }
    if (status !== ErrorCode.Ok) {
      throw takeError(status) ?? new Error(`${exportName} failed: ${String(status)}`);
    }
  };
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
  const release = makeRelease(lib, cls.release, takeError);
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
    [Symbol.dispose](): void {
      this.release();
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
