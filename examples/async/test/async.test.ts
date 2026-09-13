/**
 * End-to-end test of the FULL `@z2net/bffi` pipeline on the async
 * example: ONE call - cargo build -> loader JSON -> api.gen
 * generation -> binary resolution -> dlopen - then the whole async
 * surface: Rust futures awaited as Promises, string/domain/panic/
 * timeout deliveries, cancellation from JS, and the pumping contract
 * (promises settle only while the JS thread drains the event loop).
 *
 * Run with `bun test examples/async` from the REPO ROOT (the root
 * tsconfig paths resolve `@z2net/bffi`).
 */
import { describe, expect, test } from "bun:test";
import { dlopen } from "bun:ffi";

import {
  bffi,
  buildDeclarations,
  findProjectRoot,
  loadConfigFile,
  localArtifactPath,
  pumpUntil,
  wrapTask,
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

/** The raw symbol table: `wrapTask` composes the promise wrapper from
 * the runtime + async built-ins, which the typed API hides. */
let raw: FfiLib;

/** Awaits `promise` while pumping the event loop - the delivery job
 * executes during the drain - with a hard deadline so a broken
 * delivery fails the test instead of hanging it. */
function withPump<T>(promise: Promise<T>, timeoutMs = 5000): Promise<T> {
  const deadline = new Promise<never>((_, rejectTimeout) => {
    setTimeout(() => rejectTimeout(new Error("timed out waiting for the task")), timeoutMs);
  });
  return Promise.race([pumpUntil(promise, () => api.loop_pump()), deadline]);
}

describe("async through the full pipeline", () => {
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

  test("double_async resolves while the test pumps the loop", async () => {
    await expect(withPump(api.double_async(5n))).resolves.toBe(10n);
  });

  test("string results decode from the transient-buffer payload", async () => {
    await expect(withPump(api.shout_async("async"))).resolves.toBe("HELLO async!");
  });

  test("composite results decode through the record table (Promise<Report>)", async () => {
    await expect(withPump(api.report_async(7n))).resolves.toEqual({
      value: 7n,
      label: "report-7",
    });
  });

  test("sequence results decode as arrays (Promise<number[]>)", async () => {
    await expect(withPump(api.ticks_async(3))).resolves.toEqual([0, 1, 2]);
  });

  test("optional results map None to null (Promise<Report | null>)", async () => {
    await expect(withPump(api.maybe_report(9n))).resolves.toEqual({
      value: 9n,
      label: "report-9",
    });
    await expect(withPump(api.maybe_report(0n))).resolves.toBeNull();
  });

  test("a failing task rejects with the domain message", async () => {
    await expect(withPump(api.fail_async())).rejects.toThrow("domain failure");
  });

  test("a panicking task rejects with the panic message", async () => {
    await expect(withPump(api.panic_async())).rejects.toThrow("async boom");
  });

  test("a timeout cancels the inner future and rejects", async () => {
    await expect(withPump(api.timed_async())).rejects.toThrow("task timed out");
  });

  test("resolutions are delivered by pumping, not by magic", async () => {
    let settled = false;
    const promise = api.double_async(21n).then((value) => {
      settled = true;
      return value;
    });
    // No pump: the completed future is queued, not delivered.
    await Bun.sleep(60);
    expect(settled).toBeFalse();
    await expect(withPump(promise)).resolves.toBe(42n);
    expect(settled).toBeTrue();
  });

  test("a raw task can be cancelled from JS before wrapping", async () => {
    const task = api.spawn_slow(60_000n);
    expect(task).toBeTypeOf("bigint");
    const promise = wrapTask(raw, task);
    // A second attach on the same task is rejected.
    expect(() => wrapTask(raw, task)).toThrow(/already attached/);
    expect(api.cancel_task(task)).toBe(1);
    await expect(withPump(promise)).rejects.toThrow("task cancelled");
  });

  test("every task reached a terminal state", () => {
    expect(api.async_pending()).toBe(0n);
  });
});
