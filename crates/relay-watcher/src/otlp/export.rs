//! The exporter: a bounded hand-off from the watcher's tick to one dedicated
//! thread that encodes each [`Registry`] and posts it.
//!
//! What this guarantees, and where:
//!
//! - **The tick never waits on the collector.** [`Exporter::submit`] locks a
//!   mutex, pushes to a queue of [`QUEUE_CAPACITY`] snapshots (dropping the
//!   oldest when full) and returns. All network work is on the exporter thread.
//! - **Memory is bounded.** At most [`QUEUE_CAPACITY`] queued snapshots plus the
//!   one being sent, each at most [`MAX_POINTS`](super::registry::MAX_POINTS)
//!   points and [`MAX_REQUEST_BYTES`](super::encode::MAX_REQUEST_BYTES) encoded.
//! - **A bad collector never takes the watcher down.** A failure is a status and
//!   a backoff ([`Backoff`]: 1 s doubling to 5 min); a panic in the send path is
//!   caught and counted as a failure; the thread never returns while the exporter
//!   lives.
//! - **One log line starts an outage and one ends it.** A failure while already
//!   failing logs nothing.
//!
//! [`Sender::send`] takes the time as an argument, so backoff is tested on an
//! injected clock, not by waiting.

use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use relay_core::health::{MAX_OTLP_TEXT_CHARS, OtlpCounters, OtlpStatus};

use super::{
    encode::{Resource, encode},
    http::{Response, Transport},
    registry::Registry,
};

/// Snapshots the queue holds; a push past it drops the oldest.
pub const QUEUE_CAPACITY: usize = 2;
/// The first wait after a failure.
pub const BACKOFF_MIN: Duration = Duration::from_secs(1);
/// The longest wait after failures, and the most a `Retry-After` is honoured for.
pub const BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);

/// A queue of at most `capacity` items that drops its oldest when pushed past it.
pub struct Mailbox<T> {
    state: Mutex<MailboxState<T>>,
    ready: Condvar,
    capacity: usize,
}

struct MailboxState<T> {
    items: VecDeque<T>,
    closed: bool,
}

