//! Task bookkeeping: states, resolvers and the process-wide task table
//! (tag `0x0500`, claimed in [`bffi::core::handle`]).
//!
//! State machine (guarded by the record's mutex, one transition wins):
//!
//! ```text
//! Running --cancel()/timeout--> Cancelled ----------+
//!        `--poll Ready(AsyncValue)--> Done(Value)   +--> delivered (once)
//!        `--poll Err/Panic--------> Failed(BffiError)'
//! ```
//!
//! A resolve/reject pair attaches at most once; attaching after the
//! task reached a terminal state delivers the stored outcome
//! immediately (the delivery is marshalled onto the event loop by the
//! caller-side helper in `deliver`).

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use bffi_core::{Handle, Registry, TypeTag};

use super::value::AsyncValue;

/// The type tag of the task table.
const TASK_TAG: TypeTag = TypeTag(0x0500);

/// The outcome of a finished task. `Failed` stores the materialized
/// status code and message (JS builds the error from these); the
/// boxed source does not cross the boundary.
#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    /// The future resolved with a value.
    Done(AsyncValue),
    /// The future failed: a caught panic or a `Result` error turned
    /// into the unified format.
    Failed {
        // Diagnostic-only on the Rust side (JS builds the error from
        // the message); kept for Debug output and transport work.
        #[allow(dead_code)]
        code: bffi_core::ErrorCode,
        message: String,
    },
    /// The task was cancelled before completion.
    Cancelled,
}

/// Everything JS-facing about one task.
pub(crate) struct TaskRecord {
    /// The terminal state; `None` while the future is alive.
    outcome: Mutex<Option<Outcome>>,
    /// Set by `cancel`; the executor drops the future at the next
    /// poll boundary.
    pub cancel_requested: AtomicBool,
    /// The attached resolver pair, at most once.
    resolvers: Mutex<Option<AttachedResolvers>>,
    /// Guards the one-shot delivery (whether a terminal outcome was
    /// already handed to a resolver pair).
    delivered: AtomicBool,
}

/// An attached `(resolve_ptr, reject_ptr)` pair plus the JS thread
/// that attached it: the resolver trampolines belong to that
/// isolate, so the delivery is targeted there. `thread == 0` means
/// the process was unbound at attach time (pure-Rust usage) -
/// legacy untargeted delivery.
#[derive(Clone, Copy)]
pub(crate) struct AttachedResolvers {
    pub(crate) resolve: usize,
    pub(crate) reject: usize,
    pub(crate) thread: u64,
}

impl TaskRecord {
    pub(crate) fn new() -> Self {
        Self {
            outcome: Mutex::new(None),
            cancel_requested: AtomicBool::new(false),
            resolvers: Mutex::new(None),
            delivered: AtomicBool::new(false),
        }
    }

    /// Whether a cancel was requested before completion.
    pub(crate) fn cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Acquire)
    }

    /// Marks the task cancelled; `true` on the winning transition
    /// (the task was still running), `false` when a terminal state
    /// was already reached.
    pub(crate) fn mark_cancelled(&self) -> bool {
        let mut state = self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_some() {
            return false;
        }
        self.cancel_requested.store(true, Ordering::Release);
        *state = Some(Outcome::Cancelled);
        true
    }

    /// Marks the task completed; `true` on the winning transition.
    pub(crate) fn mark_completed(&self, value: AsyncValue) -> bool {
        let mut state = self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_some() {
            return false;
        }
        *state = Some(Outcome::Done(value));
        true
    }

    /// Marks the task failed; `true` on the winning transition.
    pub(crate) fn mark_failed(&self, error: bffi_core::BffiError) -> bool {
        let mut state = self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_some() {
            return false;
        }
        *state = Some(Outcome::Failed {
            code: error.code,
            message: error.message,
        });
        true
    }

    /// Attaches the `(resolve_ptr, reject_ptr)` pair. A second attach
    /// is rejected with [`AttachError::AlreadyAttached`].
    ///
    /// The CURRENT thread is recorded with the pair: the resolver
    /// trampolines belong to that JS isolate (the `#[bffi_async]`
    /// caller attaches from its own JS thread), and the delivery is
    /// targeted there.
    pub(crate) fn attach(&self, resolve: usize, reject: usize) -> Result<(), AttachError> {
        let mut resolvers = self
            .resolvers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if resolvers.is_some() {
            return Err(AttachError::AlreadyAttached);
        }
        *resolvers = Some(AttachedResolvers {
            resolve,
            reject,
            thread: crate::bffi_callback::binding_thread(),
        });
        Ok(())
    }

    /// The attached resolver pair, if any.
    pub(crate) fn resolver_pair(&self) -> Option<AttachedResolvers> {
        *self
            .resolvers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A snapshot of the terminal outcome, `None` while running (or
    /// when the outcome was already consumed by a delivery).
    pub(crate) fn outcome_snapshot(&self) -> Option<Outcome> {
        self.outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether the terminal outcome still needs a delivery.
    pub(crate) fn take_delivery_slot(&self) -> bool {
        !self.delivered.swap(true, Ordering::AcqRel)
    }
}

/// Attach failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttachError {
    /// A resolver pair is already attached to this task.
    AlreadyAttached,
    /// The handle is null, stale or foreign.
    InvalidHandle,
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyAttached => write!(f, "a resolver pair is already attached to this task"),
            Self::InvalidHandle => write!(f, "unknown or stale task handle"),
        }
    }
}

