// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Background Linear refresh for the read-only team-leader dashboard.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::monitor_tracker::TrackerState;
use ix_board::{LinearGraphQlSource, ReadService, RefreshOutcome, quota::QuotaDecision};
use ix_cli_kit::streams::{DiagnosticFields, Record};

pub(crate) mod quota;

use quota::{Cause, Notice};

/// Compact source classification suitable for a single diagnostic line.
pub(crate) fn source_summary(error: &ix_board::Error) -> &'static str {
    match error {
        ix_board::Error::MissingApiKey => "Authentication unavailable; ix-board auth login",
        ix_board::Error::Credential(_) => {
            "Saved credential access denied; contact credential owner"
        }
        _ => Cause::of_error(error).map_or(
            "Structured tracker read failed; inspect source availability",
            Cause::summary,
        ),
    }
}

/// Safe, actionable presentation of the approved SDK's typed authentication errors.
pub(crate) fn source_diagnostic(error: &ix_board::Error) -> String {
    let (kind, message, remedy) = match error {
        ix_board::Error::MissingApiKey => (
            "MissingAuthentication",
            "Linear authentication is unavailable",
            "Configure an approved saved credential with ix-board auth login, or open --report",
        ),
        ix_board::Error::Credential(_) => (
            "CredentialAccessDenied",
            "Linear saved credential could not be accessed",
            "Ask the credential owner to restore approved access, or open --report",
        ),
        _ => (
            "TrackerReadFailed",
            "Structured tracker read failed",
            SOURCE_AVAILABILITY_REMEDY,
        ),
    };
    render_diagnostic(kind, message, remedy)
}

/// What to do when the tracker source cannot be read or refreshed.
const SOURCE_AVAILABILITY_REMEDY: &str =
    "Inspect source availability with ix-board, or open --report";

fn render_diagnostic(kind: &str, message: &str, remedy: &str) -> String {
    let record = Record::error(kind, message).with_fields(DiagnosticFields {
        reason: Some(kind),
        remedy: Some(remedy),
        ..DiagnosticFields::default()
    });
    format!("{} · {}", record.render_human(false), remedy)
}

/// Evidence for a snapshot whose source read time and refresh outcome are both unknown.
pub(crate) const FRESHNESS_UNCONFIRMED_DETAIL: &str =
    "showing the last snapshot; its source read time is unconfirmed";

/// Automatic Linear tracker reads run every 30 minutes.
pub const TRACKER_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// Limit retries after Linear or its cached workspace read returns an error.
const TRACKER_MAX_RETRY_INTERVAL: Duration = TRACKER_REFRESH_INTERVAL;

/// One successful tracker read: the snapshot and how the shared read service obtained it.
///
/// After a failed refresh the service returns its last complete snapshot as a success and
/// records the failure only in `outcome` and `freshness`; the snapshot keeps the time its
/// source read began.
pub struct TrackerRead {
    /// The snapshot, with its original source timestamp in `fetched_at`.
    pub snapshot: TrackerState,
    /// Provenance of this read as the service reports it; `None` when it reports none.
    pub freshness: Option<ix_board::CacheFreshness>,
    /// Typed outcome of the attempt that produced this read, captured with the snapshot.
    /// `Unknown` when the service reports none; it is never rebuilt from failure text.
    pub outcome: RefreshOutcome,
}

/// One failed tracker read: the SDK error and the typed outcome of the attempt that raised it.
///
/// The service records its outcome before it returns the error, so an actual server backoff or
/// a refused refresh lock keeps its whole typed decision here even though the error with no
/// stored snapshot is the SDK's legacy rate-limit or lock error. An error raised before a
/// service exists, such as a missing credential, carries the `Unknown` outcome.
#[derive(Debug)]
pub struct TrackerFailure {
    /// The error the read returned; authentication errors keep their redacted handling.
    pub error: ix_board::Error,
    /// Typed outcome of the attempt, `Unknown` when no service ran. Boxed because a quota
    /// refusal carries a whole decision, which would make every `Err` of a read large.
    pub outcome: Box<RefreshOutcome>,
}

impl From<ix_board::Error> for TrackerFailure {
    fn from(error: ix_board::Error) -> Self {
        Self {
            error,
            outcome: Box::new(RefreshOutcome::Unknown),
        }
    }
}

impl TrackerFailure {
    /// The quota cause: the typed outcome decides first, then the SDK's legacy typed errors.
    /// `None` when the failure is not a quota matter and keeps the generic handling.
    pub(crate) fn cause(&self) -> Option<Cause> {
        match &*self.outcome {
            RefreshOutcome::QuotaRefused { decision } => Some(Cause::of_decision(decision)),
            RefreshOutcome::RefreshLockRefused => Some(Cause::RefreshLockBusy),
            RefreshOutcome::Unknown
            | RefreshOutcome::CacheReused
            | RefreshOutcome::Refreshed
            | RefreshOutcome::SourceFailed => Cause::of_error(&self.error),
        }
    }

    /// The decision the service captured for the attempt, from the typed outcome first and
    /// from an error that carries its own decision second.
    pub(crate) fn decision(&self) -> Option<&QuotaDecision> {
        match &*self.outcome {
            RefreshOutcome::QuotaRefused { decision } => Some(decision),
            RefreshOutcome::Unknown
            | RefreshOutcome::CacheReused
            | RefreshOutcome::Refreshed
            | RefreshOutcome::SourceFailed
            | RefreshOutcome::RefreshLockRefused => self.error.quota_decision(),
        }
    }
}

/// What a successful read says about the freshness of its snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Provenance {
    /// Nothing marks the snapshot stale.
    Current,
    /// The latest refresh did not succeed and the service kept its last complete snapshot.
    RefreshFailed {
        /// The typed cause of the failed attempt.
        cause: Cause,
        /// The primary statement and every reported fact.
        notice: Notice,
    },
    /// The service marks the snapshot stale but reports no typed failure, no source read age
    /// and no cooldown. That is also what it reports for a successful read of a snapshot
    /// stored without read-start evidence, so no failure may be claimed.
    Unconfirmed,
}

impl Provenance {
    /// The cause of a failed refresh; `None` for a snapshot that is current or unconfirmed.
    pub(crate) fn cause(&self) -> Option<Cause> {
        match self {
            Self::RefreshFailed { cause, .. } => Some(*cause),
            Self::Current | Self::Unconfirmed => None,
        }
    }

    fn failed(cause: Cause, decision: Option<&QuotaDecision>) -> Self {
        Self::RefreshFailed {
            cause,
            notice: Notice::new(cause, decision, true),
        }
    }
}

