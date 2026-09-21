//! Explicit instantiation acceptance: a generic local struct crosses
//! the boundary through `bffi_impl_wire!` under a distinct wire name,
//! reusing the record field matrix. The macro emits the public alias
//! (the name `#[bffi]` parameters reference), the descriptor consts
//! and the `BffiWire` impl for the concrete instantiation.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use bffi::BffiWire;
use bffi::bffi_dts::TsType;

/// A generic pair (local to the user crate: the orphan rule keeps the
/// generated `BffiWire` impl legal).
#[derive(Debug, PartialEq)]
pub struct Pair<T> {
    pub first: T,
    pub second: T,
}

// The `u32` instantiation: the docs travel inside the macro input
// onto the emitted alias.
bffi::bffi_impl_wire! {
    /// A pair of unsigned 32-bit values.
    Pair<u32> as PairU32 {
        /// The first value.
        first: u32,
        second: u32,
    }
}

/// Swaps an instantiated pair (record in, record out).
#[bffi::bffi]
fn swap(p: PairU32) -> PairU32 {
    Pair {
        first: p.second,
        second: p.first,
    }
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

#[test]
fn instantiated_pair_round_trips_through_the_shim() {
    let value = Pair::<u32> {
        first: 1,
        second: u32::MAX,
    };
    let mut wire = Vec::new();
    value.bffi_wire_encode(&mut wire);

    let mut out = 0_u64;
    let code = bffi_swap(wire.as_ptr(), wire.len() as u64, &mut out);
    assert_eq!(code, bffi::ErrorCode::Ok.as_u32());

    let bytes = read_buffer(bffi::Handle::from_raw(out));
    let (decoded, end) = PairU32::bffi_wire_decode(&bytes, 0).expect("decode");
    assert_eq!(
        decoded,
        Pair {
            first: u32::MAX,
            second: 1,
        }
    );
    assert_eq!(end, bytes.len());
}

#[test]
fn instantiated_values_round_trip_on_the_value_level() {
    for value in [
        Pair::<u32> {
            first: 0,
            second: 0,
        },
        Pair::<u32> {
            first: u32::MAX,
            second: u32::MIN,
        },
    ] {
        let mut wire = Vec::new();
        value.bffi_wire_encode(&mut wire);
        let (decoded, end) = PairU32::bffi_wire_decode(&wire, 0).expect("decode");
        assert_eq!(decoded, value);
        assert_eq!(end, wire.len());
    }
}

#[test]
fn descriptors_carry_the_instantiated_name() {
    assert_eq!(PairU32::BFFI_TS_TYPE, TsType::Record("PairU32"));

    let record = PairU32::BFFI_RECORD_DEF;
    assert_eq!(record.js_name, "PairU32");
    assert_eq!(record.fields.len(), 2);
    assert_eq!(record.fields[0].name, "first");
    assert_eq!(record.fields[0].docs, ["The first value."]);
    assert_eq!(record.fields[0].ty, TsType::Number);
    assert_eq!(record.fields[1].name, "second");
    assert_eq!(record.fields[1].ty, TsType::Number);

    // The function descriptor references the alias name.
    assert_eq!(
        bffi_meta_swap::FUNCTION.params[0].ty,
        TsType::Record("PairU32")
    );
    assert_eq!(bffi_meta_swap::FUNCTION.ret, TsType::Record("PairU32"));
}
