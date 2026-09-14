/**
 * End-to-end test of the FULL `@z2net/bffi` pipeline on the callbacks
 * example: ONE call - cargo build -> loader JSON -> api.gen
 * generation -> binary resolution -> dlopen - then the callback
 * surface: the JS -> Rust lifecycle, the Rust -> JS ownership, the
 * wrong-thread rejection and the marshal delivery on a worker thread.
 *
 * Run with `bun test examples/callbacks` from the REPO ROOT (the
 * root tsconfig paths resolve `@z2net/bffi`).
 *
 * The tests are ORDER-DEPENDENT and phased ON PURPOSE: while the
 * process is UNBOUND (before the worker registers) every thread is
 * admitted; after the worker registers, unregistered threads are
 * rejected - and the phase-C worker is deliberately not unbound until
 * the final assertions. Keep the order.
 */
import { describe, expect, test } from "bun:test";
import { JSCallback, dlopen, ptr } from "bun:ffi";

import {
  TAG_I32,
  bffi,
  buildDeclarations,
  findProjectRoot,
  bindJsCallback,
  invokeCallback,
  loadConfigFile,
  localArtifactPath,
  revokeCallback,
  setJsThread,
  type FfiLib,
} from "@z2net/bffi";
import { moduleJson, type Api } from "../.bffi/api.gen.ts";

const found = await findProjectRoot(import.meta.dir);
if (found === undefined) {
  throw new Error(".bffi/bffi.json not found above the test");
}
const root = found;
const config = await loadConfigFile(root);

let api: Api;

/** The raw symbol table for the generic callback ABI
 * (`bffi_callback_*`): the JS-side helpers compose it, the typed API
 * hides it. */
let raw: FfiLib;

/** Waits for one named message from the worker. */
function waitFor(
  worker: Worker,
  kind: string,
  timeoutMs = 10_000,
): Promise<{ kind: string; bindStatus?: number; executed?: bigint }> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`timeout waiting for the ${kind} message`));
    }, timeoutMs);
    worker.addEventListener("message", (event: MessageEvent) => {
      const data = event.data as { kind?: string; bindStatus?: number; executed?: bigint };
      if (data?.kind === kind) {
        clearTimeout(timer);
        resolve(data as { kind: string; bindStatus?: number; executed?: bigint });
      }
    });
    worker.addEventListener("error", (event: ErrorEvent) => {
      clearTimeout(timer);
      reject(event.error ?? new Error("worker error"));
    });
  });
}

