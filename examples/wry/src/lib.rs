//! Example bffi-rs native module: a real **wry** (webview) binding,
//! driven entirely from Bun - the Phase 3 maturity proof (GUI library
//! with its own OS thread and callbacks, controlled from JavaScript).
//!
//! The THREADING MODEL is the point. The webview stack (winit event
//! loop + window + wry `WebView`) lives on a dedicated native thread
//! spawned by the cdylib on the first [`webview_open`]; the Bun
//! thread stays free and drains the bffi event loop with
//! [`loop_pump`]. The two worlds meet only through:
//!
//! - a `winit::EventLoopProxy` (`send_event`): the JS-facing exports
//!   proxy every window/webview request onto the loop thread;
//! - [`bffi::invoke_wait`]: the loop-thread IPC handler parks until
//!   the JS thread delivers the callback answer by pumping;
//! - `evaluate_script`: the loop thread resolves the page-side
//!   promise once the answer is in.
//!
//! The UI -> native -> JS -> native -> UI roundtrip, step by step:
//! the page calls `window.ipc.postMessage(JSON)` (wry delivers the
//! body to the ipc handler ON THE LOOP THREAD) -> the handler stores
//! the body in the per-webview message box and calls `invoke_wait`
//! on a registered FORWARDER callback (the loop thread parks) -> the
//! JS thread pumps, the marshal job invokes the forwarder body ON
//! THE JS THREAD, which calls the bound `JSCallback` pointer
//! synchronously; the JS handler reads the request from the
//! argument, computes the reply and stores it with
//! [`webview_ipc_reply`] -> the forwarder returns, `invoke_wait`
//! wakes the loop thread, which reads the reply back and queues an
//! `Eval` command resolving `window.__bffiResolve(json)` in the
//! page.
//!
//! The callback `Value` matrix is primitives-only in v1 (no
//! `Value::Str`), so the string payload rides the message box and
//! the `JSCallback` crosses as a raw pointer with a `cstring`
//! argument - the exact raw-pointer convention of
//! `bffi_async_attach`'s resolver pair (the bffi-async delivery
//! pattern); `invoke_wait` remains the synchronization backbone.
//!
//! Aggregation lives in [`module_def`] (single source); the
//! `emit-json` binary materializes `.bffi/bffi.api.json` from it for
//! the `@z2net/bffi` pipeline.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free): the JS pipeline drains errors and reads buffers
// through them.
bffi::bffi_runtime_abi!();

// The four JS-facing generic callback exports (bffi_callback_*):
// set_thread/bind/invoke/revoke. The JS side composes them through
// `setJsThread`/`bindJsCallback`/`invokeCallback`/`revokeCallback`.
bffi::bffi_callback_abi!();

