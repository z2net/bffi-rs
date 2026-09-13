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

/// Optional flavors over every supported field kind.
#[derive(BffiRecord, Debug, PartialEq)]
pub struct Profile {
    pub nick: Option<String>,
    pub level: Option<u32>,
    pub rank: Option<u64>,
    pub muted: Option<bool>,
    pub avatar: Option<Vec<u8>>,
    pub home: Option<Point>,
    pub status: Option<JobStatus>,
    pub hits: Option<i16>,
    pub big: Option<i64>,
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

/// A `Point` with the values the `Profile` fixtures below reference.
fn profile_home() -> Point {
    Point {
        x: 1.5,
        y: -2,
        big: 7,
        flag: false,
        label: "home".to_owned(),
        payload: vec![4, 5],
        status: JobStatus::Idle,
    }
}

#[test]
fn option_fields_round_trip_some_values() {
    let value = Profile {
        nick: Some("ada".to_owned()),
        level: Some(u32::MAX),
        rank: Some(u64::MAX),
        muted: Some(true),
        avatar: Some(vec![1, 2, 3]),
        home: Some(profile_home()),
        status: Some(JobStatus::Running),
        hits: Some(i16::MIN),
        big: Some(i64::MIN),
    };
    let mut wire = Vec::new();
    value.bffi_wire_encode(&mut wire);
    let (decoded, end) = Profile::bffi_wire_decode(&wire, 0).expect("decode");
    assert_eq!(decoded, value);
    assert_eq!(end, wire.len());
}

#[test]
fn option_fields_round_trip_none_values() {
    let value = Profile {
        nick: None,
        level: None,
        rank: None,
        muted: None,
        avatar: None,
        home: None,
        status: None,
        hits: None,
        big: None,
    };
    let mut wire = Vec::new();
    value.bffi_wire_encode(&mut wire);
    // Every `None` rides exactly one TAG_UNIT byte.
    assert_eq!(wire.len(), 1 + 4 + 9);
    let (decoded, end) = Profile::bffi_wire_decode(&wire, 0).expect("decode");
    assert_eq!(decoded, value);
    assert_eq!(end, wire.len());
}

#[test]
fn option_field_wire_layout_distinguishes_none_from_some() {
    // A single `Option<u32>` field: `None` is one unit byte, `Some`
    // is the plain value record.
    #[derive(BffiRecord, Debug, PartialEq)]
    struct Maybe {
        pub level: Option<u32>,
    }
    let mut none = Vec::new();
    Maybe { level: None }.bffi_wire_encode(&mut none);
    assert_eq!(
        none,
        vec![
            bffi::bffi_types::wire::TAG_RECORD,
            1,
            0,
            0,
            0, // field count
            bffi::bffi_types::wire::TAG_UNIT,
        ]
    );

    let mut some = Vec::new();
    Maybe { level: Some(3) }.bffi_wire_encode(&mut some);
    // `Some` replaces the unit byte with the f64 value record.
    assert_eq!(some.len(), none.len() - 1 + 1 + 8);
    assert_eq!(some[5], bffi::bffi_types::wire::TAG_F64);
    assert!(Maybe::bffi_wire_decode(&none, 0).is_ok());
    assert!(Maybe::bffi_wire_decode(&some, 0).is_ok());
}

#[test]
fn option_field_descriptors_carry_the_nullable_types() {
    let record = Profile::BFFI_RECORD_DEF;
    assert_eq!(record.fields.len(), 9);
    assert_eq!(record.fields[0].ty, TsType::NullableString);
    assert_eq!(record.fields[1].ty, TsType::NullableNumber);
    assert_eq!(record.fields[2].ty, TsType::NullableBigInt);
    assert_eq!(record.fields[3].ty, TsType::NullableBoolean);
    assert_eq!(record.fields[4].ty, TsType::NullableUint8Array);
    assert_eq!(record.fields[5].ty, TsType::NullableRecord("Point"));
    assert_eq!(record.fields[6].ty, TsType::NullableRecord("JobStatus"));
    assert_eq!(record.fields[7].ty, TsType::NullableNumber);
    assert_eq!(record.fields[8].ty, TsType::NullableBigInt);
}

#[test]
fn truncated_optional_field_is_an_error() {
    let value = Profile {
        nick: Some("ada".to_owned()),
        level: None,
        rank: None,
        muted: None,
        avatar: None,
        home: None,
        status: None,
        hits: None,
        big: None,
    };
    let mut wire = Vec::new();
    value.bffi_wire_encode(&mut wire);
    // Cut inside the `Some` string payload.
    assert!(Profile::bffi_wire_decode(&wire[..wire.len() - 1], 0).is_err());
}
