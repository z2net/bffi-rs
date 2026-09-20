//! The callback table and both lifecycle directions: [`register`] /
//! [`invoke`] / [`revoke`] (JS -> Rust) and [`bind_js_callback`] /
//! [`js_callback`] (Rust -> JS).
//!
//! A Rust closure is registered explicitly and addressed by an opaque
//! [`Handle`] (criterion 5.1: registration = `register`), invoked with
//! a runtime-checked signature, and revoked terminally by [`revoke`] -
//! the single removal point for BOTH callback directions (criterion
//! 5.1: revocation = `revoke`): the registry's type-erased `remove`
//! routes by the handle's tag to either table.
//!
//! A revoked handle never resurrects: revocation drops the registry
//! slot, and the generational handle scheme keeps the stale value dead
//! even if the slot is later reused (criterion 5.2).
//!
//! Storage lives in the process-wide [`Registry`] under two
//! crate-owned tags: `0x0200` for [`NativeEntry`] and `0x0201` for
//! [`JsEntry`] (populated by [`bind_js_callback`]).

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
use std::ffi::CString;
use std::sync::{Arc, OnceLock};
// `invoke_wait` (marshal-and-wait) rides the event loop, which is a
// strict superset of this crate's own feature set.
#[cfg(feature = "event-loop")]
use std::sync::{Condvar, Mutex, PoisonError};
#[cfg(feature = "event-loop")]
use std::time::{Duration, Instant};

use bffi_core::{Handle, Registry, TypeTag};

use super::error::CallbackError;
use super::thread::ensure_js_thread;
use super::value::{CallbackSig, Value, ValueType};
#[cfg(feature = "event-loop")]
use super::{end_wait, try_begin_wait};

/// The type tag of the native (JS -> Rust) callback table.
const NATIVE_TAG: TypeTag = TypeTag(0x0200);

/// The type tag of the JS-side (Rust -> JS) callback table.
const JS_TAG: TypeTag = TypeTag(0x0201);

/// The body of a registered native callback: receives the (already
/// signature-checked) arguments and returns the callback's value.
type NativeBody = dyn Fn(&[Value]) -> Value + Send + Sync;

/// A registered native callback: its declared signature plus the Rust
/// closure invoked on every call.
struct NativeEntry {
    sig: CallbackSig,
    f: Arc<NativeBody>,
}

/// A registered JS-side callback (Rust -> JS direction): its declared
/// signature plus the opaque pointer token handed over by the JS side.
///
/// `thread` is the numeric id of the JS thread that performed the
/// binding (see `super::thread`): the JSCallback behind `ptr` belongs
/// to THAT isolate, so every call through this entry must run there.
/// `0` means the process was unbound at bind time (pure-Rust usage);
/// deliveries for such an entry use the legacy untargeted queue.
struct JsEntry {
    sig: CallbackSig,
    ptr: usize,
    #[cfg(feature = "event-loop")]
    thread: u64,
}

/// A read-only snapshot of a JS-side callback slot, returned by
/// [`js_callback`]: the declared signature plus the opaque pointer
/// token stored at [`bind_js_callback`] time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsCallbackInfo {
    /// The signature declared when the callback was bound.
    pub sig: CallbackSig,
    /// The opaque, handle-sized token handed to [`bind_js_callback`].
    /// Never dereferenced by this crate.
    pub ptr: usize,
}

/// Declares both callback tables in the global registry, exactly once
/// per process.
///
/// The memoized `Result` inside the [`OnceLock`] makes the outcome
/// sticky: every later caller gets the same result as the first
/// initializer, whether `Ok` or `Err`. A `RegistryError::
/// TagAlreadyRegistered` maps to [`CallbackError::TagInUse`] for the
/// failing tag - the tags are crate-owned, so contention means two
/// initializers raced, and the `OnceLock` keeps only one outcome.
///
/// If the [`NativeEntry`] declare succeeds but the [`JsEntry`] declare
/// fails, that partial state (native table only) is permanent: the
/// registry has no undeclare, and the memoized `Err` stops all later
/// retries.
fn tables() -> Result<(), CallbackError> {
    static TABLES: OnceLock<Result<(), CallbackError>> = OnceLock::new();
    TABLES
        .get_or_init(|| {
            Registry::global()
                .declare::<NativeEntry>(NATIVE_TAG)
                .map_err(|_| CallbackError::TagInUse(NATIVE_TAG))?;
            Registry::global()
                .declare::<JsEntry>(JS_TAG)
                .map_err(|_| CallbackError::TagInUse(JS_TAG))?;
            Ok(())
        })
        .clone()
}

/// Registers a native callback (JS -> Rust direction) and returns its
/// fresh, opaque [`Handle`].
///
/// The declared [`CallbackSig`] is enforced on every [`invoke`]: the
/// call is rejected with [`CallbackError::SignatureMismatch`] unless
/// the arguments satisfy it.
///
/// # Errors
///
/// [`CallbackError::TableFull`] when the native table has no free
/// slot, or [`CallbackError::TagInUse`] if table initialization ever
/// failed earlier in the process (the memoized outcome).
pub fn register(sig: CallbackSig, f: Arc<NativeBody>) -> Result<Handle, CallbackError> {
    tables()?;
    Registry::global()
        .insert(NATIVE_TAG, Arc::new(NativeEntry { sig, f }))
        .map_err(|_| {
            // `tables()` declared NATIVE_TAG for `NativeEntry` and the
            // registry has no undeclare, so `NotRegistered` is
            // unreachable after declare; the remaining condition is a
            // full table. `RegistryError` is `#[non_exhaustive]`, so
            // future variants are grouped defensively into the
            // wildcard (same pattern as bffi-object's wrap.rs).
            CallbackError::TableFull
        })
}