pub mod module_def;

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{
    Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use bffi::{
    BffiError, BffiRecord, CallbackError, ErrorCode, Handle, Registry, TypeTag, Value, bffi,
    invoke_wait, pump,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};
use wry::{WebView, WebViewBuilder};

// On Windows the OS main thread belongs to the Bun host, so the
// loop thread must be allowed to own the winit event loop.
#[cfg(windows)]
use winit::platform::windows::EventLoopBuilderExtWindows as _;

/// The type tag of this example's webview table in the bffi
/// registry (a user-owned slice between the framework tags:
/// object `0x0100-0x01FF`, callbacks `0x0200-0x0201`, async
/// `0x0500`, streams `0x0600`).
const WEBVIEW_TAG: TypeTag = TypeTag(0x0700);

/// How long the loop-thread IPC handler parks waiting for the JS
/// reply before the page promise is settled with an error object.
const IPC_TIMEOUT: Duration = Duration::from_secs(30);

/// How long [`webview_open`] waits for the loop thread to confirm
/// the window creation.
const CREATE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long [`ensure_loop`] waits for the freshly spawned loop
/// thread to publish its proxy.
const LOOP_START_TIMEOUT: Duration = Duration::from_secs(10);

/// The default window size when the config omits width/height.
const DEFAULT_WIDTH: u32 = 1024;
const DEFAULT_HEIGHT: u32 = 768;

/// The domain error of the module: a plain message wrapper.
#[derive(Debug)]
pub struct WryError(String);

impl std::fmt::Display for WryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WryError {}

impl From<String> for WryError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<WryError> for BffiError {
    fn from(error: WryError) -> Self {
        BffiError::new(ErrorCode::DomainError, error.0)
    }
}

/// The open request: everything optional (`None` rides the
/// `TAG_UNIT` wire byte and arrives as `null` on the JS side).
/// Exactly one of `url`/`html` should be set; `html` wins if both
/// are (wry ignores `url` when `html` is present).
#[derive(BffiRecord, Clone, Debug, Default, PartialEq)]
pub struct WebviewConfig {
    /// The URL to load.
    pub url: Option<String>,
    /// The inline HTML to load (overrides `url`).
    pub html: Option<String>,
    /// The window title.
    pub title: Option<String>,
    /// The window width (`1024` when `None`).
    pub width: Option<u32>,
    /// The window height (`768` when `None`).
    pub height: Option<u32>,
    /// Whether devtools are enabled.
    pub devtools: Option<bool>,
}

impl WebviewConfig {
    /// The window size with the fallbacks applied (`1024x768`
    /// when both fields are `None`).
    pub fn effective_size(&self) -> (u32, u32) {
        (
            self.width.unwrap_or(DEFAULT_WIDTH),
            self.height.unwrap_or(DEFAULT_HEIGHT),
        )
    }
}

/// The registry slot behind a webview handle: the IPC binding wired
/// by [`webview_bind_ipc`], if any.
#[derive(Default)]
struct WebviewSlot {
    ipc: Mutex<Option<IpcBinding>>,
}

/// One IPC binding: the raw bun:ffi `JSCallback` pointer the JS side
/// handed over (`(cstring) -> void`: the request body in, the reply
/// stored through `webview_ipc_reply`), plus the forwarder NATIVE
/// callback (`bffi::register`) whose body calls that pointer - the
/// body `invoke_wait` marshals onto the JS thread.
struct IpcBinding {
    forwarder: u64,
    js_ptr: u64,
}

/// The IPC binding of a webview slot, if any.
fn bound_ipc(slot: u64) -> Option<(u64, u64)> {
    let entry = Registry::global().get_typed::<WebviewSlot>(Handle::from_raw(slot))?;
    let binding = entry.ipc.lock().unwrap_or_else(PoisonError::into_inner);
    binding.as_ref().map(|b| (b.forwarder, b.js_ptr))
}

/// The per-webview IPC message box: the pending request body
/// written by the loop-thread handler and the reply written back by
/// the JS callback. The `Value` matrix is primitives-only in v1, so
/// the string payload rides here while `invoke_wait` synchronizes.
#[derive(Default)]
struct IpcBox {
    request: Option<String>,
    reply: Option<String>,
}

static IPC_BOXES: LazyLock<Mutex<HashMap<u64, IpcBox>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn with_boxes<R>(f: impl FnOnce(&mut HashMap<u64, IpcBox>) -> R) -> R {
    let mut boxes = IPC_BOXES.lock().unwrap_or_else(PoisonError::into_inner);
    f(&mut boxes)
}

/// Stores the request body the IPC handler is about to block on.
fn ipc_put_request(handle: u64, body: String) {
    with_boxes(|boxes| boxes.entry(handle).or_default().request = Some(body));
}

/// Takes the pending request body (one-shot) from the message box.
fn ipc_take_request(handle: u64) -> Option<String> {
    with_boxes(|boxes| {
        boxes
            .get_mut(&handle)
            .and_then(|entry| entry.request.take())
    })
}

/// Stores the reply computed by the JS callback.
fn ipc_put_reply(handle: u64, body: String) {
    with_boxes(|boxes| boxes.entry(handle).or_default().reply = Some(body));
}

/// Peeks at the stored reply WITHOUT taking it (the forwarder checks
/// that the JS handler answered before it returns).
fn ipc_peek_reply(handle: u64) -> Option<String> {
    with_boxes(|boxes| boxes.get(&handle).and_then(|entry| entry.reply.clone()))
}

/// Takes the stored reply (one-shot) on the loop thread.
fn ipc_take_reply(handle: u64) -> Option<String> {
    with_boxes(|boxes| boxes.get_mut(&handle).and_then(|entry| entry.reply.take()))
}

/// A command for the loop thread, delivered through the event-loop
/// proxy (`send_event` is the only legal cross-thread door into the
/// winit loop).
enum Command {
    /// Create a window + webview per the config.
    Create {
        config: WebviewConfig,
        reply: Sender<Result<u64, String>>,
    },
    /// Evaluate a script in the webview behind `handle`.
    Eval { handle: u64, js: String },
    /// Drop the webview behind `handle`; exit the loop when the
    /// last one goes.
    Close { handle: u64 },
}

/// The proxy of the RUNNING loop thread (`None` = not running).
static LOOP_PROXY: Mutex<Option<EventLoopProxy<Command>>> = Mutex::new(None);
/// Wakes [`ensure_loop`] when the proxy appears (or the startup
/// fails).
static LOOP_PROXY_SIGNAL: Condvar = Condvar::new();
/// The startup failure of the last spawn attempt (cleared on every
/// new spawn).
static LOOP_START_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// Sticky "the loop thread has finished" flag
/// ([`webview_poll_exit`]).
static LOOP_EXITED: AtomicBool = AtomicBool::new(false);

fn lock_proxy() -> MutexGuard<'static, Option<EventLoopProxy<Command>>> {
    LOOP_PROXY.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock_start_error() -> MutexGuard<'static, Option<String>> {
    LOOP_START_ERROR
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The proxy of a RUNNING loop thread; never spawns one.
fn loop_proxy() -> Result<EventLoopProxy<Command>, WryError> {
    lock_proxy()
        .clone()
        .ok_or_else(|| WryError("the wry loop thread is not running".to_owned()))
}

/// The proxy of a loop thread, spawning one if necessary. The JS
/// caller waits (bounded) for the loop thread to publish its proxy.
fn ensure_loop() -> Result<EventLoopProxy<Command>, WryError> {
    let mut guard = lock_proxy();
    if guard.is_none() {
        spawn_loop_thread()?;
        let deadline = Instant::now() + LOOP_START_TIMEOUT;
        while guard.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return Err(WryError(
                    "the wry loop thread did not start in time".to_owned(),
                ));
            }
            if let Some(message) = lock_start_error().clone() {
                return Err(WryError(message));
            }
            let (woken, _) = LOOP_PROXY_SIGNAL
                .wait_timeout(guard, deadline - now)
                .unwrap_or_else(PoisonError::into_inner);
            guard = woken;
        }
    }
    guard
        .clone()
        .ok_or_else(|| WryError("the wry loop thread is not running".to_owned()))
}

