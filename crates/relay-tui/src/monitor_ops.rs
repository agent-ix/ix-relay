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

/// Independent Linear snapshot refresh worker.
pub struct TrackerWorker {
    /// Latest available Linear refresh results.
    pub events: Receiver<Result<TrackerState, ix_board::Error>>,
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

    fn spawn_with_read(
        read: impl FnMut(bool) -> Result<TrackerState, ix_board::Error> + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_with_read_and_intervals(
            read,
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        )
    }

    fn spawn_with_read_and_intervals(
        mut read: impl FnMut(bool) -> Result<TrackerState, ix_board::Error> + Send + 'static,
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
        result: &Result<TrackerState, ix_board::Error>,
        refresh: Duration,
        retry: Duration,
    ) -> Self {
        // Linear refills quota continuously; its full-window reset is not a
        // mandatory pause. Reattempt on the explicit dashboard polling cadence.
        let protected = matches!(result, Err(ix_board::Error::LinearRateLimited { .. }));
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

fn read_tracker(force: bool) -> Result<TrackerState, ix_board::Error> {
    read_tracker_with_factory(force, || {
        ReadService::open_default(LinearGraphQlSource::from_env()?)
    })
}

fn read_tracker_with_factory<S: ix_board::WorkspaceSource>(
    force: bool,
    factory: impl FnOnce() -> Result<ReadService<S>, ix_board::Error>,
) -> Result<TrackerState, ix_board::Error> {
    let mut service = factory()?;
    if force {
        service.force_refresh_workspace()
    } else {
        service.workspace_snapshot()
    }
}

#[cfg(test)]
mod tests {
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

    /// Trace: FR-059-AC-6
    #[test]
    fn injected_saved_credential_factory_succeeds_without_an_environment_override() {
        let factory_calls = std::cell::Cell::new(0);
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("tracker.sqlite");
        let snapshot = read_tracker_with_factory(false, || {
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
        app.apply_tracker_refresh(Ok(snapshot), std::time::Instant::now());
        assert!(app.tracker.value.is_some());
        assert!(app.tracker.error.is_none());
        assert!(!app.collection_failed);
    }

    /// Trace: FR-059-AC-6
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
