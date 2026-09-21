/**
 * E2E: the multi-isolate Worker proof - two real Bun Workers drive
 * JS-bound callbacks through the REAL bffi-native cdylib with
 * TARGETED delivery:
 *
 * 1. each Worker registers its OWN thread (`bffi_callback_set_thread`)
 *    and binds a `(string) -> string` callback whose reply carries a
 *    Worker-unique marker (`A:echo:...` / `B:echo:...`);
 * 2. the main isolate calls the test-only `bffi_test_invoke_on_thread`
 *    export: a PLAIN native thread runs `invoke_wait` there, so the
 *    job is marshaled TARGETED to the callback's owning Worker - the
 *    main thread never pumps, only the Worker's `bffi_test_pump`
 *    timer drains its own slot queue;
 * 3. every reply must carry the marker of the Worker the call was
 *    dispatched to (alternating A/B), never the wrong one.
 *
 * The worker script is generated and written to a temp directory by
 * the test itself (Bun.write) - it is not a tracked file.
 *
 * Gated behind `BFFI_E2E=1` AND the presence of the release cdylib
 * (`cargo build --release -p bffi-native`); skipped otherwise, so the
 * regular `bun test packages` suite never depends on a build.
 *
 * Run with `BFFI_E2E=1 bun test packages/bffi/test/e2e-workers.test.ts`.
 */
import { expect, test } from "bun:test";
import { dlopen } from "bun:ffi";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import {
  ErrorCode,
  decodeUtf8,
  makeReadBuffer,
  setJsThread,
  sym,
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

/** The raw bun:ffi declarations: the runtime ABI, the generic
 * callback ABI, and the two test-only exports. Same table as
 * `e2e-string-return.test.ts`, extended with `bffi_callback_unset_thread`,
 * `bffi_test_invoke_on_thread` and `bffi_test_pump`. */
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
  bffi_callback_unset_thread: { args: [], returns: "u32" },
  bffi_callback_bind: { args: ["u8", "ptr", "u64", "u64", "pointer"], returns: "u32" },
  bffi_callback_revoke: { args: ["u64"], returns: "u32" },
  bffi_test_invoke_on_thread: { args: ["u64", "pointer"], returns: "u32" },
  bffi_test_pump: { args: [], returns: "u64" },
} as const;

/** One message of the worker protocol. */
interface WorkerMessage {
  kind: string;
  handle?: bigint;
  message?: string;
}

/** Waits for one named message from a worker (the workers-example
 * helper, reduced to this suite's protocol). */
function waitFor(worker: Worker, kind: string, timeoutMs = 10_000): Promise<WorkerMessage> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`timeout waiting for the ${kind} message`));
    }, timeoutMs);
    worker.addEventListener("message", (event: MessageEvent) => {
      const data = event.data as WorkerMessage;
      if (data?.kind === "worker-error") {
        clearTimeout(timer);
        reject(new Error(`worker error: ${data.message ?? "unknown"}`));
        return;
      }
      if (data?.kind === kind) {
        clearTimeout(timer);
        resolve(data);
      }
    });
    worker.addEventListener("error", (event: ErrorEvent) => {
      clearTimeout(timer);
      reject(event.error ?? new Error("worker error"));
    });
  });
}

/** Generates the worker-isolate script: dlopens the SAME cdylib
 * (workers are threads of one process - the tables are shared, the
 * JSCallback trampolines belong to THIS isolate), registers its own
 * JS thread, pumps its slot queue on a timer, binds one callback
 * whose reply carries the worker-unique marker, and revokes +
 * unregisters on demand. */
