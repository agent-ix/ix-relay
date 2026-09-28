// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Background read-only refresh work for the Herdr activity monitor.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::monitor::{BusAgent, BusMessage, BusState, HerdrResult, HerdrState, display_text};
use crate::monitor_tracker::{TrackerState, summarize};
use herdr_cli::{RealProcessRunner, call};
use ix_board::{LinearGraphQlSource, ReadService};
use relay_core::{AgentList, Relay, SkippedRegistration, config::CoreSettings};

/// Interval between refresh attempts.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
/// Maximum time spent waiting on the Herdr CLI.
const HERDR_TIMEOUT: Duration = Duration::from_secs(3);
/// Maximum time spent on a Redis connection and read.
const REDIS_TIMEOUT: Duration = Duration::from_secs(2);
/// Linear tracker snapshots refresh less often than local activity sources.
const TRACKER_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// Limit retries after Linear or its cached workspace read returns an error.
const TRACKER_MAX_RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// How far back to read the capped Redis message log.
const MESSAGE_WINDOW_MS: i64 = 60 * 60 * 1000;
/// Number of recent bus messages shown at most.
const MAX_MESSAGES: usize = 24;

/// One refresh result. Each source reports independently so one failure does
/// not erase the other source's last good state.
pub struct Refresh {
    /// Local Herdr snapshot result.
    pub herdr: Result<HerdrState, String>,
    /// Relay registry and activity result.
    pub bus: Result<BusState, String>,
}

/// Background refresh channel and worker.
pub struct Worker {
    /// Nonblocking event receiver for the UI thread.
    pub events: Receiver<Refresh>,
    commands: SyncSender<WorkerCommand>,
    stop_requested: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

/// Independent Linear snapshot refresh worker.
pub struct TrackerWorker {
    /// Latest available Linear refresh results.
    pub events: Receiver<Result<TrackerState, String>>,
    commands: SyncSender<TrackerCommand>,
    stop_requested: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

enum WorkerCommand {
    Refresh,
    Stop,
}

enum TrackerCommand {
    Refresh,
    Stop,
}

impl Worker {
    /// Starts the refresh worker, which may block on external commands and
    /// Redis but never on the terminal or input thread.
    pub fn spawn(settings: CoreSettings, pane_id: Option<String>) -> std::io::Result<Self> {
        Self::spawn_with_refresh(move || refresh(&settings, pane_id.as_deref()))
    }

