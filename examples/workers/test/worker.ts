/**
 * The worker isolate of the workers e2e: a real Bun 1.4 worker
 * thread hosting its OWN JS isolate over the SAME native library.
 *
 * Bun workers are threads of the SAME process: the dlopen'ed library,
 * the JS-thread table and the slot queues are shared with the main
 * thread - but the JSCallback trampolines created here belong to THIS
 * isolate, which is exactly what the delivery test proves.
 *
 * Protocol (messages from the main thread):
 * - `{ kind: "sum", n }`  - run the CPU-bound native export on this
 *   thread. Replies `{ kind: "sum", value }`.
 * - `{ kind: "bind" }`    - bind a JS doubling callback HERE (its
 *   trampoline belongs to this isolate). Replies
 *   `{ kind: "handle", handle }`.
 * - `{ kind: "proof" }`   - reply `{ kind: "proof", ranHere }` -
 *   whether the bound callback body ran on THIS thread (the
 *   cross-isolate delivery proof).
 *
 * The pump interval is this isolate's event-loop tick: it drains the
 * slot queue (targeted deliveries) and the global queue. Errors are
 * reported as `{ kind: "worker-error", message }` instead of dying
 * silently.
 */
import { dlopen } from "bun:ffi";

import {
  bindJsCallback,
  buildDeclarations,
  findProjectRoot,
  loadConfigFile,
  localArtifactPath,
  setJsThread,
  unsetJsThread,
  type FfiLib,
} from "@z2net/bffi";
import { createApiFromJson, moduleJson, type Api } from "../.bffi/api.gen.ts";

// Worker-global scope typing (bun-types does not predeclare `self`
// inside a plain module).
declare var self: Worker;

const found = await findProjectRoot(import.meta.dir);
if (found === undefined) {
  throw new Error(".bffi/bffi.json not found above the worker");
}
const root = found;
const config = await loadConfigFile(root);
const api: Api = createApiFromJson(localArtifactPath(config, root));
const raw: FfiLib = dlopen(
  localArtifactPath(config, root),
  buildDeclarations(moduleJson, config.features ?? {}),
).symbols;

// Multi-isolate registration: THIS worker thread joins the JS-thread
// table (the main thread registered itself separately).
const bindStatus = api.bind_js_thread();
if (bindStatus !== 0) {
  throw new Error(`bind_js_thread failed: ${String(bindStatus)}`);
}
setJsThread(raw);

// This isolate's event-loop tick: targeted deliveries land in this
// thread's slot queue and run HERE.
setInterval(() => {
  api.loop_pump();
}, 1);

postMessage({ kind: "ready" });

let ranHere = false;

self.onmessage = (event: MessageEvent) => {
  const data = event.data as { kind: string; n?: bigint };
  try {
    switch (data.kind) {
      case "sum": {
        const value = api.sum_to(data.n ?? 0n);
        postMessage({ kind: "sum", value });
        break;
      }
      case "bind": {
        ranHere = false;
        const bound = bindJsCallback(
          raw,
          { ret: "i32", params: ["i32"] },
          (x) => {
            ranHere = true;
            return (x as number) * 2;
          },
        );
        postMessage({ kind: "handle", handle: bound.handle });
        break;
      }
      case "proof": {
        postMessage({ kind: "proof", ranHere });
        break;
      }
      case "die": {
        // Explicit deregistration: the slot queue retires NOW, so the
        // next targeted delivery to this isolate fails fast
        // (LoopStopped) even though terminate() never runs TLS
        // destructors.
        const status = api.unbind_js_thread();
        unsetJsThread(raw);
        postMessage({ kind: "dying", status });
        break;
      }
    }
  } catch (error) {
    postMessage({
      kind: "worker-error",
      message: error instanceof Error ? error.message : String(error),
    });
  }
};
