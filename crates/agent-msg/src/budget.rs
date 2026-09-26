//! Time for one hook call: a clock behind a trait, and a deadline measured on it.
//!
//! Every wait a hook makes (a Redis timeout, a herdr call) is bounded by what
//! is left of one deadline. Reading time through [`Clock`] instead of
//! `Instant::now()` lets tests drive the deadline deterministically.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A source of elapsed time and wall-clock time.
pub(crate) trait Clock {
    /// Time since this clock started. Monotonic.
    fn elapsed(&self) -> Duration;
    /// Wall-clock Unix milliseconds, for timestamps written to disk.
    fn now_ms(&self) -> i64;
}

/// The real clock: [`Instant`] for durations, [`SystemTime`] for timestamps.
#[derive(Debug)]
pub(crate) struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Starts counting now.
    pub(crate) fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// One deadline on a [`Clock`].
pub(crate) struct Budget<'a> {
    clock: &'a dyn Clock,
    deadline: Duration,
}

impl<'a> Budget<'a> {
    /// A deadline `total` after the clock's current reading. Production code
    /// counts every deadline from the start of the call ([`Budget::starting_at`]);
    /// this is for tests that start a budget now.
    #[cfg(test)]
    pub(crate) fn new(clock: &'a dyn Clock, total: Duration) -> Self {
        Self {
            clock,
            deadline: clock.elapsed().saturating_add(total),
        }
    }

    /// A deadline `total` after `start`, a reading the clock already took: the
    /// budget of a call that began before this point.
    pub(crate) fn starting_at(clock: &'a dyn Clock, start: Duration, total: Duration) -> Self {
        Self {
            clock,
            deadline: start.saturating_add(total),
        }
    }

    /// What is left, or `None` once it is spent. Never a zero duration: some
    /// platforms treat a zero timeout as "block forever" rather than
    /// "expired", so callers get `None` and stop instead.
    pub(crate) fn remaining(&self) -> Option<Duration> {
        let left = self.deadline.saturating_sub(self.clock.elapsed());
        (!left.is_zero()).then_some(left)
    }

    /// The clock's current reading, for callers that track progress against it.
    pub(crate) fn now(&self) -> Duration {
        self.clock.elapsed()
    }

    /// What is left at `now` of the lesser of this deadline and an idle `window`
    /// that began at `last_progress`, or `None` once either is spent. Same
    /// never-zero rule as [`Budget::remaining`].
    pub(crate) fn left_within_idle(
        &self,
        now: Duration,
        last_progress: Duration,
        window: Duration,
    ) -> Option<Duration> {
        let overall = self.deadline.saturating_sub(now);
        let idle = last_progress.saturating_add(window).saturating_sub(now);
        let left = overall.min(idle);
        (!left.is_zero()).then_some(left)
    }
}

/// A clock tests move by hand, for deterministic deadlines.
#[cfg(test)]
pub(crate) mod testing {
    use std::{cell::Cell, time::Duration};

    use super::Clock;

    /// Starts at zero and only moves when [`FakeClock::advance`] says so.
    #[derive(Debug, Default)]
    pub(crate) struct FakeClock {
        elapsed: Cell<Duration>,
    }

    impl FakeClock {
        /// Moves time forward by `by`.
        pub(crate) fn advance(&self, by: Duration) {
            self.elapsed.set(self.elapsed.get() + by);
        }
    }

    /// A clock that moves `step` on every reading, so a wait that polls it is
    /// finished after a fixed number of polls instead of real time.
    #[derive(Debug)]
    pub(crate) struct TickingClock {
        elapsed: Cell<Duration>,
        step: Duration,
    }

    impl TickingClock {
        /// Advances by `step` each time it is read.
        pub(crate) fn new(step: Duration) -> Self {
            Self {
                elapsed: Cell::new(Duration::ZERO),
                step,
            }
        }
    }

    impl Clock for TickingClock {
        fn elapsed(&self) -> Duration {
            let now = self.elapsed.get();
            self.elapsed.set(now + self.step);
            now
        }

        fn now_ms(&self) -> i64 {
            1_700_000_000_000
        }
    }

    impl Clock for FakeClock {
        fn elapsed(&self) -> Duration {
            self.elapsed.get()
        }

        fn now_ms(&self) -> i64 {
            1_700_000_000_000 + i64::try_from(self.elapsed.get().as_millis()).unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::FakeClock, *};

    #[test]
    fn remaining_counts_down_and_is_none_once_spent() {
        let clock = FakeClock::default();
        let budget = Budget::new(&clock, Duration::from_millis(300));
        assert_eq!(budget.remaining(), Some(Duration::from_millis(300)));
        clock.advance(Duration::from_millis(100));
        assert_eq!(budget.remaining(), Some(Duration::from_millis(200)));
        clock.advance(Duration::from_millis(200));
        assert_eq!(budget.remaining(), None, "exactly spent is spent");
        clock.advance(Duration::from_millis(50));
        assert_eq!(budget.remaining(), None);
    }

    #[test]
    fn a_budget_is_measured_from_when_it_was_made() {
        let clock = FakeClock::default();
        clock.advance(Duration::from_secs(5));
        let budget = Budget::new(&clock, Duration::from_millis(300));
        assert_eq!(budget.remaining(), Some(Duration::from_millis(300)));
    }
}
