//! Exhaustive-bounded evidence for the reconnect backoff arithmetic (NFR-003-M-3).
//!
//! The delay after the `n`th consecutive failure is a closed-form function of
//! `n`, so it is checked against that function for every `n` from 0 to 100,000
//! and at the `u32` saturation edge, rather than sampled. A Kani harness would
//! prove the same closed form for every `n: u32` at once; the enumeration is a
//! proof over the bound and covers every distinct behaviour (the doubling
//! phase, the cap, and both saturating operations).

use std::time::Duration;

use super::*;

/// The requirement's arithmetic, stated without the implementation's
/// saturating tricks: 1 s doubled `n` times, never more than the cap.
fn specified_delay(n: u32) -> Duration {
    // 2^n seconds exceeds the cap from n = 5 (32 s), so any larger n is the cap.
    if n >= 5 {
        BACKOFF_CAP
    } else {
        Duration::from_secs(1 << n).min(BACKOFF_CAP)
    }
}

/// Tracing: TC-N003-M-03 (NFR-003-M-3)
/// ACs: NFR-003-M-3
///
/// The delay after every failure count up to 100,000 equals the specified
/// arithmetic exactly, is at most the cap (30 s, inside the row's 60 s
/// threshold) and at least the start (1 s), and `failures` counts each one.
#[test]
fn tc_n003_m03_the_delay_after_n_failures_is_the_capped_power_of_two_for_every_n_up_to_one_hundred_thousand()
 {
    let mut backoff = Backoff::default();
    for n in 0..100_000u32 {
        let delay = backoff.next_delay();
        assert_eq!(delay, specified_delay(n), "after {n} failures");
        assert!(
            (BACKOFF_START..=BACKOFF_CAP).contains(&delay),
            "{n}: {delay:?}"
        );
        assert_eq!(backoff.failures(), n + 1);
    }
}

/// Tracing: TC-N003-M-03 (NFR-003-M-3)
/// ACs: NFR-003-M-3
///
/// At the saturation edge of the failure counter neither `2^n` nor the count
/// overflows or panics: the delay stays at the cap and the count stays at
/// `u32::MAX`. (Unreachable in a run — it would take 4 billion failures — but
/// arithmetic in a supervisor that must never die should not have an edge.)
#[test]
fn tc_n003_m03_the_backoff_arithmetic_does_not_overflow_at_the_edge_of_the_failure_counter() {
    for start in [u32::MAX - 2, u32::MAX - 1, u32::MAX] {
        let mut backoff = Backoff { failures: start };
        for _ in 0..4 {
            assert_eq!(backoff.next_delay(), BACKOFF_CAP);
        }
        assert_eq!(backoff.failures(), u32::MAX);
        backoff.reset();
        assert_eq!(backoff.next_delay(), BACKOFF_START);
    }
}

/// Tracing: TC-N003-M-03 (NFR-003-M-3)
/// ACs: NFR-003-M-3
///
/// The total time spent waiting across any number of consecutive failures
/// grows by at most the cap per failure, so the longest gap between two
/// reconnection attempts is the cap (never the 60 s the row allows).
#[test]
fn tc_n003_m03_the_longest_gap_between_reconnection_attempts_is_the_cap() {
    let mut backoff = Backoff::default();
    let longest = (0..1_000).map(|_| backoff.next_delay()).max();
    assert_eq!(longest, Some(BACKOFF_CAP));
    assert!(BACKOFF_CAP <= Duration::from_secs(60));
}
