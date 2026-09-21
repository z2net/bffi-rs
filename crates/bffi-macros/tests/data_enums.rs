//! Data-carrying enum acceptance: payload variants ride the
//! `TAG_RECORD` kind-envelope (variant name + positional payload
//! fields), unit-only enums stay on the byte-identical `TAG_STR`
//! path, and the descriptor entries carry the per-variant fields for
//! the discriminated-union render.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use bffi::bffi_dts::{RecordFieldDef, TsType};
use bffi::{BffiEnum, BffiWire};

/// A shape (mixed payload and unit variants).
#[derive(BffiEnum, Debug, PartialEq)]
pub enum Shape {
    /// A circle.
    Circle(f64),
    /// A rectangle.
    Rect {
        /// Width.
        w: f64,
        /// Height.
        h: i32,
    },
    /// Nothing.
    Nothing,
}

/// A unit-only enum (the legacy string-union path).
#[derive(BffiEnum, Debug, PartialEq)]
pub enum Status {
    /// Waiting.
    Idle,
    Running,
}

#[test]
fn data_enum_round_trips_every_variant() {
    for value in [
        Shape::Circle(1.5),
        Shape::Rect { w: 3.0, h: -4 },
        Shape::Nothing,
    ] {
        let mut wire = Vec::new();
        value.bffi_wire_encode(&mut wire);
        let (decoded, end) = Shape::bffi_wire_decode(&wire, 0).expect("decode");
        assert_eq!(decoded, value);
        assert_eq!(end, wire.len());
    }
}

#[test]
fn unit_only_enum_keeps_the_string_wire() {
    let mut wire = Vec::new();
    Status::Idle.bffi_wire_encode(&mut wire);
    assert_eq!(
        wire,
        vec![
            bffi::bffi_types::wire::TAG_STR,
            4,
            0,
            0,
            0,
            b'I',
            b'd',
            b'l',
            b'e',
        ]
    );
    let (decoded, end) = Status::bffi_wire_decode(&wire, 0).expect("decode");
    assert_eq!((decoded, end), (Status::Idle, wire.len()));
}

#[test]
fn mixed_enum_unit_variant_rides_the_record_envelope() {
    let mut wire = Vec::new();
    Shape::Nothing.bffi_wire_encode(&mut wire);
    // TAG_RECORD + count(1) + TAG_STR + "Nothing".
    assert_eq!(
        wire,
        vec![
            bffi::bffi_types::wire::TAG_RECORD,
            1,
            0,
            0,
            0,
            bffi::bffi_types::wire::TAG_STR,
            7,
            0,
            0,
            0,
            b'N',
            b'o',
            b't',
            b'h',
            b'i',
            b'n',
            b'g',
        ]
    );
}

#[test]
fn payload_variant_counts_the_kind_and_the_fields() {
    let mut wire = Vec::new();
    Shape::Circle(1.5).bffi_wire_encode(&mut wire);
    // TAG_RECORD + count(2: kind + 1 field) + kind string + f64.
    assert_eq!(wire[0], bffi::bffi_types::wire::TAG_RECORD);
    assert_eq!(wire[1], 2);
    assert_eq!(wire[5], bffi::bffi_types::wire::TAG_STR);
    assert_eq!(wire[16], bffi::bffi_types::wire::TAG_F64);
}

#[test]
fn truncated_payload_is_an_error() {
    let mut wire = Vec::new();
    Shape::Rect { w: 1.0, h: 2 }.bffi_wire_encode(&mut wire);
    assert!(Shape::bffi_wire_decode(&wire[..wire.len() - 1], 0).is_err());
}

#[test]
fn unknown_variant_name_is_an_error() {
    use bffi::bffi_types::wire as w;
    let mut bogus = Vec::new();
    w::encode_record_header(&mut bogus, 1);
    w::encode_str(&mut bogus, "Box");
    assert!(Shape::bffi_wire_decode(&bogus, 0).is_err());
}

#[test]
fn descriptors_carry_the_variant_fields() {
    assert_eq!(Shape::BFFI_TS_TYPE, TsType::Enum("Shape"));

    let def = Shape::BFFI_ENUM_DEF;
    assert_eq!(def.js_name, "Shape");
    assert_eq!(def.variants.len(), 3);

    let circle = &def.variants[0];
    assert_eq!(circle.name, "Circle");
    assert_eq!(circle.docs, ["A circle."]);
    assert_eq!(circle.fields.len(), 1);
    assert_eq!(
        circle.fields[0],
        RecordFieldDef {
            name: "_0",
            docs: &[],
            ty: TsType::Number,
        }
    );

    let rect = &def.variants[1];
    assert_eq!(rect.name, "Rect");
    assert_eq!(rect.fields.len(), 2);
    assert_eq!(rect.fields[0].name, "w");
    assert_eq!(rect.fields[0].ty, TsType::Number);
    assert_eq!(rect.fields[1].name, "h");
    assert_eq!(rect.fields[1].ty, TsType::Number);

    assert_eq!(def.variants[2].name, "Nothing");
    assert!(def.variants[2].fields.is_empty());

    let status = Status::BFFI_ENUM_DEF;
    assert!(
        status
            .variants
            .iter()
            .all(|variant| variant.fields.is_empty())
    );
}
