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
use ix_board::{LinearGraphQlSource, ReadService};
use ix_cli_kit::streams::{DiagnosticFields, Record};

/// Compact source classification suitable for a single diagnostic line.
pub(crate) fn source_summary(error: &ix_board::Error) -> &'static str {
    match error {
        ix_board::Error::MissingApiKey => "Authentication unavailable; ix-board auth login",
        ix_board::Error::Credential(_) => {
            "Saved credential access denied; contact credential owner"
        }
        ix_board::Error::LinearRateLimited { .. } => "Linear rate limited",
        ix_board::Error::LinearLocalBudgetExhausted { .. } => "Linear request budget exhausted",
        _ => "Structured tracker read failed; inspect source availability",
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
            "Inspect source availability with ix-board, or open --report",
        ),
    };
    let record = Record::error(kind, message).with_fields(DiagnosticFields {
        reason: Some(kind),
        remedy: Some(remedy),
        ..DiagnosticFields::default()
    });
    format!("{} · {}", record.render_human(false), remedy)
}

/// Automatic Linear tracker reads run every 30 minutes.
pub const TRACKER_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// Limit retries after Linear or its cached workspace read returns an error.
const TRACKER_MAX_RETRY_INTERVAL: Duration = TRACKER_REFRESH_INTERVAL;

/// One successful tracker read: the snapshot and how the shared read service obtained it.
///
/// After a failed refresh the service returns its last complete snapshot as a success and records
/// the failure only in `freshness`; the snapshot keeps the time its source read began.
pub struct TrackerRead {
    /// The snapshot, with its original source timestamp in `fetched_at`.
    pub snapshot: TrackerState,
    /// Provenance of this read as the service reports it; `None` when it reports none.
    pub freshness: Option<ix_board::CacheFreshness>,
}

impl TrackerRead {
    /// The provenance when the snapshot was retained because the latest refresh did not succeed.
    pub(crate) fn retained(&self) -> Option<&ix_board::CacheFreshness> {
        self.freshness.as_ref().filter(|freshness| freshness.stale)
    }
}

