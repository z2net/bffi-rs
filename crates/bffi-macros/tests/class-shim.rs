//! Acceptance tests for the generated class shims: full lifecycle
//! (create -> getter -> method -> release) plus UAF barriers, in the
//! `#[bffi]` shim testing model (direct Rust calls, no Bun).
//!
//! Test tags (0x0150/0x0151) are unique per class: one binary shares
//! the process-wide Registry.
#![allow(clippy::expect_used, clippy::unwrap_used)]
// The probe namespace modules mirror the facade namespaces; the
// generated shims carry the docs.
#![allow(missing_docs)]

use bffi::{ErrorCode, Handle, take_last_error};

#[bffi_macros::bffi_class(tag = 0x0150)]
/// A counter.
pub struct Counter {
    /// The current value.
    pub value: u32,
    /// Private fields are never exported (kept unreachable here).
    #[allow(dead_code)]
    secret: u8,
}

#[bffi_macros::bffi_impl]
impl Counter {
    #[bffi_macros::bffi_constructor]
    /// Creates a counter.
    pub fn new(start: u32) -> Self {
        Self {
            value: start,
            secret: 0,
        }
    }

    /// Adds one and returns the new value.
    pub fn increment(&self) -> u32 {
        self.value + 1
    }

    /// Scales by a factor.
    pub fn scaled(&self, factor: u32) -> u32 {
        self.value * factor
    }

    /// Greets, returning an owned string.
    pub fn greet(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.value)
    }

    /// Sums the bytes and adds the stored value.
    pub fn byte_sum(&self, data: &[u8]) -> u32 {
        data.iter().map(|byte| u32::from(*byte)).sum::<u32>() + self.value
    }

    /// Falls back to the err channel on zero.
    pub fn divided(&self, divisor: u32) -> Result<u32, DivError> {
        self.value.checked_div(divisor).ok_or(DivError { divisor })
    }
}

/// The `E` side of the err channel.
#[derive(Debug)]
pub struct DivError {
    divisor: u32,
}

impl std::fmt::Display for DivError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "division by {divisor}", divisor = self.divisor)
    }
}

impl std::error::Error for DivError {}

impl From<DivError> for bffi::BffiError {
    fn from(error: DivError) -> Self {
        bffi::BffiError::new(bffi::ErrorCode::DomainError, error.to_string())
    }
}

/// A second class for wrong-tag tests (its own tag, own table).
#[bffi_macros::bffi_class(tag = 0x0151)]
/// A gate.
pub struct Gate {
    /// Open or closed.
    pub open: bool,
}

#[bffi_macros::bffi_impl]
impl Gate {
    #[bffi_macros::bffi_constructor]
    /// Creates a gate.
    pub fn new(open: bool) -> Self {
        Self { open }
    }
}

// Facade-only mode without the facade: `crate = "bffi_class_probe"`
// on BOTH macros makes the generated shims and descriptor resolve
// through `::bffi_class_probe::{core, types, dts, object, build}`
// instead of the direct deps. The `extern crate self` alias puts THIS
// crate into the extern prelude under the probe name, so the absolute
// paths resolve to the re-export modules below.
extern crate self as bffi_class_probe;

pub mod core {
    pub use bffi::core::*;
}
pub mod types {
    pub use bffi::types::*;
}
pub mod dts {
    pub use bffi::dts::*;
}
pub mod object {
    pub use bffi::object::*;
}
pub mod build {
    pub use bffi::build::*;
}

#[bffi_macros::bffi_class(tag = 0x0152, crate = "bffi_class_probe")]
/// A meter.
pub struct Meter {
    /// The level.
    pub level: u32,
}

#[bffi_macros::bffi_impl(crate = "bffi_class_probe")]
impl Meter {
    #[bffi_macros::bffi_constructor]
    /// Creates a meter.
    pub fn new(level: u32) -> Self {
        Self { level }
    }

    /// Doubles the level.
    pub fn doubled(&self) -> u32 {
        self.level * 2
    }

    /// Labels through the buffer path (store_bytes via the probe).
    pub fn label(&self) -> String {
        format!("level {}", self.level)
    }
}

fn cstring(bytes: &[u8]) -> *const std::os::raw::c_char {
    std::ffi::CString::new(bytes)
        .expect("test bytes contain no interior NUL")
        .into_raw()
        .cast_const()
}

#[test]
fn constructor_getter_method_release_roundtrip() {
    let mut handle = 0_u64;
    assert_eq!(bffi_counter_new(41, &mut handle), ErrorCode::Ok.as_u32());
    assert_ne!(handle, 0);

    let mut value = 0_u32;
    assert_eq!(
        bffi_counter_value_get(handle, &mut value),
        ErrorCode::Ok.as_u32()
    );
    assert_eq!(value, 41);

    let mut out = 0_u32;
    assert_eq!(
        bffi_counter_increment(handle, &mut out),
        ErrorCode::Ok.as_u32()
    );
    assert_eq!(out, 42);

    assert_eq!(
        bffi_counter_scaled(handle, 3, &mut out),
        ErrorCode::Ok.as_u32()
    );
    assert_eq!(out, 123);

    // The destructor frees the slot: the handle goes stale.
    assert_eq!(bffi_counter_release(handle), ErrorCode::Ok.as_u32());
    assert_eq!(
        bffi_counter_release(handle),
        ErrorCode::InvalidHandle.as_u32()
    );
    assert_eq!(
        bffi_counter_value_get(handle, &mut value),
        ErrorCode::InvalidHandle.as_u32()
    );
    let error = take_last_error().expect("the stale getter must store an error");
    assert_eq!(error.code, ErrorCode::InvalidHandle);
}

