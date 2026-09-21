#![allow(unused)]
use bffi::BffiRecord;

#[derive(BffiRecord)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[bffi::bffi]
fn place(p: Option<Option<Point>>) {}

fn main() {}