    fn spawn_with_refresh(
        mut refresh: impl FnMut() -> Refresh + Send + 'static,
    ) -> std::io::Result<Self> {
        let (event_tx, events) = mpsc::sync_channel(1);
        let (commands, command_rx) = mpsc::sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop_requested = Arc::clone(&stop_requested);
        let handle = thread::Builder::new()
            .name("herdr-monitor-refresh".to_owned())
            .spawn(move || {
                loop {
                    if worker_stop_requested.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = event_tx.try_send(refresh());
                    if worker_stop_requested.load(Ordering::Acquire) {
                        break;
                    }
                    match command_rx.recv_timeout(REFRESH_INTERVAL) {
                        Ok(WorkerCommand::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {
                            if worker_stop_requested.load(Ordering::Acquire) {
                                break;
                            }
                        }
                        Ok(WorkerCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break;
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

    /// Requests an early next refresh without blocking the UI thread.
    pub fn refresh_now(&self) {
        let _ = self.commands.try_send(WorkerCommand::Refresh);
    }

    /// Stops the worker and waits for its bounded in-flight read to finish.
    pub fn stop(self) {
        self.stop_requested.store(true, Ordering::Release);
        let _ = self.commands.try_send(WorkerCommand::Stop);
        let _ = self.handle.join();
    }
}

impl TrackerWorker {
    /// Starts Linear refreshes on a worker separate from Herdr and Redis.
    pub fn spawn() -> std::io::Result<Self> {
        Self::spawn_with_read(read_tracker)
    }

    fn spawn_with_read(
        read: impl FnMut() -> Result<TrackerState, String> + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_with_read_and_intervals(
            read,
            TRACKER_REFRESH_INTERVAL,
            TRACKER_MAX_RETRY_INTERVAL,
        )
    }

    fn spawn_with_read_and_intervals(
        mut read: impl FnMut() -> Result<TrackerState, String> + Send + 'static,
        refresh_interval: Duration,
        max_retry_interval: Duration,
    ) -> std::io::Result<Self> {
        let (event_tx, events) = mpsc::sync_channel(1);
        let (commands, command_rx) = mpsc::sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop_requested = Arc::clone(&stop_requested);
        let handle = thread::Builder::new()
            .name("herdr-monitor-linear-refresh".to_owned())
            .spawn(move || {
                let mut retry_interval = refresh_interval;
                loop {
                    if worker_stop_requested.load(Ordering::Acquire) {
                        break;
                    }
                    let result = read();
                    retry_interval = if result.is_ok() {
                        refresh_interval
                    } else {
                        retry_interval
                            .saturating_mul(2)
                            .min(max_retry_interval.max(refresh_interval))
                    };
                    let _ = event_tx.try_send(result);
                    if worker_stop_requested.load(Ordering::Acquire) {
                        break;
                    }
                    match command_rx.recv_timeout(retry_interval) {
                        Ok(TrackerCommand::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {
                            if worker_stop_requested.load(Ordering::Acquire) {
                                break;
                            }
                        }
                        Ok(TrackerCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break;
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

    /// Stops the worker and waits for any bounded in-flight request.
    pub fn stop(self) {
        self.stop_requested.store(true, Ordering::Release);
        let _ = self.commands.try_send(TrackerCommand::Stop);
        let _ = self.handle.join();
    }
}

fn refresh(settings: &CoreSettings, pane_id: Option<&str>) -> Refresh {
    Refresh {
        herdr: read_herdr(pane_id),
        bus: read_bus(settings),
    }
}

fn read_tracker() -> Result<TrackerState, String> {
    let source = LinearGraphQlSource::from_env().map_err(|error| error.to_string())?;
    let cache_path = tracker_cache_path()?;
    let mut service = ReadService::open(source, &cache_path).map_err(|error| error.to_string())?;
    let snapshot = service
        .workspace_snapshot()
        .map_err(|error| error.to_string())?;
    let queue = service.next("SWM").map_err(|error| error.to_string())?;
    Ok(summarize(&snapshot, &queue))
}

fn tracker_cache_path() -> Result<std::path::PathBuf, String> {
    if let Some(path) = std::env::var_os("IX_BOARD_CACHE_PATH") {
        return Ok(path.into());
    }
    let project_dirs = directories::ProjectDirs::from("com", "agent-ix", "ix-board")
        .ok_or_else(|| "ix-board cache directory is unavailable".to_owned())?;
    std::fs::create_dir_all(project_dirs.cache_dir()).map_err(|error| error.to_string())?;
    Ok(project_dirs.cache_dir().join("linear.sqlite3"))
}

fn read_herdr(pane_id: Option<&str>) -> Result<HerdrState, String> {
    let result: HerdrResult = call(
        &RealProcessRunner,
        "herdr",
        &["api", "snapshot"],
        HERDR_TIMEOUT,
    )
    .map_err(|error| error.to_string())?;
    Ok(result.into_state(pane_id))
}

fn read_bus(settings: &CoreSettings) -> Result<BusState, String> {
    let mut relay = Relay::connect(&settings.url.value, REDIS_TIMEOUT)
        .map_err(|error| error.to_string())?
        .with_prefix(settings.prefix.value.clone());
    let listed = relay.agents().map_err(|error| error.to_string())?;
    let mut state = map_agent_list(listed);
    state.messages = relay
        .log(unix_ms().saturating_sub(MESSAGE_WINDOW_MS), MAX_MESSAGES)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|message| BusMessage {
            from: message.from.to_string(),
            text: display_text(&message.text, 256),
            sent_at_ms: message.sent_at_ms,
        })
        .collect();
    Ok(state)
}

fn map_agent_list(listed: AgentList) -> BusState {
    let AgentList {
        registrations,
        skipped,
    } = listed;
    let registrations = registrations
        .into_iter()
        .map(|registration| {
            let place = registration
                .verified_place()
                .map(|place| format!("{}/{}", place.workspace, place.tab));
            BusAgent {
                name: registration.name.to_string(),
                kind: display_text(&registration.kind, 20),
                machine: registration.machine.to_string(),
                place,
                cwd: registration.cwd.map(|cwd| display_text(&cwd, 120)),
                last_seen_ms: registration.last_seen_ms,
            }
        })
        .collect();
    let skipped_registrations = skipped_labels(skipped);
    BusState {
        registrations,
        skipped_registrations,
        messages: Vec::new(),
    }
}

fn skipped_labels(skipped: Vec<SkippedRegistration>) -> Vec<String> {
    skipped
        .into_iter()
        .map(|record| display_text(&format!("{}: {}", record.field, record.error), 100))
        .collect()
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Instant,
    };

    use super::*;

    #[test]
    fn partial_registry_decode_failures_are_retained_for_the_monitor() {
        let skipped = vec![SkippedRegistration {
            field: "bad-entry".to_owned(),
            error: relay_core::Error::InvalidName {
                name: "not valid".to_owned(),
            },
            raw: br#"{"name":7}"#.to_vec(),
        }];

        let state = map_agent_list(AgentList {
            registrations: Vec::new(),
            skipped,
        });
        assert_eq!(state.skipped_registrations.len(), 1);
        assert!(state.skipped_registrations[0].starts_with("bad-entry:"));
        assert!(!state.skipped_registrations[0].contains('\u{1b}'));
    }

    #[test]
    fn slow_linear_refresh_does_not_delay_local_source_updates() {
        let (linear_entered_tx, linear_entered_rx) = mpsc::channel();
        let (release_linear_tx, release_linear_rx) = mpsc::channel();
        let tracker = TrackerWorker::spawn_with_read(move || {
            linear_entered_tx.send(()).unwrap();
            release_linear_rx.recv().unwrap();
            Err("offline".to_owned())
        })
        .unwrap();
        let local = Worker::spawn_with_refresh(|| Refresh {
            herdr: Err("herdr offline".to_owned()),
            bus: Err("bus offline".to_owned()),
        })
        .unwrap();

        let tracker_started = linear_entered_rx.recv_timeout(Duration::from_secs(2));
        let local_update = local.events.recv_timeout(Duration::from_secs(2));
        let _ = release_linear_tx.send(());
        tracker.stop();
        local.stop();

        assert!(tracker_started.is_ok());
        assert!(local_update.is_ok());
    }

    #[test]
    fn failed_linear_refreshes_back_off_and_obey_the_configured_cap() {
        let (calls_tx, calls_rx) = mpsc::channel();
        let mut call_number = 0;
        let worker = TrackerWorker::spawn_with_read_and_intervals(
            move || {
                call_number += 1;
                calls_tx.send(call_number).unwrap();
                Err("Linear rate limited".to_owned())
            },
            Duration::from_millis(100),
            Duration::from_millis(200),
        )
        .unwrap();

        assert_eq!(calls_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
        assert!(calls_rx.recv_timeout(Duration::from_millis(120)).is_err());
        assert_eq!(
            calls_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            2
        );
        assert!(calls_rx.recv_timeout(Duration::from_millis(120)).is_err());
        assert_eq!(
            calls_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            3
        );
        worker.stop();
    }

    #[test]
    fn manual_refresh_runs_again_after_a_slow_read() {
        let (calls_tx, calls_rx) = mpsc::channel();
        let mut call_number = 0;
        let worker = Worker::spawn_with_refresh(move || {
            call_number += 1;
            calls_tx.send(call_number).unwrap();
            Refresh {
                herdr: Err("offline".to_owned()),
                bus: Err("offline".to_owned()),
            }
        })
        .unwrap();

        assert_eq!(calls_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        worker.refresh_now();
        assert_eq!(calls_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
        worker.stop();
    }

    #[test]
    fn slow_read_does_not_block_ui_key_handling_or_lose_shutdown() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let worker = Worker::spawn_with_refresh(move || {
            let call = worker_calls.fetch_add(1, Ordering::Relaxed);
            assert_eq!(call, 0, "queued refresh must be canceled by shutdown");
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Refresh {
                herdr: Err("offline".to_owned()),
                bus: Err("offline".to_owned()),
            }
        })
        .unwrap();

        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.refresh_now();

        // The caller can process input while the injected refresh is still blocked.
        let mut app = crate::monitor::App::default();
        crate::monitor::handle_key(
            &mut app,
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('q'),
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert!(app.should_quit);

        let stop_requested = Arc::clone(&worker.stop_requested);
        let (done_tx, done_rx) = mpsc::channel();
        let stopper = thread::spawn(move || {
            worker.stop();
            done_tx.send(()).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while !stop_requested.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        release_tx.send(()).unwrap();
        assert!(stop_requested.load(Ordering::Acquire));
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        stopper.join().unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