/// Invokes the native callback behind `handle` with `args`.
///
/// Check order: table init -> null handle -> table lookup (wrong tag,
/// stale, or unknown handles all land in
/// [`CallbackError::InvalidHandle`]) -> signature
/// ([`CallbackError::SignatureMismatch`]) -> JS-thread gate ->
/// the call itself.
///
/// The closure's panic is deliberately NOT caught here: today it
/// unwinds into the caller - DESIGN.md §6.5 allows debug builds to
/// abort instead, for easier debugging - and catching at the FFI
/// boundary is the P2 event-loop trampoline's job (`run_extern_body`).
///
/// # Errors
///
/// [`CallbackError::InvalidHandle`] for a null, revoked, stale, or
/// foreign-kind handle; [`CallbackError::SignatureMismatch`] when
/// `args` do not satisfy the declared signature;
/// [`CallbackError::WrongThread`] on a bound process when called off
/// the JS thread; [`CallbackError::TagInUse`] if table initialization
/// ever failed earlier in the process.
pub fn invoke(handle: Handle, args: &[Value]) -> Result<Value, CallbackError> {
    tables()?;
    if handle.is_null() {
        return Err(CallbackError::InvalidHandle(handle));
    }
    let entry = Registry::global()
        .get_typed::<NativeEntry>(handle)
        .ok_or(CallbackError::InvalidHandle(handle))?;
    if !entry.sig.matches(args) {
        return Err(CallbackError::SignatureMismatch {
            expected: entry.sig.clone(),
            got: args.iter().map(|v| v.ty()).collect(),
        });
    }
    ensure_js_thread()?;
    Ok((entry.f)(args))
}

/// The shared hand-off slot of [`invoke_wait`]: the event-loop job
/// stores the invocation outcome, the calling thread parks on the
/// condition variable. Locks follow the crate-wide poison-recovery
/// pattern - the outcome is produced outside the lock (`invoke` runs
/// in the job body), so poisoning cannot happen in practice.
#[cfg(feature = "event-loop")]
struct WaitSlot {
    outcome: Mutex<Option<Result<Value, CallbackError>>>,
    signal: Condvar,
}

#[cfg(feature = "event-loop")]
impl Default for WaitSlot {
    fn default() -> Self {
        Self {
            outcome: Mutex::new(None),
            signal: Condvar::new(),
        }
    }
}

#[cfg(feature = "event-loop")]
impl WaitSlot {
    /// Stores `outcome` and wakes every waiter. Safe to call after the
    /// caller timed out: the late store lands in the abandoned slot
    /// (ignore-after-timeout) and drops together with the job's slot
    /// clone - loop and registry state are untouched.
    fn fill(&self, outcome: Result<Value, CallbackError>) {
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
        self.signal.notify_all();
    }

    /// Takes the stored outcome, waiting at most `timeout`. A filled
    /// slot always wins over an expired deadline (the outcome arrived
    /// within the wait); an empty slot at the deadline is
    /// [`CallbackError::Timeout`].
    fn take_within(&self, timeout: Duration) -> Result<Value, CallbackError> {
        let deadline = Instant::now() + timeout;
        let mut guard = self.outcome.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(outcome) = guard.take() {
                return outcome;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(CallbackError::Timeout);
            }
            let (woken, _) = self
                .signal
                .wait_timeout(guard, deadline - now)
                .unwrap_or_else(PoisonError::into_inner);
            guard = woken;
        }
    }
}

/// The value types a JS-bound parameter may take so the
/// `invoke_wait` dispatch can convert it to a C argument.
#[cfg(feature = "event-loop")]
const JS_CALL_PARAMS: &[ValueType] = &[
    ValueType::I32,
    ValueType::I64,
    ValueType::U64,
    ValueType::F64,
    ValueType::Bool,
    ValueType::Str,
];

/// The value types a JS-bound return may take so the `invoke_wait`
/// dispatch can wrap the raw C result.
#[cfg(feature = "event-loop")]
const JS_CALL_RETS: &[ValueType] = &[
    ValueType::I32,
    ValueType::I64,
    ValueType::U64,
    ValueType::F64,
    ValueType::Bool,
    ValueType::Str,
    ValueType::Unit,
];

/// The largest arity the JS-bound C-call dispatch implements (v1:
/// the concrete `extern "C"` signatures are enumerated per shape).
#[cfg(feature = "event-loop")]
const JS_CALL_MAX_PARAMS: usize = 2;

/// The value types actually received, in arrival order (the `got`
/// half of a [`CallbackError::SignatureMismatch`]).
#[cfg(feature = "event-loop")]
fn got_types(args: &[Value]) -> Vec<ValueType> {
    args.iter().map(Value::ty).collect()
}

/// Whether [`invoke_wait`]'s JS-bound dispatch can call a callback
/// declared with `sig` through its bound C pointer: the return and
/// every parameter must be inside the implemented matrix and the
/// arity within the enumerated shapes.
#[cfg(feature = "event-loop")]
fn check_js_call_matrix(sig: &CallbackSig) -> Result<(), CallbackError> {
    let ret_ok = JS_CALL_RETS.contains(&sig.ret());
    let params_ok = sig.params().len() <= JS_CALL_MAX_PARAMS
        && sig
            .params()
            .iter()
            .all(|param| JS_CALL_PARAMS.contains(param));
    if ret_ok && params_ok {
        Ok(())
    } else {
        Err(CallbackError::UnsupportedSignature { sig: sig.clone() })
    }
}

/// One argument of a JS-bound call, already converted to its C form.
#[cfg(feature = "event-loop")]
enum CArg {
    /// `i32` by value.
    I32(i32),
    /// `i64` by value.
    I64(i64),
    /// `u64` by value (the exact unsigned carrier).
    U64(u64),
    /// `f64` by value.
    F64(f64),
    /// `u8` by value (bun:ffi's spelling of `bool`).
    U8(u8),
    /// A NUL-terminated UTF-8 string (bun:ffi's `cstring`); owned by
    /// the carrier so the pointer stays valid across the call.
    Str(CString),
}

/// The prepared C side of one JS-bound call: the converted arguments
/// (owned, so `cstring` pointers live across the call) plus the
/// signature and the original values for the defensive error paths.
#[cfg(feature = "event-loop")]
struct JsCall<'a> {
    sig: &'a CallbackSig,
    args: &'a [Value],
    cargs: Vec<CArg>,
}

