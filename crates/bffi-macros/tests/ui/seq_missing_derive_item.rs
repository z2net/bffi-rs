#![allow(unused)]
use bffi_macros::bffi;

// Not derived: no `BffiRecord`, no `BffiWire` impl.
struct Sample {
    at: f64,
}

#[bffi]
fn take(samples: Vec<Sample>) {}

fn main() {}
