//! Example bffi-rs native module: the B3 typed error surface.
//!
//! `#[derive(BffiError)]` error enums cross the boundary with their
//! user code in the ABI status, the variant name as JS `e.name` and
//! the named fields as the JS `e.payload` record:
//!
//! - `find_user` rejects with `NOT_FOUND` carrying the queried id;
//! - `validate_age` rejects with `INVALID_AGE` carrying the value;
//! - `ad_hoc` shows the `String` shortcut (a plain DomainError 13).
//!
//! Aggregation lives in [`module_def`]; the `emit-json` binary
//! materializes `.bffi/bffi.api.json` for the `@z2net/bffi`
//! pipeline.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

bffi::bffi_runtime_abi!(module = crate::module_def::MODULE);

pub mod module_def;

use bffi::{BffiError, BffiRecord, bffi};

/// The typed error enum of the module. Each variant carries a stable
/// code in the reserved user range; the JS side switches on `e.code`
/// and reads `e.name` / `e.payload`.
#[derive(BffiError, Debug)]
pub enum UsersError {
    /// No user with the given id.
    #[bffi(code = 0x1001)]
    NotFound {
        /// The queried id.
        id: u64,
    },
    /// The age is outside the supported range.
    #[bffi(code = 0x1002)]
    InvalidAge {
        /// The rejected value.
        age: i32,
        /// The lowest allowed age.
        min: i32,
    },
}

/// One user record (returned as a B1 record).
#[derive(BffiRecord, Clone, Debug, PartialEq)]
pub struct User {
    pub id: u64,
    pub name: String,
}

fn users() -> Vec<User> {
    vec![
        User {
            id: 1,
            name: "ada".to_owned(),
        },
        User {
            id: 2,
            name: "grace".to_owned(),
        },
    ]
}

/// Looks a user up by id; `NOT_FOUND` carries the id.
#[bffi]
pub fn find_user(id: u64) -> Result<User, UsersError> {
    users()
        .into_iter()
        .find(|user| user.id == id)
        .ok_or(UsersError::NotFound { id })
}

/// Validates an age; `INVALID_AGE` carries the value and the bound.
#[bffi]
pub fn validate_age(age: i32) -> Result<bool, UsersError> {
    if !(0..=130).contains(&age) {
        return Err(UsersError::InvalidAge { age, min: 0 });
    }
    Ok(true)
}

/// An ad-hoc error: a plain `String` maps to DomainError (13).
#[bffi]
pub fn ad_hoc() -> Result<(), String> {
    Err("ad-hoc failure".to_owned())
}

/// BffiError itself is a valid error type (a framework code path).
#[bffi]
pub fn framework_error() -> Result<(), bffi::BffiError> {
    Err(bffi::BffiError::new(
        bffi::ErrorCode::InvalidArgument,
        "explicit framework error",
    ))
}