fn spawn_loop_thread() -> Result<(), WryError> {
    *lock_start_error() = None;
    std::thread::Builder::new()
        .name("bffi-wry-loop".to_owned())
        .spawn(loop_thread_body)
        .map_err(|error| WryError(format!("spawning the wry loop thread failed: {error}")))?;
    Ok(())
}

/// Publishes the loop-not-running state no matter HOW the thread
/// body ends (a panic unwinds through here too).
struct LoopGuard;

impl Drop for LoopGuard {
    fn drop(&mut self) {
        *lock_proxy() = None;
        LOOP_PROXY_SIGNAL.notify_all();
        LOOP_EXITED.store(true, Ordering::Release);
    }
}

fn loop_thread_body() {
    let _guard = LoopGuard;
    let mut builder = EventLoop::<Command>::with_user_event();
    // The OS main thread belongs to the Bun host process, so the
    // event loop must be allowed on this spawned thread (Windows
    // gate; other platforms keep their default restriction).
    #[cfg(windows)]
    builder.with_any_thread(true);
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            *lock_start_error() = Some(format!("the winit event loop failed to build: {error}"));
            LOOP_PROXY_SIGNAL.notify_all();
            return;
        }
    };
    *lock_proxy() = Some(event_loop.create_proxy());
    LOOP_PROXY_SIGNAL.notify_all();

    let mut app = LoopApp::default();
    // Runs until the last webview closes (then `exit()`); an error
    // return leaves the LoopGuard to publish the shutdown.
    let _ = event_loop.run_app(&mut app);
}