function workerScript(): string {
  const pkgUrl = pathToFileURL(join(import.meta.dir, "..", "src", "index.ts")).href;
  return `
import { dlopen } from "bun:ffi";
import { bindJsCallback, setJsThread, unsetJsThread } from ${JSON.stringify(pkgUrl)};

const lib = dlopen(${JSON.stringify(CDYLIB_PATH)}, ${JSON.stringify(DECLARATIONS)}).symbols;

setJsThread(lib);

const pump = lib.bffi_test_pump;
const timer = setInterval(() => {
  pump();
}, 1);

let bound;

self.onmessage = (event) => {
  const data = event.data;
  try {
    if (data.kind === "bind") {
      bound = bindJsCallback(
        lib,
        { ret: "string", params: ["string"] },
        (s) => data.marker + ":echo:" + String(s),
      );
      postMessage({ kind: "handle", handle: bound.handle });
    } else if (data.kind === "die") {
      clearInterval(timer);
      unsetJsThread(lib);
      bound.revoke();
      postMessage({ kind: "dying" });
    }
  } catch (error) {
    postMessage({
      kind: "worker-error",
      message: error instanceof Error ? error.message : String(error),
    });
  }
};

postMessage({ kind: "ready" });
`;
}

test.skipIf(!GATE)(
  "e2e: invoke_wait delivers JS-bound callbacks TARGETED across two Worker isolates",
  async () => {
    const lib: FfiLib = dlopen(CDYLIB_PATH, DECLARATIONS).symbols as FfiLib;

    // Main registers too - and never pumps: a correct targeted
    // delivery can only be executed by the owning Worker's drain.
    setJsThread(lib);

    const dir = await mkdtemp(join(tmpdir(), "bffi-e2e-workers-"));
    const workerPath = join(dir, "worker.mjs");
    await writeFile(workerPath, workerScript());

    // Spawn one isolate per marker; each binds INSIDE the worker and
    // posts the bound handle back (the trampoline belongs to that
    // isolate; the bind records the WORKER's thread on the entry).
    const spawn = async (marker: string) => {
      const worker = new Worker(workerPath);
      await waitFor(worker, "ready");
      // oxlint-disable-next-line unicorn/require-post-message-target-origin -- Worker.postMessage takes no targetOrigin
      worker.postMessage({ kind: "bind", marker });
      const { handle } = await waitFor(worker, "handle");
      expect(handle).toBeTypeOf("bigint");
      if (typeof handle !== "bigint") {
        throw new Error("the worker did not report a bigint handle");
      }
      expect(handle).not.toBe(0n);
      return { worker, handle };
    };

    const a = await spawn("A");
    const b = await spawn("B");

    const invokeOnThread = sym(lib, "bffi_test_invoke_on_thread") as (
      handle: bigint,
      out: BigUint64Array,
    ) => number;
    const readReply = makeReadBuffer(lib);

    /** One blocking dispatch onto a fresh native thread; the reply
     * must carry exactly the target's marker. */
    const dispatch = (target: { handle: bigint }, expected: string) => {
      const out = new BigUint64Array(1);
      // Blocks this (JS) thread until the native thread's
      // invoke_wait lands the reply or times out (10s).
      const status = invokeOnThread(target.handle, out);
      expect(status).toBe(ErrorCode.Ok);
      const reply = decodeUtf8(readReply(out[0] ?? 0n));
      expect(reply).toBe(expected);
    };

    try {
      // Alternating A/B: each reply is produced by the callback bound
      // in the OTHER isolate - the marker proves which isolate ran.
      await dispatch(a, "pong:A:echo:ping");
      await dispatch(b, "pong:B:echo:ping");
      await dispatch(a, "pong:A:echo:ping");
      await dispatch(b, "pong:B:echo:ping");
    } finally {
      for (const target of [a, b]) {
        try {
          // oxlint-disable-next-line unicorn/require-post-message-target-origin -- Worker.postMessage takes no targetOrigin
          target.worker.postMessage({ kind: "die" });
          await waitFor(target.worker, "dying", 2_000);
        } catch {
          // The assertion failure (if any) must not be masked by a
          // cleanup hiccup; terminate() is the backstop either way.
        }
        target.worker.terminate();
      }
      await rm(dir, { recursive: true, force: true });
    }
  },
  60_000,
);