/// Independent Linear snapshot refresh worker.
pub struct TrackerWorker {
    /// Latest available Linear refresh results.
    pub events: Receiver<Result<TrackerRead, ix_board::Error>>,
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
        read: impl FnMut(bool) -> Result<TrackerRead, ix_board::Error> + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_with_read_and_intervals(
            read,
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        )
    }

    fn spawn_with_read_and_intervals(
        mut read: impl FnMut(bool) -> Result<TrackerRead, ix_board::Error> + Send + 'static,
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
        result: &Result<TrackerRead, ix_board::Error>,
        refresh: Duration,
        retry: Duration,
    ) -> Self {
        // Linear refills quota continuously; its full-window reset is not a
        // mandatory pause. Reattempt on the explicit dashboard polling cadence.
        // A retained snapshot counts only when the service observed a cooldown.
        let protected = match result {
            Err(
                ix_board::Error::LinearRateLimited { .. }
                | ix_board::Error::LinearLocalBudgetExhausted { .. },
            ) => true,
            Ok(read) => read
                .retained()
                .is_some_and(|freshness| freshness.rate_limit_until.is_some()),
            Err(_) => false,
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

fn read_tracker(force: bool) -> Result<TrackerRead, ix_board::Error> {
    read_tracker_with_factory(force, || {
        ReadService::open_default(LinearGraphQlSource::from_env()?)
    })
}

fn read_tracker_with_factory<S: ix_board::WorkspaceSource>(
    force: bool,
    factory: impl FnOnce() -> Result<ReadService<S>, ix_board::Error>,
) -> Result<TrackerRead, ix_board::Error> {
    let mut service = factory()?;
    let snapshot = if force {
        service.force_refresh_workspace()?
    } else {
        service.workspace_snapshot()?
    };
    Ok(TrackerRead {
        snapshot,
        freshness: service.cache_freshness().cloned(),
    })
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;
    use std::{sync::mpsc, thread, time::Duration};

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

    /// Fails its reads as the shared `mode` says: 0 succeeds, 1 is rate limited, 2 is a
    /// GraphQL failure.
    struct FlakySource {
        mode: std::sync::Arc<std::sync::atomic::AtomicU8>,
    }

    impl ix_board::WorkspaceSource for FlakySource {
        fn fetch_workspace(&self) -> Result<TrackerState, ix_board::Error> {
            match self.mode.load(std::sync::atomic::Ordering::Relaxed) {
                0 => Ok(TrackerState::default()),
                1 => Err(ix_board::Error::LinearRateLimited {
                    retry_after: Duration::from_secs(60),
                }),
                _ => Err(ix_board::Error::GraphQl {
                    messages: "upstream".into(),
                }),
            }
        }
    }

    /// The service answers a failed refresh with its last complete snapshot as a success and
    /// reports the failure only in its freshness. The dashboard must keep that snapshot's
    /// original source time, show the failure at once (the snapshot is minutes old, far
    /// under the thirty-minute stale label), also on a cold start, and drop the notice on the
    /// next successful read.
    ///
    /// Trace: FR-059-AC-5, FR-064-AC-10
    #[trace("FR-059-AC-5", "FR-064-AC-10")]
    #[test]
    fn a_snapshot_kept_after_a_failed_refresh_shows_the_failure_and_its_original_time() {
        use std::sync::{Arc, atomic::AtomicU8, atomic::Ordering};

        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("tracker.sqlite");
        let mode = Arc::new(AtomicU8::new(0));
        let read = |force: bool| {
            read_tracker_with_factory(force, || {
                ReadService::open(
                    FlakySource {
                        mode: Arc::clone(&mode),
                    },
                    &cache_path,
                )
            })
            .unwrap()
        };
        let now = std::time::Instant::now();

        let good = read(false);
        assert!(
            good.retained().is_none(),
            "a read that succeeded is not retained"
        );
        let original = good.snapshot.fetched_at;
        assert!(original > 0);
        let mut app = crate::monitor::App::default();
        app.apply_tracker_refresh(Ok(good), now);
        assert!(app.tracker.error.is_none());

        // A forced refresh that Linear rate limits returns the same snapshot, stale.
        mode.store(1, Ordering::Relaxed);
        let limited = read(true);
        assert_eq!(limited.snapshot.fetched_at, original);
        let freshness = limited.retained().expect("the failed refresh is reported");
        assert!(freshness.rate_limit_until.is_some());
        assert!(freshness.age_seconds.is_some_and(|age| age < 60));
        app.apply_tracker_refresh(Ok(limited), now);
        assert_eq!(app.tracker.value.as_ref().unwrap().fetched_at, original);
        assert!(
            app.tracker
                .error
                .as_deref()
                .unwrap()
                .contains("showing the last snapshot")
        );
        assert_eq!(
            app.tracker_retry_at,
            now.checked_add(TRACKER_REFRESH_INTERVAL)
        );
        assert!(app.collection_failed);

        // Any other failure keeps the snapshot and the notice but names no cooldown.
        mode.store(2, Ordering::Relaxed);
        let failed = read(true);
        assert_eq!(failed.snapshot.fetched_at, original);
        assert!(
            failed
                .retained()
                .is_some_and(|freshness| freshness.rate_limit_until.is_none())
        );
        let wait = RefreshWait::after_read(
            &Ok(failed),
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        );
        assert!(!wait.protected, "no known cooldown, no coalescing");
        let failed = read(true);
        let mut cold = crate::monitor::App::default();
        cold.apply_tracker_refresh(Ok(failed), now);
        assert_eq!(cold.tracker.value.as_ref().unwrap().fetched_at, original);
        let error = cold.tracker.error.as_deref().unwrap();
        assert!(error.contains("Linear refresh failed"), "{error}");
        assert!(!error.contains("upstream"), "the raw cause is not shown");
        assert!(cold.tracker_retry_at.is_none());
        mode.store(1, Ordering::Relaxed);
        let limited_wait = RefreshWait::after_read(
            &Ok(read(true)),
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        );
        assert!(
            limited_wait.protected,
            "a known cooldown coalesces refreshes"
        );

        // The next successful read clears the notice and the countdown.
        mode.store(0, Ordering::Relaxed);
        let recovered = read(true);
        assert!(recovered.retained().is_none());
        app.apply_tracker_refresh(Ok(recovered), now);
        assert!(app.tracker.error.is_none());
        assert!(app.tracker_retry_at.is_none());
    }

    /// Trace: FR-059-AC-6
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

    #[test]
    fn stopping_does_not_wait_for_an_in_flight_linear_read() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (read_finished_tx, read_finished_rx) = mpsc::channel();
        let worker = TrackerWorker::spawn_with_read(move |_| {
            entered_tx.send(()).unwrap();
            let _ = release_rx.recv();
            read_finished_tx.send(()).unwrap();
            Err(ix_board::Error::MissingApiKey)
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
        });
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
            &Err(ix_board::Error::MissingApiKey),
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        );
        assert_eq!(wait.delay, Duration::from_secs(1800));
        assert!(!wait.protected);
    }

    /// The shared local request budget refuses a read before any request is sent. It is
    /// protected like a server rate limit: manual refreshes coalesce, the countdown shows,
    /// and the failure is not the generic "tracker read failed" line.
    ///
    /// Trace: FR-048-AC-5
    #[test]
    fn local_budget_exhaustion_is_protected_like_a_rate_limit() {
        let exhausted = || ix_board::Error::LinearLocalBudgetExhausted {
            retry_after: Duration::from_secs(60),
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
            Err(ix_board::Error::MissingApiKey)
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
