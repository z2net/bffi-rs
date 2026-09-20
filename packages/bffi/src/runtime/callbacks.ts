/**
 * The JS-side callback surface over the generic callback ABI
 * (`bffi_callback::bffi_callback_abi!()` exports).
 *
 * Two directions:
 *
 * - **Rust -> JS** (`bindJsCallback`): a JS function is wrapped into a
 *   `bun:ffi` JSCallback and its opaque pointer is stored under a
 *   native handle; Rust invokes it later through that handle.
 * - **JS -> Rust** (`invokeCallback`): a native body registered in
 *   Rust (`bffi_callback::register`) is invoked from JS by handle;
 *   arguments and the result travel through the framework-wide wire
 *   codec.
 *
 * Signature and argument bytes use the wire tag table
 * (`bffi_types::wire`): `I32`=1, `I64`=2, `F64`=3, `Bool`=4, `Str`=5,
 * `U64`=10. Malformed bytes and signature mismatches surface as thrown
 * `Error`s carrying the drained native message (`Error.cause` keeps
 * the domain source).
 */
import { JSCallback, ptr } from "bun:ffi";
import { ErrorCode, type FfiLib, makeTakeError, sym } from "./error.ts";
import { makeReadBuffer } from "./buffer.ts";
import { decodeValue, encodeValue, TAG_BOOL, TAG_F64, TAG_I32, TAG_I64, TAG_STR, TAG_U64, type WireValue } from "./wire.ts";
import { registerDisposer } from "./dispose.ts";

/**
 * A callback value type (the `bffi-callback` `ValueType` matrix).
 * `"u64"` is the exact unsigned carrier (a non-negative `bigint` in
 * JS); `"string"` crosses the direct C call as a `cstring` in either
 * direction (a returned pointer is call-scoped and copied out
 * immediately).
 */
export type CbType = "i32" | "i64" | "u64" | "f64" | "bool" | "string";

/** A declared callback signature: return type plus parameter types. */
export interface CallbackSig {
  ret: CbType;
  params: CbType[];
}

/** The value a callback passes to or receives from the native side. */
export type CbValue = number | bigint | boolean | string;

/** The wire tag of a callback value type (bffi_types::wire). */
function wireTag(ty: CbType): number {
  switch (ty) {
    case "i32":
      return TAG_I32;
    case "i64":
      return TAG_I64;
    case "u64":
      return TAG_U64;
    case "f64":
      return TAG_F64;
    case "bool":
      return TAG_BOOL;
    case "string":
      return TAG_STR;
  }
}

/** The `bun:ffi` JSCallback argument spelling of a callback type. */
function jsArgType(ty: CbType): "i32" | "i64" | "u64" | "f64" | "u8" | "cstring" {
  if (ty === "bool") {
    return "u8";
  }
  if (ty === "string") {
    return "cstring";
  }
  return ty;
}

/**
 * Binds a JS function for the Rust side (Rust -> JS): the function is
 * wrapped into a JSCallback with `sig`'s shape and stored under a
 * fresh native handle. Rust invokes it through that handle; revoke
 * releases both sides.
 */
