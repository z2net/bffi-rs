//! Criterion benches for the SIMD UTF-8 validator's public entry
//! (`bffi::types::bytes_to_string`): three ~4 KiB valid payloads -
//! pure ASCII, 2-byte Cyrillic, 4-byte emoji - through the full
//! validate-then-copy path. The invalid case stays in the correctness
//! gate only: a rejected input is not a throughput measurement.

#![allow(missing_docs)] // the criterion_group! expansion is undocumented
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bffi::types::bytes_to_string;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

/// The payload size every bench input rounds to (~4 KiB).
const PAYLOAD_BYTES: usize = 4096;

/// The pure ASCII unit the first payload repeats.
const ASCII: char = 'a';

/// The 2-byte Cyrillic unit the second payload repeats (U+0430).
const CYRILLIC: char = '\u{0430}';

/// The 4-byte emoji unit the third payload repeats (U+1F600).
const EMOJI: char = '\u{1F600}';

/// Builds `PAYLOAD_BYTES` bytes by repeating `unit`'s UTF-8 encoding.
fn repeated_payload(unit: char) -> Vec<u8> {
    let mut buf = [0_u8; 4];
    let encoded = unit.encode_utf8(&mut buf);
    let mut out = Vec::with_capacity(PAYLOAD_BYTES);
    while out.len() + encoded.len() <= PAYLOAD_BYTES {
        out.extend_from_slice(encoded.as_bytes());
    }
    out
}

fn bench_utf8(c: &mut Criterion) {
    let ascii = repeated_payload(ASCII);
    let cyrillic = repeated_payload(CYRILLIC);
    let emoji = repeated_payload(EMOJI);
    // The invalid tail: valid ASCII cut short by a 2-byte lead whose
    // continuation never arrives.
    let mut invalid = repeated_payload(ASCII);
    invalid.truncate(PAYLOAD_BYTES - 6);
    invalid.push(0xD0);

    // Correctness gate: every valid payload decodes to exactly its
    // source units, and the truncated tail is rejected, before
    // anything is timed.
    assert_eq!(
        bytes_to_string(&ascii)
            .expect("valid ascii")
            .chars()
            .count(),
        PAYLOAD_BYTES,
        "the ASCII payload must survive the round trip"
    );
    assert_eq!(
        bytes_to_string(&cyrillic)
            .expect("valid cyrillic")
            .chars()
            .count(),
        PAYLOAD_BYTES / 2,
        "the Cyrillic payload must survive the round trip"
    );
    assert_eq!(
        bytes_to_string(&emoji)
            .expect("valid emoji")
            .chars()
            .count(),
        PAYLOAD_BYTES / 4,
        "the emoji payload must survive the round trip"
    );
    assert!(
        bytes_to_string(&invalid).is_err(),
        "the truncated tail must be rejected"
    );

    // The three valid payloads, one group so criterion reports the
    // bytes-per-second throughput alongside the timing.
    let mut group = c.benchmark_group("utf8");
    group.throughput(Throughput::Bytes(PAYLOAD_BYTES as u64));

    // Pure ASCII: the cheapest validation + the full copy.
    group.bench_function("ascii_4k", |b| {
        b.iter(|| black_box(bytes_to_string(black_box(&ascii)).expect("valid payload")))
    });

    // 2-byte Cyrillic.
    group.bench_function("cyrillic_4k", |b| {
        b.iter(|| black_box(bytes_to_string(black_box(&cyrillic)).expect("valid payload")))
    });

    // 4-byte emoji.
    group.bench_function("emoji_4k", |b| {
        b.iter(|| black_box(bytes_to_string(black_box(&emoji)).expect("valid payload")))
    });

    group.finish();
}

criterion_group!(benches, bench_utf8);
criterion_main!(benches);
