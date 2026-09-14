# Binding GUI and event-driven libraries from Bun

bffi is a binding layer (the napi-rs analogue for the Bun
ecosystem): a native Rust library compiles to a cdylib, Bun loads
it through the typed loader and calls it. This guide covers the
one scenario plain napi-rs cannot do well: **GUI / event-driven
libraries** (wry, winit, tao, SDL, ...) whose handlers run on
OS threads and need synchronous round trips into JavaScript.

The working reference implementation is
[`wry`](https://github.com/z2net/bffi-examples/tree/main/wry)
(a full webview window driven from Bun). Read it alongside this
guide.

## The threading model

A GUI library owns an OS event loop, and that loop must run on a
dedicated thread. The pattern:

```text
Bun main thread (JS)                native loop thread (cdylib)
----------------------              ---------------------------
api.webview_open(cfg)  --proxy-->   EventLoop + Window + WebView
api.webview_eval(h, js) --proxy-->   evaluate_script
      |                                    |
      | pump the bffi event loop           | ipc_handler fires
      |<-- marshalled callback job ------- invoke_wait(js_cb, ...)
      |      (runs on THIS thread)         | (parked until reply)
      | resolve via webview_eval ---->     | wakes with the reply
```

- The cdylib spawns the loop thread on the first call (a
  `std::thread` + an `EventLoopProxy` command channel). JavaScript
  never blocks on the window: every native entry point proxies a
  command and returns immediately.
- The **JS thread stays free to pump** the bffi event loop
  (`pumpUntil` / the generated `loop_pump`, the explicit pump
  contract in `docs/DESIGN.md`). Deliveries execute on the JS
  thread while it pumps - the framework never installs hidden
  timers.
- Cross-thread legality: on Windows a winit/tao loop runs fine off
  the main thread (`with_any_thread(true)`). Linux toolkits
  (webkit2gtk/gtk-init) have stricter assumptions - gate the
  platform or pump the loop on the main thread there.

## Calling JavaScript from a native thread: invoke_wait

The one primitive that makes this work:

```rust
use bffi::{invoke_wait, Value};
use std::time::Duration;

// Runs on the GUI loop thread: calls the JS-bound callback,
// parks until the JS side answers (it pumps), returns the value.
let reply = invoke_wait(ipc_handle, &[Value::Str(body)],
                        Duration::from_secs(30))?;
```

- `invoke_wait` dispatches BOTH callback tables: natively
  registered closures (`bffi::register`, the `Value` path) and
  JS-bound callbacks (`bffi_callback_bind` - the handle JavaScript
  receives when it binds its function).
- The call is enqueued onto the bffi event loop; the calling
  thread parks on a slot; when the JS thread pumps, the job runs
  there (JSCallbacks are thread-affine - this is the only legal
  way) and the result travels back through the slot.
- **Timeout is mandatory.** A never-pumped loop must not hang the
  native thread: the wait is bounded, `ErrorCode::Timeout` (15)
  comes back, and a late result lands in the abandoned slot
  harmlessly.
- **Deadlock contract** (documented in
  `crates/bffi/CALLING-CONVENTION.md` section 9.1): the JS thread
  must pump; never call `invoke_wait` from code that itself runs
  inside a JS-thread pump callback (re-entrancy) - it ends at the
  timeout by design.
- C-call matrix for JS-bound dispatch: up to 2 parameters of
  `i32 | i64 | f64 | bool | cstring`, returns
  `i32 | i64 | f64 | bool | void`. Unsupported shapes fail fast
  (`UnsupportedSignature`) without blocking.

## Config structs: Option fields in records

GUI constructors take sparse configs. `#[derive(BffiRecord)]`
structs accept `Option<T>` fields over the whole field matrix -
`None` crosses as the wire `TAG_UNIT` byte, `Some(v)` as the plain
value; TypeScript renders the field as `T | null`:

```rust
#[derive(bffi::BffiRecord)]
pub struct WebviewConfig {
    pub url: Option<String>,
    pub html: Option<String>,
    pub title: Option<String>,
    pub width: Option<u32>,
    pub devtools: Option<bool>,
}
```

Nested `Option<Option<T>>` is rejected at compile time (E010).

## The IPC round trip

The full UI -> native -> JS -> native -> UI circle (see
[`wry/src/lib.rs`](https://github.com/z2net/bffi-examples/blob/main/wry/src/lib.rs)):

1. JavaScript binds its handler
   (`bffi_callback_bind`, signature `unit(str)`) and passes the
   handle to a native export (`webview_bind_ipc`).
2. The page posts a message (`window.ipc.postMessage`);
   the library's `ipc_handler` runs on the loop thread and calls
   `invoke_wait(ipc_handle, &[Value::Str(body)], 30s)`.
3. The JS thread pumps, the bound handler runs, its reply is
   delivered back through `webview_eval` (resolving a promise in
   the page, e.g. `window.__bffiResolve(id, json)`).
4. The loop thread wakes with the outcome and returns.

Keep the handler fast and never nest round trips inside one
another (one in-flight call per page keeps the protocol trivial).

## Keep-alive and shutdown

- **Callbacks stay alive on the JS side**: the bound JSCallback
  and its handle must live as long as the native side can call
  them (hold the objects in a module-level registry; revoke
  explicitly on shutdown). The native side re-checks handles at
  dispatch time - a revoked handle surfaces as `InvalidHandle`
  instead of a crash.
- **Shutdown protocol**: dropping the last window exits the loop
  thread (`EventLoop::exit`); publish a sticky exited-flag and let
  JavaScript observe it (`webview_poll_exit` in the example). On
  Bun exit, registered `Drop` guards on the loop thread run - do
  not rely on atexit for threads you spawned.

## Packaging notes

- **Windows**: the WebView2 loader (`WebView2Loader.dll`) must be
  findable next to the host binary - ship it with the platform
  package; the WebView2 *runtime* itself is an OS component
  (`bffi doctor` can probe for it).
- **Linux**: webkit2gtk and its GTK dependencies are dynamic
  (install `libwebkit2gtk-4.1-dev` / `libgtk-3-dev` to build);
  headless CI needs `xvfb` for real-window tests.
- **macOS**: WKWebView is part of the OS; windows must be created
  on the main thread for AppKit - the off-main-thread pattern in
  this guide is Windows-specific today.
- Binaries are plain cdylibs: the same
  `bffi pack` / platform-package flow applies unchanged.

## Checklist for a new binding

1. Pin the library versions (`=x.y.z`) - GUI crates move fast.
2. Config struct with `Option` fields; builder setters where a
   record cannot express it.
3. Loop thread + proxy command channel; every export returns
   immediately.
4. JS-bound callbacks for every library handler; `invoke_wait`
   with a bounded timeout; `evaluate`-style exports for native ->
   UI.
5. Sticky exit flag + poll export; explicit `close` export.
6. Rust tests without windows; real-window e2e behind an env
   gate (`BFFI_WRY_E2E=1` in the example).
7. README with the threading diagram and the launch command.
