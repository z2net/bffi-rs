#![allow(unused)]
use bffi_macros::bffi_async;

mod views {
    pub struct ZeroCopyStr<'a>(&'a str);
    pub struct ZeroCopyBuf<'a>(&'a [u8]);
}

use views::ZeroCopyStr;

#[bffi_async]
async fn bare(text: ZeroCopyStr<'_>) {}

#[bffi_async]
async fn qualified(data: views::ZeroCopyBuf<'_>) {}

fn main() {}
