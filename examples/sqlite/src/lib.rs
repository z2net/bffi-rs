//! Example bffi-rs native module: a small functional SQLite surface.
//!
//! The point is NOT sqlite itself - it is a REAL workload (open a
//! database, run SQL, fetch rows) crossing the bffi boundary, so the
//! `.bffi` pipeline (`@z2net/bffi`) is verified end-to-end:
//!
//! - cstring parameters (`&str`);
//! - handles (`u64` -> an `ObjectWrap`-held connection);
//! - string returns (transient-buffer handles);
//! - `Result` errors -> `ErrorCode::DomainError` (13) with `cause`.
//!
//! Aggregation lives in [`module_def`] (single source); the `emit-json`
//! binary materializes `.bffi/bffi.api.json` from it for the
//! `@z2net/bffi` pipeline.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction (the one expect below is a sticky-init invariant).
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free): the JS pipeline drains errors and reads buffers
// through them.
bffi::bffi_runtime_abi!();

pub mod module_def;

use std::sync::{Mutex, OnceLock};

use bffi::{Handle, ObjectWrap, TypeTag};
use rusqlite::Connection;

/// The type tag of the connection table (bffi-object range).
const CONN_TAG: TypeTag = TypeTag(0x0110);

/// A shared connection: rusqlite's `Connection` is `Send` but not
/// `Sync`, so the ObjectWrap holds it behind a `Mutex`.
struct SharedConnection {
    conn: Mutex<Connection>,
}

fn wrap() -> &'static ObjectWrap<SharedConnection> {
    static WRAP: OnceLock<ObjectWrap<SharedConnection>> = OnceLock::new();
    WRAP.get_or_init(|| {
        // Sticky-init invariant: the crate-owned tag 0x8000 is claimed
        // exactly once here; a duplicate claim is an unrecoverable
        // programming error (same allowance as the framework's own
        // sticky inits).
        #[allow(clippy::expect_used)]
        ObjectWrap::new(CONN_TAG).expect("connection tag 0x8000 is free")
    })
}

fn with_connection<T>(
    handle: u64,
    f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> Result<T, SqliteError> {
    let conn = wrap()
        .get(Handle::from_raw(handle))
        .map_err(|e| SqliteError(format!("bad handle: {e}")))?;
    let guard = conn
        .conn
        .lock()
        .map_err(|_| SqliteError("connection mutex poisoned".to_owned()))?;
    let result = f(&guard)?;
    Ok(result)
}

/// The domain error of the module: wraps rusqlite's error text.
#[derive(Debug)]
pub struct SqliteError(String);

impl std::fmt::Display for SqliteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SqliteError {}

impl From<SqliteError> for bffi::BffiError {
    fn from(error: SqliteError) -> Self {
        bffi::BffiError::new(bffi::ErrorCode::DomainError, error.0)
    }
}

impl From<rusqlite::Error> for SqliteError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<bffi::ObjectError> for SqliteError {
    fn from(error: bffi::ObjectError) -> Self {
        Self(error.to_string())
    }
}

/// Opens (or creates) a database and returns its connection handle.
/// Use `:memory:` for an in-memory database.
#[bffi::bffi]
pub fn open(path: &str) -> Result<u64, SqliteError> {
    let conn = Connection::open(path)?;
    let handle = wrap().wrap(SharedConnection {
        conn: Mutex::new(conn),
    })?;
    Ok(handle.as_u64())
}

/// Executes one or more SQL statements (no rows returned).
#[bffi::bffi]
pub fn exec(handle: u64, sql: &str) -> Result<(), SqliteError> {
    with_connection(handle, |conn| conn.execute_batch(sql))
}

/// Runs a query and returns ALL rows as a JSON array of arrays.
/// Values are TEXT-coerced (sqlite's dynamic typing makes this
/// lossless for text/int/real round-trips through JS strings).
#[bffi::bffi]
pub fn query(handle: u64, sql: &str) -> Result<String, SqliteError> {
    with_connection(handle, |conn| {
        let mut statement = conn.prepare(sql)?;
        let names: Vec<String> = statement
            .column_names()
            .iter()
            .map(|c| c.to_string())
            .collect();
        let mut rows = statement.query([])?;
        let mut out = String::from("[");
        let mut first_row = true;
        while let Some(row) = rows.next()? {
            if !first_row {
                out.push(',');
            }
            first_row = false;
            out.push('{');
            for (index, name) in names.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&json_object(name, &row.get_ref(index)?));
            }
            out.push('}');
        }
        out.push(']');
        Ok(out)
    })
}

/// Builds one `{"name": value}` JSON member with a TEXT-coerced value.
fn json_object(name: &str, value: &rusqlite::types::ValueRef<'_>) -> String {
    let text = match value {
        rusqlite::types::ValueRef::Null => String::new(),
        rusqlite::types::ValueRef::Integer(v) => v.to_string(),
        rusqlite::types::ValueRef::Real(v) => v.to_string(),
        rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) => {
            String::from_utf8_lossy(t).into_owned()
        }
    };
    format!("{}:{}", json_string(name), json_string(&text))
}

/// JSON string literal with minimal escaping.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Closes (releases) a connection handle.
#[bffi::bffi]
pub fn close(handle: u64) -> Result<(), SqliteError> {
    wrap()
        .release(Handle::from_raw(handle))
        .map_err(|e| SqliteError(format!("bad handle: {e}")))?;
    Ok(())
}

/// The SQLite engine version (sanity check for the pipeline).
#[bffi::bffi]
pub fn sqlite_version() -> String {
    rusqlite::version().to_owned()
}
