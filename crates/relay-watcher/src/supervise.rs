//! The watcher's outer loop: connect, tick, wait for work, back off, never exit.
//!
//! The only legitimate exits of `relay-watcher` are a command-line error and a
//! configuration error, both before this loop starts. Redis being down, a
//! dropped tunnel or a failed tick is never a reason to leave: [`Supervisor`]
//! retries forever with capped exponential backoff ([`Backoff`]), tells a
//! human once per outage ([`Supervisor::step`]), and keeps a state file fresh
//! so an offline check can see what the watcher is doing without Redis.
//!
//! After a good poll the loop does not sleep a fixed interval: it asks the
//! [`Bus`] to block until there is work ([`Bus::wait`]): a message was sent, a
//! per-lead retry is due, or the backup sweep is. The wait is on the same
//! connection, so a connection lost while waiting is an unhealthy poll like any
//! other and is retried with the same backoff.
//!
//! Everything the loop touches that is slow or ambient sits behind a seam so
//! it is deterministic under test: [`Bus`] (connect, one poll, one wait),
//! [`Clock`] (monotonic time, wall time, sleep) and [`StateSink`] (the state
//! file). The herdr notification reuses the existing [`Herdr`] seam.

use std::{io, time::Duration};

use crate::herdr::Herdr;

/// First retry delay after a failed poll.
pub const BACKOFF_START: Duration = Duration::from_secs(1);

/// Longest delay between retries of a failing poll.
pub const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// The fewest time between the end of one poll and the start of a poll a
/// message woke, so a burst of sends is looked at once, not once per send. Slept
/// on the [`Clock`], so it is deterministic under test.
pub const MIN_TICK_GAP: Duration = Duration::from_millis(100);

/// How long the watcher must be continuously without a working poll before it
/// tells a human (once per outage).
pub const NOTIFY_AFTER: Duration = Duration::from_secs(60);

/// Capped exponential backoff: 1s, 2s, 4s ... up to [`BACKOFF_CAP`], reset by
/// the next successful poll.
#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    /// The delay before the next retry, and counts one more failure.
    pub fn next_delay(&mut self) -> Duration {
        let factor = 2u32.saturating_pow(self.failures);
        self.failures = self.failures.saturating_add(1);
        BACKOFF_START.saturating_mul(factor).min(BACKOFF_CAP)
    }

    /// Forgets every failure so far: the next delay is [`BACKOFF_START`] again.
    pub fn reset(&mut self) {
        self.failures = 0;
    }

    /// Failures counted since the last reset.
    #[must_use]
    pub fn failures(&self) -> u32 {
        self.failures
    }
}

/// Monotonic time, wall-clock time and sleeping, so the loop can run against a
/// fake clock without ever sleeping.
pub trait Clock {
    /// Time since an arbitrary fixed origin; never goes backwards. Used for
    /// every duration decision.
    fn now(&self) -> Duration;
    /// Unix milliseconds, for the state file only.
    fn wall_ms(&self) -> i64;
    /// Blocks the calling thread for `duration`.
    fn sleep(&self, duration: Duration);
}

/// The real clock: [`std::time::Instant`], [`std::time::SystemTime`] and
/// [`std::thread::sleep`].
pub struct SystemClock {
    origin: std::time::Instant,
}

impl SystemClock {
    /// A clock whose [`Clock::now`] starts at zero now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn wall_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Why a poll runs, which decides how much of it is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// A message was sent somewhere on the bus: look at every local inbox.
    Message,
    /// The backup sweep is due, or the connection is new: look at every local
    /// inbox, whatever woke or did not wake the watcher meanwhile.
    Sweep,
    /// A per-lead retry timer is due: look only at the leads whose timer is.
    Retry,
}

/// What the loop needs from the bus: open a connection, run one poll on it and
/// wait for the next one. Errors are [`relay_core::Error`];
/// [`relay_core::Error::is_connection`] decides whether the connection
/// survives them.
pub trait Bus {
    /// A live connection to the bus.
    type Conn;
    /// Opens a connection.
    ///
    /// # Errors
    ///
    /// Whatever stopped the connection from opening.
    fn connect(&mut self) -> relay_core::Result<Self::Conn>;
    /// Runs one poll on `conn`. A data error for a single lead is handled
    /// inside the poll; an `Err` here fails the whole poll.
    ///
    /// # Errors
    ///
    /// A connection-level failure, or a failure that prevented the poll from
    /// reading the registry at all.
    fn poll(&mut self, conn: &mut Self::Conn, wake: Wake) -> relay_core::Result<()>;
    /// Blocks until there is work and says which kind: a message was sent, a
    /// retry timer is due or the sweep is. Returns at once when one already is.
    ///
    /// # Errors
    ///
    /// A connection-level failure, or the failure of the wait itself.
    fn wait(&mut self, conn: &mut Self::Conn) -> relay_core::Result<Wake>;
}

