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
    let source = LinearGraphQlSource::from_env()?;
    let mut service = ReadService::open_default(source)?;
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
