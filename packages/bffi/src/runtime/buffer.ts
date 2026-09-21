/**
 * Transient-buffer reading over the runtime ABI
 * (CALLING-CONVENTION.md §5): a returned byte payload lives behind a
 * `u64` handle; the bytes are copied out BEFORE `bffi_types_free`
 * releases the slot (the copy-by-default policy, mirrored on the JS
 * side).
 */
import type { FfiLib } from "#bffi/runtime/error.ts";
import { readPointer, sym } from "#bffi/runtime/error.ts";

/**
 * Builds the `readBuffer()` accessor over `lib`: copies the bytes
 * behind a handle into a fresh `Uint8Array` and releases the slot. A
 * null handle (`0n`) or an empty payload yields an empty array.
 */
export function makeReadBuffer(lib: FfiLib): (handle: bigint) => Uint8Array {
  return (handle: bigint) => {
    const len = Number(sym(lib, "bffi_buffer_length")(handle));
    const raw = sym(lib, "bffi_buffer")(handle);
    // The loose symbol typing unions every return shape; a "ptr"
    // return is a number when non-null, null when NULL.
    const dataPtr = typeof raw === "number" ? raw : null;
    if (dataPtr === null || len === 0) {
      sym(lib, "bffi_types_free")(handle);
      return new Uint8Array(0);
    }
    // Copy the bytes out before the free releases the memory.
    const copy = new Uint8Array(readPointer(dataPtr, len));
    sym(lib, "bffi_types_free")(handle);
    return copy;
  };
}

/** The shared UTF-8 decoder of the runtime (buffer string returns). */
const decoder = new TextDecoder();

/** Decodes a returned byte payload as canonical UTF-8. */
export function decodeUtf8(bytes: Uint8Array): string {
  return decoder.decode(bytes);
}