/// Closed state of the tracker notice the dashboard holds for the latest attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackerStatus {
    /// Nothing marks the snapshot or the attempt.
    #[default]
    Clear,
    /// A snapshot is shown, but neither its source read time nor a refresh outcome is
    /// confirmed; it is not certified fresh and no failure is claimed.
    Unconfirmed,
    /// The latest attempt did not produce a fresh snapshot, for this typed cause.
    Failed(Cause),
}

impl TrackerRead {
    /// A read for which the service reported no provenance and no typed outcome.
    #[cfg(test)]
    pub(crate) fn unreported(snapshot: TrackerState) -> Self {
        Self {
            snapshot,
            freshness: None,
            outcome: RefreshOutcome::Unknown,
        }
    }

    /// Classifies the attempt from the service's typed fields alone.
    ///
    /// A typed failed outcome decides first. A read the service says succeeded
    /// (`CacheReused`, `Refreshed`) claims no failure, and a stale mark on it only leaves it
    /// unconfirmed. Without a typed outcome the legacy typed freshness decides: a stale
    /// snapshot with a reported cooldown follows an actual server rate limit, one with a
    /// known source read age follows a failed refresh of unknown cause, and one with neither
    /// is unconfirmed.
    pub(crate) fn provenance(&self) -> Provenance {
        let stale = self
            .freshness
            .as_ref()
            .is_some_and(|freshness| freshness.stale);
        match &self.outcome {
            RefreshOutcome::QuotaRefused { decision } => {
                Provenance::failed(Cause::of_decision(decision), Some(decision))
            }
            RefreshOutcome::RefreshLockRefused => Provenance::failed(Cause::RefreshLockBusy, None),
            RefreshOutcome::SourceFailed => Provenance::failed(Cause::SourceFailed, None),
            RefreshOutcome::CacheReused | RefreshOutcome::Refreshed if stale => {
                Provenance::Unconfirmed
            }
            RefreshOutcome::CacheReused | RefreshOutcome::Refreshed => Provenance::Current,
            RefreshOutcome::Unknown => match &self.freshness {
                Some(freshness) if freshness.stale => {
                    if freshness.rate_limit_until.is_some() {
                        Provenance::failed(Cause::ServerBackoff, None)
                    } else if freshness.age_seconds.is_some() {
                        Provenance::failed(Cause::Unknown, None)
                    } else {
                        Provenance::Unconfirmed
                    }
                }
                Some(_) | None => Provenance::Current,
            },
        }
    }
}

/// Independent Linear snapshot refresh worker.
pub struct TrackerWorker {
    /// Latest available Linear refresh results.
    pub events: Receiver<Result<TrackerRead, TrackerFailure>>,
    commands: SyncSender<TrackerCommand>,
    stop_requested: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

enum TrackerCommand {
    Refresh,
    Stop,
}

impl TrackerWorker {
    /// Starts Linear refreshes away from the terminal input thread.
    pub fn spawn() -> std::io::Result<Self> {
        Self::spawn_with_read(read_tracker)
    }

    pub(crate) fn spawn_with_read(
        read: impl FnMut(bool) -> Result<TrackerRead, TrackerFailure> + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_with_read_and_intervals(
            read,
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        )
    }

    fn spawn_with_read_and_intervals(
        mut read: impl FnMut(bool) -> Result<TrackerRead, TrackerFailure> + Send + 'static,
        refresh_interval: Duration,
        max_retry_interval: Duration,
    ) -> std::io::Result<Self> {
        let (event_tx, events) = mpsc::sync_channel(1);
        let (commands, command_rx) = mpsc::sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop_requested = Arc::clone(&stop_requested);
        let handle = thread::Builder::new()
            .name("team-leader-dashboard-linear-refresh".to_owned())
            .spawn(move || {
                let mut retry_interval = refresh_interval;
                let mut force = false;
                loop {
                    if worker_stop_requested.load(Ordering::Acquire) {
                        break;
                    }
                    let result = read(force);
                    retry_interval = next_interval(
                        result.is_ok(),
                        retry_interval,
                        refresh_interval,
                        max_retry_interval,
                    );
                    let mut wait =
                        RefreshWait::after_read(&result, refresh_interval, retry_interval);
                    if event_tx.send(result).is_err() {
                        break;
                    }
                    if worker_stop_requested.load(Ordering::Acquire) {
                        break;
                    }
                    let started = Instant::now();
                    loop {
                        if worker_stop_requested.load(Ordering::Acquire) {
                            return;
                        }
                        if started.elapsed() >= wait.delay {
                            force = wait.force;
                            break;
                        }
                        let command =
                            command_rx.recv_timeout(wait.delay.saturating_sub(started.elapsed()));
                        match wait.receive(command) {
                            WaitOutcome::Continue => continue,
                            WaitOutcome::Read { force: requested } => {
                                force = requested;
                                break;
                            }
                            WaitOutcome::Stop => return,
                        }
                    }
                }
            })?;
        Ok(Self {
            events,
            commands,
            stop_requested,
            handle,
        })
    }

    /// Requests a read without blocking the terminal input thread.
    pub fn refresh_now(&self) {
        let _ = self.commands.try_send(TrackerCommand::Refresh);
    }

    /// Stops future refreshes without waiting for an in-flight Linear request.
    pub fn stop(self) {
        self.stop_requested.store(true, Ordering::Release);
        let _ = self.commands.try_send(TrackerCommand::Stop);
        // ix-board paginates a complete workspace snapshot and the blocking
        // HTTP client can spend up to 30 seconds per page. Detach the worker
        // during shutdown; it observes the stop flag and exits after the
        // current read returns, while the dashboard can quit immediately.
        drop(self.handle);
    }
}

/// Manual refreshes during rate limiting coalesce until the next scheduled read.
struct RefreshWait {
    delay: Duration,
    protected: bool,
    force: bool,
}

enum WaitOutcome {
    Continue,
    Read { force: bool },
    Stop,
}

impl RefreshWait {
    fn after_read(
        result: &Result<TrackerRead, TrackerFailure>,
        refresh: Duration,
        retry: Duration,
    ) -> Self {
        // Linear refills quota continuously; its full-window reset is not a
        // mandatory pause. Reattempt on the explicit dashboard polling cadence.
        // Only a typed hold to wait out protects the attempt, and no new cadence or probe
        // follows from a decision's reevaluation time: it does not grant eligibility. A kept
        // snapshot counts only when the service typed its cause; none is otherwise assumed.
        let protected = match result {
            Err(failure) => failure.cause().is_some_and(Cause::waits),
            Ok(read) => read.provenance().cause().is_some_and(Cause::waits),
        };
        Self {
            delay: if protected { refresh } else { retry },
            protected,
            force: false,
        }
    }

