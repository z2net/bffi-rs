//! Domain errors of the callback layer and their conversion to
//! [`BffiError`].
//!
//! One variant per failure mode of the callback registry; the
//! conversion targets existing `ErrorCode`s plus `WrongThread` (P2:
//! dedicated code for calls that could not be marshalled to the JS
//! thread) and `ReentrantCall` (the `invoke_wait` re-entrancy gate).
//!
//! Display texts are part of the contract: they travel across the C ABI
//! as `BffiError.message` and must stay deterministic (see the `Display`
//! impl for the exact format).

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
use std::fmt;

use bffi_core::{BffiError, ErrorCode, Handle, TypeTag};

use super::value::{CallbackSig, ValueType};

/// Everything that can go wrong in the callback layer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallbackError {
    /// The handle is null, revoked (dead), or belongs to another kind.
    InvalidHandle(Handle),
    /// The arguments received by the callback do not satisfy its
    /// declared signature.
    SignatureMismatch {
        /// The declared signature of the callback.
        expected: CallbackSig,
        /// The value types actually received, in arrival order.
        got: Vec<ValueType>,
    },
    /// The callback was invoked from a thread other than the one that
    /// owns it (P1 rule; cross-thread marshalling is deferred to P2).
    WrongThread,
    /// The callback table has no free slots.
    TableFull,
    /// The type tag is already declared - one tag serves one callback
    /// table per process.
    TagInUse(TypeTag),
    /// The `invoke_wait` timeout expired before the JS thread
    /// delivered the outcome (marshal-and-wait); the late outcome, if
    /// any, is ignored and the loop/registry state stays usable.
    Timeout,
    /// `invoke_wait` could not queue its marshal job: the event loop
    /// has been stopped (sticky, never restarts), so the wait fails
    /// immediately instead of timing out.
    LoopStopped,
    /// A NESTED `invoke_wait` was attempted from a thread already
    /// waiting inside one: the parked thread cannot also drain the
    /// event loop its job depends on, so the wait fails fast (status
    /// `16`) instead of burning the timeout (the §9.1 deadlock
    /// contract).
    ReentrantWait,
    /// A JS-bound call found the bound pointer null: the slot is live,
    /// but there is no JS trampoline behind it (`0` is a legal opaque
    /// value at bind time).
    NullPointer,
    /// A `Str` argument of a JS-bound call carries an interior NUL
    /// byte and cannot cross as a `cstring`.
    InvalidCString,
    /// The signature of a JS-bound callback cannot cross the raw
    /// C call that `invoke_wait`'s dispatch performs (a `Bytes` or
    /// `Wire` parameter, a `Str`/`Bytes`/`Wire` return, or more than
    /// two parameters). Those types ride the buffered channels
    /// (`invoke`, async results) instead of the direct C call.
    UnsupportedSignature {
        /// The declared signature that cannot be called.
        sig: CallbackSig,
    },
}

/// Renders a [`ValueType`] as it is spelled in signatures and error
/// messages (`i32`, `i64`, `u64`, `f64`, `bool`, `str`).
fn render_value_type(ty: ValueType) -> &'static str {
    match ty {
        ValueType::Unit => "unit",
        ValueType::I32 => "i32",
        ValueType::I64 => "i64",
        ValueType::U64 => "u64",
        ValueType::F64 => "f64",
        ValueType::Bool => "bool",
        ValueType::Str => "str",
        ValueType::Bytes => "bytes",
        ValueType::Wire => "wire",
    }
}

/// Renders a value-type list inline (`i32, f64`); an empty list renders
/// as `(none)`.
fn render_got(types: &[ValueType]) -> String {
    if types.is_empty() {
        return "(none)".to_owned();
    }
    let mut joined = String::new();
    for (position, ty) in types.iter().enumerate() {
        if position > 0 {
            joined.push_str(", ");
        }
        joined.push_str(render_value_type(*ty));
    }
    joined
}

/// Renders a [`CallbackSig`] as `ret(params...)`, e.g. `i32(i32, f64)`;
/// an empty parameter list renders as `i32()`.
fn render_expected(sig: &CallbackSig) -> String {
    let mut rendered = String::from(render_value_type(sig.ret()));
    rendered.push('(');
    for (position, param) in sig.params().iter().enumerate() {
        if position > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(render_value_type(*param));
    }
    rendered.push(')');
    rendered
}

