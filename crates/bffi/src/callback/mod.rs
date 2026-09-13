//! # bffi-callback
//!
//! Callback plumbing for the [bffi-rs](https://github.com/z2net/bffi-rs)
//! framework: explicit registration and revocation of thread-safe callbacks
//! in both directions (Rust -> JavaScript and JavaScript -> Rust).
//!
//! ## Mission (P1-SPEC §6)
//!
//! - Registration is explicit: a callback exists only after the host
//!   registers it, and it is addressed by an opaque `u64` handle.
//! - Revocation is explicit and terminal: a revoked handle is dead -
//!   invocation through it is rejected, and the handle never resurrects
//!   even if its registry slot is reused.
//! - Wrong-thread invocation is rejected: only the thread that owns the
//!   callback may invoke it in P1; marshalling across threads is
//!   deferred to P2 (the event-loop trampoline).
//! - Storage lives in the process-wide `Registry` (see `bffi-core`)
//!   under the type tags `0x0200` (native callback table) and `0x0201`
//!   (JS callback table).
//! - Panics raised inside a callback body are not this crate's concern:
//!   catching and converting them into JS errors is the P2 trampoline's
//!   job (DESIGN.md §6.5).
//!
//! ## Quick start
//!
//! Register a Rust closure, invoke it through its opaque handle, and
//! revoke it - revocation is terminal:
//!
//! ```
//! use std::sync::Arc;
//!
//! use bffi::{CallbackSig, Value, ValueType, invoke, register, revoke};
//!
//! let sig = CallbackSig::new(ValueType::I32, &[ValueType::I32, ValueType::I32]);
//! let handle = register(
//!     sig,
//!     Arc::new(|args: &[Value]| match args {
//!         [Value::I32(a), Value::I32(b)] => Value::I32(a + b),
//!         _ => unreachable!("invoke checks the signature before calling"),
//!     }),
//! )
//! .expect("table has room");
//!
//! let sum = invoke(handle, &[Value::I32(2), Value::I32(3)]).expect("live handle");
//! assert_eq!(sum, Value::I32(5));
//!
//! assert!(revoke(handle));
//! assert!(invoke(handle, &[Value::I32(2), Value::I32(3)]).is_err());
//! ```

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// Internal module aliases (the pre-merge crate names).
pub mod abi;
pub mod error;
pub mod registry;
pub mod thread;
pub mod value;

pub use error::CallbackError;
#[cfg(feature = "event-loop")]
pub use registry::invoke_wait;
pub use registry::{JsCallbackInfo, bind_js_callback, invoke, js_callback, register, revoke};
pub use thread::{ensure_js_thread, set_js_thread};
pub use value::{CallbackSig, Value, ValueType};
