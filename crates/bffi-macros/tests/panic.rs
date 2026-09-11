//! Acceptance test for the release panic boundary (DESIGN §6.5): a
//! panic inside the `#[bffi]` body becomes `ErrorCode::Panic` plus a
//! stored last error instead of unwinding into the host.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![cfg(not(debug_assertions))] // debug aborts by design (DESIGN §6.5)

use bffi::{ErrorCode, take_last_error};

#[bffi_macros::bffi]
fn boom() -> u32 {
    panic!("boundary!");
}

#[test]
fn panic_becomes_the_panic_code() {
    let mut out = 0_u32;
    let code = bffi_boom(&mut out);
    assert_eq!(code, ErrorCode::Panic.as_u32());
    let error = take_last_error().expect("panic must store the last error");
    assert_eq!(error.code, ErrorCode::Panic);
    assert_eq!(error.message, "boundary!");
}