/// Formats the error for humans and for the C ABI (`BffiError.message`).
///
/// The format is deterministic: signatures render as `ret(params...)`
/// (e.g. `i32(i32, f64)`, empty params -> `i32()`); value-type lists
/// render inline as `i32, f64` (empty list -> `(none)`).
impl fmt::Display for CallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHandle(handle) => {
                write!(
                    f,
                    "callback handle {handle} is revoked or belongs to another kind"
                )
            }
            Self::SignatureMismatch { expected, got } => {
                write!(
                    f,
                    "callback signature mismatch: expected {}, got {}",
                    render_expected(expected),
                    render_got(got)
                )
            }
            Self::WrongThread => {
                write!(f, "callback invoked from a non-JS thread")
            }
            Self::TableFull => {
                write!(f, "callback table is full")
            }
            Self::TagInUse(tag) => {
                write!(f, "callback type tag {tag} is already declared")
            }
            Self::Timeout => {
                write!(
                    f,
                    "callback invoke_wait timed out waiting for the JS thread"
                )
            }
            Self::LoopStopped => {
                write!(
                    f,
                    "callback invoke_wait could not be queued: the event loop is stopped"
                )
            }
            Self::ReentrantWait => {
                write!(
                    f,
                    "re-entrant invoke_wait: the calling JS thread is inside a native call; the loop cannot drain - restructure so native work runs off the JS thread"
                )
            }
            Self::NullPointer => {
                write!(f, "the bound JS callback pointer is null")
            }
            Self::InvalidCString => {
                write!(f, "callback string argument carries an interior NUL byte")
            }
            Self::UnsupportedSignature { sig } => {
                write!(
                    f,
                    "callback signature {} cannot cross the JS-bound C call",
                    render_expected(sig)
                )
            }
        }
    }
}

impl std::error::Error for CallbackError {}

/// Unified-format conversion: stale/foreign handles map to
/// `InvalidHandle`, signature mismatches to `InvalidArgument`,
/// wrong-thread calls to the dedicated `WrongThread` code (P2; the
/// message still distinguishes the cause), a full table to `TableFull`,
/// an already-declared tag to `InvalidTag`; an expired `invoke_wait`
/// timeout to the dedicated `Timeout` code, a marshal job that
/// could not be queued (stopped loop) to `Error` - the same mapping
/// the event-loop layer uses for its `Stopped` - and a nested
/// `invoke_wait` (the calling thread already waiting inside one) to
/// the dedicated `ReentrantCall` code; a null JS-bound
/// pointer to `NullPointer`, and the JS-bound C-call rejections
/// (interior NUL, uncalleble signature) to `InvalidArgument` - the
/// domain error is preserved as the source.
impl From<CallbackError> for BffiError {
    fn from(error: CallbackError) -> Self {
        let code = match &error {
            CallbackError::InvalidHandle(_) => ErrorCode::InvalidHandle,
            CallbackError::SignatureMismatch { .. } => ErrorCode::InvalidArgument,
            CallbackError::WrongThread => ErrorCode::WrongThread,
            CallbackError::TableFull => ErrorCode::TableFull,
            CallbackError::TagInUse(_) => ErrorCode::InvalidTag,
            CallbackError::Timeout => ErrorCode::Timeout,
            CallbackError::LoopStopped => ErrorCode::Error,
            CallbackError::ReentrantWait => ErrorCode::ReentrantCall,
            CallbackError::NullPointer => ErrorCode::NullPointer,
            CallbackError::InvalidCString => ErrorCode::InvalidArgument,
            CallbackError::UnsupportedSignature { .. } => ErrorCode::InvalidArgument,
        };
        BffiError::with_source(code, error.to_string(), error)
    }
}

#[cfg(test)]
mod tests {
    use crate::bffi_callback::value::{CallbackSig, ValueType};
    use crate::bffi_core::{BffiError, ErrorCode, Handle, TypeTag};

    use super::CallbackError;

