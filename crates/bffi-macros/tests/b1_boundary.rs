//! B1 acceptance: `#[bffi]` functions over records, enums and
//! sequences - the generated shims decode wire payloads in and encode
//! transient-buffer handles out, and the descriptors carry the
//! composite `TsType`s.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use bffi::bffi_dts::TsType;
use bffi::{BffiEnum, BffiRecord, BffiWire};

#[derive(BffiEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(BffiRecord, Debug, PartialEq)]
pub struct Sample {
    pub at: f64,
    pub weight: i32,
    pub label: String,
    pub axis: Axis,
}

/// Recenters a sample (record in, record out).
#[bffi::bffi]
fn recenter(sample: Sample, dx: f64) -> Sample {
    Sample {
        at: sample.at + dx,
        ..sample
    }
}

/// Sums the weights (record in, primitive out).
#[bffi::bffi]
fn total_weight(samples: Vec<Sample>) -> i64 {
    samples.iter().map(|s| i64::from(s.weight)).sum()
}

/// Distances as a sequence return.
#[bffi::bffi]
fn distances(samples: Vec<Sample>) -> Vec<f64> {
    samples.iter().map(|s| s.at.abs()).collect()
}

/// Names only (String sequence).
#[bffi::bffi]
fn labels(samples: Vec<Sample>) -> Vec<String> {
    samples.into_iter().map(|s| s.label).collect()
}

/// The dominant axis (enum out).
#[bffi::bffi]
fn classify(samples: Vec<Sample>) -> Result<Axis, SampleError> {
    if samples.is_empty() {
        return Err(SampleError("no samples".to_owned()));
    }
    Ok(samples[0].axis)
}

/// The domain error of the module.
#[derive(Debug)]
pub struct SampleError(String);

impl std::fmt::Display for SampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SampleError {}

/// Encodes one sample as a wire payload buffer.
fn wire_of(sample: &Sample) -> Vec<u8> {
    let mut out = Vec::new();
    sample.bffi_wire_encode(&mut out);
    out
}

/// Encodes samples as one sequence payload buffer.
fn wire_seq_of(samples: &[Sample]) -> Vec<u8> {
    use bffi::bffi_types::wire as w;
    let mut out = Vec::new();
    w::encode_seq_header(&mut out, samples.len());
    for sample in samples {
        sample.bffi_wire_encode(&mut out);
    }
    out
}

/// Reads a transient-buffer handle back into bytes.
fn read_buffer(handle: bffi::Handle) -> Vec<u8> {
    // SAFETY: `buffer_ptr` handed out the pointer to exactly
    // `buffer_len(handle)` owned bytes; the handle is still live.
    Vec::from(unsafe {
        std::slice::from_raw_parts(
            bffi::build::runtime::buffer_ptr(handle),
            bffi::build::runtime::buffer_len(handle) as usize,
        )
    })
}

/// Decodes one record out of a transient-buffer handle.
fn decode_record<T>(handle: bffi::Handle) -> T
where
    T: BffiWire,
{
    let bytes = read_buffer(handle);
    let (value, end) = T::bffi_wire_decode(&bytes, 0).expect("decode record");
    assert_eq!(end, bytes.len());
    assert!(bffi::build::runtime::free_buffer(handle));
    value
}

/// Decodes a `Vec<f64>` sequence out of a transient-buffer handle.
fn decode_f64_seq(handle: bffi::Handle) -> Vec<f64> {
    use bffi::bffi_types::wire as w;
    let bytes = read_buffer(handle);
    let (count, mut offset) = w::decode_seq_header(&bytes, 0).expect("seq header");
    let mut items = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let (value, next) = w::decode_f64(&bytes, offset).expect("item");
        offset = next;
        items.push(value);
    }
    assert_eq!(offset, bytes.len());
    assert!(bffi::build::runtime::free_buffer(handle));
    items
}

/// Decodes a `Vec<String>` sequence out of a transient-buffer handle.
fn decode_str_seq(handle: bffi::Handle) -> Vec<String> {
    use bffi::bffi_types::wire as w;
    let bytes = read_buffer(handle);
    let (count, mut offset) = w::decode_seq_header(&bytes, 0).expect("seq header");
    let mut items = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let (value, next) = w::decode_str(&bytes, offset).expect("item");
        offset = next;
        items.push(String::from(value));
    }
    assert_eq!(offset, bytes.len());
    assert!(bffi::build::runtime::free_buffer(handle));
    items
}