/// One open webview: the wry surface plus its host window. Field
/// order matters - the webview (the child) must drop before the
/// window (its parent).
struct WryWindow {
    webview: WebView,
    window: Window,
}

/// The winit handler: everything the JS exports proxy lands here,
/// ON the loop thread.
#[derive(Default)]
struct LoopApp {
    windows: HashMap<u64, WryWindow>,
}

impl ApplicationHandler<Command> for LoopApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, command: Command) {
        match command {
            Command::Create { config, reply } => {
                let _ = reply.send(create_webview(self, event_loop, config));
            }
            Command::Eval { handle, js } => {
                if let Some(entry) = self.windows.get(&handle) {
                    let _ = entry.webview.evaluate_script(&js);
                }
            }
            Command::Close { handle } => close_webview(self, event_loop, handle),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // The user clicked the title-bar X: same path as an explicit
        // `webview_close` - drop the entry, exit when it was the
        // last one.
        if let WindowEvent::CloseRequested = event {
            let handle = self
                .windows
                .iter()
                .find(|(_, entry)| entry.window.id() == window_id)
                .map(|(handle, _)| *handle);
            if let Some(handle) = handle {
                close_webview(self, event_loop, handle);
            }
        }
    }
}

/// Creates the window + webview for one `Create` command (ON the
/// loop thread; both are unusable from anywhere else).
fn create_webview(
    app: &mut LoopApp,
    event_loop: &ActiveEventLoop,
    config: WebviewConfig,
) -> Result<u64, String> {
    // Declared once per process; every later declare loses
    // harmlessly.
    let _ = Registry::global().declare::<WebviewSlot>(WEBVIEW_TAG);

    let (width, height) = config.effective_size();
    let attributes = Window::default_attributes()
        .with_title(config.title.clone().unwrap_or_else(default_title))
        .with_inner_size(LogicalSize::new(width, height));
    let window = event_loop
        .create_window(attributes)
        .map_err(|error| format!("window creation failed: {error}"))?;

    let handle = Registry::global()
        .insert(WEBVIEW_TAG, Arc::new(WebviewSlot::default()))
        .map_err(|error| format!("registry insert failed: {error}"))?;
    let raw = handle.as_u64();

    let proxy = loop_proxy().map_err(|error| error.to_string())?;
    let mut builder = WebViewBuilder::new()
        .with_devtools(config.devtools.unwrap_or(false))
        .with_ipc_handler(move |request: wry::http::Request<String>| {
            handle_ipc(raw, request.into_body(), proxy.clone());
        });
    if let Some(url) = config.url {
        builder = builder.with_url(url);
    }
    if let Some(html) = config.html {
        builder = builder.with_html(html);
    }
    builder = builder.with_initialization_script(IPC_BOOTSTRAP);

    match builder.build(&window) {
        Ok(webview) => {
            app.windows.insert(raw, WryWindow { webview, window });
            Ok(raw)
        }
        Err(error) => {
            let _ = Registry::global().remove(handle);
            Err(format!("webview creation failed: {error}"))
        }
    }
}

/// Drops one webview entry and its registry slot; exits the loop
/// when the last one went (the thread publishes the shutdown
/// through the [`LoopGuard`]).
fn close_webview(app: &mut LoopApp, event_loop: &ActiveEventLoop, handle: u64) {
    if app.windows.remove(&handle).is_some() {
        let _ = Registry::global().remove(Handle::from_raw(handle));
    }
    if app.windows.is_empty() {
        event_loop.exit();
    }
}

/// The IPC roundtrip ON the loop thread: park until the JS thread
/// answers, then queue the resolve script. Blocking the UI thread
/// for up to [`IPC_TIMEOUT`] is the price of the synchronous
/// roundtrip - the Bun thread pumps meanwhile.
fn handle_ipc(slot: u64, body: String, proxy: EventLoopProxy<Command>) {
    let Some((forwarder, _)) = bound_ipc(slot) else {
        // Nothing bound: drop the message instead of stalling the
        // UI thread for a guaranteed timeout.
        return;
    };
    ipc_put_request(slot, body);
    let outcome = invoke_wait(Handle::from_raw(forwarder), &[], IPC_TIMEOUT);
    let reply = if outcome.is_ok() {
        ipc_take_reply(slot)
    } else {
        None
    };
    let script = ipc_resolve_script(&outcome, reply);
    let _ = proxy.send_event(Command::Eval {
        handle: slot,
        js: script,
    });
}