    #[test]
    fn invalid_handle_display_names_the_handle_and_cause() {
        let handle = Handle::new(TypeTag(0x0200), 1, 2);
        let error = CallbackError::InvalidHandle(handle);
        assert_eq!(
            error.to_string(),
            "callback handle Handle(tag:0x0200, gen:1, idx:2) is revoked or belongs to another kind"
        );
    }

    #[test]
    fn wrong_thread_display_states_the_p1_thread_rule() {
        assert_eq!(
            CallbackError::WrongThread.to_string(),
            "callback invoked from a non-JS thread"
        );
    }

    #[test]
    fn signature_mismatch_display_renders_expected_and_got() {
        let error = CallbackError::SignatureMismatch {
            expected: CallbackSig::new(ValueType::I32, &[ValueType::I32, ValueType::F64]),
            got: vec![ValueType::I64, ValueType::Bool],
        };
        assert_eq!(
            error.to_string(),
            "callback signature mismatch: expected i32(i32, f64), got i64, bool"
        );
    }

    #[test]
    fn signature_mismatch_display_renders_empty_got_as_none() {
        let error = CallbackError::SignatureMismatch {
            expected: CallbackSig::new(ValueType::Bool, &[]),
            got: Vec::new(),
        };
        assert_eq!(
            error.to_string(),
            "callback signature mismatch: expected bool(), got (none)"
        );
    }

    #[test]
    fn table_full_and_tag_in_use_display_their_state() {
        assert_eq!(
            CallbackError::TableFull.to_string(),
            "callback table is full"
        );
        assert_eq!(
            CallbackError::TagInUse(TypeTag(0x0202)).to_string(),
            "callback type tag 0x0202 is already declared"
        );
    }

    #[test]
    fn wait_failure_displays_display_their_cause() {
        assert_eq!(
            CallbackError::Timeout.to_string(),
            "callback invoke_wait timed out waiting for the JS thread"
        );
        assert_eq!(
            CallbackError::LoopStopped.to_string(),
            "callback invoke_wait could not be queued: the event loop is stopped"
        );
        assert_eq!(
            CallbackError::ReentrantWait.to_string(),
            "re-entrant invoke_wait: the calling JS thread is inside a native call; the loop cannot drain - restructure so native work runs off the JS thread"
        );
    }

    #[test]
    fn js_bridge_failures_display_their_cause() {
        assert_eq!(
            CallbackError::NullPointer.to_string(),
            "the bound JS callback pointer is null"
        );
        assert_eq!(
            CallbackError::InvalidCString.to_string(),
            "callback string argument carries an interior NUL byte"
        );
        assert_eq!(
            CallbackError::UnsupportedSignature {
                sig: CallbackSig::new(ValueType::Unit, &[ValueType::Bytes])
            }
            .to_string(),
            "callback signature unit(bytes) cannot cross the JS-bound C call"
        );
    }

    #[test]
    fn converts_to_bffi_error_on_existing_codes() {
        let tag = TypeTag(0x0200);
        let cases = [
            (
                CallbackError::InvalidHandle(Handle::NULL),
                ErrorCode::InvalidHandle,
            ),
            (
                CallbackError::SignatureMismatch {
                    expected: CallbackSig::new(ValueType::I32, &[ValueType::I32]),
                    got: vec![ValueType::I64],
                },
                ErrorCode::InvalidArgument,
            ),
            (CallbackError::WrongThread, ErrorCode::WrongThread),
            (CallbackError::TableFull, ErrorCode::TableFull),
            (CallbackError::TagInUse(tag), ErrorCode::InvalidTag),
            (CallbackError::Timeout, ErrorCode::Timeout),
            (CallbackError::LoopStopped, ErrorCode::Error),
            (CallbackError::ReentrantWait, ErrorCode::ReentrantCall),
            (CallbackError::NullPointer, ErrorCode::NullPointer),
            (CallbackError::InvalidCString, ErrorCode::InvalidArgument),
            (
                CallbackError::UnsupportedSignature {
                    sig: CallbackSig::new(ValueType::Bytes, &[ValueType::Str]),
                },
                ErrorCode::InvalidArgument,
            ),
        ];
        for (error, code) in cases {
            let converted = BffiError::from(error);
            assert_eq!(converted.code, code);
            assert!(
                converted.source.is_some(),
                "the domain error must survive as source"
            );
        }
    }
}
