//! Handle-system invariant tests (security review: explicit ABA /
//! corruption / churn coverage through the PUBLIC API only).
//!
//! Existing coverage this file deliberately does NOT duplicate:
//! - ABA on a reused slot: `object-lifecycle.rs ::
//!   arc_outlives_release_and_reused_slot_stales_old_handle`
//! - double release: `object-lifecycle.rs :: double_release_is_invalid_handle`
//!   (pins the InvalidHandle-tolerant JS contract)
//! - foreign-tag type confusion: `object-lifecycle.rs ::
//!   foreign_type_handle_is_rejected`
//! - single-handle release/get race: `object-concurrency.rs ::
//!   release_concurrent_with_get_never_resurrects`
//! - callback-table revoke-vs-invoke race: `callback-concurrency.rs`
//! - deep unit-level coverage of the same barriers lives in the
//!   `#[cfg(test)]` modules of `core/table.rs` (stale-after-reuse,
//!   foreign tag, double remove) and `core/handle.rs` (bit layout).
//!
//! Isolation model: tests of one binary share the process-wide
//! registry, so each test reserves its own unique tag constants
//! (P1-SPEC §5), inside the bffi-object range 0x0100-0x01FF.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bffi::bffi_object::{ObjectError, ObjectWrap};
use bffi::{Handle, TypeTag};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const TAG_MANGLE: TypeTag = TypeTag(0x0170);
const TAG_CHURN: TypeTag = TypeTag(0x0171);
const TAG_RACE: TypeTag = TypeTag(0x0172);

/// Deterministic xorshift64* PRNG (no external rng dependency).
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Property loop over ~1000 corrupted handles: `get`/`release` must
/// never panic and must never resolve to anything but an exactly-live
/// handle of THIS wrap (the value must match the live map). Covers
/// bit flips in every handle region (tag / generation / slot) plus
/// fully random garbage and cross-table probes (callback + task tags).
#[test]
fn corrupted_handles_never_panic_and_never_resolve_wrongly() {
    let wrap = ObjectWrap::<u64>::new(TAG_MANGLE).expect("unique tag registers");

    // Ten live handles whose values the test knows exactly.
    let mut live: HashMap<Handle, u64> = HashMap::new();
    for value in 0..10_u64 {
        let handle = wrap.wrap(value).expect("table has room");
        live.insert(handle, value);
    }

    let mut rng = Xorshift(0xBFF1_0000_0000_0001);
    let mut mangled = Vec::new();

    for _ in 0..1000 {
        let raw = match rng.next() % 4 {
            // Flip one random bit in the tag region (bits 48..64).
            0 => {
                let source = live.keys().next().expect("non-empty").as_u64();
                source ^ (1 << (48 + rng.next() % 16))
            }
            // Flip one random bit in the generation region (24..48).
            1 => {
                let source = live.keys().next().expect("non-empty").as_u64();
                source ^ (1 << (24 + rng.next() % 24))
            }
            // Flip one random bit in the slot region (0..24).
            2 => {
                let source = live.keys().next().expect("non-empty").as_u64();
                source ^ (1 << (rng.next() % 24))
            }
            // Fully random 64 bits: null, u64::MAX, and forged
            // handles of FOREIGN tables (callback tag 0x0200, task tag
            // 0x0500) with plausible generation/slot fields.
            _ => match rng.next() % 5 {
                0 => 0,
                1 => u64::MAX,
                2 => Handle::new(
                    TypeTag(0x0200),
                    (rng.next() % 4) as u32,
                    (rng.next() % 8) as u32,
                )
                .as_u64(),
                3 => Handle::new(
                    TypeTag(0x0500),
                    (rng.next() % 4) as u32,
                    (rng.next() % 8) as u32,
                )
                .as_u64(),
                _ => rng.next(),
            },
        };
        let forged = Handle::from_raw(raw);
        mangled.push(forged);

        // THE invariant: no panic; and for the OWNING tag, an Ok only
        // for an exactly-live handle with exactly its own value - the
        // generation barrier keeps every corrupted gen/slot dead. A
        // foreign-tag handle routes to ANOTHER table (the sibling
        // tests of this process churn their own u64 tables under
        // different tags), where the documented barriers are the tag
        // routing + type downcast - only no-panic can be asserted
        // there.
        if let Ok(value) = wrap.get(forged)
            && forged.tag() == TAG_MANGLE
        {
            let expected = live
                .get(&forged)
                .unwrap_or_else(|| panic!("corrupted handle {forged} resolved but is not live"));
            assert_eq!(
                *value, *expected,
                "corrupted handle {forged} resolved wrongly"
            );
        }
        // Every genuinely live handle keeps resolving meanwhile.
        for (&handle, &value) in &live {
            assert_eq!(*wrap.get(handle).expect("live handle resolves"), value);
        }
    }

    // Release of owning-tag corrupted handles: never panics; when it
    // succeeds it must have removed exactly the mapped value of a live
    // handle. Foreign-tag handles are never released - their slots
    // belong to other tables.
    for forged in mangled.iter().copied().filter(|h| h.tag() == TAG_MANGLE) {
        if let Ok(value) = wrap.release(forged) {
            let expected = live
                .get(&forged)
                .unwrap_or_else(|| panic!("release of corrupted {forged} removed a foreign slot"));
            assert_eq!(*value, *expected);
            live.remove(&forged);
        }
    }
}

