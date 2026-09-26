//! Bounded retry for read-only commands (AGE-2028).
//!
//! `find`, `who`, `whoami`, `log` and the address lookup behind `inbox`'s display
//! read the bus and change nothing, so one timed-out read is safe to try again
//! rather than a reason to fail the command. Nothing else is: `send` and the
//! inbox pop are not idempotent (a timed-out `send` may have queued its message,
//! a timed-out pop may have removed it) and never come through here.
//!
//! A read that timed out leaves its late reply in flight on that connection, so
//! a retry never reuses it: it dials a fresh one. A command has one
//! [`RetryBudget`] shared by all its reads: two retries in all (after 100 ms and
//! 250 ms) and one [`DEADLINE`] of 5 s, with each attempt armed with at most the
//! CLI's 3 s and at most what is left. So a Redis that accepts and never answers
//! fails `who`, `log`, `whoami` and `find` in about 5 s (measured, see
//! `tests/read_retry.rs`), not a deadline per read. Only a timeout (or `EAGAIN`)
//! is retried; a refused connection is a dead tunnel, and a server-reported
//! error will not go away by asking again.

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use relay_core::Relay;

use crate::BoxError;

/// How long to wait before each retry; one retry per entry, shared by every read
/// a command makes.
pub(crate) const BACKOFF: [Duration; 2] = [Duration::from_millis(100), Duration::from_millis(250)];

/// One command's whole retry allowance: no read starts a retry that could not
/// finish inside it, and no attempt is armed with more than what is left.
pub(crate) const DEADLINE: Duration = Duration::from_secs(5);

/// The longest a single attempt may wait for a reply (the CLI's read timeout).
pub(crate) const ATTEMPT: Duration = Duration::from_secs(3);

/// The least an attempt is armed with, so a spent deadline still lets a read
/// that answers at once succeed.
pub(crate) const MIN_ATTEMPT: Duration = Duration::from_millis(200);

/// Whether `error` is a Redis read that timed out.
pub(crate) fn is_transient(error: &BoxError) -> bool {
    error
        .downcast_ref::<relay_core::Error>()
        .is_some_and(relay_core::Error::is_timeout)
}

/// The retries one command has left, across every read it makes: at most
/// [`BACKOFF`]`.len()` in all, and only while the [`DEADLINE`] holds. A hard-down
/// but accepting Redis therefore fails a command in about [`DEADLINE`] whether it
/// makes one retried read or three, not one deadline per read.
pub(crate) struct RetryBudget<'a> {
    /// Time since the command started.
    elapsed: &'a dyn Fn() -> Duration,
    /// Retries taken so far by the command, shared by all its reads.
    used: &'a Cell<usize>,
}

impl<'a> RetryBudget<'a> {
    /// A budget measured on `elapsed` that counts its retries in `used`.
    pub(crate) fn new(elapsed: &'a dyn Fn() -> Duration, used: &'a Cell<usize>) -> Self {
        Self { elapsed, used }
    }

    /// What is left of the [`DEADLINE`].
    pub(crate) fn remaining(&self) -> Duration {
        DEADLINE.saturating_sub((self.elapsed)())
    }

    /// The timeout to arm the next attempt with.
    pub(crate) fn attempt(&self) -> Duration {
        self.remaining().clamp(MIN_ATTEMPT, ATTEMPT)
    }

    /// Takes one retry and returns its backoff, or `None` when the retries are
    /// used up or the deadline could not fit another attempt after the backoff.
    fn take(&self) -> Option<Duration> {
        let delay = *BACKOFF.get(self.used.get())?;
        if self.remaining() < delay + MIN_ATTEMPT {
            return None;
        }
        self.used.set(self.used.get() + 1);
        Some(delay)
    }
}

/// Runs `op` on `conn`, armed with [`RetryBudget::attempt`]; after a transient
/// failure sleeps the next backoff, replaces `conn` with a fresh one from
/// `redial` (armed likewise) and runs `op` again. Returns the last result once
/// the budget is used up, and any failure of `redial` at once.
pub(crate) fn retry_read<C, T>(
    conn: &mut C,
    budget: &RetryBudget<'_>,
    redial: &mut dyn FnMut(Duration) -> Result<C, BoxError>,
    arm: &mut dyn FnMut(&mut C, Duration),
    sleep: &mut dyn FnMut(Duration),
    op: &mut dyn FnMut(&mut C) -> Result<T, BoxError>,
) -> Result<T, BoxError> {
    loop {
        arm(conn, budget.attempt());
        match op(conn) {
            Err(e) if is_transient(&e) => {
                let Some(delay) = budget.take() else {
                    return Err(e);
                };
                sleep(delay);
                *conn = redial(budget.attempt())?;
            }
            other => return other,
        }
    }
}

/// How to open a fresh connection to the bus the command started with, and the
/// command's retry budget.
pub(crate) struct Redial {
    url: String,
    prefix: String,
    started: Instant,
    used: Cell<usize>,
}

impl Redial {
    /// A dialer for the bus at `url` under `prefix`, for a command starting now.
    pub(crate) fn new(url: &str, prefix: &str) -> Self {
        Self {
            url: url.to_owned(),
            prefix: prefix.to_owned(),
            started: Instant::now(),
            used: Cell::new(0),
        }
    }

    /// Connects, with `timeout` for the connect and every later read.
    pub(crate) fn dial(&self, timeout: Duration) -> relay_core::Result<Relay> {
        Ok(Relay::connect(&self.url, timeout)?.with_prefix(self.prefix.clone()))
    }
}

