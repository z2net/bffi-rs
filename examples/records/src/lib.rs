//! Example bffi-rs native module: the B1 composite types of the
//! framework.
//!
//! Records (`#[derive(BffiRecord)]` structs) and unit enums
//! (`#[derive(BffiEnum)]`) cross the boundary as wire-encoded
//! payloads - one transient buffer per value, copy by default - and
//! `Vec<T>` sequences cross the same channel item by item:
//!
//! - `make_sample` builds a record from primitives;
//! - `recenter` takes a record parameter and returns one;
//! - `total_weight` / `distances` / `labels` exercise record
//!   sequences in and numbers/strings out;
//! - `classify` returns an enum (and a domain error on empty input).
//!
//! Aggregation lives in [`module_def`] (single source); the
//! `emit-json` binary materializes `.bffi/bffi.api.json` from it for
//! the `@z2net/bffi` pipeline.

// Tests unwrap/expect freely; the lib itself keeps the workspace
// restriction.
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

// The runtime ABI exports (bffi_error_*, the bffi_buffer pair,
// bffi_types_free): the JS pipeline drains errors and reads buffers
// through them.
bffi::bffi_runtime_abi!();

pub mod module_def;

use bffi::{BffiEnum, BffiRecord, bffi};

/// The measurement axis.
#[derive(BffiEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Along the X axis.
    Horizontal,
    /// Along the Y axis.
    Vertical,
}

/// One measurement sample.
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Sample {
    /// Where it was taken.
    pub at: f64,
    /// How heavy the reading was.
    pub weight: i32,
    /// The station label.
    pub label: String,
    /// The axis it was taken along.
    pub axis: Axis,
}

/// The domain error of the module: a plain message wrapper.
#[derive(Debug)]
pub struct RecordsError(String);

impl std::fmt::Display for RecordsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RecordsError {}

/// Builds a sample from primitives.
#[bffi]
pub fn make_sample(at: f64, weight: i32, label: &str, axis: Axis) -> Sample {
    Sample {
        at,
        weight,
        label: label.to_owned(),
        axis,
    }
}

/// Shifts a sample along its axis (record in, record out).
#[bffi]
pub fn recenter(sample: Sample, dx: f64) -> Sample {
    Sample {
        at: sample.at + dx,
        ..sample
    }
}

/// Sums the weights of a sample sequence.
#[bffi]
pub fn total_weight(samples: Vec<Sample>) -> i64 {
    samples.iter().map(|s| i64::from(s.weight)).sum()
}

/// The absolute positions of a sample sequence.
#[bffi]
pub fn distances(samples: Vec<Sample>) -> Vec<f64> {
    samples.iter().map(|s| s.at.abs()).collect()
}

/// The station labels of a sample sequence.
#[bffi]
pub fn labels(samples: Vec<Sample>) -> Vec<String> {
    samples.into_iter().map(|s| s.label).collect()
}

/// The axis of the first sample; an empty sequence is a domain
/// error.
#[bffi]
pub fn classify(samples: Vec<Sample>) -> Result<Axis, RecordsError> {
    let first = samples
        .first()
        .ok_or_else(|| RecordsError("no samples".to_owned()))?;
    Ok(first.axis)
}
