#![allow(unused)]
use bffi::BffiRecord;

#[derive(BffiRecord)]
pub struct Nested {
    pub maybe: Option<Option<u32>>,
}

fn main() {}
