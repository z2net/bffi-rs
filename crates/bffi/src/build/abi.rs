//! The declarative generator of the JS-facing C ABI exports.
//!
//! [`bffi_runtime_abi!`](super::bffi_runtime_abi) expands - in the USER
//! crate, not here - into the JS-facing `extern "C"` symbols
//! JavaScript calls through `bun:ffi`. Expansion must happen in the
//! user crate because the linker may drop unused `#[no_mangle]`
//! symbols that live in an rlib dependency; symbols defined by the
//! final cdylib itself always survive. This mirrors the `#[bffi]`
//! shim model (P1): the generator lives in the framework crate, the
//! generated code in the user crate (HANDOFF §3.6 - the ABI layer
//! belongs to `bffi-build`).
//!
//! Every generated export follows the `bffi_extern!` build policy
//! (DESIGN §6.5):
//!
//! - **debug** builds run the body bare (a panic aborts, easier
//!   debugging);
//! - **release** builds wrap the body in
//!   [`run_extern_body_or`](bffi::core::run_extern_body_or): the panic
//!   is stored as the last error and the export's documented "nothing"
//!   sentinel is returned (`0`, null, or the
//!   [`ErrorCode::Panic`](bffi::core::ErrorCode) numeric value for the
//!   free exports).
//!
//! The module's public helpers (`*_as_u32` / `*_as_code`) exist because
//! macro-expanded code in the user crate can only name `pub` items of
//! this crate; they keep the mapping logic testable on the Rust side.

// Internal module aliases (the pre-merge crate names).
use crate::bffi_core;
use crate::bffi_error;

/// The runtime ABI revision the generated exports report
/// (`bffi_runtime_abi_version()`) and the loader JSON records
/// (`"abiVersion"`). The JS loader's handshake compares the two and
/// refuses a cdylib built against a different revision. Bump on any
/// breaking change to the generated export set or its calling
/// convention.
pub const BFFI_ABI_VERSION: u32 = 1;

/// Maps a drained-error handle to the exported name code:
/// `1` = Error, `2` = TypeError, `3` = RangeError, `0` = invalid
/// (null/stale/foreign handle). Future `JsErrorName` variants fall
/// back to `1` (generic Error).
#[must_use]
pub fn error_name_as_u32(handle: u64) -> u32 {
    use bffi_error::JsErrorName;
    match super::runtime::error_name(bffi_core::Handle::from_raw(handle)) {
        Some(JsErrorName::TypeError) => 2,
        Some(JsErrorName::RangeError) => 3,
        Some(_) => 1,
        None => 0,
    }
}

/// Releases the drained-error slot behind `handle` and returns the
/// [`bffi::core::ErrorCode`] numeric value (`0` = Ok, `4` =
/// InvalidHandle).
#[must_use]
pub fn free_error_as_code(handle: u64) -> u32 {
    if super::runtime::free_error(bffi_core::Handle::from_raw(handle)) {
        bffi_core::ErrorCode::Ok.as_u32()
    } else {
        bffi_core::ErrorCode::InvalidHandle.as_u32()
    }
}

/// Releases the buffer slot behind `handle` and returns the
/// [`bffi::core::ErrorCode`] numeric value (`0` = Ok, `4` =
/// InvalidHandle).
#[must_use]
pub fn free_buffer_as_code(handle: u64) -> u32 {
    if super::runtime::free_buffer(bffi_core::Handle::from_raw(handle)) {
        bffi_core::ErrorCode::Ok.as_u32()
    } else {
        bffi_core::ErrorCode::InvalidHandle.as_u32()
    }
}

