//! Error codes and the thread-local *last error* transport.
//!
//! The C ABI cannot return `Result`. Failures cross the boundary as a
//! numeric [`ErrorCode`] return value, optionally paired with a detailed
//! [`BffiError`] stored in a thread-local slot - the same scheme as
//! `errno` / `sqlite3_errmsg`:
//!
//! 1. an `extern "C"` function returns an [`ErrorCode`];
//! 2. on failure it records the details via [`set_last_error`];
//! 3. a companion export drains them with [`take_last_error`] so the JS
//!    side (through `bffi-error`) can build a JS `Error`.
//!
//! The slot is thread-local because Bun calls into the library on the JS
//! thread, while callbacks may execute on other threads; concurrent calls
//! must not overwrite each other's errors.

use super::catalog::RegistryError;
use super::table::TableError;
use std::cell::RefCell;
use std::fmt;

/// Numeric status code returned by `extern "C"` functions.
///
/// `0` ([`ErrorCode::Ok`]) means success; anything else signals failure and
/// usually implies a stored [last error](self) on the same thread.
///
/// This enum may grow: match with a wildcard arm.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
#[repr(u32)]
pub enum ErrorCode {
    /// Operation completed successfully.
    Ok = 0,
    /// Unspecified failure. Details, if any, are in the last error.
    Error = 1,
    /// A Rust panic was caught at the FFI boundary.
    Panic = 2,
    /// The null handle was passed where a real object is required.
    NullHandle = 3,
    /// The handle is unknown, stale (freed), or of the wrong type.
    InvalidHandle = 4,
    /// The handle table has no free slots.
    TableFull = 5,
    /// The type tag is not registered in the registry.
    InvalidTag = 6,
    /// A byte sequence is not valid UTF-8 where UTF-8 is required.
    InvalidUtf8 = 7,
    /// A numeric value is outside the range of the target type.
    NumberOutOfRange = 8,
    /// A required pointer was null.
    NullPointer = 9,
    /// The caller-provided output buffer is too small for the result.
    BufferTooSmall = 10,
    /// An argument violated its documented contract.
    InvalidArgument = 11,
    /// A call arrived from a non-JS thread and could not be marshalled
    /// to it (see `bffi-event-loop`).
    WrongThread = 12,
    /// A domain error reported by the native function through the
    /// `Result` error channel.
    DomainError = 13,
    /// A stream pull found the buffer empty while the producer is
    /// still alive: not an error - retry after the next wake.
    /// Returned by `bffi_stream_next` only.
    Pending = 14,
    /// A bounded cross-thread wait (`bffi::callback::invoke_wait`)
    /// expired before the JS thread delivered the outcome.
    Timeout = 15,
}

impl ErrorCode {
    /// Returns the numeric value passed across the C ABI.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    /// Decodes a numeric value received from the C ABI.
    ///
    /// Returns `None` for values this build does not know; callers should
    /// map those to [`ErrorCode::Error`].
    #[must_use]
    pub const fn from_u32(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Error),
            2 => Some(Self::Panic),
            3 => Some(Self::NullHandle),
            4 => Some(Self::InvalidHandle),
            5 => Some(Self::TableFull),
            6 => Some(Self::InvalidTag),
            7 => Some(Self::InvalidUtf8),
            8 => Some(Self::NumberOutOfRange),
            9 => Some(Self::NullPointer),
            10 => Some(Self::BufferTooSmall),
            11 => Some(Self::InvalidArgument),
            12 => Some(Self::WrongThread),
            13 => Some(Self::DomainError),
            14 => Some(Self::Pending),
            15 => Some(Self::Timeout),
            _ => None,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Ok => "ok",
            Self::Error => "generic error",
            Self::Panic => "panic caught at the FFI boundary",
            Self::NullHandle => "null handle",
            Self::InvalidHandle => "unknown or stale handle",
            Self::TableFull => "handle table is full",
            Self::InvalidTag => "unknown type tag",
            Self::InvalidUtf8 => "byte sequence is not valid UTF-8",
            Self::NumberOutOfRange => "number is out of the target range",
            Self::NullPointer => "unexpected null pointer",
            Self::BufferTooSmall => "output buffer is too small",
            Self::InvalidArgument => "invalid argument",
            Self::WrongThread => "call from a non-JS thread that could not be marshalled",
            Self::DomainError => "domain error reported by the native function",
            Self::Pending => "no items ready yet - the producer is still alive",
            Self::Timeout => "the bounded wait for a callback result timed out",
        };
        f.write_str(text)
    }
}

