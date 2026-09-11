/**
 * Error draining over the runtime ABI (CALLING-CONVENTION.md §5):
 * a non-zero status means the thread-local last error holds a
 * `BffiError`; `bffi_error_take_last` moves it into a readable slot
 * whose message, cause and JS constructor name are read through the
 * paired exports, then released.
 */

/** ErrorCode values that cross the C ABI (bffi-core/src/error.rs). */
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

const decoder = new TextDecoder();

/**
 * Builds the `takeError()` drain over `lib`: returns the stored last
 * error as a JS `Error` (the Rust source travels as `Error.cause`),
 * or `null` when no error is stored.
 */
export function makeTakeError(lib: FfiLib): () => Error | null {
  return () => {
    const handle = sym(lib, "bffi_error_take_last")();
    if (typeof handle !== "bigint" || handle === 0n) {
      return null;
    }
    const name = sym(lib, "bffi_error_name")(handle);
    const len = Number(sym(lib, "bffi_error_message_len")(handle));
    const messagePtr = sym(lib, "bffi_error_message_ptr")(handle);
    let message = "";
    if (typeof messagePtr === "number" && len > 0) {
      // The pointer is valid until bffi_error_free and covers exactly
      // `len` UTF-8 bytes (CALLING-CONVENTION.md §5).
      message = decoder.decode(readPointer(messagePtr, len));
    }
    const causeLen = Number(sym(lib, "bffi_error_cause_len")(handle));
    const causePtr = sym(lib, "bffi_error_cause_ptr")(handle);
    let cause: string | undefined;
    if (typeof causePtr === "number" && causeLen > 0) {
      // Same lifetime contract as the message pair; null/0 means no
      // cause.
      cause = decoder.decode(readPointer(causePtr, causeLen));
    }
    sym(lib, "bffi_error_free")(handle);
    const Ctor = JS_ERROR_NAMES[Number(name)] ?? Error;
    return cause === undefined ? new Ctor(message) : new Ctor(message, { cause });
  };
}

/** Reads `len` bytes at a non-null data pointer into a fresh view. */
export function readPointer(pointer: number, len: number): Uint8Array {
  return new Uint8Array(toArrayBuffer(pointer, 0, len));
}
