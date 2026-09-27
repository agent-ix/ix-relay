// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Background read-only refresh work for the Herdr activity monitor.

use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::monitor::{BusAgent, BusMessage, BusState, HerdrResult, HerdrState, display_text};
use herdr_cli::{RealProcessRunner, call};
use relay_core::{Relay, config::CoreSettings};

/// Interval between refresh attempts.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
/// Maximum time spent waiting on the Herdr CLI.
const HERDR_TIMEOUT: Duration = Duration::from_secs(3);
/// Maximum time spent on a Redis connection and read.
const REDIS_TIMEOUT: Duration = Duration::from_secs(2);
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
    handle: JoinHandle<()>,
}

enum WorkerCommand {
    Refresh,
    Stop,
}

impl Worker {
    /// Starts the refresh worker, which may block on external commands and
    /// Redis but never on the terminal or input thread.
    pub fn spawn(settings: CoreSettings, pane_id: Option<String>) -> std::io::Result<Self> {
        let (event_tx, events) = mpsc::sync_channel(1);
        let (commands, command_rx) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name("herdr-monitor-refresh".to_owned())
            .spawn(move || {
                loop {
                    let _ = event_tx.try_send(refresh(&settings, pane_id.as_deref()));
                    match command_rx.recv_timeout(REFRESH_INTERVAL) {
                        Ok(WorkerCommand::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Ok(WorkerCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            events,
            commands,
            handle,
        })
    }

    /// Requests an early next refresh without blocking the UI thread.
    pub fn refresh_now(&self) {
        let _ = self.commands.try_send(WorkerCommand::Refresh);
    }

    /// Stops the worker and waits for its bounded in-flight read to finish.
    pub fn stop(self) {
        let _ = self.commands.try_send(WorkerCommand::Stop);
        let _ = self.handle.join();
    }
}

fn refresh(settings: &CoreSettings, pane_id: Option<&str>) -> Refresh {
    Refresh {
        herdr: read_herdr(pane_id),
        bus: read_bus(settings),
    }
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
    let registrations = listed
        .registrations
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
    let messages = relay
        .log(unix_ms().saturating_sub(MESSAGE_WINDOW_MS), MAX_MESSAGES)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|message| BusMessage {
            from: message.from.to_string(),
            text: display_text(&message.text, 256),
            sent_at_ms: message.sent_at_ms,
        })
        .collect();
    Ok(BusState {
        registrations,
        messages,
    })
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}