/// The unified failure format: a machine-readable code, a human-readable
/// message, and the originating domain error when there is one
/// (DESIGN §7, "Error format").
///
/// This is the Rust-side error representation that travels through the
/// thread-local last-error slot; turning it into a JS `Error` is the job of
/// the `bffi-error` crate. Only [`code`](BffiError::code) and
/// [`message`](BffiError::message) cross the C ABI - [`source`](BffiError::source)
/// exists for Rust-side diagnostics and lossless domain-error conversion.
#[derive(Debug)]
pub struct BffiError {
    /// Machine-readable status code.
    pub code: ErrorCode,
    /// Human-readable details, safe to copy across the FFI boundary.
    pub message: String,
    /// The originating domain error, if the failure was converted from a
    /// typed error (e.g. [`TableError`], [`RegistryError`], or a
    /// `bffi-types` conversion error).
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
    /// The B3 rich fields (present only on derived errors): boxed to
    /// keep [`BffiError`] small across `Result` channels.
    pub rich: Option<Box<ErrorRich>>,
}

/// The rich error fields of `#[derive(BffiError)]` types.
#[derive(Debug, Default)]
pub struct ErrorRich {
    /// The user-defined status that replaces [`ErrorCode`] in the ABI
    /// return (the reserved range `0x1000..=0xFFFF`). JS reads it as
    /// `e.code`.
    pub user_code: Option<u32>,
    /// The derived variant name (`NotFound`, ...) - JavaScript sets
    /// `e.name` to it.
    pub variant: Option<String>,
    /// The pre-encoded wire record of the variant's payload fields
    /// (B1 encoders); JavaScript decodes it into `e.payload`.
    pub payload: Option<Vec<u8>>,
    /// The captured Rust backtrace (present only when
    /// `RUST_BACKTRACE` is active); JavaScript exposes it as
    /// `e.nativeStack`.
    pub backtrace: Option<String>,
}

/// The gated backtrace capture: `Backtrace::capture` is a no-op
/// (cheap env check) unless `RUST_BACKTRACE` is active.
fn capture_backtrace() -> Option<String> {
    let backtrace = std::backtrace::Backtrace::capture();
    if matches!(
        backtrace.status(),
        std::backtrace::BacktraceStatus::Captured
    ) {
        Some(backtrace.to_string())
    } else {
        None
    }
}

impl BffiError {
    /// Creates an error from a code and a message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,

            // Backtrace capture is gated by RUST_BACKTRACE: the rich
            // slot is allocated only when a stack was actually
            // captured.
            rich: capture_backtrace().map(|stack| {
                Box::new(ErrorRich {
                    backtrace: Some(stack),
                    ..Default::default()
                })
            }),
        }
    }

    /// Creates an error that keeps the originating domain error.
    ///
    /// Any concrete error type that is `Error + Send + Sync + 'static` is
    /// accepted and boxed into [`BffiError::source`].
    #[must_use]
    pub fn with_source(
        code: ErrorCode,
        message: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            source: Some(source.into()),
            rich: capture_backtrace().map(|stack| {
                Box::new(ErrorRich {
                    backtrace: Some(stack),
                    ..Default::default()
                })
            }),
        }
    }

    /// Sets the user-defined status (the reserved `0x1000..=0xFFFF`
    /// range; `#[derive(BffiError)]` emits this).
    #[must_use]
    pub fn with_user_code(mut self, code: u32) -> Self {
        self.rich_mut().user_code = Some(code);
        self
    }

    /// Sets the derived variant name (JavaScript `e.name`).
    #[must_use]
    pub fn with_variant(mut self, name: impl Into<String>) -> Self {
        self.rich_mut().variant = Some(name.into());
        self
    }

    /// Sets the pre-encoded payload record (JavaScript `e.payload`).
    #[must_use]
    pub fn with_payload(mut self, payload: Vec<u8>) -> Self {
        self.rich_mut().payload = Some(payload);
        self
    }

    /// The rich-fields slot, created on demand.
    fn rich_mut(&mut self) -> &mut ErrorRich {
        self.rich.get_or_insert_with(Default::default)
    }

    /// The status that crosses the C ABI: the user-defined code when
    /// present, otherwise the framework code.
    #[must_use]
    pub fn status_u32(&self) -> u32 {
        self.rich
            .as_ref()
            .and_then(|rich| rich.user_code)
            .unwrap_or_else(|| self.code.as_u32())
    }
}