/// Churn invariant: 10k wrap/get/release cycles with slot reuse; every
/// live handle must always resolve to ITS OWN value, long-lived
/// "parked" handles must survive the churn around them, and released
/// handles must stay dead. (RSS growth is not portable to assert;
/// correctness under churn is the pinned invariant here.)
#[test]
fn slot_reuse_churn_keeps_every_handle_its_own_value() {
    let wrap = ObjectWrap::<u64>::new(TAG_CHURN).expect("unique tag registers");

    // Handles parked across the whole churn: their slots are NOT
    // reused, but every other slot is, thousands of times.
    let mut parked = Vec::new();
    let mut retired = Vec::new();

    for i in 0..10_000_u64 {
        let handle = wrap.wrap(i).expect("table has room");
        assert_eq!(*wrap.get(handle).expect("fresh handle resolves"), i);

        if i % 200 == 0 {
            parked.push((handle, i));
        } else {
            wrap.release(handle).expect("live handle releases");
            retired.push(handle);
        }

        // Spot-check every 100th iteration: parked handles still hold
        // their own values after thousands of slot reuses.
        if i % 100 == 0 {
            for &(handle, value) in &parked {
                assert_eq!(
                    *wrap.get(handle).expect("parked handle stays live"),
                    value,
                    "parked handle resolved to a foreign value"
                );
            }
        }
    }

    // A sample of retired handles must never come back to life.
    for handle in retired.into_iter().step_by(997) {
        assert_eq!(
            wrap.get(handle).err(),
            Some(ObjectError::InvalidHandle(handle)),
            "a released handle resurrected across churn"
        );
    }

    // Parked survivors release cleanly and resolve until released.
    for (handle, value) in parked {
        assert_eq!(*wrap.get(handle).expect("parked until release"), value);
        assert!(wrap.release(handle).is_ok());
    }
}

/// Concurrent free/use invariant: 4 hammer threads wrap + publish +
/// release the same shared ObjectWrap while a reader continuously
/// resolves the published handles. A read must either fail cleanly
/// (handle already released - stale reads are ALWAYS Err) or return
/// exactly the value that handle was wrapped with - never a foreign
/// value, never a panic. Unlike `object-concurrency.rs` (which keeps
/// each handle out of racing hands), this races wrap/release/get on
/// the SAME handles.
#[test]
fn concurrent_wrap_release_reader_never_sees_a_foreign_value() {
    let wrap = ObjectWrap::<u64>::new(TAG_RACE).expect("unique tag registers");

    // handle -> the value it was wrapped with (published on wrap,
    // removed on release; the reader snapshots it).
    let published: Arc<Mutex<HashMap<Handle, u64>>> = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicUsize::new(0));

    const HAMMERS: usize = 4;
    const ROUNDS: usize = 1500;

    thread::scope(|scope| {
        // The reader: resolves published handles until all hammers
        // stopped. Ok must carry exactly the published value.
        let reader_published = Arc::clone(&published);
        let reader_stop = Arc::clone(&stop);
        scope.spawn(move || {
            loop {
                if reader_stop.load(Ordering::Acquire) == HAMMERS {
                    break;
                }
                let snapshot: Vec<(Handle, u64)> = reader_published
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .map(|(&handle, &value)| (handle, value))
                    .collect();
                for (handle, value) in snapshot {
                    // Stale (already released) reads are Err: fine. An Ok
                    // must be THIS handle's own value - a reused slot with
                    // a stale generation must never resolve.
                    if let Ok(seen) = wrap.get(handle) {
                        assert_eq!(
                            *seen, value,
                            "handle {handle} resolved to a foreign value mid-race"
                        );
                    }
                }
                thread::yield_now();
            }
        });

        // The hammers: wrap -> publish -> release -> unpublish, with a
        // yield between publish and release so the reader really races
        // the release window.
        let mut joins = Vec::new();
        for t in 0..HAMMERS {
            let published = Arc::clone(&published);
            joins.push(scope.spawn(move || {
                for i in 0..ROUNDS as u64 {
                    let value = (t as u64) << 32 | i;
                    let handle = wrap.wrap(value).expect("table has room");
                    published
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(handle, value);
                    thread::yield_now();
                    // Exactly one release wins; a racing double release
                    // is InvalidHandle (pinned by object-lifecycle.rs).
                    if let Ok(released) = wrap.release(handle) {
                        assert_eq!(*released, value);
                    }
                    published
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&handle);
                }
            }));
        }
        for join in joins {
            join.join().expect("hammer must not panic");
        }
        stop.store(HAMMERS, Ordering::Release);
    });

    // After the race everything was released: nothing may resolve.
    let leftover: Vec<Handle> = published
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .keys()
        .copied()
        .collect();
    for handle in leftover {
        assert!(wrap.get(handle).is_err(), "no handle may outlive the race");
    }
}
