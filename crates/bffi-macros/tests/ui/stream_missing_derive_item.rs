#![allow(unused)]
use bffi_macros::bffi_stream;

// Not derived: no `BffiRecord`, no `BffiWire` impl.
struct Tick {
    at: f64,
}

#[bffi_stream]
fn ticks() -> impl Iterator<Item = Tick> + Send {
    std::iter::empty()
}

fn main() {}
