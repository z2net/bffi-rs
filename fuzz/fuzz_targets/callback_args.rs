#![no_main]

//! Fuzz target for the callback argument decoder
//! (`bffi::bffi_callback::abi::decode_args`): malformed argument
//! buffers must surface as clean `InvalidArgument` errors - never a
//! panic, never an out-of-bounds read - across the full tag table,
//! including the composite `Wire` records whose children the decoder
//! walks recursively.

use libfuzzer_sys::fuzz_target;

use bffi::bffi_callback::abi::decode_args;

fuzz_target!(|data: &[u8]| {
    let _ = decode_args(data);
});
