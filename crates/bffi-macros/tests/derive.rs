//! Acceptance tests for the B1 derive macros: the generated wire
//! codec round-trips every supported field kind, nested records and
//! enums compose, and the descriptor consts match the expected
//! table entries.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use bffi::bffi_dts::TsType;
use bffi::{BffiEnum, BffiRecord, BffiWire};

/// The status of a job.
#[derive(BffiEnum, Debug, PartialEq)]
pub enum JobStatus {
    /// Waiting.
    Idle,
    Running,
    Done,
}

/// A point in 2D space.
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Point {
    /// The x coordinate.
    pub x: f64,
    pub y: i32,
    pub big: i64,
    pub flag: bool,
    pub label: String,
    pub payload: Vec<u8>,
    pub status: JobStatus,
}

/// A rectangle over two points.
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Rect {
    pub top_left: Point,
    pub bottom_right: Point,
}

#[test]
fn record_round_trips_every_field_kind() {
    let value = Point {
        x: 1.25,
        y: -7,
        big: i64::MAX,
        flag: true,
        label: "héllo".to_owned(),
        payload: vec![1, 2, 3],
        status: JobStatus::Running,
    };
    let mut wire = Vec::new();
    value.bffi_wire_encode(&mut wire);
    let (decoded, end) = Point::bffi_wire_decode(&wire, 0).expect("decode");
    assert_eq!(decoded, value);
    assert_eq!(end, wire.len());
}

#[test]
fn nested_records_round_trip() {
    let a = Point {
        x: 0.0,
        y: 1,
        big: 2,
        flag: false,
        label: String::new(),
        payload: Vec::new(),
        status: JobStatus::Idle,
    };
    let b = Point {
        x: 9.5,
        y: 3,
        big: 4,
        flag: true,
        label: "b".to_owned(),
        payload: vec![9],
        status: JobStatus::Done,
    };
    let rect = Rect {
        top_left: a,
        bottom_right: b,
    };
    let mut wire = Vec::new();
    rect.bffi_wire_encode(&mut wire);
    let (decoded, end) = Rect::bffi_wire_decode(&wire, 0).expect("decode");
    assert_eq!(decoded, rect);
    assert_eq!(end, wire.len());
}

#[test]
fn enum_round_trips_and_rejects_unknown() {
    let mut wire = Vec::new();
    JobStatus::Done.bffi_wire_encode(&mut wire);
    let (decoded, end) = JobStatus::bffi_wire_decode(&wire, 0).expect("decode");
    assert_eq!((decoded, end), (JobStatus::Done, wire.len()));

    // An unknown variant name is an error, not a panic.
    let mut bogus = Vec::new();
    bffi::bffi_types::wire::encode_str(&mut bogus, "Paused");
    assert!(JobStatus::bffi_wire_decode(&bogus, 0).is_err());
}

#[test]
fn malformed_record_wire_is_an_error() {
    // Empty input.
    assert!(Point::bffi_wire_decode(&[], 0).is_err());
    // Right tag, wrong field count.
    let mut wrong = Vec::new();
    bffi::bffi_types::wire::encode_record_header(&mut wrong, 2);
    assert!(Point::bffi_wire_decode(&wrong, 0).is_err());
    // Truncated after a correct header.
    let value = Point {
        x: 0.0,
        y: 0,
        big: 0,
        flag: false,
        label: String::new(),
        payload: Vec::new(),
        status: JobStatus::Idle,
    };
    let mut wire = Vec::new();
    value.bffi_wire_encode(&mut wire);
    assert!(Point::bffi_wire_decode(&wire[..wire.len() - 1], 0).is_err());
}

#[test]
fn descriptor_consts_match_the_types() {
    assert_eq!(Point::BFFI_TS_TYPE, TsType::Record("Point"));
    assert_eq!(JobStatus::BFFI_TS_TYPE, TsType::Enum("JobStatus"));

    let record = Point::BFFI_RECORD_DEF;
    assert_eq!(record.js_name, "Point");
    assert_eq!(record.fields.len(), 7);
    assert_eq!(record.fields[0].name, "x");
    assert_eq!(record.fields[0].docs, ["The x coordinate."]);
    assert_eq!(record.fields[0].ty, TsType::Number);
    assert_eq!(record.fields[1].ty, TsType::Number);
    assert_eq!(record.fields[2].ty, TsType::BigInt);
    assert_eq!(record.fields[3].ty, TsType::Boolean);
    assert_eq!(record.fields[4].ty, TsType::String);
    assert_eq!(record.fields[5].ty, TsType::Uint8Array);
    assert_eq!(record.fields[6].ty, TsType::Enum("JobStatus"));

    let enumeration = JobStatus::BFFI_ENUM_DEF;
    assert_eq!(enumeration.js_name, "JobStatus");
    assert_eq!(enumeration.variants.len(), 3);
    assert_eq!(enumeration.variants[0].name, "Idle");
    assert_eq!(enumeration.variants[0].docs, ["Waiting."]);
}

#[test]
fn narrow_widths_survive_the_wire() {
    /// An edge record exercising every narrow width.
    #[derive(BffiRecord, Debug, PartialEq)]
    struct Narrow {
        pub a: i8,
        pub b: u8,
        pub c: i16,
        pub d: u16,
        pub e: u32,
        pub f: f32,
    }
    let value = Narrow {
        a: i8::MIN,
        b: u8::MAX,
        c: i16::MIN,
        d: u16::MAX,
        e: u32::MAX,
        f: f32::MIN_POSITIVE,
    };
    let mut wire = Vec::new();
    value.bffi_wire_encode(&mut wire);
    let (decoded, end) = Narrow::bffi_wire_decode(&wire, 0).expect("decode");
    assert_eq!(decoded, value);
    assert_eq!(end, wire.len());
}
