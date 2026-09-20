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
///
/// `thread` is the numeric id of the JS thread that performed the
/// binding (see `super::thread`): the JSCallback behind `ptr` belongs
/// to THAT isolate, so every call through this entry must run there.
/// `0` means the process was unbound at bind time (pure-Rust usage);
/// deliveries for such an entry use the legacy untargeted queue.
pub(super) struct JsEntry {
    pub(super) sig: CallbackSig,
    pub(super) ptr: usize,
    #[cfg(feature = "event-loop")]
    pub(super) thread: u64,
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
pub(super) fn tables() -> Result<(), CallbackError> {
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
    use std::sync::Arc;

    use crate::bffi_core::{Handle, Registry, TypeTag};

    use super::{bind_js_callback, invoke, js_callback, register, revoke};
    use crate::bffi_callback::error::CallbackError;
    use crate::bffi_callback::process_state_lock;
    use crate::bffi_callback::value::{CallbackSig, Value, ValueType};

    // NOTE (test isolation): these tests run in the lib test binary
    // where the process must stay UNBOUND for their whole body - they
    // never call `set_js_thread` themselves, and every one of them
    // holds the crate-wide `process_state_lock` so a parallel test
    // thread's binding keeper (thread.rs, event_loop.rs) cannot flip
    // the global gate mid-test. While unbound, `ensure_js_thread`
    // admits every caller, so `invoke` needs no binding ritual, and
    // `invoke_wait` (module `wait`) takes the direct path even on a
    // spawned thread.

    #[test]
    fn register_invoke_returns_the_closure_result() {
        let _guard = process_state_lock();
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
        let _guard = process_state_lock();
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
        let _guard = process_state_lock();
        let sig = CallbackSig::new(ValueType::Bool, &[]);
        let handle = register(sig, Arc::new(|_| Value::Bool(false))).unwrap();

        assert!(revoke(handle));
        assert!(!revoke(handle));
    }

    #[test]
    fn invoke_rejects_wrong_arity_and_types() {
        let _guard = process_state_lock();
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
        let _guard = process_state_lock();
        assert_eq!(
            invoke(Handle::NULL, &[]).err(),
            Some(CallbackError::InvalidHandle(Handle::NULL))
        );
        assert!(!revoke(Handle::NULL));
    }

    #[test]
    fn bind_then_js_callback_returns_the_slot() {
        let _guard = process_state_lock();
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
        let _guard = process_state_lock();
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
        let _guard = process_state_lock();
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
        let _guard = process_state_lock();
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