/// The body of the registered forwarder callback: it runs ON THE
/// JS THREAD (inside the `invoke_wait` marshal job, while the JS
/// side pumps) - the only thread where calling the JSCallback
/// pointer is synchronous. Reads the pending request from the
/// message box, hands it to the JS handler (which answers through
/// `webview_ipc_reply`) and reports whether an answer arrived.
fn forward_ipc(slot: u64) -> Value {
    let Some((_, js_ptr)) = bound_ipc(slot) else {
        return Value::Bool(false);
    };
    let Some(body) = ipc_take_request(slot) else {
        return Value::Bool(false);
    };
    call_js_ipc(js_ptr, &body);
    Value::Bool(ipc_peek_reply(slot).is_some())
}

/// Calls a bun:ffi `JSCallback` pointer declared as
/// `(cstring) -> void` with `body`.
///
/// # Safety contract
///
/// `js_ptr` must be the pointer of a live bun:ffi `JSCallback`
/// declared as `{ args: ["cstring"], returns: "void" }` (the
/// `webview_bind_ipc` contract), and the call MUST happen on the
/// JS thread - here: inside the forwarder body, which only runs
/// there (the `invoke_wait` marshal job during the pump). The
/// pointer stays valid while the JS side keeps the `JSCallback`
/// alive and has not closed it.
fn call_js_ipc(js_ptr: u64, body: &str) {
    if js_ptr == 0 {
        return;
    }
    // SAFETY: see the function contract above; the transmute is the
    // exact pattern of bffi-async's `resolve_by_ptr`/`reject_by_ptr`
    // (a stored bun:ffi trampoline pointer called back on the JS
    // thread).
    let callback: extern "C" fn(*const std::os::raw::c_char) =
        unsafe { std::mem::transmute(js_ptr as usize) };
    let body = match std::ffi::CString::new(body) {
        Ok(body) => body,
        Err(_) => return,
    };
    callback(body.as_ptr());
}

/// The script that settles the page-side promise: the reply JSON as
/// a value, `null` without a reply, an `{ "error": ... }` object on
/// the failure paths (timeout, dead handle, stopped loop).
fn ipc_resolve_script(outcome: &Result<Value, CallbackError>, reply: Option<String>) -> String {
    match (outcome, reply) {
        (Ok(_), Some(reply)) => format!("window.__bffiResolve({reply});"),
        (Ok(_), None) => "window.__bffiResolve(null);".to_owned(),
        (Err(error), _) => format!(
            "window.__bffiResolve({{ \"error\": \"{}\" }});",
            escape_js(&error.to_string())
        ),
    }
}