#[cfg(feature = "event-loop")]
impl<'a> JsCall<'a> {
    /// Converts the arguments to their C forms, guided by the
    /// declared signature.
    fn marshal(sig: &'a CallbackSig, args: &'a [Value]) -> Result<Self, CallbackError> {
        let mismatch = || CallbackError::SignatureMismatch {
            expected: sig.clone(),
            got: got_types(args),
        };
        let mut cargs = Vec::with_capacity(args.len());
        for (param, value) in sig.params().iter().zip(args.iter()) {
            let converted = match (param, value) {
                (ValueType::I32, Value::I32(v)) => CArg::I32(*v),
                (ValueType::I64, Value::I64(v)) => CArg::I64(*v),
                (ValueType::U64, Value::U64(v)) => CArg::U64(*v),
                // The exact-carrier allowance of `CallbackSig::matches`
                // (the JS encoder picks the tag by value): convert the
                // in-range view at the same width.
                (ValueType::U64, Value::I64(v)) if *v >= 0 => CArg::U64(*v as u64),
                (ValueType::I64, Value::U64(v)) if *v <= i64::MAX as u64 => CArg::I64(*v as i64),
                (ValueType::F64, Value::F64(v)) => CArg::F64(*v),
                (ValueType::Bool, Value::Bool(v)) => CArg::U8(u8::from(*v)),
                (ValueType::Str, Value::Str(text)) => {
                    CArg::Str(CString::new(text.as_str()).map_err(|_| {
                        // An interior NUL cannot cross as a cstring.
                        CallbackError::InvalidCString
                    })?)
                }
                // Unreachable after `matches` + the matrix check;
                // defended for the mismatch error only.
                _ => return Err(mismatch()),
            };
            cargs.push(converted);
        }
        Ok(Self { sig, args, cargs })
    }

    /// The mismatch error of this call (defensive paths only).
    fn mismatch(&self) -> CallbackError {
        CallbackError::SignatureMismatch {
            expected: self.sig.clone(),
            got: got_types(self.args),
        }
    }

    /// The `i`-th argument as a C `i32`.
    fn arg_i32(&self, i: usize) -> Result<i32, CallbackError> {
        match self.cargs.get(i) {
            Some(CArg::I32(v)) => Ok(*v),
            _ => Err(self.mismatch()),
        }
    }

    /// The `i`-th argument as a C `i64`.
    fn arg_i64(&self, i: usize) -> Result<i64, CallbackError> {
        match self.cargs.get(i) {
            Some(CArg::I64(v)) => Ok(*v),
            _ => Err(self.mismatch()),
        }
    }

    /// The `i`-th argument as a C `u64` (the exact unsigned carrier).
    fn arg_u64(&self, i: usize) -> Result<u64, CallbackError> {
        match self.cargs.get(i) {
            Some(CArg::U64(v)) => Ok(*v),
            _ => Err(self.mismatch()),
        }
    }

    /// The `i`-th argument as a C `f64`.
    fn arg_f64(&self, i: usize) -> Result<f64, CallbackError> {
        match self.cargs.get(i) {
            Some(CArg::F64(v)) => Ok(*v),
            _ => Err(self.mismatch()),
        }
    }

    /// The `i`-th argument as a C `u8` (bun:ffi's `bool`).
    fn arg_u8(&self, i: usize) -> Result<u8, CallbackError> {
        match self.cargs.get(i) {
            Some(CArg::U8(v)) => Ok(*v),
            _ => Err(self.mismatch()),
        }
    }

    /// The `i`-th argument as a borrowed `cstring` pointer.
    fn arg_cstring(&self, i: usize) -> Result<*const std::os::raw::c_char, CallbackError> {
        match self.cargs.get(i) {
            Some(CArg::Str(text)) => Ok(text.as_ptr()),
            _ => Err(self.mismatch()),
        }
    }
}

/// Invokes the JS callback bound behind `handle` with `args` on the
/// CURRENT thread: the shared body of [`invoke_wait`]'s direct path
/// and of its marshal job (both are required to run on the JS thread,
/// where calling a `bun:ffi` JSCallback pointer is synchronous).
///
/// Check order: table lookup (dead handles land in
/// [`CallbackError::InvalidHandle`]) -> isolate gate (a bound entry's
/// `JSCallback` belongs to exactly one JS thread;
/// [`CallbackError::WrongThread`] elsewhere) -> signature
/// ([`CallbackError::SignatureMismatch`]) -> null pointer
/// ([`CallbackError::NullPointer`]) -> the C-call matrix
/// ([`CallbackError::UnsupportedSignature`]) -> the call itself.
#[cfg(feature = "event-loop")]
fn invoke_js_entry(handle: Handle, args: &[Value]) -> Result<Value, CallbackError> {
    let entry = Registry::global()
        .get_typed::<JsEntry>(handle)
        .ok_or(CallbackError::InvalidHandle(handle))?;
    if entry.thread != 0 && entry.thread != super::thread::current_thread_id() {
        // The trampoline belongs to another isolate: calling it from
        // here would cross a JS-thread boundary (undefined behavior
        // for bun:ffi JSCallbacks).
        return Err(CallbackError::WrongThread);
    }
    if !entry.sig.matches(args) {
        return Err(CallbackError::SignatureMismatch {
            expected: entry.sig.clone(),
            got: got_types(args),
        });
    }
    if entry.ptr == 0 {
        return Err(CallbackError::NullPointer);
    }
    check_js_call_matrix(&entry.sig)?;
    call_js_ptr(&entry.sig, entry.ptr, args)
}

