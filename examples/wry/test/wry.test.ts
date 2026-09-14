/**
 * End-to-end test of the FULL `@z2net/bffi` pipeline on the wry
 * example: ONE call - cargo build -> loader JSON -> api.gen
 * generation -> binary resolution -> dlopen - then the real thing:
 * a webview window on a dedicated native thread, the UI -> ipc ->
 * `invoke_wait` -> JS callback -> `evaluate_script` resolve
 * roundtrip, and the clean shutdown.
 *
 * WINDOWED BY DESIGN: it opens a real OS window, so it is
 * ENV-GATED - skipped entirely unless `BFFI_WRY_E2E=1`:
 *
 *   BFFI_WRY_E2E=1 bun test examples/wry   # from the REPO ROOT
 *
 * (The WebView2 runtime must be installed on Windows.) The Rust
 * unit tests of the example (`cargo test -p bffi-example-wry`)
 * cover the no-window logic and always run.
 */
import { describe, expect, test } from "bun:test";
import { JSCallback, dlopen, ptr } from "bun:ffi";

import {
  TAG_STR,
  TAG_UNIT,
  bffi,
  buildDeclarations,
  findProjectRoot,
  loadConfigFile,
  localArtifactPath,
  pumpUntil,
  setJsThread,
  type FfiLib,
} from "@z2net/bffi";
import { moduleJson, type Api } from "../.bffi/api.gen.ts";

const E2E = process.env.BFFI_WRY_E2E === "1";
const maybeTest = test.skipIf(!E2E);

const found = await findProjectRoot(import.meta.dir);
if (found === undefined) {
  throw new Error(".bffi/bffi.json not found above the test");
}
const root = found;
const config = await loadConfigFile(root);

let api: Api;

/** The raw symbol table for the generic callback ABI
 * (`bffi_callback_*`): `setJsThread`/`bindJsCallback` compose it,
 * the typed API hides it. */
let raw: FfiLib;

/** The webview handle, set by the roundtrip test (the callback
 * closes over it). */
let handle: bigint | undefined;

/** The minimal page: two roundtrips. The FIRST starts from the page
 * itself and retries until the JS side has bound its ipc callback
 * (the 2s bootstrap timeout makes a too-early attempt reject) - so
 * page load and callback binding never race. The SECOND is driven
 * by `webview_eval` on the known-good DOM. */
const PAGE_HTML = `<!doctype html>
<html>
  <head><meta charset="utf-8"><title>bffi wry</title></head>
  <body>
    <button id="ping">ping</button>
    <button id="again">again</button>
    <script>
      const send = async (arg) => {
        const reply = await window.__bffiCall("echo", [arg]);
        window.ipc.postMessage(JSON.stringify({
          method: "resolved",
          args: [JSON.stringify(reply)],
        }));
      };
      document.getElementById("ping").addEventListener("click", () => send("ping"));
      document.getElementById("again").addEventListener("click", () => send("eval"));
      // Retry the first roundtrip until the ipc callback is bound.
      (async () => {
        while (true) {
          try {
            await send("ping");
            return;
          } catch (error) {
            await new Promise((r) => setTimeout(r, 100));
          }
        }
      })();
    </script>
  </body>
</html>`;