fn sample(at: f64, weight: i32, label: &str, axis: Axis) -> Sample {
    Sample {
        at,
        weight,
        label: label.to_owned(),
        axis,
    }
}

#[test]
fn record_params_and_returns_round_trip() {
    let input = sample(1.5, 10, "a", Axis::Vertical);
    let wire = wire_of(&input);
    let mut out = 0_u64;
    let code = bffi_recenter(wire.as_ptr(), wire.len() as u64, 2.5, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok);
    let output: Sample = decode_record(bffi::Handle::from_raw(out));
    assert_eq!(output, sample(4.0, 10, "a", Axis::Vertical));
}

#[test]
fn record_sequences_cross_both_ways() {
    let samples = vec![
        sample(1.0, 2, "a", Axis::Horizontal),
        sample(-3.0, 40, "b", Axis::Vertical),
    ];
    let wire = wire_seq_of(&samples);

    let mut out = 0_i64;
    let code = bffi_total_weight(wire.as_ptr(), wire.len() as u64, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok);
    assert_eq!(out, 42);

    let mut handle_val = 0_u64;
    let code = bffi_distances(wire.as_ptr(), wire.len() as u64, &mut handle_val);
    assert_eq!(code, bffi::ErrorCode::Ok);
    let handle = bffi::Handle::from_raw(handle_val);
    let distances = decode_f64_seq(handle);
    assert_eq!(distances, vec![1.0, 3.0]);
}

#[test]
fn string_sequences_round_trip() {
    let samples = vec![
        sample(0.0, 1, "x", Axis::Horizontal),
        sample(0.0, 1, "y", Axis::Horizontal),
    ];
    let wire = wire_seq_of(&samples);
    let mut handle_val = 0_u64;
    let code = bffi_labels(wire.as_ptr(), wire.len() as u64, &mut handle_val);
    assert_eq!(code, bffi::ErrorCode::Ok);
    let labels = decode_str_seq(bffi::Handle::from_raw(handle_val));
    assert_eq!(labels, ["x", "y"]);
}

#[test]
fn enum_result_return_resolves() {
    let samples = vec![sample(0.0, 1, "a", Axis::Vertical)];
    let wire = wire_seq_of(&samples);
    let mut out = 0_u64;
    let code = bffi_classify(wire.as_ptr(), wire.len() as u64, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok);
    let axis: Axis = decode_record(bffi::Handle::from_raw(out));
    assert_eq!(axis, Axis::Vertical);
}

#[test]
fn enum_result_err_reports_the_domain_error() {
    let wire = wire_seq_of(&[]);
    let mut out = 0_u64;
    let code = bffi_classify(wire.as_ptr(), wire.len() as u64, &mut out);
    assert_eq!(code, bffi::ErrorCode::DomainError);
    let error = bffi::take_last_error().expect("stored");
    assert!(error.message.contains("no samples"));
}

#[test]
fn null_record_payload_is_rejected() {
    let mut out = 0_u64;
    let code = bffi_recenter(std::ptr::null(), 0, 0.0, &mut out);
    assert_eq!(code, bffi::ErrorCode::NullPointer);
}

#[test]
fn malformed_record_payload_is_an_error() {
    // Garbage bytes: the header tag check must reject.
    let garbage = [0xFF_u8; 8];
    let mut out = 0_u64;
    let code = bffi_recenter(garbage.as_ptr(), garbage.len() as u64, 0.0, &mut out);
    assert_ne!(code, bffi::ErrorCode::Ok);
}

#[test]
fn descriptors_carry_the_composite_types() {
    assert_eq!(
        bffi_meta_recenter::FUNCTION.params[0].ty,
        TsType::Record("Sample")
    );
    assert_eq!(bffi_meta_recenter::FUNCTION.ret, TsType::Record("Sample"));
    assert_eq!(
        bffi_meta_total_weight::FUNCTION.params[0].ty,
        TsType::RecordArray("Sample")
    );
    assert_eq!(bffi_meta_total_weight::FUNCTION.ret, TsType::BigInt);
    assert_eq!(bffi_meta_distances::FUNCTION.ret, TsType::NumberArray);
    assert_eq!(bffi_meta_labels::FUNCTION.ret, TsType::StringArray);
    assert_eq!(bffi_meta_classify::FUNCTION.ret, TsType::Enum("Axis"));
    assert_eq!(
        bffi_meta_recenter::FUNCTION.abi.params[0],
        bffi::AbiType::PtrLen
    );
    assert_eq!(
        bffi_meta_recenter::FUNCTION.abi.out,
        Some(bffi::AbiOut::Handle)
    );
}