impl<T> Mailbox<T> {
    /// An empty mailbox of `capacity` (at least 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(MailboxState {
                items: VecDeque::new(),
                closed: false,
            }),
            ready: Condvar::new(),
            capacity: capacity.max(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MailboxState<T>> {
        // The state is a queue and a flag; a panic elsewhere cannot leave it
        // half-updated, so a poisoned lock is still usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `item`. Returns the oldest item if the queue was full and dropped
    /// it. Never blocks beyond the lock. A closed mailbox drops `item` itself.
    pub fn push(&self, item: T) -> Option<T> {
        let mut state = self.lock();
        if state.closed {
            return Some(item);
        }
        let dropped = if state.items.len() >= self.capacity {
            state.items.pop_front()
        } else {
            None
        };
        state.items.push_back(item);
        drop(state);
        self.ready.notify_one();
        dropped
    }

    /// Waits for the next item, oldest first. `None` once the mailbox is closed
    /// and empty.
    pub fn recv(&self) -> Option<T> {
        let mut state = self.lock();
        loop {
            if let Some(item) = state.items.pop_front() {
                return Some(item);
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// How many items are queued.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        self.lock().items.len()
    }

    /// Wakes a waiting [`Mailbox::recv`] for good: it returns what is left, then
    /// `None`.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }
}

/// Counters the exporter keeps, readable from any thread.
#[derive(Debug, Default)]
pub struct Stats {
    /// Pushes the collector accepted.
    pub sent: AtomicU64,
    /// Failed pushes (any reason, retryable or not).
    pub failed: AtomicU64,
    /// Snapshots never sent: pushed out of a full queue, or dequeued while
    /// backing off.
    pub dropped_snapshots: AtomicU64,
    /// Data points a collector said it rejected (`partial_success`).
    pub rejected_points: AtomicU64,
}

impl Stats {
    /// The four counters as the heartbeat carries them.
    #[must_use]
    pub fn counters(&self) -> OtlpCounters {
        OtlpCounters {
            sent: self.sent.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            dropped_snapshots: self.dropped_snapshots.load(Ordering::Relaxed),
            rejected_points: self.rejected_points.load(Ordering::Relaxed),
        }
    }
}

/// How long one send may run before the heartbeat calls the exporter stuck. A
/// request is over within 5 s, but a name lookup has no deadline of its own (std has
/// none), so a hung resolver holds the exporter thread; this makes that visible.
pub const STUCK_AFTER: Duration = Duration::from_secs(30);

struct StatusInner {
    status: OtlpStatus,
    /// When the send now in progress began, if one is.
    in_flight_since: Option<Instant>,
}

/// The exporter's state as the heartbeat reports it, shared with the watcher.
#[derive(Clone)]
pub struct StatusHandle {
    inner: Arc<Mutex<StatusInner>>,
    stats: Arc<Stats>,
}

impl std::fmt::Debug for StatusHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StatusHandle").field(&self.get()).finish()
    }
}

impl StatusHandle {
    /// A handle holding `initial`, reading its counters from `stats`.
    #[must_use]
    pub fn new(initial: OtlpStatus, stats: Arc<Stats>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StatusInner {
                status: initial,
                in_flight_since: None,
            })),
            stats,
        }
    }

    /// No exporter: nothing is sent.
    #[must_use]
    pub fn disabled() -> Self {
        Self::new(OtlpStatus::Disabled, Arc::default())
    }

    /// No exporter, because an inherited setting was ignored: `note` says which.
    #[must_use]
    pub fn ignoring(note: String) -> Self {
        Self::new(OtlpStatus::Ignoring { note }, Arc::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StatusInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The counters this handle reads.
    #[must_use]
    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }

    /// The current state, with live counters, and `failing (stuck)` when a send has
    /// been running longer than [`STUCK_AFTER`].
    #[must_use]
    pub fn get(&self) -> OtlpStatus {
        self.get_at(Instant::now())
    }

    /// [`StatusHandle::get`] at `now`.
    #[must_use]
    pub fn get_at(&self, now: Instant) -> OtlpStatus {
        let inner = self.lock();
        let counters = self.stats.counters();
        let stuck = inner
            .in_flight_since
            .is_some_and(|since| now.saturating_duration_since(since) > STUCK_AFTER);
        match &inner.status {
            OtlpStatus::Sending {
                target, dropped, ..
            } if !stuck => OtlpStatus::Sending {
                target: target.clone(),
                dropped: *dropped,
                counters,
            },
            OtlpStatus::Sending { target, .. } | OtlpStatus::Failing { target, .. } if stuck => {
                OtlpStatus::Failing {
                    target: target.clone(),
                    reason: "stuck".to_owned(),
                    counters,
                }
            }
            OtlpStatus::Failing { target, reason, .. } => OtlpStatus::Failing {
                target: target.clone(),
                reason: reason.clone(),
                counters,
            },
            other => other.clone(),
        }
    }

    fn set(&self, status: OtlpStatus) {
        self.lock().status = status;
    }

    fn begin(&self, now: Instant) {
        self.lock().in_flight_since = Some(now);
    }

    fn end(&self) {
        self.lock().in_flight_since = None;
    }
}

/// Capped exponential backoff with a not-before instant.
#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
    not_before: Option<Instant>,
}

impl Backoff {
    /// The wait after the `n`th consecutive failure (1-based): 1 s, 2 s, 4 s, ...
    /// capped at [`BACKOFF_MAX`].
    #[must_use]
    pub fn delay_after(n: u32) -> Duration {
        let shift = n.saturating_sub(1).min(16);
        BACKOFF_MIN.saturating_mul(1u32 << shift).min(BACKOFF_MAX)
    }

    /// Records a failure at `now` and returns how long to wait: the backoff, or
    /// the collector's `Retry-After` (bounded by [`BACKOFF_MAX`]) when longer.
    pub fn fail(&mut self, now: Instant, retry_after: Option<Duration>) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let mut wait = Self::delay_after(self.failures);
        if let Some(asked) = retry_after {
            wait = wait.max(asked.min(BACKOFF_MAX));
        }
        // Checked: a wait too large to add is capped by BACKOFF_MAX anyway, but the
        // arithmetic never panics.
        self.not_before = Some(now.checked_add(wait).unwrap_or(now));
        wait
    }

    /// Forgets every failure.
    pub fn succeed(&mut self) {
        self.failures = 0;
        self.not_before = None;
    }

    /// The instant before which nothing is sent, if failing.
    #[must_use]
    pub fn not_before(&self) -> Option<Instant> {
        self.not_before
    }
}

/// What one [`Sender::send`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// The collector accepted the request; `rejected` data points were refused
    /// in a `partial_success`.
    Sent {
        /// Points in the request.
        points: u64,
        /// Points the collector said it rejected.
        rejected: u64,
    },
    /// Still backing off: the snapshot was dropped unsent.
    Skipped,
    /// A retryable failure: the next send waits `wait`.
    Backoff {
        /// A stable code, such as `timeout` or `http_503`.
        reason: String,
        /// How long nothing is sent for.
        wait: Duration,
    },
    /// The collector refused this request for good (a 4xx other than 429):
    /// nothing is retried, the next snapshot is tried as usual.
    Refused {
        /// A stable code, such as `http_400`.
        reason: String,
    },
}