/// Where the watcher's [`WatcherFile`] goes.
pub trait StateSink {
    /// Replaces the stored state with `file`.
    ///
    /// # Errors
    ///
    /// The I/O error that stopped the write; the caller logs it and carries on.
    fn write(&mut self, file: &WatcherFile) -> io::Result<()>;
}

pub use relay_core::health::{LastError, WatcherFile};

/// Whether the last poll worked, and how long it has not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Health {
    Healthy,
    Unhealthy {
        /// [`Clock::now`] at the first failure of this outage.
        since: Duration,
        /// Whether this outage's one notification attempt has been made.
        notified: bool,
        /// Whether the latest failure was a connection-level one (Redis
        /// unreachable) rather than a failing poll on a working connection
        /// (`WRONGTYPE`, `NOAUTH`, `LOADING` ...); it picks the wording.
        unreachable: bool,
    },
}

/// `text` with control characters removed and cut to a notification-sized
/// length: a server-supplied error message goes into a desktop notification.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// What one [`Supervisor::step`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Whether the poll succeeded.
    pub ok: bool,
    /// How long to sleep before the next step: the next backoff delay after a
    /// failure, none after a success (the loop waits for work instead, see
    /// [`Bus::wait`]).
    pub sleep: Duration,
}

/// Connects, polls, backs off and reports until told to stop.
pub struct Supervisor<B: Bus, C: Clock, H: Herdr, S: StateSink> {
    bus: B,
    clock: C,
    herdr: H,
    sink: S,
    /// Why the next poll runs. A new connection, or a failure, resets it to a
    /// [`Wake::Sweep`]: whatever was missed meanwhile is looked for everywhere.
    wake: Wake,
    /// When the last poll ended on [`Clock::now`], for [`MIN_TICK_GAP`].
    last_poll_end: Option<Duration>,
    /// Whether the last wait failed without losing the connection (a command
    /// refused, say): the outage stays open across the good polls in between,
    /// so it backs off, logs once and notifies like any other outage.
    wait_failed: bool,
    conn: Option<B::Conn>,
    backoff: Backoff,
    health: Health,
    file: WatcherFile,
    sink_failing: bool,
    /// Set when the process is asked to stop (SIGTERM, SIGINT).
    stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl<B: Bus, C: Clock, H: Herdr, S: StateSink> Supervisor<B, C, H, S> {
    /// A supervisor that polls as [`Bus::wait`] says while healthy. `pid` is
    /// recorded in the state file.
    pub fn new(bus: B, clock: C, herdr: H, sink: S, pid: u32) -> Self {
        let file = WatcherFile {
            pid,
            started_at_ms: clock.wall_ms(),
            last_ok_tick_ms: None,
            last_error: None,
        };
        Self {
            bus,
            clock,
            herdr,
            sink,
            wake: Wake::Sweep,
            last_poll_end: None,
            wait_failed: false,
            conn: None,
            backoff: Backoff::default(),
            health: Health::Healthy,
            file,
            sink_failing: false,
            stop: None,
        }
    }

    /// The flag a signal handler sets to stop the loop: while it is set, a call that a
    /// signal interrupted (`EINTR`) is a clean stop, not a failure to log or record.
    #[must_use]
    pub fn stop_on(mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.stop = Some(flag);
        self
    }

    fn stopping(&self) -> bool {
        self.stop
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Connects if there is no connection, then polls once. A connection-level
    /// error drops the connection so the next step reconnects; any other error
    /// keeps it. Either way a failure is an unhealthy poll.
    fn attempt(&mut self) -> relay_core::Result<()> {
        let conn = match self.conn.take() {
            Some(conn) => conn,
            None => {
                let conn = self.bus.connect()?;
                eprintln!("relay-watcher: connected to redis");
                self.wake = Wake::Sweep;
                conn
            }
        };
        let conn = self.conn.insert(conn);
        let result = self.bus.poll(conn, self.wake);
        if let Err(e) = &result {
            // Whatever this poll left undone is looked for again everywhere.
            self.wake = Wake::Sweep;
            if e.is_connection() {
                self.conn = None;
            }
        }
        result
    }

    /// Blocks until the bus has work and records why, for the next poll. A
    /// failure to wait is an unhealthy poll: it is recorded like one, the
    /// connection is dropped if it is what failed, and the next backoff delay
    /// is slept before the loop tries again.
    fn await_work(&mut self) {
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        match self.bus.wait(conn) {
            Ok(wake) => {
                self.wake = wake;
                if self.wait_failed {
                    self.wait_failed = false;
                    self.on_ok();
                }
                if wake == Wake::Message
                    && let Some(end) = self.last_poll_end
                {
                    let gap = MIN_TICK_GAP.saturating_sub(self.clock.now().saturating_sub(end));
                    if !gap.is_zero() {
                        self.clock.sleep(gap);
                    }
                }
            }
            Err(e) => {
                self.wake = Wake::Sweep;
                if e.is_connection() {
                    self.conn = None;
                } else {
                    self.wait_failed = true;
                }
                self.on_failure(&e);
                self.persist();
                self.clock.sleep(self.backoff.next_delay());
            }
        }
    }

    /// One iteration: attempt a poll, update health, notify and persist state.
    /// Returns how long to wait before the next one.
    pub fn step(&mut self) -> Step {
        let step = match self.attempt() {
            Ok(()) => {
                if self.wait_failed {
                    // The poll worked but the wait it follows does not: still one
                    // outage, not a recovery.
                    self.file.last_ok_tick_ms = Some(self.clock.wall_ms());
                } else {
                    self.on_ok();
                }
                Step {
                    ok: true,
                    sleep: Duration::ZERO,
                }
            }
            Err(e) => {
                self.on_failure(&e);
                Step {
                    ok: false,
                    sleep: self.backoff.next_delay(),
                }
            }
        };
        self.last_poll_end = Some(self.clock.now());
        self.persist();
        step
    }

    fn on_ok(&mut self) {
        if let Health::Unhealthy {
            since, unreachable, ..
        } = self.health
        {
            let what = if unreachable {
                "redis is back"
            } else {
                "watcher polling again"
            };
            eprintln!(
                "relay-watcher: {what} after {}s and {} failed poll(s)",
                self.clock.now().saturating_sub(since).as_secs(),
                self.backoff.failures()
            );
        }
        self.health = Health::Healthy;
        self.backoff.reset();
        self.file.last_ok_tick_ms = Some(self.clock.wall_ms());
    }

    fn on_failure(&mut self, error: &relay_core::Error) {
        if self.stopping() && error.is_interrupted() {
            return;
        }
        let now = self.clock.now();
        let unreachable = error.is_connection();
        self.file.last_error = Some(LastError {
            at_ms: self.clock.wall_ms(),
            message: error.to_string(),
        });
        let (since, notified) = match self.health {
            Health::Healthy => {
                if unreachable {
                    eprintln!("relay-watcher: redis unreachable: {error}; retrying with backoff");
                } else {
                    eprintln!("relay-watcher: watcher cannot poll: {error}; retrying with backoff");
                }
                (now, false)
            }
            Health::Unhealthy {
                since, notified, ..
            } => (since, notified),
        };
        let notified = notified || self.notify_if_due(now.saturating_sub(since), error);
        self.health = Health::Unhealthy {
            since,
            notified,
            unreachable,
        };
    }

    /// Fires this outage's single notification once it has lasted
    /// [`NOTIFY_AFTER`]. Returns whether the attempt was made (a failed
    /// notification is logged, not retried: one attempt per outage).
    fn notify_if_due(&self, down_for: Duration, error: &relay_core::Error) -> bool {
        if down_for < NOTIFY_AFTER {
            return false;
        }
        eprintln!(
            "relay-watcher: no working poll for {}s; notifying",
            down_for.as_secs()
        );
        let (title, body) = if error.is_connection() {
            (
                "ix-relay: redis unreachable".to_owned(),
                format!(
                    "The ix-relay watcher has had no working Redis poll for {}s. Queued messages \
                     are not being delivered until it recovers; see `journalctl --user -u \
                     relay-watcher`.",
                    NOTIFY_AFTER.as_secs()
                ),
            )
        } else {
            (
                "ix-relay: watcher cannot poll".to_owned(),
                format!(
                    "watcher cannot poll: {} (for {}s). Queued messages are not being delivered \
                     until it recovers.",
                    printable(&error.to_string()),
                    NOTIFY_AFTER.as_secs()
                ),
            )
        };
        if let Err(e) = self.herdr.notify(&title, &body) {
            eprintln!("relay-watcher: could not show the outage notification: {e}");
        }
        true
    }

    /// Writes the state file, logging only when the write starts or stops
    /// failing.
    fn persist(&mut self) {
        match self.sink.write(&self.file) {
            Ok(()) => {
                if self.sink_failing {
                    eprintln!("relay-watcher: state file writes work again");
                }
                self.sink_failing = false;
            }
            Err(e) => {
                if !self.sink_failing {
                    eprintln!("relay-watcher: cannot write its state file: {e}");
                }
                self.sink_failing = true;
            }
        }
    }

    /// Runs [`Supervisor::step`] between waits for as long as `keep_going`
    /// says so: after a good poll it waits for work ([`Bus::wait`]), after a
    /// failed one it sleeps the backoff. Production passes a flag that a SIGTERM or
    /// SIGINT sets; a test passes a counter.
    pub fn run_until(&mut self, mut keep_going: impl FnMut() -> bool) {
        while keep_going() {
            let step = self.step();
            if self.stopping() {
                break;
            }
            if step.ok {
                self.await_work();
            } else {
                self.clock.sleep(step.sleep);
            }
        }
    }
}

#[cfg(test)]
mod evidence;

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        rc::Rc,
    };

