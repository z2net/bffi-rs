#![allow(unused)]
use bffi::BffiRecord;

#[derive(BffiRecord)]
pub struct Bad {
    pub scores: Vec<Option<u32>>,
}

fn main() {}
