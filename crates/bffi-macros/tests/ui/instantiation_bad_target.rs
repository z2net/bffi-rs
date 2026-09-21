#![allow(unused)]
use bffi::bffi_impl_wire;

bffi_impl_wire! {
    (u32, u32) as Tuple {
        first: u32,
    }
}

fn main() {}