/// Calls the raw JS-callback pointer through the concrete `extern "C"`
/// signature the stored [`CallbackSig`] declares, and wraps the raw
/// result back into a [`Value`].
///
/// # Safety contract
///
/// `ptr` must be a live `bun:ffi` JSCallback pointer declared with
/// exactly this C shape (the bun:ffi spellings: `i32`, `i64`, `u64`,
/// `f64`, `u8` for `Bool`, `cstring` for `Str`, `void` for `Unit`),
/// and the call MUST happen on the JS thread - here: inside the
/// `invoke_wait` dispatch, whose direct path and marshal job both
/// run there. The pointer stays valid while the JS side keeps the
/// `JSCallback` alive and has not closed it.
#[cfg(feature = "event-loop")]
fn call_js_ptr(sig: &CallbackSig, ptr: usize, args: &[Value]) -> Result<Value, CallbackError> {
    /// Builds the whole call dispatch: for every callable shape (up
    /// to [`JS_CALL_MAX_PARAMS`] parameters) the five return arms -
    /// each transmutes the bound pointer to the declared C
    /// signature, calls it, and wraps the raw result into a
    /// [`Value`].
    macro_rules! js_dispatch {
        ($js:expr, $sig:expr, $ptr:expr; $( [$($fty:ty => $vt:ident),*] => ($($arg:expr),*) ),* $(,)?) => {
            match ($sig.ret(), $sig.params()) {
                $(
                    (ValueType::Unit, &[$(ValueType::$vt),*]) => {{
                        // SAFETY: the pointer is a live JSCallback with
                        // this declared shape, called on the JS thread
                        // (see the safety contract above).
                        let call: extern "C" fn($($fty),*) = unsafe { std::mem::transmute($ptr) };
                        call($($arg),*);
                        Ok(Value::Unit)
                    }},
                    (ValueType::Bool, &[$(ValueType::$vt),*]) => {{
                        // SAFETY: see the `Unit` arm.
                        let call: extern "C" fn($($fty),*) -> u8 = unsafe { std::mem::transmute($ptr) };
                        Ok(Value::Bool(call($($arg),*) != 0))
                    }},
                    (ValueType::I32, &[$(ValueType::$vt),*]) => {{
                        // SAFETY: see the `Unit` arm.
                        let call: extern "C" fn($($fty),*) -> i32 = unsafe { std::mem::transmute($ptr) };
                        Ok(Value::I32(call($($arg),*)))
                    }},
                    (ValueType::I64, &[$(ValueType::$vt),*]) => {{
                        // SAFETY: see the `Unit` arm.
                        let call: extern "C" fn($($fty),*) -> i64 = unsafe { std::mem::transmute($ptr) };
                        Ok(Value::I64(call($($arg),*)))
                    }},
                    (ValueType::U64, &[$(ValueType::$vt),*]) => {{
                        // SAFETY: see the `Unit` arm.
                        let call: extern "C" fn($($fty),*) -> u64 = unsafe { std::mem::transmute($ptr) };
                        Ok(Value::U64(call($($arg),*)))
                    }},
                    (ValueType::F64, &[$(ValueType::$vt),*]) => {{
                        // SAFETY: see the `Unit` arm.
                        let call: extern "C" fn($($fty),*) -> f64 = unsafe { std::mem::transmute($ptr) };
                        Ok(Value::F64(call($($arg),*)))
                    }},
                    (ValueType::Str, &[$(ValueType::$vt),*]) => {{
                        // SAFETY: see the `Unit` arm.
                        let call: extern "C" fn($($fty),*) -> *const std::os::raw::c_char = unsafe { std::mem::transmute($ptr) };
                        let raw = call($($arg),*);
                        if raw.is_null() {
                            return Err(CallbackError::NullPointer);
                        }
                        // The cstring return is call-scoped (bun:ffi's
                        // transcode buffer lives for the duration of
                        // the call): clone the bytes out before the
                        // call returns.
                        // SAFETY: `raw` is non-null and NUL-terminated
                        // for the duration of this call.
                        let text = unsafe { std::ffi::CStr::from_ptr(raw) }
                            .to_string_lossy()
                            .into_owned();
                        Ok(Value::Str(text))
                    }},
                )*
                // Unreachable after `check_js_call_matrix`; kept total
                // so the dispatch stays honest about its limits.
                _ => Err(CallbackError::UnsupportedSignature { sig: $sig.clone() }),
            }
        };
    }

    let js = JsCall::marshal(sig, args)?;
    js_dispatch!(js, sig, ptr;
        [] => (),
        [i32 => I32] => (js.arg_i32(0)?),
        [i64 => I64] => (js.arg_i64(0)?),
        [u64 => U64] => (js.arg_u64(0)?),
        [f64 => F64] => (js.arg_f64(0)?),
        [u8 => Bool] => (js.arg_u8(0)?),
        [*const std::os::raw::c_char => Str] => (js.arg_cstring(0)?),
        [i32 => I32, i32 => I32] => (js.arg_i32(0)?, js.arg_i32(1)?),
        [i32 => I32, i64 => I64] => (js.arg_i32(0)?, js.arg_i64(1)?),
        [i32 => I32, u64 => U64] => (js.arg_i32(0)?, js.arg_u64(1)?),
        [i32 => I32, f64 => F64] => (js.arg_i32(0)?, js.arg_f64(1)?),
        [i32 => I32, u8 => Bool] => (js.arg_i32(0)?, js.arg_u8(1)?),
        [i32 => I32, *const std::os::raw::c_char => Str] => (js.arg_i32(0)?, js.arg_cstring(1)?),
        [i64 => I64, i32 => I32] => (js.arg_i64(0)?, js.arg_i32(1)?),
        [i64 => I64, i64 => I64] => (js.arg_i64(0)?, js.arg_i64(1)?),
        [i64 => I64, u64 => U64] => (js.arg_i64(0)?, js.arg_u64(1)?),
        [i64 => I64, f64 => F64] => (js.arg_i64(0)?, js.arg_f64(1)?),
        [i64 => I64, u8 => Bool] => (js.arg_i64(0)?, js.arg_u8(1)?),
        [i64 => I64, *const std::os::raw::c_char => Str] => (js.arg_i64(0)?, js.arg_cstring(1)?),
        [u64 => U64, i32 => I32] => (js.arg_u64(0)?, js.arg_i32(1)?),
        [u64 => U64, i64 => I64] => (js.arg_u64(0)?, js.arg_i64(1)?),
        [u64 => U64, u64 => U64] => (js.arg_u64(0)?, js.arg_u64(1)?),
        [u64 => U64, f64 => F64] => (js.arg_u64(0)?, js.arg_f64(1)?),
        [u64 => U64, u8 => Bool] => (js.arg_u64(0)?, js.arg_u8(1)?),
        [u64 => U64, *const std::os::raw::c_char => Str] => (js.arg_u64(0)?, js.arg_cstring(1)?),
        [f64 => F64, i32 => I32] => (js.arg_f64(0)?, js.arg_i32(1)?),
        [f64 => F64, i64 => I64] => (js.arg_f64(0)?, js.arg_i64(1)?),
        [f64 => F64, u64 => U64] => (js.arg_f64(0)?, js.arg_u64(1)?),
        [f64 => F64, f64 => F64] => (js.arg_f64(0)?, js.arg_f64(1)?),
        [f64 => F64, u8 => Bool] => (js.arg_f64(0)?, js.arg_u8(1)?),
        [f64 => F64, *const std::os::raw::c_char => Str] => (js.arg_f64(0)?, js.arg_cstring(1)?),
        [u8 => Bool, i32 => I32] => (js.arg_u8(0)?, js.arg_i32(1)?),
        [u8 => Bool, i64 => I64] => (js.arg_u8(0)?, js.arg_i64(1)?),
        [u8 => Bool, u64 => U64] => (js.arg_u8(0)?, js.arg_u64(1)?),
        [u8 => Bool, f64 => F64] => (js.arg_u8(0)?, js.arg_f64(1)?),
        [u8 => Bool, u8 => Bool] => (js.arg_u8(0)?, js.arg_u8(1)?),
        [u8 => Bool, *const std::os::raw::c_char => Str] => (js.arg_u8(0)?, js.arg_cstring(1)?),
        [*const std::os::raw::c_char => Str, i32 => I32] => (js.arg_cstring(0)?, js.arg_i32(1)?),
        [*const std::os::raw::c_char => Str, i64 => I64] => (js.arg_cstring(0)?, js.arg_i64(1)?),
        [*const std::os::raw::c_char => Str, u64 => U64] => (js.arg_cstring(0)?, js.arg_u64(1)?),
        [*const std::os::raw::c_char => Str, f64 => F64] => (js.arg_cstring(0)?, js.arg_f64(1)?),
        [*const std::os::raw::c_char => Str, u8 => Bool] => (js.arg_cstring(0)?, js.arg_u8(1)?),
        [*const std::os::raw::c_char => Str, *const std::os::raw::c_char => Str] => (js.arg_cstring(0)?, js.arg_cstring(1)?),
    )
}

