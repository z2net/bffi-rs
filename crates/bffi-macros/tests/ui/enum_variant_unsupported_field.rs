#![allow(unused)]
use bffi::BffiEnum;

#[derive(BffiEnum)]
pub enum Bad {
    Scores(Vec<Option<u32>>),
}

fn main() {}
