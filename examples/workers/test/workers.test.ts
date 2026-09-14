/**
 * End-to-end test of the FULL `@z2net/bffi` pipeline on the workers
 * example: ONE call - cargo build -> loader JSON -> api.gen
 * generation -> binary resolution -> dlopen - then the MULTI-ISOLATE
 * surface on Bun 1.4 workers:
 *
 * 1. two JS isolates (main + worker) call the native library
 *    CONCURRENTLY (the CPU-bound `sum_to`);
 * 2. a DETACHED native thread marshals a JS-bound callback onto its
 *    OWNING isolate (`invoke_wait` targeted delivery) - the callback
 *    body provably runs in the worker, not in main;
 * 3. after the worker dies, the targeted delivery reports
 *    `LoopStopped` (the slot queue was retired with the thread).
 *
 * Run with `bun test examples/workers` from the REPO ROOT (the root
 * tsconfig paths resolve `@z2net/bffi`).
 */
import { describe, expect, test } from "bun:test";

import { ErrorCode, bffi } from "@z2net/bffi";
import type { Api } from "../.bffi/api.gen.ts";

let api: Api;

/** The pending sentinel the native side reports before an outcome. */
const WAIT_PENDING = 0xffff_ffff;

/** Waits for one named message from the worker. */
function waitFor<T extends { kind: string }>(
  worker: Worker,
  kind: string,
  timeoutMs = 10_000,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`timeout waiting for the ${kind} message`));
    }, timeoutMs);
    worker.addEventListener("message", (event: MessageEvent) => {
      const data = event.data as T & { kind?: string };
      if (data?.kind === "worker-error") {
        clearTimeout(timer);
        reject(
          new Error(
            `worker error: ${(event.data as { message?: string }).message ?? "unknown"}`,
          ),
        );
        return;
      }
      if (data?.kind === kind) {
        clearTimeout(timer);
        resolve(data as T);
      }
    });
    worker.addEventListener("error", (event: ErrorEvent) => {
      clearTimeout(timer);
      reject(event.error ?? new Error("worker error"));
    });
  });
}

describe("multi-isolate workers through the full pipeline", () => {
  // A COLD release build (LTO, whole workspace) takes minutes; bun's
  // default per-test timeout is 5s.
  test(
    "the pipeline builds, generates and loads the module",
    async () => {
      api = await bffi({ config: `${import.meta.dir}/../.bffi/bffi.json` });
      expect(api).toBeTypeOf("object");
    },
    600_000,
  );

  test(
    "two isolates run native code concurrently",
    async () => {
      const worker = new Worker(new URL("./worker.ts", import.meta.url));
      await waitFor(worker, "ready");

      // BOTH isolates call the same native export at the same time;
      // the results must be identical and correct.
      const n = 5_000_000n;
      const mainSum = api.sum_to(n);
      worker.postMessage({ kind: "sum", n });
      const { value } = await waitFor<{ kind: string; value: bigint }>(
        worker,
        "sum",
      );
      const expected = (n * (n + 1n)) / 2n;
      expect(mainSum).toBe(expected);
      expect(value).toBe(expected);
      expect(value).toBeGreaterThan(0n);
    },
    30_000,
  );

  test(
    "invoke_wait delivers a JS-bound callback to its OWNING isolate",
    async () => {
      const worker = new Worker(new URL("./worker.ts", import.meta.url));
      await waitFor(worker, "ready");

      // The worker binds a JS callback (the trampoline belongs to the
      // WORKER's isolate) and sends the handle back.
      worker.postMessage({ kind: "bind" });
      const { handle } = await waitFor<{ kind: string; handle: bigint }>(
        worker,
        "handle",
      );
      expect(handle).toBeTypeOf("bigint");
      expect(handle).not.toBe(0n);

      // A DETACHED native thread invoke_waits the callback: the job
      // is queued on the worker's slot queue and runs THERE.
      await api.spawn_invoke_wait(handle, 21, 10_000n);

      const deadline = Date.now() + 10_000;
      while (api.wait_code() === WAIT_PENDING && Date.now() < deadline) {
        await Bun.sleep(5);
      }
      expect(api.wait_code()).toBe(ErrorCode.Ok);
      expect(api.wait_value()).toBe(42); // 21 * 2, computed in the worker

      // The delivery proof: the callback body ran on the worker
      // thread, not here.
      worker.postMessage({ kind: "proof" });
      const proof = await waitFor<{ kind: string; ranHere: boolean }>(
        worker,
        "proof",
      );
      expect(proof.ranHere).toBeTrue();

      // Graceful owner death: the worker UNBINDS itself (the explicit
      // shutdown half of bind_js_thread), retiring its slot queue, so
      // the next targeted delivery reports LoopStopped
      // (ErrorCode::Error) immediately instead of timing out.
      worker.postMessage({ kind: "die" });
      const dying = await waitFor<{ kind: string; status: number }>(
        worker,
        "dying",
      );
      expect(dying.status).toBe(0);
      await worker.terminate();
      await api.spawn_invoke_wait(handle, 21, 2_000n);
      const deadDeadline = Date.now() + 5_000;
      let code = api.wait_code();
      while (code === WAIT_PENDING && Date.now() < deadDeadline) {
        await Bun.sleep(10);
        code = api.wait_code();
      }
      expect(code).toBe(ErrorCode.Error);
    },
    30_000,
  );
});