/// Invokes the native callback behind `handle` with `args` from ANY
/// thread, blocking until the outcome arrives or `timeout` expires
/// (marshal-and-wait, the cross-thread counterpart of [`invoke`]).
///
/// The dispatch is by handle kind:
///
/// - A JS-bound handle (`bind_js_callback`, tag `0x0201`): the same
///   slot mechanics, but the job calls the bound `bun:ffi`
///   JSCallback pointer on the JS thread - the arguments cross as
///   the declared C types (`i32`/`i64`/`u64`/`f64`/`u8`/`cstring`)
///   and the raw C result is wrapped back into a [`Value`]. The
///   signature and the C-call matrix are checked on the CALLING
///   thread first (fail fast, nothing queued); the job re-validates
///   the lookup, so a revocation during the wait crosses the slot as
///   [`CallbackError::InvalidHandle`].
/// - A native handle (`register`, tag `0x0200`): as before.
///
/// - The calling thread is the bound JS thread, or the process is
///   unbound (the P1 policy admits every caller then): the call runs
///   directly - no channel, no extra latency.
/// - Any other native thread: the call is marshalled through the
///   event loop. A queued job runs the invocation on the JS thread
///   (i.e. while it drains with `bffi::pump` / `bffi::run`) and
///   stores the full outcome in a shared [`WaitSlot`]; the calling
///   thread parks on the slot until it fills. The value AND every
///   [`CallbackError`] - dead handle, signature mismatch - cross the
///   slot unchanged.
///
/// Timeout semantics: an expired `timeout` yields
/// [`CallbackError::Timeout`] even though the job may still run
/// later; the late outcome is stored into the abandoned slot and
/// ignored (ignore-after-timeout), leaving the loop and the registry
/// fully usable - a following `invoke_wait` is unaffected. A body
/// that panics is contained by the loop's boundary policy and never
/// reaches the slot, so the waiter observes the timeout. A loop
/// stopped after the job was queued does not wake the waiter either
/// - the timeout is the only exit there.
///
/// Deadlock contract: the JS thread MUST keep draining the loop while
/// a native thread waits. A re-entrant wait - the JS thread itself
/// inside a native call that `invoke_wait`s back into JS - can never
/// complete: the parked thread cannot also drain the loop. Every wait
/// section is therefore wrapped in a per-thread wait-depth gate, and a
/// NESTED `invoke_wait` on the same thread fails FAST with
/// [`CallbackError::ReentrantWait`] (status `16`) instead of burning
/// the timeout.
///
/// # Errors
///
/// [`CallbackError::InvalidHandle`] / [`CallbackError::SignatureMismatch`]
/// / [`CallbackError::NullPointer`] / [`CallbackError::
/// UnsupportedSignature`] from the invoked entry;
/// [`CallbackError::ReentrantWait`] when the CALLING thread is already
/// waiting inside another `invoke_wait`;
/// [`CallbackError::Timeout`] when `timeout` expired before the JS
/// thread delivered; [`CallbackError::LoopStopped`] when the event
/// loop has been stopped, so the job could not be queued at all
/// (immediate failure, nothing to wait for).
#[cfg(feature = "event-loop")]
pub fn invoke_wait(
    handle: Handle,
    args: &[Value],
    timeout: Duration,
) -> Result<Value, CallbackError> {
    tables()?;
    if handle.is_null() {
        return Err(CallbackError::InvalidHandle(handle));
    }
    // JS-bound table first: it shares the slot mechanics but has no
    // ordinary `invoke` to delegate to (its pointer is called, not
    // its body).
    if Registry::global().get_typed::<JsEntry>(handle).is_some() {
        return invoke_js_entry_wait(handle, args, timeout);
    }
    if ensure_js_thread().is_ok() {
        return invoke(handle, args);
    }
    // Re-entrancy gate: a thread already parked inside a wait section
    // cannot also drain the loop its job depends on, so a nested
    // `invoke_wait` fails fast instead of burning the timeout. The
    // depth is released before every return below.
    if try_begin_wait().is_err() {
        return Err(CallbackError::ReentrantWait);
    }
    let slot = Arc::new(WaitSlot::default());
    let job_slot = Arc::clone(&slot);
    let job_args = args.to_vec();
    let outcome = match crate::bffi_event_loop::enqueue(Box::new(move || {
        job_slot.fill(invoke(handle, &job_args));
    })) {
        Ok(()) => slot.take_within(timeout),
        Err(_) => Err(CallbackError::LoopStopped),
    };
    end_wait();
    outcome
}