    use proptest::prelude::*;

    use super::*;
    use crate::{
        herdr::{AgentGet, ProcessError},
        test_support::{connection_error, data_error, interrupted_error},
    };
    use relay_core::PaneId;

    /// A clock that only moves when the loop sleeps.
    #[derive(Clone, Default)]
    struct FakeClock(Rc<Cell<Duration>>);

    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            self.0.get()
        }
        fn wall_ms(&self) -> i64 {
            1_000_000 + i64::try_from(self.0.get().as_millis()).unwrap()
        }
        fn sleep(&self, duration: Duration) {
            self.0.set(self.0.get() + duration);
        }
    }

    /// One scripted result of the next connect or poll.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Script {
        ConnectFails,
        PollOk,
        PollConnectionError,
        PollDataError,
    }

    #[test]
    fn the_test_errors_are_classified_as_the_loop_will_see_them() {
        assert!(connection_error().is_connection());
        assert!(!data_error().is_connection());
    }

    /// Plays a script; `Conn` is the number of the connection, so a test can
    /// see whether a step reconnected.
    struct ScriptBus {
        script: VecDeque<Script>,
        connects: Rc<Cell<u32>>,
        polled_on: Rc<RefCell<Vec<u32>>>,
        /// The wake each poll ran for, in order.
        wakes: Rc<RefCell<Vec<Wake>>>,
        /// What the next waits answer, oldest first; a message when empty.
        waits: VecDeque<relay_core::Result<Wake>>,
        /// How many waits were made.
        waited: Rc<Cell<u32>>,
    }

    impl ScriptBus {
        fn new(script: impl IntoIterator<Item = Script>) -> Self {
            Self {
                script: script.into_iter().collect(),
                connects: Rc::default(),
                polled_on: Rc::default(),
                wakes: Rc::default(),
                waits: VecDeque::new(),
                waited: Rc::default(),
            }
        }
    }

    impl Bus for ScriptBus {
        type Conn = u32;

        fn connect(&mut self) -> relay_core::Result<u32> {
            if self.script.front() == Some(&Script::ConnectFails) {
                self.script.pop_front();
                return Err(connection_error());
            }
            self.connects.set(self.connects.get() + 1);
            Ok(self.connects.get())
        }

        fn poll(&mut self, conn: &mut u32, wake: Wake) -> relay_core::Result<()> {
            self.polled_on.borrow_mut().push(*conn);
            self.wakes.borrow_mut().push(wake);
            match self.script.pop_front() {
                None | Some(Script::PollOk) => Ok(()),
                Some(Script::PollDataError) => Err(data_error()),
                Some(Script::PollConnectionError | Script::ConnectFails) => Err(connection_error()),
            }
        }

        fn wait(&mut self, _conn: &mut u32) -> relay_core::Result<Wake> {
            self.waited.set(self.waited.get() + 1);
            self.waits.pop_front().unwrap_or(Ok(Wake::Message))
        }
    }

    #[derive(Clone, Default)]
    struct FakeHerdr {
        notifications: Rc<RefCell<Vec<(String, String)>>>,
        fail: Rc<Cell<bool>>,
    }

    impl Herdr for FakeHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(None)
        }
        fn notify(&self, title: &str, body: &str) -> Result<(), ProcessError> {
            self.notifications
                .borrow_mut()
                .push((title.to_owned(), body.to_owned()));
            if self.fail.get() {
                Err(ProcessError::Timeout {
                    program: "herdr".into(),
                    secs: 1,
                })
            } else {
                Ok(())
            }
        }
    }

    #[derive(Clone, Default)]
    struct RecordingSink(Rc<RefCell<Vec<WatcherFile>>>);

    impl StateSink for RecordingSink {
        fn write(&mut self, file: &WatcherFile) -> io::Result<()> {
            self.0.borrow_mut().push(file.clone());
            Ok(())
        }
    }

    struct Rig {
        sup: Supervisor<ScriptBus, FakeClock, FakeHerdr, RecordingSink>,
        clock: FakeClock,
        herdr: FakeHerdr,
        sink: RecordingSink,
        connects: Rc<Cell<u32>>,
        polled_on: Rc<RefCell<Vec<u32>>>,
        wakes: Rc<RefCell<Vec<Wake>>>,
        waited: Rc<Cell<u32>>,
    }

    fn rig(script: impl IntoIterator<Item = Script>) -> Rig {
        rig_with(ScriptBus::new(script))
    }

    fn rig_with(bus: ScriptBus) -> Rig {
        let (connects, polled_on) = (bus.connects.clone(), bus.polled_on.clone());
        let (wakes, waited) = (bus.wakes.clone(), bus.waited.clone());
        let (clock, herdr, sink) = (
            FakeClock::default(),
            FakeHerdr::default(),
            RecordingSink::default(),
        );
        let sup = Supervisor::new(bus, clock.clone(), herdr.clone(), sink.clone(), 4242);
        Rig {
            sup,
            clock,
            herdr,
            sink,
            connects,
            polled_on,
            wakes,
            waited,
        }
    }

    /// Steps `n` times, waiting or sleeping like `run_until` does; returns
    /// each step.
    fn drive(rig: &mut Rig, n: usize) -> Vec<Step> {
        (0..n)
            .map(|_| {
                let step = rig.sup.step();
                if step.ok {
                    rig.sup.await_work();
                } else {
                    rig.clock.sleep(step.sleep);
                }
                step
            })
            .collect()
    }

    fn script_strategy() -> impl Strategy<Value = Script> {
        prop_oneof![
            Just(Script::ConnectFails),
            Just(Script::PollOk),
            Just(Script::PollConnectionError),
            Just(Script::PollDataError),
        ]
    }

    /// Tracing: TC-N003-M-03 (NFR-003-M-3). Evidence: unit test over a fake clock; the row's method is fault-injection.
    #[test]
    fn backoff_doubles_from_one_second_to_the_cap_and_resets() {
        let mut backoff = Backoff::default();
        let secs: Vec<u64> = (0..8).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
        backoff.reset();
        assert_eq!(backoff.next_delay(), BACKOFF_START);
    }

    proptest! {
        /// The delay never shrinks while failures continue, never exceeds the
        /// cap, and restarts from the first delay after a reset.
        /// Tracing: TC-N003-M-03 (NFR-003-M-3). Evidence: unit test over a fake clock; the row's method is fault-injection.
        #[test]
        fn backoff_is_monotone_up_to_the_cap_and_resets_after_success(
            runs in proptest::collection::vec(0usize..40, 1..8)
        ) {
            let mut backoff = Backoff::default();
            for failures in runs {
                let mut previous = Duration::ZERO;
                for i in 0..failures {
                    let delay = backoff.next_delay();
                    prop_assert!(delay >= previous, "shrank at failure {i}");
                    prop_assert!(delay <= BACKOFF_CAP);
                    if i == 0 {
                        prop_assert_eq!(delay, BACKOFF_START);
                    }
                    previous = delay;
                }
                backoff.reset();
            }
        }

        /// The loop has no exit: however connects and polls fail, stepping
        /// goes on, every retry delay is at most the cap, a good poll is
        /// followed by a wait for work rather than a sleep, and a retry after
        /// a failure is never sooner than the backoff start.
        /// Tracing: TC-N003-M-02 (NFR-003-M-2). Evidence: unit test over a fake clock and bus; the row's method is fault-injection.
        /// Tracing: TC-N003-M-03 (NFR-003-M-3), for the sleep-bound half.
        #[test]
        fn the_loop_keeps_going_and_every_sleep_is_within_the_cap(
            script in proptest::collection::vec(script_strategy(), 0..60),
        ) {
            let steps = script.len() + 5;

            // `run_until` is the production loop. Run for `steps` iterations, it
            // must poll, persist and sleep exactly as `step` plus a sleep does:
            // same elapsed time, one state write per iteration.
            let mut looped = rig(script.clone());
            let mut left = steps;
            looped.sup.run_until(|| {
                let go = left > 0;
                left = left.saturating_sub(1);
                go
            });
            let mut twin = rig(script.clone());
            drive(&mut twin, steps);
            prop_assert_eq!(looped.clock.now(), twin.clock.now());
            prop_assert_eq!(looped.sink.0.borrow().len(), steps);
            prop_assert_eq!(&*looped.polled_on.borrow(), &*twin.polled_on.borrow());
            prop_assert_eq!(looped.waited.get(), twin.waited.get());

            // Every step's sleep is nothing after a success (it waits for work
            // instead: one wait per good poll), and within [start, cap] after a
            // failure.
            let mut stepped = rig(script);
            let mut good = 0;
            for step in drive(&mut stepped, steps) {
                if step.ok {
                    good += 1;
                    prop_assert_eq!(step.sleep, Duration::ZERO);
                } else {
                    prop_assert!(step.sleep >= BACKOFF_START && step.sleep <= BACKOFF_CAP);
                }
            }
            prop_assert_eq!(stepped.waited.get(), good);
        }

        /// A connection-level failure means the next step opens a new
        /// connection; a data error or a good poll keeps the current one.
        /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: unit test; the row's method is fault-injection.
        #[test]
        fn only_connection_errors_and_failed_connects_cost_the_connection(
            script in proptest::collection::vec(script_strategy(), 1..60),
        ) {
            let mut rig = rig(script.clone());
            drive(&mut rig, script.len() + 1);
            // Model: reconnect iff the previous attempt left no connection.
            let mut open = false;
            let mut expected = 0;
            let mut queue: VecDeque<Script> = script.into();
            for _ in 0..queue.len() + 1 {
                if !open {
                    if queue.front() == Some(&Script::ConnectFails) {
                        queue.pop_front();
                        continue;
                    }
                    expected += 1;
                    open = true;
                }
                match queue.pop_front() {
                    None | Some(Script::PollOk | Script::PollDataError) => {}
                    Some(Script::PollConnectionError | Script::ConnectFails) => open = false,
                }
            }
            prop_assert_eq!(rig.connects.get(), expected);
        }

        /// One notification per outage that lasted 60s, and none for shorter
        /// ones: counted from the observed trace of when each poll ran and
        /// whether it worked.
        #[test]
        fn notifications_equal_the_outages_that_reached_sixty_seconds(
            script in proptest::collection::vec(script_strategy(), 0..80),
        ) {
            let mut rig = rig(script.clone());
            let mut trace = Vec::new();
            for _ in 0..script.len() + 3 {
                let at = rig.clock.now();
                let step = rig.sup.step();
                trace.push((at, step.ok));
                if step.ok {
                    rig.sup.await_work();
                } else {
                    rig.clock.sleep(step.sleep);
                }
            }
            let mut expected = 0;
            let mut outage_start: Option<Duration> = None;
            let mut counted = false;
            for (at, ok) in trace {
                if ok {
                    outage_start = None;
                    counted = false;
                } else {
                    let start = *outage_start.get_or_insert(at);
                    if !counted && at - start >= NOTIFY_AFTER {
                        expected += 1;
                        counted = true;
                    }
                }
            }
            prop_assert_eq!(rig.herdr.notifications.borrow().len(), expected);
        }
    }

    /// Tracing: TC-N003-M-02 (NFR-003-M-2). Evidence: unit test over a fake clock and bus; the row's method is fault-injection.
    #[test]
    fn a_long_outage_notifies_once_and_a_second_outage_notifies_again() {
        let down = |n| std::iter::repeat_n(Script::ConnectFails, n);
        let script: Vec<Script> = down(20)
            .chain([Script::PollOk])
            .chain(down(20))
            .chain([Script::PollOk])
            .collect();
        let mut rig = rig(script);
        drive(&mut rig, 42);
        let sent = rig.herdr.notifications.borrow();
        assert_eq!(sent.len(), 2, "one per outage, not one per failed poll");
        assert_eq!(sent[0].0, "ix-relay: redis unreachable");
    }

    #[test]
    fn a_failing_poll_on_a_working_connection_is_not_called_unreachable() {
        let mut rig = rig(std::iter::repeat_n(Script::PollDataError, 30));
        drive(&mut rig, 30);
        let sent = rig.herdr.notifications.borrow();
        assert_eq!(sent.len(), 1, "still one notification per outage");
        assert_eq!(sent[0].0, "ix-relay: watcher cannot poll");
        assert!(
            sent[0].1.starts_with("watcher cannot poll: "),
            "{}",
            sent[0].1
        );
        assert_eq!(rig.connects.get(), 1, "a data error keeps the connection");
    }

    #[test]
    fn a_short_outage_never_notifies() {
        let script = std::iter::repeat_n(Script::ConnectFails, 3).chain([Script::PollOk]);
        let mut rig = rig(script);
        drive(&mut rig, 6);
        assert!(rig.herdr.notifications.borrow().is_empty());
    }

    #[test]
    fn a_failing_notification_is_attempted_once_per_outage() {
        let mut rig = rig(std::iter::repeat_n(Script::ConnectFails, 30));
        rig.herdr.fail.set(true);
        drive(&mut rig, 30);
        assert_eq!(rig.herdr.notifications.borrow().len(), 1);
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: unit test; the row's method is fault-injection.
    #[test]
    fn a_connection_error_reconnects_and_a_data_error_does_not() {
        let mut rig = rig([Script::PollDataError, Script::PollConnectionError]);
        drive(&mut rig, 3);
        assert_eq!(*rig.polled_on.borrow(), [1, 1, 2]);
        assert_eq!(rig.connects.get(), 2);
    }

    /// Tracing: TC-038-07
    /// ACs: FR-038-AC-7
    ///
    /// The poll after a wait runs for the reason the wait gave, and every new
    /// connection starts with a sweep.
    #[test]
    fn the_next_poll_runs_for_the_reason_the_wait_gave() {
        let mut bus = ScriptBus::new([Script::PollOk, Script::PollOk, Script::PollOk]);
        bus.waits = [Ok(Wake::Retry), Ok(Wake::Message)].into();
        let mut rig = rig_with(bus);
        drive(&mut rig, 3);
        assert_eq!(
            *rig.wakes.borrow(),
            [Wake::Sweep, Wake::Retry, Wake::Message]
        );
    }

    /// Tracing: TC-038-07
    /// ACs: FR-038-AC-7
    ///
    /// A connection lost while waiting is an unhealthy poll: recorded, backed
    /// off, reconnected, and the new connection sweeps. A data error while
    /// waiting keeps the connection.
    #[test]
    fn a_failed_wait_is_an_unhealthy_poll_that_reconnects_only_when_the_connection_died() {
        let mut bus = ScriptBus::new([Script::PollOk, Script::PollOk, Script::PollOk]);
        bus.waits = [Err(connection_error()), Err(data_error())].into();
        let mut rig = rig_with(bus);
        drive(&mut rig, 3);
        assert_eq!(rig.connects.get(), 2, "only the first failure dropped it");
        assert_eq!(
            *rig.wakes.borrow(),
            [Wake::Sweep, Wake::Sweep, Wake::Sweep],
            "a failed wait is followed by a sweep"
        );
        let written = rig.sink.0.borrow();
        assert!(
            written.iter().any(|f| f.last_error.is_some()),
            "the failed wait is recorded in the state file"
        );
        assert_eq!(
            rig.clock.now(),
            2 * BACKOFF_START + MIN_TICK_GAP,
            "each failed wait slept a backoff delay (the good poll between reset it), and the last message wake its debounce"
        );
    }

    /// Tracing: TC-038-07
    /// ACs: FR-038-AC-7
    ///
    /// A wait that keeps failing without losing the connection is one outage:
    /// the good polls between the failures do not reset the backoff or the
    /// health, so the delays grow to the cap and the one notification fires.
    #[test]
    fn a_persistent_wait_failure_is_one_outage_that_backs_off_and_notifies_once() {
        let mut bus = ScriptBus::new(std::iter::repeat_n(Script::PollOk, 12));
        bus.waits = (0..12).map(|_| Err(data_error())).collect();
        let mut rig = rig_with(bus);
        drive(&mut rig, 12);
        assert_eq!(rig.herdr.notifications.borrow().len(), 1);
        assert!(
            rig.clock.now() > NOTIFY_AFTER,
            "the delays grew: {:?}",
            rig.clock.now()
        );
        assert_eq!(rig.connects.get(), 1, "a data error keeps the connection");
        // The first good wait ends the outage.
        rig.sup.step();
        rig.sup.await_work();
        assert!(rig.sup.file.last_ok_tick_ms.is_some());
    }

    /// Tracing: TC-038-10
    /// ACs: FR-038-AC-10
    ///
    /// A message wake right after a poll waits out the rest of the minimum gap on
    /// the clock; a retry or sweep wake does not.
    #[test]
    fn a_message_wake_is_debounced_on_the_clock_and_others_are_not() {
        let mut bus = ScriptBus::new([Script::PollOk, Script::PollOk]);
        bus.waits = [Ok(Wake::Message), Ok(Wake::Retry)].into();
        let mut rig = rig_with(bus);
        rig.sup.step();
        rig.sup.await_work();
        assert_eq!(rig.clock.now(), MIN_TICK_GAP);
        rig.sup.step();
        rig.sup.await_work();
        assert_eq!(rig.clock.now(), MIN_TICK_GAP, "a retry wake sleeps nothing");
    }

    #[test]
    fn the_state_file_tracks_the_last_ok_tick_and_keeps_the_last_error() {
        let mut rig = rig([Script::PollOk, Script::ConnectFails, Script::PollOk]);
        drive(&mut rig, 3);
        let written = rig.sink.0.borrow();
        assert_eq!(written.len(), 3, "rewritten on every step");
        let [first, failed, recovered] = &written[..] else {
            panic!("expected three writes");
        };
        assert_eq!(first.pid, 4242);
        assert_eq!(first.started_at_ms, 1_000_000);
        assert_eq!(first.last_ok_tick_ms, Some(1_000_000));
        assert!(first.last_error.is_none());
        assert_eq!(failed.last_ok_tick_ms, first.last_ok_tick_ms);
        let error = failed.last_error.as_ref().expect("the failure is recorded");
        assert_eq!(error.at_ms, 1_000_100, "after the first poll's debounce");
        assert!(!error.message.is_empty());
        assert_eq!(recovered.last_ok_tick_ms, Some(1_001_100));
        assert_eq!(
            recovered.last_error, failed.last_error,
            "kept after recovery"
        );
        assert_eq!(recovered.started_at_ms, first.started_at_ms);
    }

    /// Tracing: TC-044-08
    /// ACs: FR-044-AC-8
    ///
    /// A stop by SIGTERM interrupts the blocked wait (`EINTR`): with the stop flag set
    /// that is a clean stop, neither logged nor recorded as the watcher's last error;
    /// without it the same error is the failure it looks like.
    #[test]
    fn tc_044_08_an_interrupted_wait_while_stopping_is_not_a_recorded_failure() {
        let run = |stopping: bool| {
            let mut bus = ScriptBus::new([Script::PollOk]);
            bus.waits = VecDeque::from([Err(interrupted_error())]);
            let sink = RecordingSink::default();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(stopping));
            let mut sup = Supervisor::new(
                bus,
                FakeClock::default(),
                FakeHerdr::default(),
                sink.clone(),
                4242,
            )
            .stop_on(stop);
            assert!(sup.step().ok);
            sup.await_work();
            sup.persist();
            sink.0.borrow().last().and_then(|f| f.last_error.clone())
        };
        assert!(run(true).is_none(), "a stop is not a failure");
        assert!(run(false).is_some(), "the same error, unrequested, is one");
    }
}