    fn receive(&mut self, command: Result<TrackerCommand, mpsc::RecvTimeoutError>) -> WaitOutcome {
        match command {
            Ok(TrackerCommand::Refresh) => {
                self.force = true;
                if self.protected {
                    WaitOutcome::Continue
                } else {
                    WaitOutcome::Read { force: true }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => WaitOutcome::Read { force: self.force },
            Ok(TrackerCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                WaitOutcome::Stop
            }
        }
    }
}

fn next_interval(success: bool, previous: Duration, refresh: Duration, cap: Duration) -> Duration {
    if success {
        refresh
    } else {
        previous.saturating_mul(2).min(cap.max(refresh))
    }
}

fn read_tracker(force: bool) -> Result<TrackerRead, TrackerFailure> {
    read_tracker_with_factory(force, || {
        ReadService::open_default(LinearGraphQlSource::from_env()?)
    })
}

/// One read through the shared service. The outcome is taken after the read, whether it
/// returned a snapshot or an error, before the service is dropped; an error from the factory
/// has no service and so no outcome.
fn read_tracker_with_factory<S: ix_board::WorkspaceSource>(
    force: bool,
    factory: impl FnOnce() -> Result<ReadService<S>, ix_board::Error>,
) -> Result<TrackerRead, TrackerFailure> {
    let mut service = factory()?;
    let read = if force {
        service.force_refresh_workspace()
    } else {
        service.workspace_snapshot()
    };
    let outcome = service.refresh_outcome().clone();
    match read {
        Ok(snapshot) => Ok(TrackerRead {
            snapshot,
            freshness: service.cache_freshness().cloned(),
            outcome,
        }),
        Err(error) => Err(TrackerFailure {
            error,
            outcome: Box::new(outcome),
        }),
    }
}

#[cfg(test)]
mod tests {
    use ix_board::quota::QuotaState;
    use ix_trace_rs::trace;
    use std::{sync::mpsc, thread, time::Duration};

    use super::quota::Action;
    use super::*;

    struct FixtureSource {
        reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl ix_board::WorkspaceSource for FixtureSource {
        fn fetch_workspace(&self) -> Result<TrackerState, ix_board::Error> {
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(TrackerState::default())
        }
    }

    #[test]
    fn injected_read_service_factory_populates_tracker_without_live_sources() {
        let factory_calls = std::cell::Cell::new(0);
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("tracker.sqlite");
        let read = read_tracker_with_factory(false, || {
            factory_calls.set(factory_calls.get() + 1);
            ReadService::open(
                FixtureSource {
                    reads: std::sync::Arc::clone(&reads),
                },
                &cache_path,
            )
        })
        .unwrap();
        assert_eq!(
            factory_calls.get(),
            1,
            "dashboard must use the injected authenticated factory"
        );
        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 1);
        let mut app = crate::monitor::App::default();
        app.apply_tracker_refresh(Ok(read), std::time::Instant::now());
        assert!(app.tracker.value.is_some());
        assert!(app.tracker.error.is_none());
        assert!(!app.collection_failed);
    }

    /// How the fake source answers its next read. The typed refusals are the quota decisions of
    /// [`quota::fixture`], returned the way the SDK's admission returns them.
    #[derive(Clone, Copy, Debug)]
    enum Scenario {
        Succeeds,
        RateLimited,
        GraphQlFailure,
        LocalBudgetExhausted,
        LockRefused,
        ReserveHeld,
        LocalHeld,
        ServerBackoff,
        MixedBackoff,
        Quarantined,
        EvidenceOnly,
        Rejected,
        Unexplained,
        Simultaneous,
    }

    struct ScenarioSource {
        scenario: std::sync::Arc<std::sync::Mutex<Scenario>>,
    }

    impl ix_board::WorkspaceSource for ScenarioSource {
        fn fetch_workspace(&self) -> Result<TrackerState, ix_board::Error> {
            let refused = |decision| {
                Err(ix_board::Error::QuotaAdmissionRefused {
                    decision: Box::new(decision),
                })
            };
            match *self.scenario.lock().unwrap() {
                Scenario::Succeeds => Ok(TrackerState::default()),
                Scenario::RateLimited => Err(ix_board::Error::LinearRateLimited {
                    retry_after: Duration::from_secs(60),
                }),
                Scenario::GraphQlFailure => Err(ix_board::Error::GraphQl {
                    messages: "upstream".into(),
                }),
                Scenario::LocalBudgetExhausted => {
                    Err(ix_board::Error::LinearLocalBudgetExhausted {
                        retry_after: Duration::from_secs(60),
                    })
                }
                Scenario::LockRefused => Err(ix_board::Error::RefreshLockTimeout),
                Scenario::ReserveHeld => refused(quota::fixture::reserve_held()),
                Scenario::LocalHeld => refused(quota::fixture::local_held()),
                Scenario::ServerBackoff => refused(quota::fixture::server_backoff()),
                Scenario::MixedBackoff => refused(quota::fixture::mixed_backoff()),
                Scenario::Quarantined => refused(quota::fixture::quarantined()),
                Scenario::EvidenceOnly => refused(quota::fixture::evidence_only()),
                Scenario::Rejected => refused(quota::fixture::rejected()),
                Scenario::Unexplained => refused(quota::fixture::unexplained()),
                Scenario::Simultaneous => refused(quota::fixture::simultaneous()),
            }
        }
    }

    /// A real read service over a private cache with a scripted source. [`Self::seeded`] holds
    /// one good snapshot so each later failed refresh has a snapshot to keep; [`Self::cold`]
    /// holds none.
    struct KeptSnapshot {
        _directory: tempfile::TempDir,
        cache_path: std::path::PathBuf,
        scenario: std::sync::Arc<std::sync::Mutex<Scenario>>,
        /// The source time of the seeded snapshot.
        original: i64,
    }

    impl KeptSnapshot {
        fn cold() -> Self {
            let directory = tempfile::tempdir().unwrap();
            Self {
                cache_path: directory.path().join("tracker.sqlite"),
                _directory: directory,
                scenario: std::sync::Arc::new(std::sync::Mutex::new(Scenario::Succeeds)),
                original: 0,
            }
        }

        fn seeded() -> Self {
            let mut fixture = Self::cold();
            let good = fixture.read(Scenario::Succeeds, false);
            assert_eq!(good.provenance(), Provenance::Current);
            assert_eq!(good.outcome, RefreshOutcome::Refreshed);
            assert!(good.snapshot.fetched_at > 0);
            fixture.original = good.snapshot.fetched_at;
            fixture
        }

        /// One tracker read, as the worker does it, with the source answering `scenario`.
        fn try_read(&self, scenario: Scenario, force: bool) -> Result<TrackerRead, TrackerFailure> {
            *self.scenario.lock().unwrap() = scenario;
            read_tracker_with_factory(force, || {
                ReadService::open(
                    ScenarioSource {
                        scenario: std::sync::Arc::clone(&self.scenario),
                    },
                    &self.cache_path,
                )
            })
        }

        fn read(&self, scenario: Scenario, force: bool) -> TrackerRead {
            self.try_read(scenario, force).unwrap()
        }

        /// The dashboard after one forced read that kept the seeded snapshot.
        fn applied(&self, scenario: Scenario, now: std::time::Instant) -> crate::monitor::App {
            let mut app = crate::monitor::App::default();
            app.apply_tracker_refresh(Ok(self.read(scenario, true)), now);
            assert_eq!(
                app.tracker.value.as_ref().unwrap().fetched_at,
                self.original,
                "the kept snapshot keeps its source time"
            );
            app
        }

        /// The failure of one forced read with no snapshot to keep.
        fn failure(&self, scenario: Scenario) -> TrackerFailure {
            let Err(failure) = self.try_read(scenario, true) else {
                panic!("{scenario:?} must fail on a cold cache");
            };
            failure
        }

        /// The dashboard after one forced read with no snapshot to keep.
        fn applied_cold(&self, scenario: Scenario, now: std::time::Instant) -> crate::monitor::App {
            let mut app = crate::monitor::App::default();
            app.apply_tracker_refresh(Err(self.failure(scenario)), now);
            assert!(app.tracker.value.is_none() && app.collection_failed);
            app
        }
    }

    fn protected_after(read: TrackerRead) -> bool {
        RefreshWait::after_read(
            &Ok(read),
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        )
        .protected
    }

    fn protected_after_failure(failure: TrackerFailure) -> bool {
        RefreshWait::after_read(
            &Err(failure),
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        )
        .protected
    }

    /// A snapshot kept after a rate-limited refresh keeps its source time, shows the failure
    /// and the countdown at once (it is seconds old, far under the thirty-minute stale label)
    /// and coalesces manual refreshes, because the service reported a cooldown.
    #[trace("FR-059-AC-5", "FR-064-AC-10", "FR-048-AC-5")]
    #[test]
    fn a_snapshot_kept_after_a_rate_limited_refresh_shows_the_cooldown_and_its_original_time() {
        let fixture = KeptSnapshot::seeded();
        let now = std::time::Instant::now();
        let limited = fixture.read(Scenario::RateLimited, true);
        assert_eq!(limited.snapshot.fetched_at, fixture.original);
        assert_eq!(limited.outcome, RefreshOutcome::Unknown);
        assert_eq!(limited.provenance().cause(), Some(Cause::ServerBackoff));
        assert!(
            limited
                .freshness
                .as_ref()
                .and_then(|freshness| freshness.age_seconds)
                .is_some_and(|age| age < 60)
        );
        assert!(protected_after(fixture.read(Scenario::RateLimited, true)));

        let mut app = crate::monitor::App::default();
        app.apply_tracker_refresh(Ok(limited), now);
        assert_eq!(
            app.tracker.value.as_ref().unwrap().fetched_at,
            fixture.original
        );
        assert_eq!(
            app.tracker.error.as_deref(),
            Some("Linear rate limited; showing the last snapshot")
        );
        assert_eq!(
            app.tracker_retry_at,
            now.checked_add(TRACKER_REFRESH_INTERVAL)
        );
        assert!(app.collection_failed);
    }

    /// A snapshot kept after any other failed refresh shows on a cold start, names no cause
    /// and invents no cooldown. The service types it as a source failure, not a quota hold.
    #[trace("FR-059-AC-5", "FR-064-AC-10", "FR-048-AC-5")]
    #[test]
    fn a_snapshot_kept_after_a_failed_refresh_shows_on_a_cold_start_without_cause_or_cooldown() {
        let fixture = KeptSnapshot::seeded();
        let failed = fixture.read(Scenario::GraphQlFailure, true);
        assert_eq!(failed.snapshot.fetched_at, fixture.original);
        assert_eq!(failed.outcome, RefreshOutcome::SourceFailed);
        assert_eq!(failed.provenance().cause(), Some(Cause::SourceFailed));
        assert!(!protected_after(
            fixture.read(Scenario::GraphQlFailure, true)
        ));

        let mut cold = crate::monitor::App::default();
        cold.apply_tracker_refresh(Ok(failed), std::time::Instant::now());
        assert_eq!(
            cold.tracker.value.as_ref().unwrap().fetched_at,
            fixture.original
        );
        let error = cold.tracker.error.as_deref().unwrap();
        assert!(error.contains("Linear refresh failed"), "{error}");
        assert!(!error.contains("upstream"), "the raw cause is not shown");
        assert!(cold.tracker_retry_at.is_none());
        assert!(cold.collection_failed);
    }

    /// With a snapshot, the service reports the legacy exhausted-budget error with no typed
    /// outcome: the metadata stays unknown, the notice stays neutral and no cooldown or
    /// coalescing is invented.
    #[trace("FR-048-AC-5")]
    #[test]
    fn a_kept_snapshot_after_local_budget_exhaustion_has_the_neutral_notice_and_no_cooldown() {
        let fixture = KeptSnapshot::seeded();
        let refused = fixture.read(Scenario::LocalBudgetExhausted, true);
        assert_eq!(refused.snapshot.fetched_at, fixture.original);
        assert_eq!(refused.outcome, RefreshOutcome::Unknown);
        assert_eq!(refused.provenance().cause(), Some(Cause::Unknown));
        assert!(!protected_after(
            fixture.read(Scenario::LocalBudgetExhausted, true)
        ));

        let mut app = crate::monitor::App::default();
        app.apply_tracker_refresh(Ok(refused), std::time::Instant::now());
        let error = app.tracker.error.as_deref().unwrap();
        assert!(error.contains("Linear refresh failed"), "{error}");
        assert!(!error.contains("budget"), "no cause is claimed: {error}");
        assert!(app.tracker_retry_at.is_none());
    }

    /// The next successful read clears the notice and the countdown.
    #[trace("FR-059-AC-5", "FR-064-AC-10")]
    #[test]
    fn a_successful_read_after_a_kept_snapshot_clears_the_notice_and_the_countdown() {
        let fixture = KeptSnapshot::seeded();
        let now = std::time::Instant::now();
        let mut app = crate::monitor::App::default();
        app.apply_tracker_refresh(Ok(fixture.read(Scenario::RateLimited, true)), now);
        assert!(app.tracker.error.is_some() && app.tracker_retry_at.is_some());

        let recovered = fixture.read(Scenario::Succeeds, true);
        assert_eq!(recovered.provenance(), Provenance::Current);
        app.apply_tracker_refresh(Ok(recovered), now);
        assert!(app.tracker.error.is_none());
        assert!(app.tracker_retry_at.is_none());
        assert_eq!(app.tracker_status, TrackerStatus::Clear);
    }

    fn freshness(
        stale: bool,
        age_seconds: Option<i64>,
        rate_limit_until: Option<i64>,
    ) -> ix_board::CacheFreshness {
        ix_board::CacheFreshness {
            last_successful_sync: None,
            last_full_reconciliation: None,
            stale,
            age_seconds,
            rate_limit_until,
            failure_reason: Some("never read".into()),
        }
    }

    fn classify(
        outcome: RefreshOutcome,
        freshness: Option<ix_board::CacheFreshness>,
    ) -> Provenance {
        TrackerRead {
            snapshot: TrackerState::default(),
            freshness,
            outcome,
        }
        .provenance()
    }

    /// Without a typed outcome the legacy typed freshness decides: a stale snapshot with a
    /// reported cooldown follows an actual server rate limit, with a known read age a failed
    /// refresh of unknown cause, and with neither it is unconfirmed.
    #[trace("FR-059-AC-5")]
    #[test]
    fn an_untyped_outcome_stays_unknown_unless_typed_freshness_reports_a_failure() {
        let unknown = |freshness| classify(RefreshOutcome::Unknown, freshness);
        assert_eq!(unknown(None), Provenance::Current);
        assert_eq!(
            unknown(Some(freshness(false, Some(5), None))),
            Provenance::Current
        );
        assert_eq!(
            unknown(Some(freshness(true, Some(5), None))).cause(),
            Some(Cause::Unknown)
        );
        assert_eq!(
            unknown(Some(freshness(true, None, Some(1_000)))).cause(),
            Some(Cause::ServerBackoff)
        );
        assert_eq!(
            unknown(Some(freshness(true, None, None))),
            Provenance::Unconfirmed
        );
    }

    /// A typed successful outcome claims no failure: a stale mark on it leaves it unconfirmed,
    /// and a typed failure is not hidden by freshness that says nothing is stale.
    #[trace("FR-059-AC-5")]
    #[test]
    fn a_typed_outcome_decides_before_the_freshness_fields() {
        for success in [RefreshOutcome::CacheReused, RefreshOutcome::Refreshed] {
            assert_eq!(
                classify(success.clone(), Some(freshness(false, Some(5), None))),
                Provenance::Current
            );
            assert_eq!(
                classify(success.clone(), Some(freshness(true, None, Some(1_000)))),
                Provenance::Unconfirmed,
                "a successful attempt shows no failure whatever the freshness marks"
            );
        }
        let current = Some(freshness(false, Some(5), None));
        assert_eq!(
            classify(RefreshOutcome::SourceFailed, current.clone()).cause(),
            Some(Cause::SourceFailed)
        );
        assert_eq!(
            classify(RefreshOutcome::RefreshLockRefused, current.clone()).cause(),
            Some(Cause::RefreshLockBusy)
        );
        let refused = RefreshOutcome::QuotaRefused {
            decision: quota::fixture::quarantined(),
        };
        assert_eq!(
            classify(refused, current).cause(),
            Some(Cause::EvidenceQuarantined)
        );
    }

    /// A server reserve hold while local usage is zero keeps the snapshot and its time, is not
    /// a rate limit and not a local hold, and still waits like any hold to wait out.
    #[trace("FR-048-AC-5", "FR-059-AC-5", "FR-064-AC-10")]
    #[test]
    fn a_reserve_hold_with_zero_local_usage_is_neither_a_rate_limit_nor_a_local_hold() {
        let fixture = KeptSnapshot::seeded();
        let now = std::time::Instant::now();
        let read = fixture.read(Scenario::ReserveHeld, true);
        assert!(matches!(
            read.outcome,
            RefreshOutcome::QuotaRefused { ref decision } if decision.state == QuotaState::ServerReserveHeld
        ));
        assert_eq!(read.provenance().cause(), Some(Cause::ServerReserve));
        assert!(protected_after(fixture.read(Scenario::ReserveHeld, true)));

        let app = fixture.applied(Scenario::ReserveHeld, now);
        assert_eq!(
            app.tracker_status,
            TrackerStatus::Failed(Cause::ServerReserve)
        );
        assert_eq!(
            app.tracker_retry_at,
            now.checked_add(TRACKER_REFRESH_INTERVAL)
        );
        assert_eq!(
            app.tracker.error_summary.as_deref(),
            Some("Linear server reserve held; showing the last snapshot")
        );
        let detail = app.tracker.error.as_deref().unwrap();
        for shown in ["local usage 0", "remaining 3", "reserve 7", "capacity 40"] {
            assert!(detail.contains(shown), "{shown}: {detail}");
        }
        for absent in ["rate limited", "local allocation held", "owning deadline"] {
            assert!(!detail.contains(absent), "{absent}: {detail}");
        }
        assert!(app.collection_failed);
    }

    /// An exhausted local window is its own hold, distinct from the reserve hold.
    #[trace("FR-048-AC-5", "FR-059-AC-5")]
    #[test]
    fn a_local_window_hold_is_reported_as_the_local_allocation() {
        let fixture = KeptSnapshot::seeded();
        let app = fixture.applied(Scenario::LocalHeld, std::time::Instant::now());
        assert_eq!(
            app.tracker_status,
            TrackerStatus::Failed(Cause::LocalWindow)
        );
        let detail = app.tracker.error.as_deref().unwrap();
        assert!(detail.starts_with("Linear local allocation held; showing the last snapshot"));
        assert!(detail.contains("local usage 880") && detail.contains("capacity 900"));
        assert!(!detail.contains("server reserve"), "{detail}");
        assert!(app.tracker_retry_at.is_some());
    }

    /// Only a typed server backoff is a rate limit, and it keeps the snapshot with a countdown.
    #[trace("FR-048-AC-5", "FR-059-AC-5")]
    #[test]
    fn an_actual_server_backoff_decision_is_the_rate_limit() {
        let fixture = KeptSnapshot::seeded();
        let now = std::time::Instant::now();
        let read = fixture.read(Scenario::ServerBackoff, true);
        assert_eq!(read.provenance().cause(), Some(Cause::ServerBackoff));
        assert!(read.freshness.as_ref().unwrap().rate_limit_until.is_some());
        let app = fixture.applied(Scenario::ServerBackoff, now);
        assert_eq!(
            app.tracker_status,
            TrackerStatus::Failed(Cause::ServerBackoff)
        );
        assert_eq!(
            app.tracker.error_summary.as_deref(),
            Some("Linear rate limited; showing the last snapshot")
        );
        assert!(app.tracker_retry_at.is_some());
    }

    /// Quarantined quota evidence is unavailable evidence, never a 429 and never a countdown:
    /// its reevaluation time is unknown and nothing waits on it.
    #[trace("FR-048-AC-5", "FR-059-AC-5")]
    #[test]
    fn quarantined_quota_evidence_is_not_a_rate_limit_and_shows_no_countdown() {
        let fixture = KeptSnapshot::seeded();
        assert!(!protected_after(fixture.read(Scenario::Quarantined, true)));
        let app = fixture.applied(Scenario::Quarantined, std::time::Instant::now());
        assert_eq!(
            app.tracker_status,
            TrackerStatus::Failed(Cause::EvidenceQuarantined)
        );
        assert!(app.tracker_retry_at.is_none(), "no timer is invented");
        let detail = app.tracker.error.as_deref().unwrap();
        assert!(detail.starts_with("Linear quota evidence quarantined; showing the last snapshot"));
        assert!(detail.contains("2 pre-fence reservations are pending"));
        assert!(detail.contains("reevaluation time unknown"));
        assert!(!detail.contains("rate limited"), "{detail}");
        assert_eq!(
            app.tracker_notice.as_ref().unwrap().actions,
            [Action::ReconcileEvidence]
        );
    }

    /// Every simultaneous blocker reaches the dashboard under the one primary summary.
    #[trace("FR-048-AC-5", "FR-059-AC-5")]
    #[test]
    fn simultaneous_blockers_all_reach_the_dashboard_under_one_primary_summary() {
        let fixture = KeptSnapshot::seeded();
        let app = fixture.applied(Scenario::Simultaneous, std::time::Instant::now());
        assert_eq!(
            app.tracker.error_summary.as_deref(),
            Some("Linear quota evidence quarantined; showing the last snapshot")
        );
        let detail = app.tracker.error.as_deref().unwrap();
        for blocker in [
            "hold: evidence unavailable",
            "hold: local allocation held requests",
            "hold: server reserve held endpoint requests",
        ] {
            assert!(detail.contains(blocker), "{blocker}: {detail}");
        }
        assert!(detail.contains("earliest reevaluation 8m 20s after the attempt"));
    }

    /// A shared refresh lock the service could not take is its own typed notice, not a hold.
    #[trace("FR-048-AC-5", "FR-059-AC-5")]
    #[test]
    fn a_refused_refresh_lock_is_its_own_notice_and_does_not_coalesce() {
        let fixture = KeptSnapshot::seeded();
        let read = fixture.read(Scenario::LockRefused, true);
        assert_eq!(read.outcome, RefreshOutcome::RefreshLockRefused);
        assert!(!protected_after(read));
        let app = fixture.applied(Scenario::LockRefused, std::time::Instant::now());
        assert_eq!(
            app.tracker_status,
            TrackerStatus::Failed(Cause::RefreshLockBusy)
        );
        assert!(app.tracker_retry_at.is_none());
    }

    /// A served cache and a real refresh each carry their typed outcome, and a refresh after a
    /// hold clears the notice, the countdown and the status.
    #[trace("FR-059-AC-5", "FR-064-AC-10")]
    #[test]
    fn cache_reuse_and_a_real_refresh_are_current_and_recovery_clears_a_hold() {
        let fixture = KeptSnapshot::seeded();
        let reused = fixture.read(Scenario::Succeeds, false);
        assert_eq!(reused.outcome, RefreshOutcome::CacheReused);
        assert_eq!(reused.provenance(), Provenance::Current);

        let now = std::time::Instant::now();
        let mut app = fixture.applied(Scenario::ReserveHeld, now);
        assert!(app.tracker.error.is_some() && app.tracker_retry_at.is_some());
        let refreshed = fixture.read(Scenario::Succeeds, true);
        assert_eq!(refreshed.outcome, RefreshOutcome::Refreshed);
        app.apply_tracker_refresh(Ok(refreshed), now);
        assert!(app.tracker.error.is_none() && app.tracker.error_summary.is_none());
        assert!(app.tracker_retry_at.is_none());
        assert_eq!(app.tracker_status, TrackerStatus::Clear);
        assert!(
            app.tracker.value.as_ref().unwrap().fetched_at >= fixture.original,
            "a real refresh publishes a new source time"
        );
    }

    /// An explicit backoff that outranks quarantined reservations and a local hold keeps its
    /// primary state, every blocker, the pending reservations, the earliest reevaluation and
    /// the reconcile action, and still counts down: the action stays in the notice.
    #[trace("FR-048-AC-5", "FR-059-AC-5")]
    #[test]
    fn a_backoff_with_quarantine_shows_every_decision_fact_and_the_reconcile_action() {
        let fixture = KeptSnapshot::seeded();
        let now = std::time::Instant::now();
        let read = fixture.read(Scenario::MixedBackoff, true);
        assert!(protected_after(read));
        let app = fixture.applied(Scenario::MixedBackoff, now);
        assert_eq!(
            app.tracker_status,
            TrackerStatus::Failed(Cause::ServerBackoff)
        );
        assert_eq!(
            app.tracker_retry_at,
            now.checked_add(TRACKER_REFRESH_INTERVAL)
        );
        let notice = app.tracker_notice.as_ref().unwrap();
        assert_eq!(notice.actions, [Action::ReconcileEvidence]);
        assert_eq!(notice.facts.len(), 6, "{:?}", notice.facts);
        let detail = app.tracker.error.as_deref().unwrap();
        assert!(detail.starts_with("Linear rate limited; showing the last snapshot"));
        assert!(detail.contains("reconcile the quota evidence"), "{detail}");
        assert!(!detail.contains("not a server limit"), "{detail}");
    }

    /// With no snapshot the typed outcome the service recorded survives the error, so a cold
    /// actual backoff carries the same decision facts and action as the warm one, not only the
    /// SDK's legacy retry duration.
    #[trace("FR-048-AC-5", "FR-064-AC-10")]
    #[test]
    fn a_cold_backoff_with_quarantine_carries_the_same_facts_as_a_warm_one() {
        let warm =
            KeptSnapshot::seeded().applied(Scenario::MixedBackoff, std::time::Instant::now());
        let cold_fixture = KeptSnapshot::cold();
        let failure = cold_fixture.failure(Scenario::MixedBackoff);
        assert!(
            matches!(failure.error, ix_board::Error::LinearRateLimited { .. }),
            "the SDK keeps its legacy typed error for a backoff with no snapshot"
        );
        assert!(matches!(
            *failure.outcome,
            RefreshOutcome::QuotaRefused { .. }
        ));
        assert!(protected_after_failure(failure));

        let cold = cold_fixture.applied_cold(Scenario::MixedBackoff, std::time::Instant::now());
        assert_eq!(
            cold.tracker_status,
            TrackerStatus::Failed(Cause::ServerBackoff)
        );
        assert!(cold.tracker_retry_at.is_some());
        let (cold_notice, warm_notice) = (
            cold.tracker_notice.as_ref().unwrap(),
            warm.tracker_notice.as_ref().unwrap(),
        );
        assert_eq!(cold_notice.facts, warm_notice.facts);
        assert_eq!(cold_notice.actions, warm_notice.actions);
        assert_eq!(cold_notice.summary, "Linear rate limited");
        let detail = cold.tracker.error.as_deref().unwrap();
        for shown in [
            "hold: evidence unavailable",
            "hold: local allocation held requests",
            "3 pre-fence reservations are pending",
            "reconcile the quota evidence",
        ] {
            assert!(detail.contains(shown), "{shown}: {detail}");
        }
        assert!(!detail.contains("showing the last"), "{detail}");
    }

    /// With no snapshot, every non-backoff refusal also keeps its decision and its waiting rule.
    #[trace("FR-048-AC-5", "FR-064-AC-10")]
    #[test]
    fn a_cold_refusal_keeps_its_decision_and_its_waiting_rule() {
        let fixture = KeptSnapshot::cold();
        let now = std::time::Instant::now();
        let reserve = fixture.applied_cold(Scenario::ReserveHeld, now);
        assert_eq!(
            reserve.tracker_status,
            TrackerStatus::Failed(Cause::ServerReserve)
        );
        assert!(reserve.tracker_retry_at.is_some());
        let detail = reserve.tracker.error.as_deref().unwrap();
        assert!(detail.starts_with("Linear server reserve held"), "{detail}");
        assert!(detail.contains("local usage 0") && !detail.contains("showing the last"));

        let quarantine = fixture.applied_cold(Scenario::Quarantined, now);
        assert_eq!(
            quarantine.tracker_status,
            TrackerStatus::Failed(Cause::EvidenceQuarantined)
        );
        assert!(quarantine.tracker_retry_at.is_none());
        assert!(!protected_after_failure(
            fixture.failure(Scenario::Quarantined)
        ));
    }

    /// A refused refresh lock is the same typed lock notice with and without a snapshot.
    #[trace("FR-048-AC-5", "FR-064-AC-10")]
    #[test]
    fn a_cold_refused_refresh_lock_is_the_lock_notice() {
        let fixture = KeptSnapshot::cold();
        let failure = fixture.failure(Scenario::LockRefused);
        assert!(matches!(failure.error, ix_board::Error::RefreshLockTimeout));
        assert_eq!(*failure.outcome, RefreshOutcome::RefreshLockRefused);
        assert!(!protected_after_failure(failure));
        let app = fixture.applied_cold(Scenario::LockRefused, std::time::Instant::now());
        assert_eq!(
            app.tracker_status,
            TrackerStatus::Failed(Cause::RefreshLockBusy)
        );
        assert_eq!(
            app.tracker.error.as_deref(),
            Some("Linear refresh lock busy")
        );
        assert!(app.tracker_retry_at.is_none());
    }

    /// An error with no typed decision stays what it was: a legacy rate limit is the only rate
    /// limit and keeps no invented detail, and a source failure keeps the generic handling.
    #[trace("FR-048-AC-5", "FR-059-AC-5")]
    #[test]
    fn a_cold_legacy_error_stays_legacy_and_a_source_failure_stays_generic() {
        let fixture = KeptSnapshot::cold();
        let now = std::time::Instant::now();
        let limited = fixture.applied_cold(Scenario::RateLimited, now);
        assert_eq!(
            limited.tracker_status,
            TrackerStatus::Failed(Cause::ServerBackoff)
        );
        assert_eq!(
            limited.tracker.error.as_deref(),
            Some("Linear rate limited")
        );
        assert!(limited.tracker_retry_at.is_some());

        let failed = fixture.applied_cold(Scenario::GraphQlFailure, now);
        assert_eq!(
            failed.tracker_status,
            TrackerStatus::Failed(Cause::SourceFailed)
        );
        assert!(failed.tracker_notice.is_none());
        let error = failed.tracker.error.as_deref().unwrap();
        assert!(error.contains("Structured tracker read failed"), "{error}");
        assert!(!error.contains("upstream"), "{error}");
        assert!(failed.tracker_retry_at.is_none());
    }

    /// Refusals that do not wait show their own notice and actions at the dashboard, start no
    /// countdown and do not coalesce manual refreshes, with and without a snapshot.
    fn assert_does_not_wait(scenario: Scenario, cause: Cause, actions: &[Action]) {
        let warm = KeptSnapshot::seeded();
        assert!(!protected_after(warm.read(scenario, true)), "{scenario:?}");
        let app = warm.applied(scenario, std::time::Instant::now());
        assert_eq!(app.tracker_status, TrackerStatus::Failed(cause));
        assert!(app.tracker_retry_at.is_none(), "{scenario:?}: no countdown");
        assert_eq!(app.tracker_notice.as_ref().unwrap().actions, actions);

        let cold = KeptSnapshot::cold();
        assert!(!protected_after_failure(cold.failure(scenario)));
        let app = cold.applied_cold(scenario, std::time::Instant::now());
        assert_eq!(app.tracker_status, TrackerStatus::Failed(cause));
        assert!(app.tracker_retry_at.is_none());
        assert_eq!(app.tracker_notice.as_ref().unwrap().actions, actions);
        let detail = app.tracker.error.as_deref().unwrap();
        assert!(detail.starts_with(cause.summary()), "{detail}");
        assert!(!detail.contains("rate limited"), "{detail}");
    }

    #[trace("FR-048-AC-5")]
    #[test]
    fn a_rejected_request_does_not_wait_and_names_the_request_action() {
        assert_does_not_wait(
            Scenario::Rejected,
            Cause::RequestRejected,
            &[Action::InspectRequest],
        );
    }

    #[trace("FR-048-AC-5")]
    #[test]
    fn unavailable_evidence_without_pending_reservations_does_not_wait() {
        assert_does_not_wait(
            Scenario::EvidenceOnly,
            Cause::EvidenceUnavailable,
            &[Action::ReconcileEvidence],
        );
    }

    #[trace("FR-048-AC-5")]
    #[test]
    fn an_unexplained_refusal_does_not_wait_and_names_the_source_action() {
        assert_does_not_wait(
            Scenario::Unexplained,
            Cause::RefusalUnexplained,
            &[Action::InspectSource],
        );
    }

    #[trace("FR-059-AC-6")]
    #[test]
    fn injected_authentication_denial_is_classified_and_quit_retains_collection_failure() {
        let result = read_tracker_with_factory::<FixtureSource>(false, || {
            Err(ix_board::Error::Credential(
                ix_cli_kit::secrets::SecretError::Locked,
            ))
        });
        let mut app = crate::monitor::App::default();
        app.apply_tracker_refresh(result, std::time::Instant::now());
        assert!(app.collection_failed);
        let diagnostic = app.tracker.error.as_deref().unwrap();
        assert!(diagnostic.contains("saved credential could not be accessed"));
        assert!(diagnostic.contains("credential owner"));
        crate::monitor::handle_key(
            &mut app,
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('q'),
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert!(app.should_quit);
        assert!(app.collection_failed);
        let (outcome, final_diagnostic) = crate::quit_outcome(&app);
        assert_eq!(outcome, ix_cli_kit::exit::Outcome::Partial);
        assert!(final_diagnostic.unwrap().contains("credential owner"));
        let missing = source_diagnostic(&ix_board::Error::MissingApiKey);
        assert!(missing.contains("ix-board auth login"));
        assert!(missing.contains("--report"));
    }

    /// A credential error keeps its redacted handling whatever outcome the service recorded:
    /// the typed outcome decides only a quota refusal or a lock refusal.
    #[trace("FR-059-AC-6")]
    #[test]
    fn a_credential_error_keeps_its_redacted_handling_whatever_the_outcome() {
        let credential = || ix_board::Error::Credential(ix_cli_kit::secrets::SecretError::Locked);
        for outcome in [RefreshOutcome::Unknown, RefreshOutcome::SourceFailed] {
            let failure = TrackerFailure {
                error: credential(),
                outcome: Box::new(outcome),
            };
            assert_eq!(failure.cause(), None);
            let mut app = crate::monitor::App::default();
            app.apply_tracker_refresh(Err(failure), std::time::Instant::now());
            assert!(app.tracker_notice.is_none() && app.tracker_retry_at.is_none());
            assert!(
                app.tracker
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("credential owner")
            );
        }
    }

    #[test]
    fn stopping_does_not_wait_for_an_in_flight_linear_read() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (read_finished_tx, read_finished_rx) = mpsc::channel();
        let worker = TrackerWorker::spawn_with_read(move |_| {
            entered_tx.send(()).unwrap();
            let _ = release_rx.recv();
            read_finished_tx.send(()).unwrap();
            Err(ix_board::Error::MissingApiKey.into())
        })
        .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let (stopped_tx, stopped_rx) = mpsc::channel();
        let stopper = thread::spawn(move || {
            worker.stop();
            stopped_tx.send(()).unwrap();
        });
        let returned_before_release = stopped_rx.recv_timeout(Duration::from_millis(200)).is_ok();
        let _ = release_tx.send(());
        stopper.join().unwrap();
        read_finished_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(returned_before_release, "stop waited for the Linear read");
    }

    #[test]
    fn failed_linear_refreshes_back_off_and_obey_the_configured_cap() {
        let refresh = Duration::from_millis(100);
        let cap = Duration::from_millis(400);
        let retry = next_interval(false, refresh, refresh, cap);
        assert_eq!(retry, Duration::from_millis(200));
        let retry = next_interval(false, retry, refresh, cap);
        assert_eq!(retry, cap);
        assert_eq!(next_interval(false, retry, refresh, cap), cap);
        assert_eq!(next_interval(true, retry, refresh, cap), refresh);
    }

    /// Trace: FR-048-AC-5
    #[test]
    fn automatic_reads_and_rate_limited_retries_use_thirty_minutes() {
        assert_eq!(TRACKER_REFRESH_INTERVAL, Duration::from_secs(1800));
        let rate_limit = Err(ix_board::Error::LinearRateLimited {
            retry_after: Duration::from_secs(3600),
        }
        .into());
        let wait = RefreshWait::after_read(
            &rate_limit,
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        );
        assert_eq!(wait.delay, Duration::from_secs(1800));
        assert!(wait.protected);
        assert_eq!(
            next_interval(
                false,
                TRACKER_REFRESH_INTERVAL,
                TRACKER_REFRESH_INTERVAL,
                TRACKER_MAX_RETRY_INTERVAL
            ),
            Duration::from_secs(1800)
        );
        let wait = RefreshWait::after_read(
            &Err(ix_board::Error::MissingApiKey.into()),
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        );
        assert_eq!(wait.delay, Duration::from_secs(1800));
        assert!(!wait.protected);
    }

    /// With no snapshot to keep, the shared local request budget refuses a read as an error
    /// before any request is sent. That error is protected like a server rate limit: manual
    /// refreshes coalesce, the countdown shows, and the failure is not the generic
    /// "tracker read failed" line. With a snapshot the service reports no typed cause; see
    /// `a_kept_snapshot_after_local_budget_exhaustion_has_the_neutral_notice_and_no_cooldown`.
    #[trace("FR-048-AC-5")]
    #[test]
    fn local_budget_exhaustion_without_a_snapshot_is_protected_like_a_rate_limit() {
        let exhausted = || {
            TrackerFailure::from(ix_board::Error::LinearLocalBudgetExhausted {
                retry_after: Duration::from_secs(60),
            })
        };
        let wait = RefreshWait::after_read(
            &Err(exhausted()),
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        );
        assert_eq!(wait.delay, TRACKER_REFRESH_INTERVAL);
        assert!(wait.protected);

        let mut app = crate::monitor::App::default();
        app.apply_tracker_refresh(Err(exhausted()), std::time::Instant::now());
        assert!(app.tracker_retry_at.is_some());
        assert_eq!(
            app.tracker.error.as_deref(),
            Some("Linear request budget exhausted")
        );
        assert!(app.collection_failed);
        assert!(app.tracker.value.is_none());
    }

    /// Trace: FR-048-AC-5
    #[test]
    fn rate_limit_cooldown_coalesces_manual_refreshes_until_timeout_and_allows_stop() {
        let mut wait = RefreshWait {
            delay: Duration::from_secs(300),
            protected: true,
            force: false,
        };
        for _ in 0..3 {
            assert!(matches!(
                wait.receive(Ok(TrackerCommand::Refresh)),
                WaitOutcome::Continue
            ));
        }
        assert!(matches!(
            wait.receive(Err(mpsc::RecvTimeoutError::Timeout)),
            WaitOutcome::Read { force: true }
        ));
        assert!(matches!(
            wait.receive(Ok(TrackerCommand::Stop)),
            WaitOutcome::Stop
        ));
        let mut wait = RefreshWait {
            delay: Duration::from_secs(300),
            protected: true,
            force: false,
        };
        assert!(matches!(
            wait.receive(Err(mpsc::RecvTimeoutError::Timeout)),
            WaitOutcome::Read { force: false }
        ));
        assert!(matches!(
            wait.receive(Err(mpsc::RecvTimeoutError::Disconnected)),
            WaitOutcome::Stop
        ));
    }

    /// Trace: FR-048-AC-5
    #[test]
    fn manual_refresh_runs_again_after_current_read() {
        let (calls_tx, calls_rx) = mpsc::channel();
        let mut call_number = 0;
        let worker = TrackerWorker::spawn_with_read(move |force| {
            call_number += 1;
            calls_tx.send((call_number, force)).unwrap();
            Err(ix_board::Error::MissingApiKey.into())
        })
        .unwrap();

        assert_eq!(
            calls_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (1, false)
        );
        worker.refresh_now();
        assert_eq!(
            calls_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (2, true)
        );
        worker.stop();
    }
}