/// The JS-bound half of [`invoke_wait`]: fail-fast validation on the
/// calling thread, then the direct call (the owning JS thread or an
/// unbound process) or the marshal job with the shared [`WaitSlot`].
///
/// The marshal is TARGETED when the entry recorded its owning
/// isolate: the job is queued on that thread's slot queue and never
/// crosses an isolate boundary. Entries bound while the process was
/// unbound (`thread == 0`, pure-Rust usage) keep the legacy
/// untargeted delivery.
///
/// The wait section carries the same per-thread re-entrancy gate as
/// the native path: a nested wait fails fast with
/// [`CallbackError::ReentrantWait`].
#[cfg(feature = "event-loop")]
fn invoke_js_entry_wait(
    handle: Handle,
    args: &[Value],
    timeout: Duration,
) -> Result<Value, CallbackError> {
    let thread = {
        let entry = Registry::global()
            .get_typed::<JsEntry>(handle)
            .ok_or(CallbackError::InvalidHandle(handle))?;
        check_js_call_matrix(&entry.sig)?;
        if !entry.sig.matches(args) {
            return Err(CallbackError::SignatureMismatch {
                expected: entry.sig.clone(),
                got: got_types(args),
            });
        }
        entry.thread
    };
    if ensure_js_thread().is_ok() {
        return invoke_js_entry(handle, args);
    }
    // The same re-entrancy gate as the native path: a thread already
    // parked inside a wait section cannot also drain the loop its job
    // depends on. The depth is released before every return below.
    if try_begin_wait().is_err() {
        return Err(CallbackError::ReentrantWait);
    }
    let slot = Arc::new(WaitSlot::default());
    let job_slot = Arc::clone(&slot);
    let job_args = args.to_vec();
    let queued = if thread == 0 {
        crate::bffi_event_loop::enqueue(Box::new(move || {
            // Fresh lookup: a revocation during the wait lands here, like
            // the native path - no call through a dead slot.
            job_slot.fill(invoke_js_entry(handle, &job_args));
        }))
    } else {
        crate::bffi_event_loop::enqueue_to(
            thread,
            Box::new(move || {
                // Fresh lookup: a revocation during the wait lands here,
                // like the native path - no call through a dead slot.
                job_slot.fill(invoke_js_entry(handle, &job_args));
            }),
        )
    };
    let outcome = match queued {
        Ok(()) => slot.take_within(timeout),
        Err(_) => Err(CallbackError::LoopStopped),
    };
    end_wait();
    outcome
}

/// Binds a JS-side callback (Rust -> JS direction) and returns its
/// fresh, opaque [`Handle`].
///
/// `ptr` is an opaque, handle-sized token - in practice the
/// `JSCallback` pointer `bun:ffi` hands out. Two things happen with
/// it: [`js_callback`] hands it back as part of the slot snapshot,
/// and [`invoke_wait`]'s JS-bound dispatch CALLS it - on the JS
/// thread, through the concrete `extern "C"` shape the stored
/// [`CallbackSig`] declares (`i32`/`i64`/`u64`/`f64`/`u8`/`cstring`
/// parameters, `i32`/`i64`/`u64`/`f64`/`u8`/`cstring`/`void` return;
/// see [`check_js_call_matrix`]). A `ptr` of `0` is a legal opaque value
/// in v1 - no validation is performed beyond the type; an invocation
/// through such a handle reports [`CallbackError::NullPointer`].
///
/// The declared [`CallbackSig`] is carried verbatim in the slot;
/// checking the arguments when the callback is raised is shared by
/// the JS side (its own convention) and this crate's dispatch.
///
/// # Errors
///
/// [`CallbackError::TableFull`] when the JS table has no free slot, or
/// [`CallbackError::TagInUse`] if table initialization ever failed
/// earlier in the process (the memoized outcome).
pub fn bind_js_callback(sig: CallbackSig, ptr: usize) -> Result<Handle, CallbackError> {
    tables()?;
    Registry::global()
        .insert(
            JS_TAG,
            Arc::new(JsEntry {
                sig,
                ptr,
                #[cfg(feature = "event-loop")]
                thread: super::thread::binding_thread(),
            }),
        )
        .map_err(|_| {
            // `tables()` declared JS_TAG for `JsEntry` and the registry
            // has no undeclare, so `NotRegistered` is unreachable after
            // declare; the remaining condition is a full table.
            // `RegistryError` is `#[non_exhaustive]`, so future variants
            // are grouped defensively into the wildcard (same pattern as
            // bffi-object's wrap.rs).
            CallbackError::TableFull
        })
}