/// Encodes and posts one snapshot at a time, with backoff, status and logging.
pub struct Sender {
    resource: Resource,
    target: String,
    status: StatusHandle,
    stats: Arc<Stats>,
    backoff: Backoff,
    in_outage: bool,
    log: Box<dyn FnMut(String) + Send>,
}

fn reason_code(status: u16) -> String {
    format!("http_{status}")
}

/// The `partialSuccess.rejectedDataPoints` of a 2xx body, as far as it parses.
fn rejected_points(body: &[u8]) -> u64 {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return 0;
    };
    let rejected = &value["partialSuccess"]["rejectedDataPoints"];
    rejected
        .as_u64()
        .or_else(|| rejected.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

impl Sender {
    /// A sender for `resource` posting to `target` (`host:port`, shown in the
    /// status and logs).
    pub fn new(
        resource: Resource,
        target: String,
        status: StatusHandle,
        log: Box<dyn FnMut(String) + Send>,
    ) -> Self {
        let stats = Arc::clone(status.stats());
        let target: String = target.chars().take(MAX_OTLP_TEXT_CHARS).collect();
        status.set(OtlpStatus::Sending {
            target: target.clone(),
            dropped: 0,
            counters: OtlpCounters::default(),
        });
        Self {
            resource,
            target,
            status,
            stats,
            backoff: Backoff::default(),
            in_outage: false,
            log,
        }
    }

    fn fail(&mut self, reason: &str) {
        self.stats.failed.fetch_add(1, Ordering::Relaxed);
        self.status.set(OtlpStatus::Failing {
            target: self.target.clone(),
            reason: reason.chars().take(MAX_OTLP_TEXT_CHARS).collect(),
            counters: OtlpCounters::default(),
        });
        if !self.in_outage {
            self.in_outage = true;
            (self.log)(format!(
                "cannot push metrics to {} ({reason}); deliveries are not affected, retrying \
                 with backoff",
                self.target
            ));
        }
    }

    /// Records a send that panicked as a failed push and ends its in-flight mark.
    fn aborted(&mut self) {
        self.status.end();
        self.fail("internal_error");
    }

    /// Sends `registry` at `now`, unless still backing off.
    pub fn send(
        &mut self,
        registry: &Registry,
        now: Instant,
        transport: &mut dyn Transport,
    ) -> SendOutcome {
        if self.backoff.not_before().is_some_and(|until| now < until) {
            self.stats.dropped_snapshots.fetch_add(1, Ordering::Relaxed);
            return SendOutcome::Skipped;
        }
        let encoded = match encode(registry, &self.resource) {
            Ok(encoded) => encoded,
            Err(_) => {
                let wait = self.backoff.fail(now, None);
                self.fail("encode_failed");
                return SendOutcome::Backoff {
                    reason: "encode_failed".to_owned(),
                    wait,
                };
            }
        };
        self.status.begin(now);
        let answer = transport.post(&encoded.body);
        self.status.end();
        match answer {
            Err(e) => {
                let wait = self.backoff.fail(now, None);
                self.fail(e.code());
                SendOutcome::Backoff {
                    reason: e.code().to_owned(),
                    wait,
                }
            }
            Ok(Response {
                status: 200..=299,
                body,
                ..
            }) => {
                self.backoff.succeed();
                let rejected = rejected_points(&body);
                self.stats
                    .rejected_points
                    .fetch_add(rejected, Ordering::Relaxed);
                if encoded.points > 0 && rejected >= encoded.points {
                    // A 2xx that rejected every point delivered nothing: a collector
                    // that cannot use this encoding says so this way.
                    self.fail("rejected_points");
                    return SendOutcome::Refused {
                        reason: "rejected_points".to_owned(),
                    };
                }
                self.stats.sent.fetch_add(1, Ordering::Relaxed);
                self.status.set(OtlpStatus::Sending {
                    target: self.target.clone(),
                    dropped: registry.dropped().saturating_add(encoded.cut),
                    counters: OtlpCounters::default(),
                });
                if std::mem::take(&mut self.in_outage) {
                    (self.log)(format!("pushing metrics to {} works again", self.target));
                }
                SendOutcome::Sent {
                    points: encoded.points,
                    rejected,
                }
            }
            Ok(Response {
                status,
                retry_after_secs,
                ..
            }) => {
                let reason = reason_code(status);
                if matches!(status, 400..=499) && status != 429 {
                    // The request itself is refused; backing off would not help
                    // and the next snapshot may well be accepted.
                    self.fail(&reason);
                    return SendOutcome::Refused { reason };
                }
                // 429, 5xx and a redirect (never followed, and a misconfigured
                // endpoint stays misconfigured): wait, honouring Retry-After.
                let wait = self
                    .backoff
                    .fail(now, retry_after_secs.map(Duration::from_secs));
                self.fail(&reason);
                SendOutcome::Backoff { reason, wait }
            }
        }
    }
}

/// The exporter thread and the queue that feeds it.
pub struct Exporter {
    mailbox: Arc<Mailbox<Registry>>,
    stats: Arc<Stats>,
    thread: Option<JoinHandle<()>>,
}

impl Exporter {
    /// Starts the thread. It sends each queued snapshot through `transport`.
    ///
    /// # Errors
    ///
    /// The OS refusing to start a thread.
    pub fn start(mut sender: Sender, mut transport: Box<dyn Transport>) -> std::io::Result<Self> {
        let stats = Arc::clone(&sender.stats);
        let mailbox = Arc::new(Mailbox::new(QUEUE_CAPACITY));
        let queue = Arc::clone(&mailbox);
        let thread = std::thread::Builder::new()
            .name("otlp-exporter".to_owned())
            .spawn(move || {
                while let Some(registry) = queue.recv() {
                    // A panic in the send path is a failed push, not a dead
                    // exporter and never a dead watcher.
                    let sent = catch_unwind(AssertUnwindSafe(|| {
                        sender.send(&registry, Instant::now(), transport.as_mut())
                    }));
                    if sent.is_err() {
                        sender.aborted();
                    }
                }
            })?;
        Ok(Self {
            mailbox,
            stats,
            thread: Some(thread),
        })
    }

    /// Hands `registry` to the exporter thread and returns at once. When the
    /// queue is full the oldest snapshot is dropped and counted.
    pub fn submit(&self, registry: Registry) {
        if self.mailbox.push(registry).is_some() {
            self.stats.dropped_snapshots.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Snapshots waiting to be sent.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        self.mailbox.queued()
    }

    /// The counters.
    #[cfg(test)]
    pub(crate) fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        self.mailbox.close();
        // Not joined: a send in flight may be waiting on a collector, and the
        // process must be able to exit without waiting for it.
        drop(self.thread.take());
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::mpsc};

    use relay_core::config::otlp::{OtlpEndpoint, OtlpHeaders};

    use super::*;
    use crate::otlp::{
        http::{HttpTransport, Timeouts, TransportError},
        registry::Attrs,
        series::PANE_INFO,
        test_collector::{FakeCollector, Reply},
    };

    fn snap(time: u64) -> Registry {
        snap_n(time, 1)
    }

    /// A snapshot of `n` points.
    fn snap_n(time: u64, n: usize) -> Registry {
        let mut r = Registry::new(time, 1);
        for i in 0..n {
            r.gauge(&PANE_INFO, Attrs::new().with("i", &i.to_string()), 1);
        }
        r
    }

    /// Answers from a script, one entry per post, and counts the posts.
    struct Script {
        answers: VecDeque<Result<Response, TransportError>>,
        posts: Arc<AtomicU64>,
    }

    impl Transport for Script {
        fn post(&mut self, _body: &[u8]) -> Result<Response, TransportError> {
            self.posts.fetch_add(1, Ordering::SeqCst);
            self.answers.pop_front().unwrap_or(Ok(status(200)))
        }
    }

    fn status(code: u16) -> Response {
        Response {
            status: code,
            retry_after_secs: None,
            body: Vec::new(),
        }
    }

    type Lines = Arc<Mutex<Vec<String>>>;

    fn sender() -> (Sender, StatusHandle, Arc<Stats>, Lines) {
        let stats = Arc::new(Stats::default());
        let status = StatusHandle::new(OtlpStatus::Disabled, Arc::clone(&stats));
        let lines: Lines = Arc::default();
        let sink = Arc::clone(&lines);
        let s = Sender::new(
            Resource::new("svc", None, "hub", "0", &[]),
            "collector:4318".into(),
            status.clone(),
            Box::new(move |l| sink.lock().unwrap().push(l)),
        );
        (s, status, stats, lines)
    }

    fn script(answers: Vec<Result<Response, TransportError>>) -> (Script, Arc<AtomicU64>) {
        let posts = Arc::new(AtomicU64::new(0));
        (
            Script {
                answers: answers.into(),
                posts: Arc::clone(&posts),
            },
            posts,
        )
    }

    /// Tracing: TC-045-13
    /// ACs: FR-045-AC-13
    #[test]
    fn tc_045_13_backoff_starts_at_one_second_doubles_and_stops_at_five_minutes() {
        let waits: Vec<u64> = (1..=12)
            .map(|n| Backoff::delay_after(n).as_secs())
            .collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
        assert_eq!(Backoff::delay_after(u32::MAX), BACKOFF_MAX, "no overflow");
        assert_eq!(Backoff::delay_after(0), BACKOFF_MIN);
    }

    /// While backing off nothing is sent, on an injected clock: a snapshot that
    /// arrives early is dropped and counted, one at the not-before instant is sent,
    /// each failure doubles the wait, and a success resets it.
    ///
    /// Tracing: TC-045-13
    /// ACs: FR-045-AC-13
    #[test]
    fn tc_045_13_a_failing_collector_is_left_alone_until_its_backoff_passes() {
        let (mut sender, _status, stats, _lines) = sender();
        let (mut transport, posts) = script(vec![
            Err(TransportError::Connect),
            Err(TransportError::Timeout),
            Ok(status(200)),
            Err(TransportError::Io),
        ]);
        let t0 = Instant::now();
        let sec = Duration::from_secs;
        let s = snap(1);
        let mut send = |at: Duration| sender.send(&s, t0 + at, &mut transport);
        assert_eq!(
            send(sec(0)),
            SendOutcome::Backoff {
                reason: "connect_failed".into(),
                wait: sec(1)
            }
        );
        assert_eq!(send(Duration::from_millis(999)), SendOutcome::Skipped);
        assert_eq!(
            posts.load(Ordering::SeqCst),
            1,
            "nothing was posted while backing off"
        );
        assert_eq!(
            send(sec(1)),
            SendOutcome::Backoff {
                reason: "timeout".into(),
                wait: sec(2)
            }
        );
        assert_eq!(send(sec(2)), SendOutcome::Skipped);
        assert_eq!(
            send(sec(3)),
            SendOutcome::Sent {
                points: 1,
                rejected: 0
            }
        );
        // A success forgot the failures: the next wait is one second again.
        assert_eq!(
            send(sec(4)),
            SendOutcome::Backoff {
                reason: "io_error".into(),
                wait: sec(1)
            }
        );
        assert_eq!(stats.dropped_snapshots.load(Ordering::SeqCst), 2);
        assert_eq!(
            (
                stats.sent.load(Ordering::SeqCst),
                stats.failed.load(Ordering::SeqCst)
            ),
            (1, 3)
        );
    }

    /// `Retry-After` is honoured when it asks for longer than the backoff, and
    /// bounded at five minutes; a shorter one does not shorten the backoff.
    ///
    /// Tracing: TC-045-17
    /// ACs: FR-045-AC-17
    #[test]
    fn tc_045_17_retry_after_is_honoured_and_bounded() {
        let retry = |code: u16, secs: Option<u64>| Response {
            status: code,
            retry_after_secs: secs,
            body: Vec::new(),
        };
        for (code, asked, want) in [
            (429, Some(7), 7),
            (503, Some(90), 90),
            (429, Some(100_000), 300),
            (429, Some(0), 1),
            (503, None, 1),
        ] {
            let (mut sender, ..) = sender();
            let (mut transport, _) = script(vec![Ok(retry(code, asked))]);
            let out = sender.send(&snap(1), Instant::now(), &mut transport);
            assert_eq!(
                out,
                SendOutcome::Backoff {
                    reason: format!("http_{code}"),
                    wait: Duration::from_secs(want)
                },
                "{code} {asked:?}"
            );
        }
    }

    /// A 4xx other than 429 is the collector refusing this request: it is not
    /// retried, does not back off the next snapshot, and is logged once; a 3xx is
    /// not followed and backs off like a server error.
    ///
    /// Tracing: TC-045-17
    /// ACs: FR-045-AC-17
    #[test]
    fn tc_045_17_a_refusal_is_not_retried_and_a_redirect_is_not_followed() {
        let (mut sender, status_handle, _stats, lines) = sender();
        let (mut transport, posts) = script(vec![
            Ok(status(400)),
            Ok(status(400)),
            Ok(status(301)),
            Ok(status(200)),
        ]);
        let t0 = Instant::now();
        assert_eq!(
            sender.send(&snap(1), t0, &mut transport),
            SendOutcome::Refused {
                reason: "http_400".into()
            }
        );
        assert_eq!(
            sender.send(&snap(2), t0, &mut transport),
            SendOutcome::Refused {
                reason: "http_400".into()
            },
            "the next snapshot is tried at once"
        );
        assert_eq!(lines.lock().unwrap().len(), 1, "one line for the episode");
        assert!(
            matches!(status_handle.get(), OtlpStatus::Failing { reason, .. } if reason == "http_400")
        );
        assert!(matches!(
            sender.send(&snap(3), t0, &mut transport),
            SendOutcome::Backoff { reason, .. } if reason == "http_301"
        ));
        assert_eq!(
            posts.load(Ordering::SeqCst),
            3,
            "a redirect is not followed"
        );
    }

    /// One log line starts an outage however many pushes fail, one says it ended,
    /// and a second outage logs again; the status follows.
    ///
    /// Tracing: TC-045-18
    /// ACs: FR-045-AC-18
    #[test]
    fn tc_045_18_one_log_line_starts_an_outage_and_one_ends_it() {
        let (mut sender, status_handle, _stats, lines) = sender();
        let (mut transport, _) = script(vec![
            Err(TransportError::Connect),
            Err(TransportError::Connect),
            Err(TransportError::Connect),
            Ok(status(200)),
            Err(TransportError::Timeout),
        ]);
        let t0 = Instant::now();
        let mut at = Duration::ZERO;
        for _ in 0..5 {
            let _ = sender.send(&snap(1), t0 + at, &mut transport);
            at += Duration::from_secs(600);
            if lines.lock().unwrap().len() == 1 {
                assert!(matches!(status_handle.get(), OtlpStatus::Failing { .. }));
            }
        }
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].contains("cannot push metrics to collector:4318 (connect_failed)"),
            "{lines:?}"
        );
        assert!(lines[1].contains("works again"), "{lines:?}");
        assert!(lines[2].contains("(timeout)"), "{lines:?}");
    }

    /// A 2xx with `partialSuccess.rejectedDataPoints` (a string or a number, as
    /// collectors write it) counts the rejected points and is a success while some
    /// point was accepted; one that rejects every point delivered nothing, so it is a
    /// failure (`rejected_points`) that the status and the log show, with no backoff.
    ///
    /// Tracing: TC-045-18
    /// ACs: FR-045-AC-18
    #[test]
    fn tc_045_18_partial_success_counts_rejected_points_and_a_total_rejection_is_a_failure() {
        for (body, want) in [
            (
                r#"{"partialSuccess":{"rejectedDataPoints":"3","errorMessage":"x"}}"#,
                3,
            ),
            (r#"{"partialSuccess":{"rejectedDataPoints":4}}"#, 4),
            (r#"{"partialSuccess":{}}"#, 0),
            (r#"{}"#, 0),
            ("not json", 0),
            ("", 0),
        ] {
            let (mut sender, _status, stats, _) = sender();
            let (mut transport, _) = script(vec![Ok(Response {
                status: 200,
                retry_after_secs: None,
                body: body.as_bytes().to_vec(),
            })]);
            assert_eq!(
                sender.send(&snap_n(1, 5), Instant::now(), &mut transport),
                SendOutcome::Sent {
                    points: 5,
                    rejected: want
                },
                "{body}"
            );
            assert_eq!(stats.rejected_points.load(Ordering::SeqCst), want);
        }
        let (mut sender, status_handle, stats, lines) = sender();
        let all = Response {
            status: 200,
            retry_after_secs: None,
            body: br#"{"partialSuccess":{"rejectedDataPoints":"5"}}"#.to_vec(),
        };
        let (mut transport, _) = script(vec![Ok(all.clone()), Ok(all)]);
        let t0 = Instant::now();
        assert_eq!(
            sender.send(&snap_n(1, 5), t0, &mut transport),
            SendOutcome::Refused {
                reason: "rejected_points".into()
            }
        );
        assert_eq!(
            sender.send(&snap_n(2, 5), t0, &mut transport),
            SendOutcome::Refused {
                reason: "rejected_points".into()
            },
            "no backoff: the next snapshot is tried at once"
        );
        assert!(
            matches!(status_handle.get(), OtlpStatus::Failing { reason, .. } if reason == "rejected_points")
        );
        assert_eq!(
            stats.sent.load(Ordering::SeqCst),
            0,
            "nothing was delivered"
        );
        assert_eq!(lines.lock().unwrap().len(), 1);
    }

    /// Tracing: TC-045-19
    /// ACs: FR-045-AC-19
    #[test]
    fn tc_045_19_the_queue_drops_its_oldest_and_never_holds_more_than_its_capacity() {
        let m = Mailbox::new(2);
        assert_eq!(m.push(1), None);
        assert_eq!(m.push(2), None);
        assert_eq!(m.push(3), Some(1));
        assert_eq!(m.push(4), Some(2));
        assert_eq!(m.queued(), 2);
        assert_eq!((m.recv(), m.recv()), (Some(3), Some(4)));
        m.close();
        assert_eq!(m.recv(), None);
        assert_eq!(m.push(5), Some(5), "a closed mailbox takes nothing");
    }

    /// A transport that blocks in `post` until the test lets it go, telling the test
    /// when it has entered and what it sent.
    struct Gated {
        entered: mpsc::Sender<()>,
        gate: mpsc::Receiver<()>,
        sent: mpsc::Sender<u64>,
    }

    impl Transport for Gated {
        fn post(&mut self, body: &[u8]) -> Result<Response, TransportError> {
            let _ = self.entered.send(());
            let _ = self.gate.recv();
            let text = String::from_utf8_lossy(body);
            let time = text
                .split(r#""timeUnixNano":""#)
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            let _ = self.sent.send(time);
            Ok(status(200))
        }
    }

    /// A collector that never answers holds the exporter thread and nothing else:
    /// `submit` returns at once however many snapshots arrive, the queue holds at
    /// most its capacity, the oldest are dropped and counted, and the newest are
    /// what is sent once the collector lets go. A queue with no bound fails this.
    ///
    /// Tracing: TC-045-19
    /// ACs: FR-045-AC-19
    #[test]
    fn tc_045_19_a_stuck_collector_holds_the_exporter_thread_and_nothing_else() {
        let (entered_tx, entered) = mpsc::channel();
        let (gate_tx, gate) = mpsc::channel();
        let (sent_tx, sent) = mpsc::channel();
        let (sender, _status, _stats, _lines) = sender();
        let exporter = Exporter::start(
            sender,
            Box::new(Gated {
                entered: entered_tx,
                gate,
                sent: sent_tx,
            }),
        )
        .unwrap();
        exporter.submit(snap(1));
        entered.recv().unwrap(); // the thread is inside `post`, holding snapshot 1
        for time in 2..=6 {
            exporter.submit(snap(time)); // returns at once: the thread is stuck
        }
        assert_eq!(exporter.queued(), QUEUE_CAPACITY);
        assert_eq!(exporter.stats().dropped_snapshots.load(Ordering::SeqCst), 3);
        for _ in 0..3 {
            gate_tx.send(()).unwrap();
        }
        let got: Vec<u64> = (0..3).map(|_| sent.recv().unwrap()).collect();
        assert_eq!(got, [1, 5, 6], "the one in flight, then the newest two");
    }

    /// A collector stub that panics on its first post does not kill the exporter:
    /// the panic is a counted failed push and the next snapshot is sent.
    ///
    /// Tracing: TC-045-19
    /// ACs: FR-045-AC-19
    #[test]
    fn tc_045_19_a_panic_in_the_send_path_is_a_failed_push_not_a_dead_exporter() {
        struct PanicsOnce(Option<mpsc::Sender<()>>, bool);
        impl Transport for PanicsOnce {
            fn post(&mut self, _body: &[u8]) -> Result<Response, TransportError> {
                if !std::mem::replace(&mut self.1, true) {
                    panic!("the transport blew up");
                }
                if let Some(done) = self.0.take() {
                    let _ = done.send(());
                }
                Ok(status(200))
            }
        }
        let (done_tx, done) = mpsc::channel();
        let (sender, _status, stats, lines) = sender();
        let exporter = Exporter::start(sender, Box::new(PanicsOnce(Some(done_tx), false))).unwrap();
        exporter.submit(snap(1));
        exporter.submit(snap(2));
        done.recv().unwrap();
        assert_eq!(stats.failed.load(Ordering::SeqCst), 1);
        assert!(lines.lock().unwrap()[0].contains("internal_error"));
    }

    /// Against a real listener and the real transport: a 500, a 429 with
    /// `Retry-After`, garbage and a connection closed without an answer each back
    /// off, in that order and by those waits, on an injected clock; then a 2xx sends.
    /// A configured header value never reaches a log line or the status.
    ///
    /// Tracing: TC-045-20
    /// ACs: FR-045-AC-20
    #[test]
    fn tc_045_20_a_misbehaving_collector_backs_the_sender_off_and_leaks_no_header() {
        let collector = FakeCollector::start(vec![
            Reply::status(500),
            Reply::with_header(429, "Retry-After: 30"),
            Reply::Raw(b"\x00\x01garbage\r\n\r\n".to_vec()),
            Reply::Raw(Vec::new()),
            Reply::json(200, r#"{"partialSuccess":{"rejectedDataPoints":"2"}}"#),
        ]);
        let mut transport = HttpTransport::new(
            OtlpEndpoint::parse(&collector.url(), true).unwrap(),
            OtlpHeaders::parse("Authorization=Bearer s3cret-token").unwrap(),
            Timeouts::default(),
            "ua".into(),
        );
        let (mut sender, status_handle, _stats, lines) = sender();
        let t0 = Instant::now();
        let mut at = Duration::ZERO;
        let mut outcomes = Vec::new();
        for n in 1..=5 {
            outcomes.push(sender.send(&snap_n(n, 5), t0 + at, &mut transport));
            at += Duration::from_secs(1000);
        }
        let reason = |o: &SendOutcome| match o {
            SendOutcome::Backoff { reason, wait } => (reason.clone(), wait.as_secs()),
            other => panic!("{other:?}"),
        };
        assert_eq!(reason(&outcomes[0]), ("http_500".into(), 1));
        assert_eq!(
            reason(&outcomes[1]),
            ("http_429".into(), 30),
            "Retry-After beats the 2 s backoff"
        );
        assert_eq!(reason(&outcomes[2]), ("bad_response".into(), 4));
        assert_eq!(reason(&outcomes[3]), ("bad_response".into(), 8));
        assert_eq!(
            outcomes[4],
            SendOutcome::Sent {
                points: 5,
                rejected: 2
            }
        );
        let seen = collector.recv();
        assert!(
            seen.head.contains("Authorization: Bearer s3cret-token"),
            "the header is sent"
        );
        let shown = format!("{:?} {:?}", lines.lock().unwrap(), status_handle.get());
        assert!(!shown.contains("s3cret-token"), "{shown}");
    }

    /// The heartbeat sees the exporter's live counters, and a send that has been
    /// running longer than the stuck limit (a hung name lookup, say) reads as
    /// `failing (stuck)` until it ends; a send within the limit does not.
    ///
    /// Tracing: TC-045-21
    /// ACs: FR-045-AC-21
    #[test]
    fn tc_045_21_the_status_shows_live_counters_and_a_stuck_send() {
        let stats = Arc::new(Stats::default());
        let handle = StatusHandle::new(
            OtlpStatus::Sending {
                target: "c:1".into(),
                dropped: 2,
                counters: OtlpCounters::default(),
            },
            Arc::clone(&stats),
        );
        stats.sent.store(4, Ordering::Relaxed);
        stats.dropped_snapshots.store(1, Ordering::Relaxed);
        let t0 = Instant::now();
        let counters = |s: OtlpStatus| match s {
            OtlpStatus::Sending { counters, .. } | OtlpStatus::Failing { counters, .. } => counters,
            other => panic!("{other:?}"),
        };
        assert_eq!(counters(handle.get_at(t0)).sent, 4);
        assert_eq!(counters(handle.get_at(t0)).dropped_snapshots, 1);
        handle.begin(t0);
        assert!(
            matches!(handle.get_at(t0 + STUCK_AFTER), OtlpStatus::Sending { .. }),
            "at the limit"
        );
        let stuck = handle.get_at(t0 + STUCK_AFTER + Duration::from_secs(1));
        assert!(
            matches!(&stuck, OtlpStatus::Failing { target, reason, .. } if target == "c:1" && reason == "stuck"),
            "{stuck:?}"
        );
        assert_eq!(counters(stuck).sent, 4);
        handle.end();
        assert!(matches!(
            handle.get_at(t0 + Duration::from_secs(999)),
            OtlpStatus::Sending { .. }
        ));
        // A disabled handle is never stuck, whatever the clock says.
        let off = StatusHandle::disabled();
        off.begin(t0);
        assert_eq!(
            off.get_at(t0 + Duration::from_secs(999)),
            OtlpStatus::Disabled
        );
    }

    /// A hostile `Retry-After` is bounded, and the time arithmetic on it never panics.
    ///
    /// Tracing: TC-045-13
    /// ACs: FR-045-AC-13
    #[test]
    fn tc_045_13_a_hostile_retry_after_is_bounded_and_never_panics() {
        let mut backoff = Backoff::default();
        let now = Instant::now();
        assert_eq!(backoff.fail(now, Some(Duration::MAX)), BACKOFF_MAX);
        assert!(backoff.not_before().is_some_and(|t| t > now));
        for _ in 0..100 {
            backoff.fail(now, Some(Duration::MAX));
        }
        assert_eq!(backoff.fail(now, None), BACKOFF_MAX);
    }
}