describe("wry through the full pipeline", () => {
  // A COLD release build (LTO, the winit/wry/windows stack, whole
  // workspace) takes minutes; bun's default per-test timeout is 5s.
  maybeTest(
    "the pipeline builds, generates and loads the wry module",
    async () => {
      api = await bffi({ config: `${import.meta.dir}/../.bffi/bffi.json` });
      expect(api).toBeTypeOf("object");
      raw = dlopen(
        localArtifactPath(config, root),
        buildDeclarations(moduleJson, config.features ?? {}),
      ).symbols;
    },
    900_000,
  );

  maybeTest(
    "the IPC roundtrip crosses threads and the shutdown is clean",
    async () => {
      // Bind THIS thread as the process-wide JS thread: the
      // loop-thread `invoke_wait` marshals onto it, and the pump
      // below delivers the job. The first binder wins (sticky).
      setJsThread(raw);

      // The handler body runs ON this thread while the test pumps:
      // the request body arrives as the cstring argument, the reply
      // goes back through `webview_ipc_reply`. The "resolved" method
      // proves the page promise WAS resolved (the native
      // `evaluate_script` leg of the roundtrip).
      const resolvers: Array<(reply: string) => void> = [];
      let settledCount = 0;
      const replyAt = (n: number): Promise<string> =>
        new Promise<string>((resolve) => {
          resolvers[n] = resolve;
        });
      const first = replyAt(0);
      const handler = new JSCallback((request: string) => {
        if (handle === undefined) {
          return;
        }
        const message = JSON.parse(request) as { method: string; args: string[] };
        if (message.method === "echo") {
          api.webview_ipc_reply(handle, JSON.stringify({ pong: message.args[0] }));
        } else if (message.method === "resolved") {
          const resolve = resolvers[settledCount];
          settledCount += 1;
          resolve?.(message.args[0] ?? "");
        }
      }, { args: ["cstring"], returns: "void" });
      if (handler.ptr === null) {
        throw new Error("bun:ffi produced a null JSCallback pointer");
      }

      // Bind the handler through the generic callback ABI (the
      // examples/callbacks pattern): sig = unit(str) - a void
      // return over one cstring parameter - and hand the HANDLE
      // (not the raw pointer) to `webview_bind_ipc`.
      const bind = raw.bffi_callback_bind;
      if (bind === undefined) {
        throw new Error("bffi_callback_bind export is missing");
      }
      const sig = new Uint8Array([TAG_UNIT, TAG_STR]);
      const bindOut = new BigUint64Array(1);
      const bindStatus = bind(
        sig[0] ?? 0,
        ptr(sig.subarray(1)),
        sig.length - 1,
        BigInt(handler.ptr),
        bindOut,
      );
      expect(bindStatus).toBe(0);
      const ipcHandle = bindOut[0] ?? 0n;
      expect(ipcHandle).not.toBe(0n);

      handle = api.webview_open({
        html: PAGE_HTML,
        title: "bffi wry",
        width: null,
        height: null,
        devtools: null,
        url: null,
      });
      expect(handle).toBeTypeOf("bigint");
      api.webview_bind_ipc(handle, ipcHandle);

      /** Awaits `promise` while pumping the loop, with a hard
       * deadline so a broken bridge fails instead of hanging. */
      const withPump = (promise: Promise<string>): Promise<string> => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const deadline = new Promise<never>((_, rejectDeadline) => {
          timer = setTimeout(
            () => rejectDeadline(new Error("the IPC roundtrip never completed")),
            30_000,
          );
        });
        return Promise.race([pumpUntil(promise, () => api.loop_pump()), deadline]).finally(() => {
          clearTimeout(timer);
        });
      };

      // Roundtrip 1 (page-driven, retries until the bind landed):
      // page click -> ipc handler (loop thread) -> invoke_wait (this
      // thread pumps) -> reply -> resolve script -> page promise ->
      // "resolved" follow-up message.
      expect(await withPump(first)).toBe(JSON.stringify({ pong: "ping" }));

      // Roundtrip 2 (eval-driven, on the known-good DOM): proves the
      // `webview_eval` export reaches the page.
      const second = replyAt(1);
      api.webview_eval(handle, `document.getElementById("again").click()`);
      expect(await withPump(second)).toBe(JSON.stringify({ pong: "eval" }));

      // Shutdown: the last close stops the loop thread, and the
      // sticky flag reports it.
      api.webview_close(handle);
      handler.close();

      const exitDeadline = Date.now() + 10_000;
      while (!api.webview_poll_exit() && Date.now() < exitDeadline) {
        await Bun.sleep(20);
      }
      expect(api.webview_poll_exit()).toBeTrue();
      handle = undefined;
    },
    120_000,
  );
});