/// Reads back the JS-side callback behind `handle`.
///
/// Returns a [`JsCallbackInfo`] snapshot: the signature declared at
/// [`bind_js_callback`] time plus the opaque pointer token. Check
/// order: table init -> null handle -> table lookup (wrong tag, stale,
/// or unknown handles all land in [`CallbackError::InvalidHandle`]).
///
/// Revocation goes through the SAME [`revoke`] as the native direction
/// (criterion 5.1: removal = revoke, both directions) - after
/// `revoke(handle)`, this call reports
/// [`CallbackError::InvalidHandle`].
///
/// # Errors
///
/// [`CallbackError::InvalidHandle`] for a null, revoked, stale, or
/// foreign-kind handle; [`CallbackError::TagInUse`] if table
/// initialization ever failed earlier in the process (the memoized
/// outcome).
pub fn js_callback(handle: Handle) -> Result<JsCallbackInfo, CallbackError> {
    tables()?;
    if handle.is_null() {
        return Err(CallbackError::InvalidHandle(handle));
    }
    let entry = Registry::global()
        .get_typed::<JsEntry>(handle)
        .ok_or(CallbackError::InvalidHandle(handle))?;
    Ok(JsCallbackInfo {
        sig: entry.sig.clone(),
        ptr: entry.ptr,
    })
}