#[test]
fn string_and_result_method_returns_travel_the_p2_channels() {
    let mut handle = 0_u64;
    assert_eq!(bffi_counter_new(7, &mut handle), ErrorCode::Ok.as_u32());

    let mut out_handle = 0_u64;
    assert_eq!(
        bffi_counter_greet(handle, cstring(b"val"), &mut out_handle),
        ErrorCode::Ok.as_u32()
    );
    // SAFETY: `buffer_ptr` handed out the pointer to exactly
    // `buffer_len(handle)` owned bytes; the handle is still live.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            bffi::build::runtime::buffer_ptr(bffi::Handle::from_raw(out_handle)),
            bffi::build::runtime::buffer_len(bffi::Handle::from_raw(out_handle)) as usize,
        )
    };
    assert_eq!(bytes, b"val-7");
    assert!(bffi::build::runtime::free_buffer(bffi::Handle::from_raw(
        out_handle
    )));

    let mut out = 0_u32;
    assert_eq!(
        bffi_counter_divided(handle, 3, &mut out),
        ErrorCode::Ok.as_u32()
    );
    assert_eq!(out, 2);
    assert_eq!(
        bffi_counter_divided(handle, 0, &mut out),
        ErrorCode::DomainError.as_u32()
    );
    let error = take_last_error().expect("an Err must store the domain error");
    assert_eq!(error.message, "division by 0");

    // The `&[u8]` method parameter: the shim receives the (ptr, len)
    // pair and the method sees a borrowed view.
    let data = [5_u8, 10, 20];
    assert_eq!(
        bffi_counter_byte_sum(handle, data.as_ptr(), data.len() as u64, &mut out),
        ErrorCode::Ok.as_u32()
    );
    assert_eq!(out, 42);

    // Empty view through a null pointer is fine.
    assert_eq!(
        bffi_counter_byte_sum(handle, std::ptr::null(), 0, &mut out),
        ErrorCode::Ok.as_u32()
    );
    assert_eq!(out, 7);

    bffi_counter_release(handle);
}

#[test]
fn invalid_and_foreign_handles_are_rejected_with_clear_errors() {
    // Null handle.
    let mut value = 0_u32;
    assert_eq!(
        bffi_counter_value_get(0, &mut value),
        ErrorCode::InvalidHandle.as_u32()
    );
    let error = take_last_error().expect("an invalid handle must store an error");
    assert_eq!(error.code, ErrorCode::InvalidHandle);

    // Live Gate handle used against Counter: the tag barrier rejects.
    let mut gate = 0_u64;
    assert_eq!(bffi_gate_new(true, &mut gate), ErrorCode::Ok.as_u32());
    assert_eq!(
        bffi_counter_value_get(gate, &mut value),
        ErrorCode::InvalidHandle.as_u32()
    );

    // Forged generation on a real Counter handle.
    let mut counter = 0_u64;
    assert_eq!(bffi_counter_new(1, &mut counter), ErrorCode::Ok.as_u32());
    let live = Handle::from_raw(counter);
    let forged = Handle::new(live.tag(), live.generation() + 1, live.index());
    assert_eq!(
        bffi_counter_value_get(forged.as_u64(), &mut value),
        ErrorCode::InvalidHandle.as_u32()
    );

    bffi_gate_release(gate);
    bffi_counter_release(counter);
}

#[test]
fn constructor_null_out_pointer_is_rejected() {
    let code = bffi_counter_new(1, std::ptr::null_mut());
    assert_eq!(code, ErrorCode::NullPointer.as_u32());
    let error = take_last_error().expect("null out-pointer must store a last error");
    assert_eq!(error.code, ErrorCode::NullPointer);
}

#[test]
fn crate_option_class_lifecycle_runs_through_the_probe_namespaces() {
    let mut handle = 0_u64;
    assert_eq!(bffi_meter_new(8, &mut handle), ErrorCode::Ok.as_u32());
    assert_ne!(handle, 0);

    let mut level = 0_u32;
    assert_eq!(
        bffi_meter_level_get(handle, &mut level),
        ErrorCode::Ok.as_u32()
    );
    assert_eq!(level, 8);

    let mut out = 0_u32;
    assert_eq!(bffi_meter_doubled(handle, &mut out), ErrorCode::Ok.as_u32());
    assert_eq!(out, 16);

    // The buffer path through `bffi_class_probe::build`.
    let mut label = 0_u64;
    assert_eq!(bffi_meter_label(handle, &mut label), ErrorCode::Ok.as_u32());
    // SAFETY: `buffer_ptr` handed out the pointer to exactly
    // `buffer_len(label)` owned bytes; the handle is still live.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            bffi::build::runtime::buffer_ptr(bffi::Handle::from_raw(label)),
            bffi::build::runtime::buffer_len(bffi::Handle::from_raw(label)) as usize,
        )
    };
    assert_eq!(bytes, b"level 8");
    assert!(bffi::build::runtime::free_buffer(bffi::Handle::from_raw(
        label
    )));

    assert_eq!(bffi_meter_release(handle), ErrorCode::Ok.as_u32());
}