/// [`retry_read`] for a real connection: sleeps for real, redials through
/// `redial` and shares its retries and deadline with every other read of the
/// command. The closure `op` must only read.
pub(crate) fn read<T>(
    relay: &mut Relay,
    redial: &Redial,
    mut op: impl FnMut(&mut Relay) -> Result<T, BoxError>,
) -> Result<T, BoxError> {
    let elapsed = || redial.started.elapsed();
    let budget = RetryBudget::new(&elapsed, &redial.used);
    retry_read(
        relay,
        &budget,
        &mut |timeout| Ok(redial.dial(timeout)?),
        &mut |conn, timeout| {
            // Failing to arm leaves the timeout it had, which is bounded too.
            let _ = conn.set_timeout(timeout);
        },
        &mut std::thread::sleep,
        &mut op,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn timeout() -> BoxError {
        let io = std::io::Error::from(std::io::ErrorKind::WouldBlock);
        Box::new(relay_core::Error::Redis(redis::RedisError::from(io)))
    }

    fn refused() -> BoxError {
        let io = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        Box::new(relay_core::Error::Redis(redis::RedisError::from(io)))
    }

    /// A command's world on a virtual clock: every attempt that times out costs
    /// the timeout it was armed with, every backoff its length.
    struct World {
        now: Cell<Duration>,
        used: Cell<usize>,
        armed: Cell<Duration>,
        sleeps: RefCell<Vec<Duration>>,
        dials: Cell<u32>,
        /// What a timed-out attempt costs, at most the armed timeout.
        cost: Cell<Duration>,
    }

    impl World {
        fn new() -> Self {
            Self {
                now: Cell::default(),
                used: Cell::default(),
                armed: Cell::new(ATTEMPT),
                sleeps: RefCell::default(),
                dials: Cell::default(),
                cost: Cell::new(Duration::MAX),
            }
        }

        /// One read whose attempts answer as `answers` (oldest first); a
        /// timeout costs the armed timeout.
        fn read(
            &self,
            answers: &mut Vec<Result<&'static str, BoxError>>,
        ) -> Result<&'static str, BoxError> {
            let elapsed = || self.now.get();
            let budget = RetryBudget::new(&elapsed, &self.used);
            let mut conn = 0u32;
            retry_read(
                &mut conn,
                &budget,
                &mut |_| {
                    self.dials.set(self.dials.get() + 1);
                    Ok(self.dials.get())
                },
                &mut |_, timeout| self.armed.set(timeout),
                &mut |d| {
                    self.sleeps.borrow_mut().push(d);
                    self.now.set(self.now.get() + d);
                },
                &mut |_| {
                    let answer = answers.remove(0);
                    if answer.as_ref().is_err_and(is_transient) {
                        self.now
                            .set(self.now.get() + self.armed.get().min(self.cost.get()));
                    }
                    answer
                },
            )
        }
    }

    /// Tracing: TC-039-01
    /// ACs: FR-039-AC-1
    ///
    /// A blip that clears is retried on a fresh connection each time, after the
    /// documented backoff, and the command succeeds.
    #[test]
    fn a_timeout_that_clears_is_retried_on_a_fresh_connection() {
        let w = World::new();
        w.cost.set(Duration::from_millis(500));
        let result = w.read(&mut vec![Err(timeout()), Err(timeout()), Ok("read")]);
        assert_eq!(result.unwrap(), "read");
        assert_eq!(*w.sleeps.borrow(), BACKOFF);
        assert_eq!(
            w.dials.get(),
            2,
            "every retry dials; the timed-out one is not reused"
        );
    }

    /// Tracing: TC-039-01
    /// ACs: FR-039-AC-1
    ///
    /// Nothing but a timeout is retried: a refused connection, a data error and
    /// success all return at once, untouched.
    #[test]
    fn only_a_timeout_is_retried() {
        for answer in [
            Err(refused()),
            Err(Box::new(relay_core::Error::InvalidTicket { ticket: "x".into() }) as BoxError),
            Ok("read"),
        ] {
            let w = World::new();
            let _ = w.read(&mut vec![answer]);
            assert!(w.sleeps.borrow().is_empty());
            assert_eq!(w.dials.get(), 0);
        }
    }

    /// Tracing: TC-039-01
    /// ACs: FR-039-AC-1
    ///
    /// However many reads a command makes, a Redis that accepts and never
    /// answers fails it inside the one deadline (plus the least attempt a spent
    /// deadline still allows for a later read), and the retries are shared.
    #[test]
    fn a_hard_down_redis_fails_a_command_inside_one_deadline_however_many_reads_it_makes() {
        for reads in 1..=3 {
            let w = World::new();
            for _ in 0..reads {
                let result = w.read(&mut vec![Err(timeout()), Err(timeout()), Err(timeout())]);
                assert!(result.is_err_and(|e| is_transient(&e)));
            }
            assert!(
                w.now.get() <= DEADLINE + MIN_ATTEMPT * 3,
                "{reads} reads took {:?}",
                w.now.get()
            );
            assert!(w.used.get() <= BACKOFF.len(), "retries are shared");
        }
    }

    /// A failure to redial ends the retry with that failure, not a hang.
    #[test]
    fn a_failed_redial_ends_the_retry() {
        let elapsed = || Duration::ZERO;
        let used = Cell::new(0);
        let budget = RetryBudget::new(&elapsed, &used);
        let mut conn = 0u32;
        let result = retry_read(
            &mut conn,
            &budget,
            &mut |_| Err(refused()),
            &mut |_, _| {},
            &mut |_| {},
            &mut |_| Err::<(), _>(timeout()),
        );
        assert!(result.is_err_and(|e| !is_transient(&e)));
    }
}