/// Revokes the callback behind `handle`, whatever direction it
/// belongs to.
///
/// This is the single removal point for both callback directions
/// (criterion 5.1: revocation = `revoke`): the registry's type-erased
/// `remove` routes by the handle's tag to the native OR the JS table.
/// Revocation is terminal - the handle never resurrects, even if its
/// slot is reused (criterion 5.2).
///
/// Revocation is contained to the two callback tags: a handle owned by
/// any other table (another crate's or an undeclared user tag) is a
/// no-op that returns `false` and leaves its slot untouched. Within
/// the two tags removal stays type-erased - callers never need to know
/// which kind a handle belongs to.
///
/// Returns `true` iff a live callback slot was removed. Table
/// initialization failure is ignored gracefully (nothing was ever
/// stored, so there is nothing to remove), as are the null handle and
/// any handle outside the callback tags.
pub fn revoke(handle: Handle) -> bool {
    if tables().ok().is_none() {
        return false;
    }
    if handle.is_null() {
        return false;
    }
    if !matches!(handle.tag(), NATIVE_TAG | JS_TAG) {
        return false;
    }
    Registry::global().remove(handle)
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;
    use std::os::raw::c_char;
    use std::sync::Arc;

    use crate::bffi_core::{Handle, Registry, TypeTag};

    use super::{bind_js_callback, invoke, invoke_wait, js_callback, register, revoke};
    use crate::bffi_callback::error::CallbackError;
    use crate::bffi_callback::value::{CallbackSig, Value, ValueType};
    use std::time::Duration;

    // NOTE (test isolation): these tests run in the lib test binary
    // where the process stays UNBOUND - they must never call
    // `set_js_thread` (see `src/thread.rs`). `ensure_js_thread` admits
    // every caller while unbound, so `invoke` needs no binding ritual,
    // and `invoke_wait` takes the direct path even on a spawned
    // thread. The JS-bound dispatch therefore runs on the calling
    // thread - the stand-in fns below play the JSCallback pointers.

    /// Stand-in for a `bun:ffi` JSCallback pointer: `(i32) -> i32`.
    extern "C" fn fake_js_double(x: i32) -> i32 {
        x.wrapping_mul(2)
    }

    /// Stand-in for a `bun:ffi` JSCallback pointer: `() -> u8` (bool).
    extern "C" fn fake_js_ping() -> u8 {
        1
    }

    /// Stand-in for a `bun:ffi` JSCallback pointer:
    /// `(cstring) -> i32` (the string's length).
    extern "C" fn fake_js_len(text: *const c_char) -> i32 {
        if text.is_null() {
            return 0;
        }
        // SAFETY: the test contract passes a NUL-terminated literal
        // that outlives the call.
        unsafe { CStr::from_ptr(text) }.count_bytes() as i32
    }

    #[test]
    fn register_invoke_returns_the_closure_result() {
        let sig = CallbackSig::new(ValueType::I32, &[ValueType::I32, ValueType::I32]);
        let handle = register(
            sig,
            Arc::new(|args: &[Value]| {
                let mut sum = 0;
                for arg in args {
                    if let Value::I32(n) = arg {
                        sum += *n;
                    }
                }
                Value::I32(sum)
            }),
        )
        .unwrap();

        let result = invoke(handle, &[Value::I32(2), Value::I32(3)]).unwrap();
        assert_eq!(result, Value::I32(5));
    }

    #[test]
    fn invoke_after_revoke_is_invalid_handle() {
        let sig = CallbackSig::new(ValueType::Bool, &[]);
        let handle = register(sig, Arc::new(|_| Value::Bool(true))).unwrap();

        assert!(revoke(handle));
        assert_eq!(
            invoke(handle, &[]).err(),
            Some(CallbackError::InvalidHandle(handle))
        );
    }

    #[test]
    fn revoke_returns_true_exactly_once() {
        let sig = CallbackSig::new(ValueType::Bool, &[]);
        let handle = register(sig, Arc::new(|_| Value::Bool(false))).unwrap();

        assert!(revoke(handle));
        assert!(!revoke(handle));
    }

    #[test]
    fn invoke_rejects_wrong_arity_and_types() {
        let expected = CallbackSig::new(ValueType::I32, &[ValueType::I32, ValueType::I32]);
        let handle = register(expected.clone(), Arc::new(|_| Value::I32(0))).unwrap();

        let arity_error = invoke(handle, &[Value::I32(1)]).unwrap_err();
        assert_eq!(
            arity_error,
            CallbackError::SignatureMismatch {
                expected: expected.clone(),
                got: vec![ValueType::I32],
            }
        );

        let type_error = invoke(handle, &[Value::I32(1), Value::I64(2)]).unwrap_err();
        assert_eq!(
            type_error,
            CallbackError::SignatureMismatch {
                expected,
                got: vec![ValueType::I32, ValueType::I64],
            }
        );
    }

    #[test]
    fn invoke_rejects_null_handle() {
        assert_eq!(
            invoke(Handle::NULL, &[]).err(),
            Some(CallbackError::InvalidHandle(Handle::NULL))
        );
        assert!(!revoke(Handle::NULL));
    }

    #[test]
    fn invoke_wait_delegates_to_the_sync_invoke_in_an_unbound_process() {
        // NOTE: this lib test binary stays UNBOUND (see the module
        // note below), so `invoke_wait` takes the direct path even on
        // a spawned thread - no marshal job may reach the loop.
        let sig = CallbackSig::new(ValueType::I32, &[ValueType::I32, ValueType::I32]);
        let handle = register(
            sig,
            Arc::new(|args: &[Value]| match args {
                [Value::I32(a), Value::I32(b)] => Value::I32(a + b),
                _ => unreachable!("invoke checks the signature before calling"),
            }),
        )
        .unwrap();

        let joined = std::thread::spawn(move || {
            invoke_wait(
                handle,
                &[Value::I32(2), Value::I32(3)],
                Duration::from_secs(5),
            )
        })
        .join()
        .unwrap();
        assert_eq!(joined, Ok(Value::I32(5)));
    }

    #[test]
    fn invoke_wait_js_bound_calls_the_bound_pointer() {
        // The dispatch transmutes the bound pointer to the declared C
        // shape; a plain extern "C" fn stands in for the JSCallback.
        let handle = bind_js_callback(
            CallbackSig::new(ValueType::I32, &[ValueType::I32]),
            fake_js_double as extern "C" fn(i32) -> i32 as usize,
        )
        .unwrap();
        let result = invoke_wait(handle, &[Value::I32(21)], Duration::ZERO);
        assert_eq!(result, Ok(Value::I32(42)));

        // The bool return (bun:ffi spells it `u8`).
        let handle = bind_js_callback(
            CallbackSig::new(ValueType::Bool, &[]),
            fake_js_ping as extern "C" fn() -> u8 as usize,
        )
        .unwrap();
        let result = invoke_wait(handle, &[], Duration::ZERO);
        assert_eq!(result, Ok(Value::Bool(true)));
    }

    #[test]
    fn invoke_wait_js_bound_crosses_strings_as_cstrings() {
        let handle = bind_js_callback(
            CallbackSig::new(ValueType::I32, &[ValueType::Str]),
            fake_js_len as extern "C" fn(*const c_char) -> i32 as usize,
        )
        .unwrap();
        let result = invoke_wait(handle, &[Value::Str("héllo".to_owned())], Duration::ZERO);
        // "héllo" is 6 bytes of UTF-8.
        assert_eq!(result, Ok(Value::I32(6)));
    }

    #[test]
    fn invoke_wait_js_bound_null_pointer_is_rejected() {
        let handle = bind_js_callback(CallbackSig::new(ValueType::Bool, &[]), 0).unwrap();
        assert_eq!(
            invoke_wait(handle, &[], Duration::ZERO).err(),
            Some(CallbackError::NullPointer)
        );
    }

    #[test]
    fn invoke_wait_js_bound_rejects_uncalleble_signatures_early() {
        // `Bytes` parameters have no C spelling in the dispatch.
        let handle = bind_js_callback(
            CallbackSig::new(ValueType::Unit, &[ValueType::Bytes]),
            fake_js_ping as extern "C" fn() -> u8 as usize,
        )
        .unwrap();
        assert_eq!(
            invoke_wait(handle, &[], Duration::ZERO).err(),
            Some(CallbackError::UnsupportedSignature {
                sig: CallbackSig::new(ValueType::Unit, &[ValueType::Bytes]),
            })
        );
    }

    #[test]
    fn invoke_wait_js_bound_rejects_wrong_typing_without_a_queue() {
        let handle = bind_js_callback(CallbackSig::new(ValueType::Bool, &[]), 0).unwrap();
        // The mismatch is detected on the CALLING thread - no timeout
        // wait, no marshal job.
        assert_eq!(
            invoke_wait(handle, &[Value::I32(1)], Duration::ZERO).err(),
            Some(CallbackError::SignatureMismatch {
                expected: CallbackSig::new(ValueType::Bool, &[]),
                got: vec![ValueType::I32],
            })
        );
        assert_eq!(crate::bffi_event_loop::pending(), 0);
    }

    #[test]
    fn bind_then_js_callback_returns_the_slot() {
        let sig = CallbackSig::new(ValueType::Bool, &[ValueType::I32]);
        let ptr = 0xdead_beef_usize;
        let handle = bind_js_callback(sig, ptr).unwrap();

        let info = js_callback(handle).unwrap();
        assert_eq!(
            info.sig,
            CallbackSig::new(ValueType::Bool, &[ValueType::I32])
        );
        assert_eq!(info.ptr, ptr);
    }

    #[test]
    fn js_callback_after_revoke_is_invalid_handle() {
        let sig = CallbackSig::new(ValueType::Bool, &[]);
        let handle = bind_js_callback(sig, 0).unwrap();

        assert!(revoke(handle));
        assert_eq!(
            js_callback(handle).err(),
            Some(CallbackError::InvalidHandle(handle))
        );
    }

    #[test]
    fn revoke_removes_js_slots_through_the_same_entry_point() {
        let native = register(
            CallbackSig::new(ValueType::Bool, &[]),
            Arc::new(|_| Value::Bool(true)),
        )
        .unwrap();
        let js = bind_js_callback(CallbackSig::new(ValueType::Bool, &[]), 0).unwrap();

        assert!(revoke(native));
        assert!(revoke(js));
        assert!(!revoke(native));
        assert!(!revoke(js));
    }

    #[test]
    fn revoke_ignores_handles_outside_callback_tags() {
        const FOREIGN: TypeTag = TypeTag(0x8100);

        Registry::global().declare::<u32>(FOREIGN).unwrap();
        let foreign = Registry::global()
            .insert(FOREIGN, Arc::new(42_u32))
            .unwrap();

        assert!(!revoke(foreign));
        assert!(Registry::global().get_typed::<u32>(foreign).is_some());
        assert!(!revoke(Handle::new(TypeTag(0x8F00), 0, 0)));
    }
}