/// Generates the JS-facing runtime exports.
///
/// Call once, at the root of the cdylib crate:
///
/// ```text
/// bffi::build::bffi_runtime_abi!();
/// ```
///
/// The optional `module = ...` argument additionally generates the
/// `bffi_module_exports_hash` export, recomputing the exported-surface
/// hash ([`loader_json::module_exports_hash`](super::loader_json::module_exports_hash))
/// at runtime from the passed descriptor:
///
/// ```text
/// bffi::build::bffi_runtime_abi!(module = crate::module_def::MODULE);
/// ```
///
/// `$module` must resolve to a `ModuleDef` (or `&ModuleDef`) visible
/// at the call site - typically the same aggregated descriptor the
/// loader JSON was rendered from, so the JS loader can compare its
/// `"exportsHash"` field against the binary's value (the JSON↔binary
/// integrity check).
///
/// The user crate must depend on `bffi-core` and `bffi-build` (the
/// generated code uses `$crate::bffi_core` and `$crate` = `::bffi_build`
/// paths only).
///
/// # Generated exports
///
/// | Symbol | Signature | Sentinel on panic |
/// |---|---|---|
/// | `bffi_runtime_abi_version` | `() -> u32` | `0` (never panics; the constant [`BFFI_ABI_VERSION`]) |
/// | `bffi_error_take_last` | `() -> u64` | `0` (detail stays in the TLS slot) |
/// | `bffi_error_name` | `(h: u64) -> u32` | `0` (reads as "invalid") |
/// | `bffi_error_message_ptr` | `(h: u64) -> *const u8` | null |
/// | `bffi_error_message_len` | `(h: u64) -> u64` | `0` |
/// | `bffi_error_cause_ptr` | `(h: u64) -> *const u8` | null |
/// | `bffi_error_cause_len` | `(h: u64) -> u64` | `0` |
/// | `bffi_error_free` | `(h: u64) -> u32` | `ErrorCode::Panic` value |
/// | `bffi_buffer` | `(h: u64) -> *const u8` | null |
/// | `bffi_buffer_length` | `(h: u64) -> u64` | `0` |
/// | `bffi_types_free` | `(h: u64) -> u32` | `ErrorCode::Panic` value |
/// | `bffi_module_exports_hash` | `() -> u64` (only with `module = ...`) | `0` |
///
/// Numeric `0` sentinels overlap with legitimate values ("no error",
/// "invalid handle", "empty message"); each export's doc comment spells
/// out the meaning. Free exports return the [`bffi::core::ErrorCode`]
/// numeric value: `0` = Ok, `4` = InvalidHandle.
#[macro_export]
macro_rules! bffi_runtime_abi {
    () => {
        $crate::__bffi_runtime_abi_exports!();
    };
    (module = $module:expr) => {
        $crate::__bffi_runtime_abi_exports!();
        $crate::__bffi_module_exports_hash_export!($module);
    };
}