export function bindJsCallback(
  lib: FfiLib,
  sig: CallbackSig,
  fn: (...args: CbValue[]) => CbValue,
): {
  handle: bigint;
  revoke(): void;
  [Symbol.dispose](): void;
} {
  const takeError = makeTakeError(lib);
  const wrapped = new JSCallback(
    (...raw: unknown[]) => {
      const args = raw.map((value, index) => {
        const ty = sig.params[index];
        if (ty === undefined) {
          return value;
        }
        return ty === "bool" ? (value ?? 0) !== 0 : value;
      });
      const result = fn(...(args as CbValue[]));
      return sig.ret === "bool" ? (result === true ? 1 : 0) : result;
    },
    {
      args: sig.params.map(jsArgType),
      returns: jsArgType(sig.ret),
    },
  );
  if (wrapped.ptr === null) {
    throw new Error("bun:ffi produced a null JSCallback pointer");
  }

  const sigBytes = new Uint8Array(1 + sig.params.length);
  sigBytes[0] = wireTag(sig.ret);
  for (const [index, param] of sig.params.entries()) {
    sigBytes[1 + index] = wireTag(param);
  }
  const out = new BigUint64Array(1);
  const status = sym(lib, "bffi_callback_bind")(
    sigBytes[0],
    sigBytes.length > 1 ? ptr(sigBytes.subarray(1)) : null,
    sig.params.length,
    BigInt(wrapped.ptr),
    out,
  );
  if (status !== ErrorCode.Ok) {
    throw takeError() ?? new Error(`bffi_callback_bind failed: ${String(status)}`);
  }
  const handle = out[0] ?? 0n;
  /** The strict path: a second revoke throws (the terminal-revocation
   * contract). Also closes the JSCallback, which previously leaked
   * for the bind lifetime. */
  const revoke = (): void => {
    revokeCallback(lib, handle);
    wrapped.close();
    disposers.delete(dispose);
  };
  const dispose = (): void => {
    revoke();
  };
  const disposers = registerDisposer(lib, dispose);
  let disposed = false;
  const result = {
    handle,
    revoke,
    /** The dispose protocol half: IDEMPOTENT (a `using` block may
     * run it once, and a second run must not throw). */
    [Symbol.dispose](): void {
      if (disposed) {
        return;
      }
      disposed = true;
      revoke();
    },
  };
  return result;
}

/**
 * Invokes the native callback behind `handle` (JS -> Rust) with the
 * wire-encoded arguments; the encoded result record is decoded into
 * the JS value. Signature mismatches and dead handles throw the
 * drained native error.
 */
export function invokeCallback(
  lib: FfiLib,
  handle: bigint,
  ...args: readonly WireValue[]
): WireValue {
  const takeError = makeTakeError(lib);
  const readBuffer = makeReadBuffer(lib);
  const bytes: number[] = [];
  for (const arg of args) {
    encodeValue(bytes, arg);
  }
  const encoded = new Uint8Array(bytes);
  const out = new BigUint64Array(1);
  const status = sym(lib, "bffi_callback_invoke")(
    handle,
    encoded.length > 0 ? ptr(encoded) : null,
    encoded.length,
    out,
  );
  if (status !== ErrorCode.Ok) {
    throw takeError() ?? new Error(`bffi_callback_invoke failed: ${String(status)}`);
  }
  return decodeValue(readBuffer(out[0] ?? 0n));
}

/**
 * Revokes the callback behind `handle`, whatever direction it belongs
 * to. Revocation is terminal: a second revoke throws
 * (`ErrorCode::InvalidHandle`).
 */
export function revokeCallback(lib: FfiLib, handle: bigint): void {
  const takeError = makeTakeError(lib);
  const status = sym(lib, "bffi_callback_revoke")(handle);
  if (status !== ErrorCode.Ok) {
    throw takeError() ?? new Error(`bffi_callback_revoke failed: ${String(status)}`);
  }
}

/**
 * Binds the calling thread as one of the process's JS threads
 * (multi-isolate: every Bun 1.4 Worker calls this once on its own
 * thread; idempotent). Required once before native callbacks may be
 * invoked on that thread.
 */
export function setJsThread(lib: FfiLib): void {
  const takeError = makeTakeError(lib);
  const status = sym(lib, "bffi_callback_set_thread")();
  if (status !== ErrorCode.Ok) {
    throw takeError() ?? new Error(`bffi_callback_set_thread failed: ${String(status)}`);
  }
}

/**
 * Deregisters the calling thread: the explicit shutdown half of
 * {@link setJsThread}. A Worker calls this right before exiting so
 * its slot queue retires immediately and further targeted deliveries
 * to it fail fast (LoopStopped) instead of timing out. Idempotent.
 */
export function unsetJsThread(lib: FfiLib): void {
  const takeError = makeTakeError(lib);
  const status = sym(lib, "bffi_callback_unset_thread")();
  if (status !== ErrorCode.Ok) {
    throw takeError() ?? new Error(`bffi_callback_unset_thread failed: ${String(status)}`);
  }
}
