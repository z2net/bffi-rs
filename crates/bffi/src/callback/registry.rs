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
use super::value::{CallbackSig, Value};

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
struct JsEntry {
    sig: CallbackSig,
    ptr: usize,
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

/// Invokes the native callback behind `handle` with `args` from ANY
/// thread, blocking until the outcome arrives or `timeout` expires
/// (marshal-and-wait, the cross-thread counterpart of [`invoke`]).
///
/// - The calling thread is the bound JS thread, or the process is
///   unbound (the P1 policy admits every caller then): delegates to
///   [`invoke`] directly - no channel, no extra latency.
/// - Any other native thread: the call is marshalled through the
///   event loop. A queued job runs the ordinary [`invoke`] on the JS
///   thread (i.e. while it drains with `bffi::pump` / `bffi::run`)
///   and stores the full outcome in a shared [`WaitSlot`]; the
///   calling thread parks on the slot until it fills. The value AND
///   every [`CallbackError`] - dead handle, signature mismatch -
///   cross the slot unchanged.
///
/// Timeout semantics: an expired `timeout` yields
/// [`CallbackError::Timeout`] even though the job may still run
/// later; the late outcome is stored into the abandoned slot and
/// ignored (ignore-after-timeout), leaving the loop and the registry
/// fully usable - a following `invoke_wait` is unaffected. A native
/// body that panics is contained by the loop's boundary policy and
/// never reaches the slot, so the waiter observes the timeout. A
/// loop stopped after the job was queued does not wake the waiter
/// either - the timeout is the only exit there.
///
/// Deadlock contract: the JS thread MUST keep draining the loop while
/// a native thread waits. A re-entrant wait - the JS thread itself
/// inside a native call that `invoke_wait`s back into JS - never
/// completes and ends in the timeout.
///
/// # Errors
///
/// [`CallbackError::InvalidHandle`] / [`CallbackError::SignatureMismatch`]
/// from the inner [`invoke`]; [`CallbackError::Timeout`] when
/// `timeout` expired before the JS thread delivered; [`CallbackError::
/// LoopStopped`] when the event loop has been stopped, so the job
/// could not be queued at all (immediate failure, nothing to wait
/// for).
#[cfg(feature = "event-loop")]
pub fn invoke_wait(
    handle: Handle,
    args: &[Value],
    timeout: Duration,
) -> Result<Value, CallbackError> {
    if ensure_js_thread().is_ok() {
        return invoke(handle, args);
    }
    let slot = Arc::new(WaitSlot::default());
    let job_slot = Arc::clone(&slot);
    let job_args = args.to_vec();
    crate::bffi_event_loop::enqueue(Box::new(move || {
        job_slot.fill(invoke(handle, &job_args));
    }))
    .map_err(|_| CallbackError::LoopStopped)?;
    slot.take_within(timeout)
}

/// Binds a JS-side callback (Rust -> JS direction) and returns its
/// fresh, opaque [`Handle`].
///
/// `ptr` is an opaque, handle-sized token - in the P2 world, the
/// `JSCallback` pointer `bun:ffi` hands out. It is stored and handed
/// back by [`js_callback`] but NEVER dereferenced here: the whole
/// crate is zero-unsafe. A `ptr` of `0` is a legal opaque value in v1
/// - no validation is performed beyond the type.
///
/// The declared [`CallbackSig`] is carried verbatim in the slot;
/// checking it when the callback is actually raised is the JS side's
/// job.
///
/// # Errors
///
/// [`CallbackError::TableFull`] when the JS table has no free slot, or
/// [`CallbackError::TagInUse`] if table initialization ever failed
/// earlier in the process (the memoized outcome).
pub fn bind_js_callback(sig: CallbackSig, ptr: usize) -> Result<Handle, CallbackError> {
    tables()?;
    Registry::global()
        .insert(JS_TAG, Arc::new(JsEntry { sig, ptr }))
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
    use std::sync::Arc;

    use crate::bffi_core::{Handle, Registry, TypeTag};

    use super::{bind_js_callback, invoke, invoke_wait, js_callback, register, revoke};
    use crate::bffi_callback::error::CallbackError;
    use crate::bffi_callback::value::{CallbackSig, Value, ValueType};
    use std::time::Duration;

    // NOTE (test isolation): these tests run in the lib test binary
    // where the process stays UNBOUND - they must never call
    // `set_js_thread` (see `src/thread.rs`). `ensure_js_thread` admits
    // every caller while unbound, so `invoke` needs no binding ritual.

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
