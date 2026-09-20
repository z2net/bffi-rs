/**
 * Callback surface tests over a mock implementation of the generic
 * callback ABI: the mock encodes/decodes the same wire protocol as
 * the Rust side (`bffi_callback::abi`), so the JS wrapper's protocol
 * conformance is exercised end-to-end without dlopen. The mock cannot
 * dereference real pointers, so decoded arguments travel through a
 * side channel (`queueArgs`) - the byte format itself is covered by
 * the Rust-side `bffi-callback` tests.
 *
 * Run with `bun test packages/bffi`.
 */
import { describe, expect, test } from "bun:test";
import { ptr } from "bun:ffi";

import {
  bindJsCallback,
  type CallbackSig,
  type CbValue,
  invokeCallback,
  setJsThread,
  TAG_BOOL,
  TAG_I32,
  TAG_I64,
  TAG_STR,
  TAG_U64,
  type WireValue,
} from "../src/index.ts";
import type { FfiLib } from "../src/runtime/error.ts";

/** Builds the mock native side: a JS re-implementation of the four
 * callback exports with the same status contract. */
function makeCallbackMock() {
  const natives = new Map<bigint, { body: (args: WireValue[]) => WireValue }>();
  const jsBinds = new Map<bigint, { retTag: number; jsPtr: bigint }>();
  const buffers: { bytes: Uint8Array; pointer: number }[] = [];
  const pending: { args?: WireValue[] } = {};
  let nextHandle = 100n;

  const encode = (value: WireValue): Uint8Array => {
    const bytes: number[] = [];
    if (value === undefined) {
      bytes.push(0);
    } else if (typeof value === "number") {
      bytes.push(TAG_I32);
      const view = new DataView(new ArrayBuffer(4));
      view.setInt32(0, value, true);
      for (let i = 0; i < 4; i++) {
        bytes.push(view.getUint8(i));
      }
    } else if (typeof value === "bigint") {
      bytes.push(TAG_I64);
      const view = new DataView(new ArrayBuffer(8));
      view.setBigInt64(0, value, true);
      for (let i = 0; i < 8; i++) {
        bytes.push(view.getUint8(i));
      }
    } else if (typeof value === "boolean") {
      bytes.push(TAG_BOOL, value ? 1 : 0);
    } else {
      // Strings/bytes/composites are outside the ValueType matrix.
      throw new Error("mock: unsupported callback value kind");
    }
    return new Uint8Array(bytes);
  };

  const store = (bytes: Uint8Array): bigint => {
    buffers.length = 0;
    buffers.push({ bytes, pointer: ptr(bytes) });
    return 1n;
  };

  const lib = {
    bffi_error_take_last: () => null,
    bffi_error_name: () => 0,
    bffi_error_message_len: () => 0,
    bffi_error_message_ptr: () => null,
    bffi_error_cause_len: () => 0,
    bffi_error_cause_ptr: () => null,
    bffi_error_free: () => 0,
    bffi_buffer_length: () => buffers[0]?.bytes.length ?? 0,
    bffi_buffer: () => buffers[0]?.pointer ?? null,
    bffi_types_free: () => 0,
    bffi_callback_set_thread: () => 0,
    bffi_callback_bind: (
      retTag: number,
      _paramsPtr: unknown,
      len: number,
      jsPtr: number | bigint,
      out: BigUint64Array,
    ) => {
      void _paramsPtr;
      void len;
      const handle = nextHandle++;
      jsBinds.set(handle, { retTag, jsPtr: BigInt(jsPtr) });
      out[0] = handle;
      return 0;
    },
    bffi_callback_invoke: (
      handle: bigint,
      _argsPtr: unknown,
      _len: number,
      out: BigUint64Array,
    ) => {
      const native = natives.get(handle);
      if (!native) {
        return 4; // InvalidHandle
      }
      const encoded = encode(native.body(pending.args ?? []));
      pending.args = undefined;
      out[0] = store(encoded);
      return 0;
    },
    bffi_callback_revoke: (handle: bigint) =>
      natives.delete(handle) || jsBinds.delete(handle) ? 0 : 4,
  } as unknown as FfiLib;

  return {
    lib,
    /** Registers a mock native body (emulates Rust `register`). */
    registerNative(body: (args: WireValue[]) => WireValue): bigint {
      const handle = nextHandle++;
      natives.set(handle, { body });
      return handle;
    },
    /** Queues the decoded arguments the next invoke feeds the body. */
    queueArgs(args: WireValue[]): void {
      pending.args = args;
    },
    /** The bound JS slots: handle -> (return tag, pointer token). */
    jsBinds,
  };
}

describe("callbacks over the mock ABI", () => {
  test("setJsThread returns void on Ok", () => {
    const mock = makeCallbackMock();
    expect(() => setJsThread(mock.lib)).not.toThrow();
  });

  test("invokeCallback round-trips arguments and the result", () => {
    const mock = makeCallbackMock();
    const handle = mock.registerNative((args) => {
      expect(args).toEqual([2, 3n, true]);
      return 10n;
    });
    mock.queueArgs([2, 3n, true]);
    expect(invokeCallback(mock.lib, handle, 2, 3n, true)).toBe(10n);
  });

  test("invokeCallback surfaces dead handles as a thrown error", () => {
    const mock = makeCallbackMock();
    expect(() => invokeCallback(mock.lib, 999n)).toThrow(
      "bffi_callback_invoke failed: 4",
    );
  });

  test("bindJsCallback stores the pointer under a handle and revokes", () => {
    const mock = makeCallbackMock();
    const sig: CallbackSig = { ret: "i32", params: ["i32", "bool"] };
    const bound = bindJsCallback(mock.lib, sig, (a: CbValue, b: CbValue) => {
      void b;
      return (a as number) * 2;
    });
    expect(bound.handle).toBe(100n);

    const bind = mock.jsBinds.get(bound.handle);
    expect(bind).toBeDefined();
    expect(bind?.retTag).toBe(TAG_I32);
    expect(bind?.jsPtr).toBeGreaterThan(0n);

    bound.revoke();
    expect(() => bound.revoke()).toThrow("bffi_callback_revoke failed: 4");
  });

  test("bindJsCallback records the extended u64 and string tags", () => {
    const mock = makeCallbackMock();

    const u64Bound = bindJsCallback(mock.lib, { ret: "u64", params: [] }, () => 1n);
    expect(mock.jsBinds.get(u64Bound.handle)?.retTag).toBe(TAG_U64);
    u64Bound.revoke();

    const strBound = bindJsCallback(
      mock.lib,
      { ret: "string", params: ["string"] },
      (s: CbValue) => `hi ${String(s)}`,
    );
    expect(mock.jsBinds.get(strBound.handle)?.retTag).toBe(TAG_STR);
    strBound.revoke();
  });
});