impl fmt::Display for BffiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BffiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|boxed| &**boxed as &(dyn std::error::Error + 'static))
    }
}

impl From<ErrorCode> for BffiError {
    fn from(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.to_string(),
            source: None,
            rich: None,
        }
    }
}

/// Ad-hoc domain errors: a plain string becomes a `DomainError`
/// (code 13) without requiring `#[derive(BffiError)]`.
impl From<String> for BffiError {
    fn from(message: String) -> Self {
        Self::new(ErrorCode::DomainError, message)
    }
}

impl From<&str> for BffiError {
    fn from(message: &str) -> Self {
        Self::new(ErrorCode::DomainError, message)
    }
}

/// Unified-format conversion: a full table becomes a [`BffiError`] with
/// [`ErrorCode::TableFull`], keeping [`TableError`] as the source.
impl From<TableError> for BffiError {
    fn from(error: TableError) -> Self {
        Self::with_source(ErrorCode::TableFull, error.to_string(), error)
    }
}

/// Unified-format conversion: registry failures become [`BffiError`] with
/// the closest transport code, keeping [`RegistryError`] as the source.
impl From<RegistryError> for BffiError {
    fn from(error: RegistryError) -> Self {
        let code = match &error {
            RegistryError::TagAlreadyRegistered(_) | RegistryError::NotRegistered(_) => {
                ErrorCode::InvalidTag
            }
            RegistryError::TableFull(_) => ErrorCode::TableFull,
        };
        Self::with_source(code, error.to_string(), error)
    }
}

thread_local! {
    static LAST_ERROR: RefCell<Option<BffiError>> = const { RefCell::new(None) };
}

/// Stores a detailed error as this thread's last error, replacing any
/// previous one.
///
/// Called by FFI entry points right before returning a non-OK
/// [`ErrorCode`].
pub fn set_last_error(error: BffiError) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(error));
}

