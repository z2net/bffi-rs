# examples/wry - a real webview window driven from Bun

The Phase 3 maturity proof: a **wry** (webview) binding as a bffi
native module. The window, the winit event loop and the wry surface
live on a DEDICATED NATIVE THREAD spawned inside the cdylib; the Bun
thread stays free and drains the bffi event loop. UI -> native ->
JS -> native -> UI roundtrips run over the generic callback ABI's
marshal-and-wait (`bffi::invoke_wait`).

## File map

```
examples/wry/
├── Cargo.toml            # bffi (facade) + wry =0.57.0 + winit =0.30.13
├── .bffi/                # bffi.json (features: runtime + callbacks), loader JSON, api.gen.ts
├── src/
│   ├── lib.rs            # the surface + the loop thread + the IPC bridge
│   ├── module_def.rs     # the single ModuleDef aggregation (WebviewConfig record)
│   └── bin/emit_json.rs  # writes .bffi/bffi.api.json
└── test/
    └── wry.test.ts       # env-gated e2e (WINDOWED by design - see below)
```

ABI macros expanded in the crate (same contract as the other
examples): `bffi_runtime_abi!()` (the error drain + buffer pair) and
`bffi_callback_abi!()` (`bffi_callback_set_thread` / `_bind` /
`_invoke` / `_revoke`).

## The threading model

```
 Bun thread (JS)                     bffi-wry-loop thread (native)
 ---------------                     ------------------------------
 setJsThread(raw)  (binds THIS
  thread as THE JS thread)
 webview_open(config)  --Create-->  winit EventLoop + Window + wry
        (waits, bounded)             WebView built HERE;
 webview_bind_ipc(handle, ptr)       ipc_handler wired HERE
 webview_eval(handle, js) --Eval---> evaluate_script
                                     .
                          page click: window.ipc.postMessage(JSON)
                                     .
                                     ipc handler (loop thread):
                                     1. box.request = body
                                     2. invoke_wait(forwarder, [], 30s)
                                        - the loop thread PARKS -
 webview_ipc_reply(handle, reply)    3. [JS thread, during pump]:
        ^                                marshal job runs the forwarder
        |                                body ON THE JS THREAD, which
        |                                calls the JSCallback pointer
        |                                synchronously (the request is
        |                                its cstring argument); the JS
        |                                handler computes the reply and
        |                                stores it in the box
                                     4. the loop thread wakes, reads
                                        box.reply and queues an Eval:
                                        window.__bffiResolve(<reply JSON>)
                          the page promise settles; a follow-up
                          "resolved" IPC message proves it
 webview_close(handle)  --Close--->  drop the webview + window; the
 webview_poll_exit() -> true         LAST close exits the loop and
                                     finishes the thread
```

Key points:

- **The loop thread is spawned on the first `webview_open`** and
  published through a global proxy slot; every window/webview
  request from JS crosses via `EventLoopProxy::send_event` (the only
  legal cross-thread door into winit). The thread exits when the
  last webview closes; a new `webview_open` spawns a fresh one.
- **`invoke_wait` is the synchronization backbone.** The ipc handler
  parks the loop thread; the JS side delivers by pumping
  (`loop_pump` + `pumpUntil` from `@z2net/bffi`). Without a pumper
  the wait ends in the documented `Timeout` (30s here).
- **The forwarder is a registered NATIVE callback** (`bffi::register`)
  whose body runs ON THE JS THREAD inside the marshal job - the only
  thread where calling the bound `JSCallback` pointer is
  synchronous. The pointer convention (`cstring -> void`, raw
  pointer handed over by JS) is exactly `bffi_async_attach`'s
  resolver pattern.
- **The callback `Value` matrix is primitives-only in v1** (no
  `Value::Str`), so the string payload rides the per-webview message
  box (`webview_ipc_reply`) while `invoke_wait` carries the signal.

## The surface

| Export | Kind | Purpose |
| --- | --- | --- |
| `webview_open(config) -> u64` | typed | spawns the loop thread on first use; creates the window + webview (`WebviewConfig` record: `url`/`html`/`title`/`width`/`height`/`devtools`, all optional, default size 1024x768) and returns the registry handle (tag `0x0700`) |
| `webview_eval(handle, js)` | typed | queues `evaluate_script` onto the loop thread (fire-and-forget) |
| `webview_close(handle)` | typed | drops the webview + window; the last close stops the loop thread |
| `webview_bind_ipc(handle, js_ptr)` | typed | wires the IPC: `js_ptr` is the raw `JSCallback` pointer of a `{ args: ["cstring"], returns: "void" }` callback (the `bffi_async_attach` convention); native registers the forwarder for it |
| `webview_ipc_reply(handle, reply)` | typed | the JS handler's answer: the reply JSON, embedded verbatim into the resolve script (must be valid JSON) |
| `webview_poll_exit() -> bool` | typed | sticky flag: the loop thread has finished |
| `loop_pump() -> u64` | typed | the non-blocking drain the JS side calls while native threads wait |

Native-only pieces (not exports): the `webview:__bffi` bootstrap
script injected into every page (`__bffiCall`/`__bffiResolve` - a
call with no native answer within 2s rejects, so a page may retry
until the JS side bound its ipc callback), the ipc handler, the
forwarder body and the message box.

## The e2e suite (test/wry.test.ts) and its CI safety

The suite opens a REAL OS window, so it is **env-gated**: without
`BFFI_WRY_E2E=1` every test is skipped (bun reports `2 skip`,
`0 fail` - CI-safe). The Rust unit tests (`cargo test -p
bffi-example-wry`) cover the no-window logic (config fallbacks, the
message box, the resolve-script escaping, registry slots) and always
run.

```sh
cargo test -p bffi-example-wry      # always green, no windows
bun test examples/wry               # skipped without the env
```

## Run (with a real window)

```sh
# from the repository ROOT; WebView2 runtime required on Windows
BFFI_WRY_E2E=1 bun test examples/wry
```

What you should see: a small window flashes open, its page runs two
IPC roundtrips (the first is page-driven with a bind retry, the
second is driven by `webview_eval` clicking a button), the window
closes and the loop thread shuts down - `2 pass` in about a second.

## Gotchas

- **Bind the JS thread first.** `setJsThread(raw)` makes the pump
  contract explicit: while a native thread waits in `invoke_wait`,
  SOMEONE must call `loop_pump`. (An unbound process degrades to
  direct cross-thread `JSCallback` calls - works, but outside the
  documented contract.)
- **Re-entrancy**: never call `webview_open` (it blocks the JS
  thread) while some native thread is inside `invoke_wait` waiting
  for that same JS thread - the marshal job would starve until the
  30s timeout.
- **The ipc handler blocks the UI thread** for up to 30s per
  message; the page freezes meanwhile (the price of the synchronous
  roundtrip in v1).
- **Linux/macOS**: the example spawns the event loop off the OS main
  thread, which winit only allows on Windows (`with_any_thread`);
  other platforms keep their default restriction. Windows is the
  supported target of this example (WebView2).
- **`webview_ipc_reply` payloads are embedded as JS expressions**:
  they must be valid JSON (the handler contract), otherwise the
  resolve script throws in the page and the promise never settles.
