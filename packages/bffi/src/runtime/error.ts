/**
 * Error draining over the runtime ABI (CALLING-CONVENTION.md §5):
 * a non-zero status means the thread-local last error holds a
 * `BffiError`; `bffi_error_take_last` moves it into a readable slot
 * whose message, cause and JS constructor name are read through the
 * paired exports, then released.
 */

/** ErrorCode values that cross the C ABI (crates/bffi/src/core/error.rs). */
export const ErrorCode = {
  Ok: 0,
  Error: 1,
  Panic: 2,
  NullHandle: 3,
  InvalidHandle: 4,
  TableFull: 5,
  InvalidTag: 6,
  InvalidUtf8: 7,
  NumberOutOfRange: 8,
  NullPointer: 9,
  BufferTooSmall: 10,
  InvalidArgument: 11,
  WrongThread: 12,
  DomainError: 13,
  Pending: 14,
  Timeout: 15,
  ReentrantCall: 16,
} as const;

/** `bffi_error_name` values: `1` = Error, `2` = TypeError, `3` = RangeError. */
const JS_ERROR_NAMES: Record<number, ErrorConstructor> = {
  1: Error,
  2: TypeError,
  3: RangeError,
};

/** A dlopen'ed symbol: opaque arguments, unknown return. */
export type FfiSymbol = (...args: unknown[]) => unknown;
export type FfiLib = Record<string, FfiSymbol>;

/**
 * Fetches one export by symbol name: `bun:ffi` symbol tables are
 * untyped, so every accessor goes through this guard - a missing
 * export is a loud loader error, not an undefined call.
 */
export function sym(lib: FfiLib, name: string): FfiSymbol {
  const symbol = lib[name];
  if (typeof symbol !== "function") {
    throw new Error(`missing export: ${name}`);
  }
  return symbol;
}

import { toArrayBuffer } from "bun:ffi";
import { decodeValue } from "#bffi/runtime/wire.ts";

const decoder = new TextDecoder();

/**
 * Builds the `takeError()` drain over `lib`: returns the stored last
 * error as a JS `Error` enriched with the B3 fields — `e.code` (the
 * ABI status, including derived user codes), `e.name` (the derived
 * variant name), `e.payload` (the decoded wire record) and
 * `e.nativeStack` (the captured Rust backtrace) — or `null` when no
 * error is stored.
 */
export function makeTakeError(lib: FfiLib): (status?: number) => Error | null {
  const takeLast = sym(lib, "bffi_error_take_last");
  const nameSym = sym(lib, "bffi_error_name");
  const messagePtr = sym(lib, "bffi_error_message_ptr");
  const messageLen = sym(lib, "bffi_error_message_len");
  const causePtr = sym(lib, "bffi_error_cause_ptr");
  const causeLen = sym(lib, "bffi_error_cause_len");
  // The B3 rich accessors are optional: libraries built from older
  // bffi versions do not export them, and mock test libraries omit
  // them. Every read guards on the symbol being present.
  const userCode = symOptional(lib, "bffi_error_user_code");
  const variantPtr = symOptional(lib, "bffi_error_variant_ptr");
  const variantLen = symOptional(lib, "bffi_error_variant_len");
  const payloadPtr = symOptional(lib, "bffi_error_payload_ptr");
  const payloadLen = symOptional(lib, "bffi_error_payload_len");
  const stackPtr = symOptional(lib, "bffi_error_stack_ptr");
  const stackLen = symOptional(lib, "bffi_error_stack_len");
  const free = sym(lib, "bffi_error_free");
  return (status?: number): Error | null => {
    const handle = takeLast();
    if (typeof handle !== "bigint" || handle === 0n) {
      return null;
    }
    const jsName = Number(nameSym(handle));
    const len = Number(messageLen(handle));
    const messagePointer = messagePtr(handle);
    let message = "";
    if (typeof messagePointer === "number" && len > 0) {
      // The pointer is valid until bffi_error_free and covers exactly
      // `len` UTF-8 bytes (CALLING-CONVENTION.md §5).
      message = decoder.decode(readPointer(messagePointer, len));
    }
    const causeLenBytes = Number(causeLen(handle));
    const causePointer = causePtr(handle);
    let cause: string | undefined;
    if (typeof causePointer === "number" && causeLenBytes > 0) {
      // Same lifetime contract as the message pair; null/0 means no
      // cause.
      cause = decoder.decode(readPointer(causePointer, causeLenBytes));
    }
    const Ctor = JS_ERROR_NAMES[jsName] ?? Error;
    const error = cause === undefined ? new Ctor(message) : new Ctor(message, { cause });

    // B3 rich fields (all optional, best-effort): every accessor
    // group is guarded by its own symbol presence.
    if (userCode && variantPtr && variantLen && payloadPtr && payloadLen && stackPtr && stackLen) {
      const richCode = Number(userCode(handle));
      if (richCode !== 0) {
        (error as Error & { code: number }).code = status ?? richCode;
      } else if (status !== undefined && status !== 0) {
        (error as Error & { code: number }).code = status;
      }
      const vLen = Number(variantLen(handle));
      const vPointer = variantPtr(handle) as unknown as number;
      if (vLen > 0) {
        error.name = decoder.decode(readPointer(vPointer, vLen));
      }
      const pLen = Number(payloadLen(handle));
      const pPointer = payloadPtr(handle) as unknown as number;
      if (pLen > 0) {
        const decoded = decodeValue(new Uint8Array(toArrayBuffer(pPointer, 0, pLen)));
        // A TAG_RECORD payload decodes as {fields: [...]} - unwrap it
        // to the positional array so e.payload matches the Rust-side
        // variant fields directly.
        (error as Error & { payload: unknown }).payload =
          decoded !== null && typeof decoded === "object" && "fields" in decoded
            ? (decoded as { fields: unknown[] }).fields
            : decoded;
      }
      const sLen = Number(stackLen(handle));
      const sPointer = stackPtr(handle) as unknown as number;
      if (sLen > 0) {
        (error as Error & { nativeStack: string }).nativeStack = decoder.decode(
          readPointer(sPointer, sLen),
        );
      }
    }
    free(handle);
    return error;
  };
}

/** Reads `len` bytes at a non-null data pointer into a fresh view. */
export function readPointer(pointer: number, len: number): Uint8Array {
  return new Uint8Array(toArrayBuffer(pointer, 0, len));
}

/** A `sym` that tolerates missing exports: returns `undefined` when
 * the library does not provide the symbol (mock libraries, older
 * bffi builds). */
export function symOptional(lib: FfiLib, name: string): FfiSymbol | undefined {
  const symbol = (lib as Record<string, unknown>)[name];
  return typeof symbol === "function" ? (symbol as FfiSymbol) : undefined;
}