impl std::error::Error for AttachError {}

/// Unified-format conversion: both failures map onto existing codes
/// (`InvalidHandle` and `InvalidArgument` respectively); the domain
/// error is preserved as the source.
impl From<AttachError> for bffi_core::BffiError {
    fn from(error: AttachError) -> Self {
        use bffi_core::ErrorCode;
        let code = match &error {
            AttachError::InvalidHandle => ErrorCode::InvalidHandle,
            AttachError::AlreadyAttached => ErrorCode::InvalidArgument,
        };
        bffi_core::BffiError::with_source(code, error.to_string(), error)
    }
}

fn tables() -> Result<(), bffi_core::RegistryError> {
    static TABLES: OnceLock<Result<(), bffi_core::RegistryError>> = OnceLock::new();
    *TABLES.get_or_init(|| {
        Registry::global()
            .declare::<TaskRecord>(TASK_TAG)
            .map_err(|_| bffi_core::RegistryError::TagAlreadyRegistered(TASK_TAG))
    })
}

/// Registers a fresh task record and returns its handle.
pub(crate) fn register() -> Result<Handle, bffi_core::RegistryError> {
    tables()?;
    Registry::global()
        .insert(TASK_TAG, Arc::new(TaskRecord::new()))
        .map_err(|_| bffi_core::RegistryError::TableFull(TASK_TAG))
}

/// The task record behind `handle`, or `None` for null/stale/foreign
/// handles.
#[must_use]
pub(crate) fn record(handle: Handle) -> Option<Arc<TaskRecord>> {
    Registry::global().get_typed::<TaskRecord>(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<TaskRecord> {
        let handle = register().expect("task table has room");
        record(handle).expect("just inserted")
    }

    #[test]
    fn cancel_wins_only_once() {
        let task = fresh();
        assert!(task.mark_cancelled(), "first cancel wins");
        assert!(!task.mark_cancelled(), "second cancel is a no-op");
        assert!(
            !task.mark_completed(AsyncValue::Unit),
            "terminal states are final"
        );
    }

    #[test]
    fn first_terminal_transition_wins() {
        let task = fresh();
        assert!(task.mark_failed(bffi_core::BffiError::new(
            bffi_core::ErrorCode::Panic,
            "boom"
        )));
        assert!(!task.mark_cancelled(), "failed is final");
        assert!(!task.mark_completed(AsyncValue::I32(1)), "failed is final");
    }

    #[test]
    fn attach_then_terminal_state_is_visible_in_the_snapshot() {
        let task = fresh();
        task.attach(1, 2).expect("attach ok");
        // The test process never registers a JS thread, so the
        // recorded owner is the legacy unbound sentinel.
        let pair = task.resolver_pair().expect("attached");
        assert_eq!((pair.resolve, pair.reject, pair.thread), (1, 2, 0));
        assert!(task.outcome_snapshot().is_none(), "still running");

        task.mark_completed(AsyncValue::I32(7));
        assert!(matches!(
            task.outcome_snapshot(),
            Some(Outcome::Done(AsyncValue::I32(7)))
        ));
    }

    #[test]
    fn double_attach_is_rejected() {
        let task = fresh();
        task.attach(1, 2).expect("first attach ok");
        assert_eq!(
            task.attach(3, 4).expect_err("second attach must fail"),
            AttachError::AlreadyAttached
        );
    }

    #[test]
    fn failed_outcome_materializes_code_and_message() {
        let task = fresh();
        assert!(task.mark_failed(bffi_core::BffiError::new(
            bffi_core::ErrorCode::Panic,
            "boom"
        )));
        match task.outcome_snapshot() {
            Some(Outcome::Failed { code, message }) => {
                assert_eq!(code, bffi_core::ErrorCode::Panic);
                assert_eq!(message, "boom");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
