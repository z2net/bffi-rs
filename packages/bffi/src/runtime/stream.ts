/**
 * Stream delivery (B2.1, pull-chunk): a `#[bffi_stream]` export
 * returns a stream handle (`u64`); the generic `bffi_stream_next`
 * export pulls up to `max` items as one `TAG_SEQ` transient buffer
 * (`0` handle = exhausted) and `bffi_stream_drop` releases the
 * iterator early. The wrapper turns the handle into a JS
 * `AsyncIterableIterator` - `for await (const item of ...)` works -
 * with an adaptive chunk budget and a GC finalizer for the drop.
 */
import type { FfiLib } from "./error.ts";
import { ErrorCode, sym, symOptional } from "./error.ts";
import { makeReadBuffer } from "./buffer.ts";
import { decodeAt } from "./wire.ts";
import { tablesOf, wireToJs, type CompositeTables } from "../loader/composite.ts";
import type { ModuleJson } from "../loader/loader.ts";
import { JSCallback } from "bun:ffi";

/** The initial chunk budget; doubled on full chunks, capped here. */
const INITIAL_MAX = 32;
const MAX_BUDGET = 1024;

/** The pending-wait fallback when a wake fires between the Pending
 * return and the wait registration (a lost wake-up must not hang
 * the pull; the wake makes the common case instant). */
const PENDING_FALLBACK_MS = 4;

/**
 * Wraps a bffi stream handle into a JS `AsyncIterableIterator`.
 * Items decode through the module's composite tables by `itemTs`
 * (the inner type of the descriptor's
 * `AsyncIterableIterator<T>` return).
 *
 * A push producer's wake trampoline (best-effort, older builds
 * without `bffi_stream_set_wake` keep the poll-retry contract)
 * resolves the Pending wait instantly through the event loop; the
 * bounded timer stays as the lost-wake-up fallback.
 */
export function wrapStream<T = unknown>(
  lib: FfiLib,
  handle: bigint,
  itemTs: string,
  json: ModuleJson,
): AsyncIterableIterator<T> {
  const readBuffer = makeReadBuffer(lib);
  const tables: CompositeTables = tablesOf(json);
  const nextSym = sym(lib, "bffi_stream_next");
  const dropSym = sym(lib, "bffi_stream_drop");
  let queue: unknown[] = [];
  let done = false;
  let max = INITIAL_MAX;

  // The wake trampoline: resolved whenever the native side signals
  // that a waiting pull can resume (an item landed in an empty
  // buffer, or the producer completed/failed). The callback is
  // never closed while the stream can still fire - a queued wake
  // job after a close would call a dangling trampoline.
  const setWake = symOptional(lib, "bffi_stream_set_wake");
  let wakeNotify: (() => void) | null = null;
  let wakeCb: JSCallback | null = null;
  if (setWake) {
    const cb = new JSCallback(
      () => {
        wakeNotify?.();
      },
      { args: ["u64"], returns: "void" },
    );
    if (cb.ptr !== null) {
      wakeCb = cb;
      setWake(handle, BigInt(cb.ptr));
    }
  }

  const waitForWake = async (): Promise<void> => {
    if (wakeCb === null) {
      await new Promise((resolve) => setTimeout(resolve, 1));
      return;
    }
    await new Promise<void>((resolve) => {
      wakeNotify = resolve;
      setTimeout(() => {
        wakeNotify = null;
        resolve();
      }, PENDING_FALLBACK_MS);
    });
  };

  const registry = new FinalizationRegistry((finalizeHandle: bigint) => {
    try {
      sym(lib, "bffi_stream_drop")(finalizeHandle);
    } catch {
      // The library may already be closed; the GC drop is best
      // effort by contract.
    }
  });
  const token = {};
  registry.register(token, handle);

  const iterator: AsyncIterableIterator<T> = {
    [Symbol.asyncIterator](): AsyncIterableIterator<T> {
      return iterator;
    },
    next(): Promise<IteratorResult<T>> {
      const pull = async (): Promise<IteratorResult<T>> => {
        // Poll contract: an empty live buffer reports Pending - the
        // wake trampoline (when registered) resumes the wait the
        // moment the producer delivers; the bounded timer covers a
        // lost wake-up.
        while (queue.length === 0 && !done) {
          const out = new BigUint64Array(1);
          const status = nextSym(handle, max, out);
          if (status === ErrorCode.Pending) {
            await waitForWake();
            continue;
          }
          if (status !== ErrorCode.Ok) {
            throw new Error(`bffi_stream_next failed: ${String(status)}`);
          }
          const chunkHandle = out[0] ?? 0n;
          if (chunkHandle === 0n) {
            done = true;
            break;
          }
          const bytes = readBuffer(chunkHandle);
          const decoded = decodeAt(bytes, 0).value;
          if (!Array.isArray(decoded)) {
            throw new Error("stream chunk payload is not a sequence");
          }
          for (const [index, raw] of decoded.entries()) {
            // Error items arrive as Error instances - yielded as-is.
            queue.push(
              raw instanceof Error
                ? raw
                : wireToJs(tables, itemTs, raw, `stream[${String(index)}]`),
            );
          }
          if (decoded.length === max && max < MAX_BUDGET) {
            max = Math.min(max * 2, MAX_BUDGET);
          }
        }
        if (queue.length > 0) {
          const value = queue.shift() as T;
          return { value, done: false };
        }
        return { value: undefined as T, done: true };
      };
      return pull();
    },
    return(value?: T): Promise<IteratorResult<T>> {
      // Early exit: release the native stream; further next() calls
      // report done (the handle drop is sticky by contract).
      if (!done) {
        dropSym(handle);
        done = true;
        queue = [];
      }
      return Promise.resolve({ value: value as T, done: true });
    },
  };
  void token;
  return iterator;
}

/** Extracts the item type from an `AsyncIterableIterator<T>` ts
 * name (also the Result form `...<T | Error>`); `null` when the
 * name is not a stream type. */
export function streamItemTs(ts: string): string | null {
  const match = /^AsyncIterableIterator<(.+?)>(?: \| Error)?$/.exec(ts);
  if (!match) {
    return null;
  }
  let item = match[1] ?? "";
  if (item.endsWith(" | Error")) {
    item = item.slice(0, -" | Error".length);
  }
  return item;
}