describe("callbacks through the full pipeline", () => {
  // A COLD release build (LTO, whole workspace) takes minutes; bun's
  // default per-test timeout is 5s.
  test(
    "the pipeline builds, generates and loads the module",
    async () => {
      api = await bffi({ config: `${import.meta.dir}/../.bffi/bffi.json` });
      expect(api).toBeTypeOf("object");
      raw = dlopen(
        localArtifactPath(config, root),
        buildDeclarations(moduleJson, config.features ?? {}),
      ).symbols;
    },
    600_000,
  );

  test("phase A: JS -> Rust register/invoke/revoke while unbound", () => {
    const handle = api.callback_register();
    expect(handle).toBeTypeOf("bigint");

    expect(invokeCallback(raw, handle, 21)).toBe(42);

    // An EMPTY argument slice against the registered i32(i32)
    // signature surfaces the signature mismatch.
    expect(() => invokeCallback(raw, handle)).toThrow(/signature mismatch/);

    // Revocation is terminal: a dead handle reports the revoked
    // error, and the SECOND revoke reports it too.
    revokeCallback(raw, handle);
    expect(() => invokeCallback(raw, handle, 21)).toThrow(/revoked/);
    expect(() => revokeCallback(raw, handle)).toThrow(/revoked/);
  });

  test("phase B: Rust -> JS storage, identity and revocation", () => {
    // A JS function wrapped into a native pointer (bun:ffi
    // JSCallback); the pointer travels to Rust as an opaque token.
    const jsCallback = new JSCallback((x: number) => x * 2, {
      args: ["i32"],
      returns: "i32",
    });
    expect(jsCallback.ptr).not.toBeNull();
    const fnPointer = jsCallback.ptr ?? 0;

    // The generic bind ABI: sig = [ret tag, params...] (wire tags).
    const sig = new Uint8Array([TAG_I32, TAG_I32]);
    const out = new BigUint64Array(1);
    const bind = raw.bffi_callback_bind;
    if (bind === undefined) {
      throw new Error("bffi_callback_bind export is missing");
    }
    const status = bind(
      sig[0] ?? 0,
      sig.length > 1 ? ptr(sig.subarray(1)) : null,
      sig.length - 1,
      BigInt(fnPointer),
      out,
    );
    expect(status).toBe(0);
    const jsHandle = out[0] ?? 0n;
    expect(jsHandle).not.toBe(0n);

    // The read-back token equals the pointer we handed over.
    expect(api.callback_ptr(jsHandle)).toBe(BigInt(fnPointer));

    // Revocation is terminal for the JS direction too.
    revokeCallback(raw, jsHandle);
    expect(() => api.callback_ptr(jsHandle)).toThrow(/revoked/);
    jsCallback.close();
  });

  test(
    "phase C: wrong-thread rejection and marshal delivery via a worker",
    async () => {
      const handle = api.callback_register();
      expect(handle).toBeTypeOf("bigint");

      const worker = new Worker(new URL("./worker.ts", import.meta.url));
      const boundMessage = await waitFor(worker, "bound");
      expect(boundMessage.bindStatus).toBe(0);

      // The main thread is now the WRONG thread: the direct invoke is
      // rejected with WrongThread (12) - both through the status
      // variant and the generic invoke ABI.
      expect(api.callback_invoke_status(handle, 1)).toBe(12);
      expect(() => invokeCallback(raw, handle, 1)).toThrow(/non-JS thread/);

      // The marshal path: the job is delivered to the worker's run()
      // loop. The runner may not be up yet when "bound" arrives (the
      // worker posts before entering run()), so retry until the
      // marshal is accepted; marshal without a runner reports 12.
      const deadline = Date.now() + 10_000;
      let marshalStatus = api.marshal_invoke(handle, 21);
      while (marshalStatus !== 0 && Date.now() < deadline) {
        await Bun.sleep(5);
        marshalStatus = api.marshal_invoke(handle, 21);
      }
      expect(marshalStatus).toBe(0);

      // The job runs ON THE WORKER THREAD (where invoke passes the
      // JS-thread gate) and stores 42 into the process-wide slot.
      while (api.last_invoked() !== 42 && Date.now() < deadline) {
        await Bun.sleep(5);
      }
      expect(api.last_invoked()).toBe(42);

      // Shutdown: the sticky stop unblocks the worker's run(), the
      // worker reports its executed count (exactly the one marshalled
      // job - retries that saw "no runner" never enqueued), then
      // terminate() is belt and suspenders.
      api.loop_stop();
      const done = await waitFor(worker, "loop-done");
      expect(done.executed).toBe(1n);
      worker.terminate();

      // Multi-isolate (Bun 1.4): a late setJsThread from the main
      // thread REGISTERS it as a second JS thread - that is how a
      // Worker joins. The old sticky single-binding policy is gone;
      // only UNREGISTERED threads are rejected.
      setJsThread(raw);
      // And now the main thread passes the JS-thread gate for
      // isolate-independent native callbacks:
      expect(api.callback_invoke_status(handle, 1)).toBe(0);
    },
    20_000,
  );

  test("bindJsCallback supports the dispose protocol", () => {
    const bound = bindJsCallback(
      raw,
      { ret: "i32", params: ["i32"] },
      (x) => (x as number) + 1,
    );
    expect(bound.handle).not.toBe(0n);
    // Dispose = revoke + JSCallback close: the native handle is dead.
    bound[Symbol.dispose]();
    expect(() => api.callback_ptr(bound.handle)).toThrow(/revoked/);
    // A second dispose is a no-op (idempotent revoke).
    bound[Symbol.dispose]();
  });
});
