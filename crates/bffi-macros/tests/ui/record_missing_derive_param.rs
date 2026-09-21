#![allow(unused)]
use bffi_macros::bffi;

// Not derived: no `BffiRecord`, no `BffiWire` impl.
struct Point {
    x: f64,
    y: f64,
}

#[bffi]
fn move_to(p: Point) {}

fn main() {}