/// Removes and returns this thread's last error, clearing the slot.
///
/// The JS side calls this (through a generated export) after observing a
/// non-OK status code.
#[must_use]
pub fn take_last_error() -> Option<BffiError> {
    LAST_ERROR.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bffi_core::handle::TypeTag;
    use std::thread;

    #[test]
    fn codes_roundtrip_through_u32() {
        for code in [
            ErrorCode::Ok,
            ErrorCode::Error,
            ErrorCode::Panic,
            ErrorCode::NullHandle,
            ErrorCode::InvalidHandle,
            ErrorCode::TableFull,
            ErrorCode::InvalidTag,
            ErrorCode::InvalidUtf8,
            ErrorCode::NumberOutOfRange,
            ErrorCode::NullPointer,
            ErrorCode::BufferTooSmall,
            ErrorCode::InvalidArgument,
            ErrorCode::WrongThread,
            ErrorCode::DomainError,
            ErrorCode::Timeout,
        ] {
            assert_eq!(ErrorCode::from_u32(code.as_u32()), Some(code));
        }
        assert_eq!(ErrorCode::from_u32(0), Some(ErrorCode::Ok));
    }

    #[test]
    fn unknown_codes_decode_to_none() {
        assert_eq!(ErrorCode::from_u32(999), None);
        assert_eq!(ErrorCode::from_u32(u32::MAX), None);
    }

    #[test]
    fn rich_fields_round_trip_and_status_prefers_the_user_code() {
        let error = BffiError::new(ErrorCode::DomainError, "no row")
            .with_user_code(0x1001)
            .with_variant("NotFound")
            .with_payload(vec![1, 2, 3]);

        assert_eq!(error.status_u32(), 0x1001, "user code wins");
        let rich = error.rich.as_ref().expect("rich slot allocated");
        assert_eq!(rich.user_code, Some(0x1001));
        assert_eq!(rich.variant.as_deref(), Some("NotFound"));
        assert_eq!(rich.payload.as_deref(), Some(&[1, 2, 3][..]));
    }

    #[test]
    fn framework_errors_keep_the_framework_status() {
        let error = BffiError::new(ErrorCode::InvalidHandle, "gone");
        assert_eq!(error.status_u32(), ErrorCode::InvalidHandle.as_u32());
        // A captured backtrace may allocate the rich slot (when
        // RUST_BACKTRACE is active), but never a user code.
        let has_user_code = error
            .rich
            .as_ref()
            .is_some_and(|rich| rich.user_code.is_some());
        assert!(!has_user_code, "framework errors carry no user code");
    }

    #[test]
    fn string_errors_map_onto_domain_error() {
        let error: BffiError = "ad-hoc".to_owned().into();
        assert_eq!(error.code, ErrorCode::DomainError);
        assert_eq!(error.message, "ad-hoc");
        let error: BffiError = "borrowed".into();
        assert_eq!(error.code, ErrorCode::DomainError);
    }

    #[test]
    fn display_includes_code_and_message() {
        let error = BffiError::new(ErrorCode::InvalidHandle, "no such object");
        assert_eq!(error.to_string(), "unknown or stale handle: no such object");
    }

    #[test]
    fn error_from_code_uses_code_text_as_message() {
        let error = BffiError::from(ErrorCode::TableFull);
        assert_eq!(error.code, ErrorCode::TableFull);
        assert_eq!(error.message, error.code.to_string());
        assert!(error.source.is_none(), "code-only errors have no source");
    }

    #[test]
    fn with_source_boxes_the_original_error() {
        let original = std::io::Error::other("disk gone");
        let error = BffiError::with_source(ErrorCode::Error, "io failed", original);

        let recovered = error
            .source
            .as_ref()
            .and_then(|boxed| boxed.downcast_ref::<std::io::Error>())
            .expect("original io::Error must be preserved");
        assert_eq!(recovered.to_string(), "disk gone");

        let as_dyn: &(dyn std::error::Error + 'static) = &error;
        assert!(
            as_dyn.source().is_some(),
            "trait source() exposes the chain"
        );
    }

    #[test]
    fn table_error_converts_losslessly() {
        let error = BffiError::from(TableError::Full);
        assert_eq!(error.code, ErrorCode::TableFull);
        assert_eq!(error.message, "handle table has no free slots");

        let recovered = error
            .source
            .as_ref()
            .and_then(|boxed| boxed.downcast_ref::<TableError>());
        assert_eq!(recovered, Some(&TableError::Full));
    }

    #[test]
    fn registry_errors_map_to_codes_with_source() {
        let cases = [
            (
                RegistryError::TagAlreadyRegistered(TypeTag(0x8001)),
                ErrorCode::InvalidTag,
            ),
            (
                RegistryError::NotRegistered(TypeTag(0x8002)),
                ErrorCode::InvalidTag,
            ),
            (
                RegistryError::TableFull(TypeTag(0x8003)),
                ErrorCode::TableFull,
            ),
        ];
        for (registry_error, expected_code) in cases {
            let error = BffiError::from(registry_error);
            assert_eq!(error.code, expected_code, "for {registry_error:?}");
            let recovered = error
                .source
                .as_ref()
                .and_then(|boxed| boxed.downcast_ref::<RegistryError>());
            assert!(recovered.is_some(), "source must stay a RegistryError");
        }
    }

    #[test]
    fn last_error_can_be_set_and_taken() {
        assert!(
            take_last_error().is_none(),
            "test threads must start with a clean slot"
        );

        set_last_error(BffiError::new(ErrorCode::Error, "first"));
        set_last_error(BffiError::new(ErrorCode::Error, "second"));
        assert_eq!(take_last_error().map(|e| e.message), Some("second".into()));
        assert!(take_last_error().is_none(), "take must clear the slot");
    }

    #[test]
    fn last_error_is_thread_local() {
        set_last_error(BffiError::new(ErrorCode::Error, "main-only"));

        let seen_by_thread = thread::spawn(|| take_last_error().is_some())
            .join()
            .expect("thread must not panic");
        assert!(!seen_by_thread, "other threads must see no error");

        assert_eq!(
            take_last_error().map(|e| e.message),
            Some("main-only".into())
        );
    }
}