/// Implementation detail of [`bffi_runtime_abi!`]: the export set
/// every expansion carries, plus the constant ABI revision export the
/// loader handshake reads. Not public API - expand
/// [`bffi_runtime_abi!`] instead.
#[doc(hidden)]
#[macro_export]
macro_rules! __bffi_runtime_abi_exports {
    () => {
        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the runtime ABI revision this cdylib was built
        /// against ([`BFFI_ABI_VERSION`]); the JS loader's handshake
        /// compares it against the loader JSON's `"abiVersion"` field
        /// and refuses a mismatched binary.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_runtime_abi_version() -> u32 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::abi::BFFI_ABI_VERSION
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || $crate::bffi_build::abi::BFFI_ABI_VERSION,
                    0_u32,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// drains the thread-local last error into a readable slot.
        /// Returns the handle as a raw `u64`, `0` when no error is
        /// stored.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_take_last() -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::take_error().as_u64()
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || $crate::bffi_build::runtime::take_error().as_u64(),
                    0_u64,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the JS constructor name of the drained error
        /// (`1` = Error, `2` = TypeError, `3` = RangeError), `0` for
        /// null/stale/foreign handles.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_name(handle: u64) -> u32 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::abi::error_name_as_u32(handle)
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || $crate::bffi_build::abi::error_name_as_u32(handle),
                    0_u32,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the pointer to the drained error's UTF-8 message
        /// bytes (valid until `bffi_error_free`), null for invalid
        /// handles.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_message_ptr(handle: u64) -> *const u8 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_message_ptr($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_message_ptr(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    ::std::ptr::null(),
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the byte length of the drained error's message,
        /// `0` for invalid handles.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_message_len(handle: u64) -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_message_len($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_message_len(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    0_u64,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the pointer to the drained error's UTF-8 cause bytes
        /// (the source's `Display` string; valid until
        /// `bffi_error_free`), null when the error has no source or the
        /// handle is invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_cause_ptr(handle: u64) -> *const u8 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_cause_ptr($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_cause_ptr(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    ::std::ptr::null(),
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the byte length of the drained error's cause, `0`
        /// when the error has no source or the handle is invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_cause_len(handle: u64) -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_cause_len($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_cause_len(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    0_u64,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the user-defined status of the drained error (the
        /// `#[derive(BffiError)]` code; `0` = none), `0` for invalid
        /// handles.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_user_code(handle: u64) -> u32 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_user_code($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_user_code(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    0_u32,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the pointer to the derived variant name bytes (valid
        /// until `bffi_error_free`), null when absent or the handle is
        /// invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_variant_ptr(handle: u64) -> *const u8 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_variant_ptr($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_variant_ptr(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    ::std::ptr::null(),
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the byte length of the derived variant name, `0`
        /// when absent or the handle is invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_variant_len(handle: u64) -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_variant_len($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_variant_len(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    0_u64,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the pointer to the pre-encoded payload record bytes
        /// (valid until `bffi_error_free`), null when the error has no
        /// payload or the handle is invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_payload_ptr(handle: u64) -> *const u8 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_payload_ptr($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_payload_ptr(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    ::std::ptr::null(),
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the byte length of the payload record, `0` when the
        /// error has no payload or the handle is invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_payload_len(handle: u64) -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_payload_len($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_payload_len(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    0_u64,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the pointer to the captured Rust backtrace bytes
        /// (valid until `bffi_error_free`), null when none was captured
        /// or the handle is invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_stack_ptr(handle: u64) -> *const u8 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_stack_ptr($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_stack_ptr(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    ::std::ptr::null(),
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the byte length of the captured backtrace, `0` when
        /// none was captured or the handle is invalid.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_stack_len(handle: u64) -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::error_stack_len($crate::bffi_core::Handle::from_raw(
                    handle,
                ))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::error_stack_len(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    0_u64,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// releases the drained-error slot. Returns the
        /// [`ErrorCode`](bffi::core::ErrorCode) numeric value
        /// (`0` = Ok, `4` = InvalidHandle).
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_error_free(handle: u64) -> u32 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::abi::free_error_as_code(handle)
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || $crate::bffi_build::abi::free_error_as_code(handle),
                    $crate::bffi_core::ErrorCode::Panic.as_u32(),
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the pointer to the bytes behind the buffer handle
        /// (valid until `bffi_types_free`), null for invalid handles.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_buffer(handle: u64) -> *const u8 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::buffer_ptr($crate::bffi_core::Handle::from_raw(handle))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::buffer_ptr(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    ::std::ptr::null(),
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// returns the byte length behind the buffer handle, `0` for
        /// invalid handles.
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_buffer_length(handle: u64) -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::runtime::buffer_len($crate::bffi_core::Handle::from_raw(handle))
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || {
                        $crate::bffi_build::runtime::buffer_len(
                            $crate::bffi_core::Handle::from_raw(handle),
                        )
                    },
                    0_u64,
                )
            }
        }

        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// releases the buffer slot. Returns the
        /// [`ErrorCode`](bffi::core::ErrorCode) numeric value
        /// (`0` = Ok, `4` = InvalidHandle).
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_types_free(handle: u64) -> u32 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::abi::free_buffer_as_code(handle)
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || $crate::bffi_build::abi::free_buffer_as_code(handle),
                    $crate::bffi_core::ErrorCode::Panic.as_u32(),
                )
            }
        }
    };
}

/// Implementation detail of [`bffi_runtime_abi!`] with the
/// `module = ...` argument: generates the `bffi_module_exports_hash`
/// export over the passed descriptor. Not public API - expand
/// [`bffi_runtime_abi!`] instead.
#[doc(hidden)]
#[macro_export]
macro_rules! __bffi_module_exports_hash_export {
    ($module:expr) => {
        /// C ABI export generated by `bffi::build::bffi_runtime_abi!()`:
        /// recomputes the FNV-1a 64-bit hash of the module's exported
        /// surface (see
        /// [`module_exports_hash`](bffi::build::loader_json::module_exports_hash))
        /// at runtime; the JS loader compares it against the loader
        /// JSON's `"exportsHash"` field to guarantee the JSON and the
        /// loaded binary describe the same exports. `0` is the panic
        /// sentinel (an FNV-1a value is never `0`).
        #[unsafe(no_mangle)]
        pub extern "C" fn bffi_module_exports_hash() -> u64 {
            #[cfg(debug_assertions)]
            {
                $crate::bffi_build::loader_json::module_exports_hash(&$module)
            }
            #[cfg(not(debug_assertions))]
            {
                $crate::bffi_core::boundary::run_extern_body_or(
                    || $crate::bffi_build::loader_json::module_exports_hash(&$module),
                    0_u64,
                )
            }
        }
    };
}