/// Escapes `text` for embedding inside a double-quoted JS string.
fn escape_js(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Injected into every page: the tiny promise bridge the UI code
/// uses. The native side resolves through
/// `window.__bffiResolve(json)` with the reply JSON as a VALUE. A
/// call that gets no native answer within 2s rejects - so a page
/// that starts before the JS side bound its ipc callback can retry.
const IPC_BOOTSTRAP: &str = r#"window.__bffiPending = null;
window.__bffiCall = (method, args) => new Promise((resolve, reject) => {
  if (window.__bffiPending !== null) { reject(new Error("a call is already in flight")); return; }
  const pending = { resolve, reject };
  window.__bffiPending = pending;
  window.ipc.postMessage(JSON.stringify({ method, args }));
  setTimeout(() => {
    if (window.__bffiPending === pending) {
      window.__bffiPending = null;
      reject(new Error("no native answer (is the ipc callback bound?)"));
    }
  }, 2000);
});
window.__bffiResolve = (json) => {
  const pending = window.__bffiPending;
  window.__bffiPending = null;
  if (pending !== null) { pending.resolve(json); }
};"#;

fn default_title() -> String {
    "bffi wry".to_owned()
}

/// The registry slot behind a webview handle.
fn webview_slot(handle: u64) -> Result<Arc<WebviewSlot>, WryError> {
    Registry::global()
        .get_typed::<WebviewSlot>(Handle::from_raw(handle))
        .ok_or_else(|| WryError(format!("invalid or closed webview handle: {handle}")))
}

/// Opens a webview window described by `config` (inline `html` or a
/// `url`, optional `title`/`width`/`height`/`devtools`) and returns
/// its opaque handle. The first call spawns the dedicated loop
/// thread (winit event loop + window + wry surface live there);
/// the Bun thread only waits, bounded, for the creation reply.
#[bffi]
pub fn webview_open(config: WebviewConfig) -> Result<u64, WryError> {
    let proxy = ensure_loop()?;
    let (reply, answer) = channel();
    proxy
        .send_event(Command::Create { config, reply })
        .map_err(|_| {
            WryError("the wry loop thread stopped before the window was created".to_owned())
        })?;
    match answer.recv_timeout(CREATE_TIMEOUT) {
        Ok(created) => created.map_err(WryError),
        Err(_) => Err(WryError(
            "the wry loop thread did not answer the create request".to_owned(),
        )),
    }
}

/// Evaluates `js` in the webview behind `handle` (proxied onto the
/// loop thread; fire-and-forget - the script's result stays in the
/// page).
#[bffi]
pub fn webview_eval(handle: u64, js: &str) -> Result<(), WryError> {
    webview_slot(handle)?;
    loop_proxy()?
        .send_event(Command::Eval {
            handle,
            js: js.to_owned(),
        })
        .map_err(|_| {
            WryError("the wry loop thread stopped before the script was delivered".to_owned())
        })
}

/// Closes the webview behind `handle` (also the title-bar X path):
/// the loop thread drops the window and exits when it was the last
/// one - then [`webview_poll_exit`] flips to `true`.
#[bffi]
pub fn webview_close(handle: u64) -> Result<(), WryError> {
    webview_slot(handle)?;
    loop_proxy()?
        .send_event(Command::Close { handle })
        .map_err(|_| {
            WryError("the wry loop thread stopped before the close was delivered".to_owned())
        })
}

/// Wires the IPC roundtrip of the webview behind `handle` to a JS
/// handler: `js_ptr` is the raw `bun:ffi` `JSCallback` pointer of a
/// callback declared as `{ args: ["cstring"], returns: "void" }`
/// (the request body arrives as the argument; the handler answers
/// through [`webview_ipc_reply`] - the same raw-pointer convention
/// as `bffi_async_attach`). Native side registers a forwarder
/// callback whose body `invoke_wait` marshals onto the JS thread,
/// where the pointer call is synchronous.
#[bffi]
pub fn webview_bind_ipc(handle: u64, js_ptr: u64) -> Result<(), WryError> {
    let slot = webview_slot(handle)?;
    if js_ptr == 0 {
        return Err(WryError("the ipc callback pointer is null".to_owned()));
    }
    let forwarder = bffi::register(
        bffi::CallbackSig::new(bffi::ValueType::Bool, &[]),
        Arc::new(move |_| forward_ipc(handle)),
    )
    .map_err(|error| WryError(format!("forwarder registration failed: {error}")))?;
    *slot.ipc.lock().unwrap_or_else(PoisonError::into_inner) = Some(IpcBinding {
        forwarder: forwarder.as_u64(),
        js_ptr,
    });
    Ok(())
}

/// The IPC message box, JS side: stores the reply JSON (embedded
/// into the resolve script verbatim, so it MUST be valid JSON).
/// Called by the bound callback before it returns.
#[bffi]
pub fn webview_ipc_reply(handle: u64, reply: &str) -> Result<(), WryError> {
    webview_slot(handle)?;
    ipc_put_reply(handle, reply.to_owned());
    Ok(())
}

/// Whether the loop thread has finished (the last webview closed).
#[bffi]
pub fn webview_poll_exit() -> bool {
    LOOP_EXITED.load(Ordering::Acquire)
}

/// The non-blocking drain the JS side calls while native threads
/// wait (`pumpUntil` from `@z2net/bffi`): the marshalled callback
/// invocation executes here, on the bound JS thread.
#[bffi]
pub fn loop_pump() -> u64 {
    pump()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DEFAULT_HEIGHT, DEFAULT_WIDTH, IpcBox, WEBVIEW_TAG, WebviewConfig, WebviewSlot, escape_js,
        ipc_put_reply, ipc_put_request, ipc_resolve_script, ipc_take_reply, ipc_take_request,
        webview_slot,
    };
    use bffi::{CallbackError, Registry, Value};

    #[test]
    fn effective_size_falls_back_to_defaults() {
        let mut config = WebviewConfig::default();
        assert_eq!(config.effective_size(), (DEFAULT_WIDTH, DEFAULT_HEIGHT));
        config.width = Some(800);
        assert_eq!(config.effective_size(), (800, DEFAULT_HEIGHT));
        config.height = Some(600);
        assert_eq!(config.effective_size(), (800, 600));
    }

    #[test]
    fn ipc_box_roundtrips_request_and_reply() {
        const HANDLE: u64 = 0xA11CE;
        assert_eq!(ipc_take_request(HANDLE), None);
        ipc_put_request(HANDLE, r#"{"method":"echo"}"#.to_owned());
        assert_eq!(
            ipc_take_request(HANDLE).as_deref(),
            Some(r#"{"method":"echo"}"#)
        );
        // Take is one-shot: the box is empty again.
        assert_eq!(ipc_take_request(HANDLE), None);
        assert_eq!(ipc_take_reply(HANDLE), None);
        ipc_put_reply(HANDLE, r#"{"pong":"ping"}"#.to_owned());
        assert_eq!(
            ipc_take_reply(HANDLE).as_deref(),
            Some(r#"{"pong":"ping"}"#)
        );
        assert_eq!(ipc_take_reply(HANDLE), None);
    }

    #[test]
    fn ipc_boxes_are_isolated_per_handle() {
        const A: u64 = 0xB000;
        const B: u64 = 0xB001;
        ipc_put_request(A, "a".to_owned());
        ipc_put_request(B, "b".to_owned());
        assert_eq!(ipc_take_request(A).as_deref(), Some("a"));
        assert_eq!(ipc_take_request(B).as_deref(), Some("b"));
    }

    #[test]
    fn escape_js_neutralizes_the_dangerous_bytes() {
        assert_eq!(escape_js("plain"), "plain");
        assert_eq!(escape_js("a\"b"), "a\\\"b");
        assert_eq!(escape_js("a\\b"), "a\\\\b");
        assert_eq!(escape_js("a\nb\rc\td"), "a\\nb\\rc\\td");
    }

    #[test]
    fn resolve_script_embeds_the_reply_or_the_error() {
        let ok = Ok(Value::Bool(true));
        assert_eq!(
            ipc_resolve_script(&ok, Some(r#"{"pong":"ping"}"#.to_owned())),
            r#"window.__bffiResolve({"pong":"ping"});"#
        );
        assert_eq!(ipc_resolve_script(&ok, None), "window.__bffiResolve(null);");
        let error = Err(CallbackError::Timeout);
        let script = ipc_resolve_script(&error, None);
        assert!(
            script.starts_with(r#"window.__bffiResolve({ "error": ""#)
                && script.ends_with(r#"" });"#),
            "the failure script must embed an escaped error object: {script}"
        );
    }

    #[test]
    fn webview_slots_live_in_the_registry() {
        let _ = Registry::global().declare::<WebviewSlot>(WEBVIEW_TAG);
        let handle = Registry::global()
            .insert(WEBVIEW_TAG, Arc::new(WebviewSlot::default()))
            .expect("the example table has room");
        assert!(webview_slot(handle.as_u64()).is_ok());
        assert!(webview_slot(0).is_err());
        // After removal the generational handle stays dead even if
        // the slot index is reused.
        let stale = handle.as_u64();
        assert!(Registry::global().remove(handle));
        assert!(webview_slot(stale).is_err());
    }

    #[test]
    fn ipc_box_default_is_empty() {
        let entry = IpcBox::default();
        assert_eq!(entry.request, None);
        assert_eq!(entry.reply, None);
    }
}
