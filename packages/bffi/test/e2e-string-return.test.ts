/**
 * E2E: the JS-bound callback string round trip through the REAL
 * bun:ffi JSCallback runtime (no mock). One synchronous call of the
 * test-only `bffi_test_roundtrip_string` export exercises every leg of
 * the cstring path:
 *
 * 1. Rust `Value::Str("ping")` -> `invoke_wait` dispatch -> cstring
 *    argument into the bound JSCallback (Rust -> JS);
 * 2. the JS return value transcodes through bun:ffi's call-scoped
 *    buffer back into a Rust `CStr` -> `Value::Str` (JS -> Rust);
 * 3. the `pong:` reply wraps through the transient-buffer pair and is
 *    read back over the runtime ABI.
 *
 * Gated behind `BFFI_E2E=1` AND the presence of the release cdylib
 * (`cargo build --release -p bffi-native`); skipped otherwise, so the
 * regular `bun test packages/bffi` suite never depends on a build.
 *
 * Run with `BFFI_E2E=1 bun test packages/bffi/test/e2e-string-return.test.ts`.
 */
import { expect, test } from "bun:test";
import { dlopen } from "bun:ffi";
import { existsSync } from "node:fs";
import { join } from "node:path";

import {
  ErrorCode,
  bindJsCallback,
  decodeUtf8,
  makeReadBuffer,
  setJsThread,
  sym,
  type CbValue,
  type FfiLib,
} from "#bffi";

/** The release cdylib name of the `bffi-native` reference crate. */
const CDYLIB_NAME =
  process.platform === "win32"
    ? "bffi_native.dll"
    : process.platform === "darwin"
      ? "libbffi_native.dylib"
      : "libbffi_native.so";

/** Repo root, resolved from this file upward (packages/bffi/test). */
const REPO_ROOT = join(import.meta.dir, "..", "..", "..");
const CDYLIB_PATH = join(REPO_ROOT, "target", "release", CDYLIB_NAME);

const GATE = process.env.BFFI_E2E === "1" && existsSync(CDYLIB_PATH);

/** The raw bun:ffi declarations: the runtime ABI (error drain +
 * buffer pair, CALLING-CONVENTION.md §5), the generic callback ABI,
 * and the test-only round-trip export. Shapes mirror
 * `src/loader/loader.ts` (RUNTIME_DECLARATIONS / CALLBACK_DECLARATIONS). */
const DECLARATIONS = {
  bffi_error_take_last: { args: [], returns: "u64" },
  bffi_error_name: { args: ["u64"], returns: "u32" },
  bffi_error_message_ptr: { args: ["u64"], returns: "ptr" },
  bffi_error_message_len: { args: ["u64"], returns: "u64" },
  bffi_error_cause_ptr: { args: ["u64"], returns: "ptr" },
  bffi_error_cause_len: { args: ["u64"], returns: "u64" },
  bffi_error_free: { args: ["u64"], returns: "u32" },
  bffi_buffer: { args: ["u64"], returns: "ptr" },
  bffi_buffer_length: { args: ["u64"], returns: "u64" },
  bffi_types_free: { args: ["u64"], returns: "u32" },
  bffi_callback_set_thread: { args: [], returns: "u32" },
  bffi_callback_bind: { args: ["u8", "ptr", "u64", "u64", "pointer"], returns: "u32" },
  bffi_callback_revoke: { args: ["u64"], returns: "u32" },
  bffi_test_roundtrip_string: { args: ["u64", "pointer"], returns: "u32" },
} as const;

test.skipIf(!GATE)("e2e: cstring round trip through the real JSCallback runtime", () => {
  const lib: FfiLib = dlopen(CDYLIB_PATH, DECLARATIONS).symbols as FfiLib;

  // The test runs synchronously on the JS thread: register it, so the
  // `invoke_wait` dispatch inside the export takes the direct path.
  setJsThread(lib);

  const bound = bindJsCallback(
    lib,
    { ret: "string", params: ["string"] },
    (s: CbValue) => `echo:${String(s)}`,
  );
  try {
    const out = new BigUint64Array(1);
    const roundtrip = sym(lib, "bffi_test_roundtrip_string") as (
      handle: bigint,
      out: BigUint64Array,
    ) => number;
    const status = roundtrip(bound.handle, out);
    expect(status).toBe(ErrorCode.Ok);

    const reply = decodeUtf8(makeReadBuffer(lib)(out[0] ?? 0n));
    expect(reply).toBe("pong:echo:ping");
  } finally {
    bound.revoke();
  }
});
