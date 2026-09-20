/**
 * Async task delivery: an `#[bffi_async]` export returns a task
 * handle (`u64`); the promise is settled by resolve/reject JSCallbacks
 * the native side invokes on the JS thread while the event loop is
 * pumped (DESIGN §7 - the Bun tick is loader-side, so keep pumping
 * until the promise settles; `pumpUntil` exists for exactly that).
 */
import { JSCallback } from "bun:ffi";
import type { FfiLib } from "./error.ts";
import { ErrorCode, makeTakeError, sym } from "./error.ts";
import { makeReadBuffer } from "./buffer.ts";
import { decodeValue, type WireValue } from "./wire.ts";
import { isCompositeTs, tablesOf, wireToJs } from "../loader/composite.ts";
import type { ModuleJson } from "../loader/loader.ts";

/**
 * Wraps a bffi-async task handle into a JS `Promise`. The
 * resolve/reject callbacks are handed to the native side as
 * `bun:ffi` JSCallback pointers; the value is decoded from the
 * transient-buffer payload, the rejection carries the native message.
 *
 * When `retTs` + `json` are given (the typed loader path), a
 * composite return (`Promise<Sample>` / `Promise<number[]>`) maps
 * through the module's composite tables exactly like the sync
 * buffer channel.
 */
export function wrapTask<T extends WireValue = WireValue>(
  lib: FfiLib,
  task: bigint,
  retTs?: string,
  json?: ModuleJson,
): Promise<T> {
  const readBuffer = makeReadBuffer(lib);
  return new Promise<T>((resolve, reject) => {
    let settled = false;
    const resolveCb = new JSCallback(
      (valueHandle: bigint) => {
        if (settled) {
          return;
        }
        settled = true;
        try {
          const decoded = decodeValue(readBuffer(valueHandle));
          if (retTs !== undefined && json !== undefined) {
            // The ret ts is the promised spelling (`Promise<Sample>` /
            // `Promise<Sample | null>`): the wire payload is the
            // inner value's encoding.
            const inner = retTs.startsWith("Promise<") && retTs.endsWith(">")
              ? retTs.slice("Promise<".length, -1)
              : retTs;
            // `Option::None` rides the unit record (undefined) and
            // maps to `null` for the `| null` spellings.
            if (inner.endsWith(" | null") && decoded === undefined) {
              resolve(null as unknown as T);
              return;
            }
            const tables = tablesOf(json);
            if (isCompositeTs(inner, tables)) {
              const bare = inner.endsWith(" | null")
                ? inner.slice(0, -" | null".length)
                : inner;
              resolve(wireToJs(tables, bare, decoded, "task") as T);
              return;
            }
          }
          resolve(decoded as T);
        } catch (error) {
          reject(error as Error);
        }
      },
      { args: ["u64"], returns: "void" },
    );
    const rejectCb = new JSCallback(
      (message: string) => {
        if (settled) {
          return;
        }
        settled = true;
        reject(new Error(message));
      },
      { args: ["cstring"], returns: "void" },
    );
    if (resolveCb.ptr === null || rejectCb.ptr === null) {
      throw new Error("bun:ffi produced a null JSCallback pointer");
    }
    const out = new Uint32Array(1);
    const status = sym(lib, "bffi_async_attach")(
      task,
      BigInt(resolveCb.ptr),
      BigInt(rejectCb.ptr),
      out,
    );
    if (status !== ErrorCode.Ok) {
      const takeError = makeTakeError(lib);
      reject(takeError() ?? new Error(`bffi_async_attach failed: ${String(status)}`));
    }
  });
}

/**
 * Awaits a task promise while pumping the native event loop: calls
 * `pump` (the non-blocking drain, e.g. a generated `loopPump`
 * export) after every macrotask tick until the promise settles.
 *
 * The pump contract stays explicit (the loader never starts a hidden
 * interval): pass the pump export you were handed, or a no-op when
 * the task settles without loop delivery.
 */
export async function pumpUntil<T>(promise: Promise<T>, pump: () => unknown): Promise<T> {
  const settled = Promise.withResolvers<true>();
  void promise.then(() => settled.resolve(true), () => settled.resolve(true));
  for (;;) {
    // The loop condition is the race result itself: the settle signal
    // (fired from the promise callbacks above) versus a fresh tick,
    // so no iteration ever reads a flag the loop cannot see.
    if (await Promise.race([settled.promise, Promise.resolve(false)])) {
      break;
    }
    pump();
    // Yield a macrotask so the delivery job the pump just executed
    // (which settles the promise through its JSCallback) runs.
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  return promise;
}
