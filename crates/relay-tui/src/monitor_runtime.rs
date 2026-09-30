// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Independently scheduled, bounded Herdr and reporting refreshes.

use relay_core::{
    Relay,
    work::{WorkBinding, WorkReport},
};
use relay_ops::{
    OpsError,
    work::{WorkObservation, observe_work_binding},
};
use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

/// Herdr observations and explicit Redis reports refresh once a minute.
pub const RUNTIME_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// One source result with its actual read completion time.
pub enum RuntimeRefresh {
    /// Measured Herdr activity or a typed observation failure.
    Observation(Result<WorkObservation, OpsError>, Instant),
    /// Explicit reports or a typed store failure.
    Report(Result<Option<WorkReport>, relay_core::Error>, Instant),
}

/// Independent background readers, with two queued results and responsive shutdown.
pub struct RuntimeWorker {
    /// Results delivered without silently dropping updates.
    pub events: Receiver<RuntimeRefresh>,
    stops: Vec<SyncSender<()>>,
}

fn spawn_reader(
    name: &str,
    events: SyncSender<RuntimeRefresh>,
    read: impl Fn() -> RuntimeRefresh + Send + 'static,
) -> std::io::Result<SyncSender<()>> {
    let (stop, stopped) = mpsc::sync_channel(1);
    // Each blocking read has a transport timeout. Detach on UI shutdown;
    // dropping events unblocks a pending send, and the stop channel prevents
    // another read. No joins on the terminal input thread.
    thread::Builder::new().name(name.into()).spawn(move || {
        loop {
            if stopped.try_recv().is_ok() {
                break;
            }
            if events.send(read()).is_err() {
                break;
            }
            match stopped.recv_timeout(RUNTIME_REFRESH_INTERVAL) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    })?;
    Ok(stop)
}

impl RuntimeWorker {
    /// Starts separate Herdr and Redis polls off the UI thread.
    pub fn spawn(
        binding: WorkBinding,
        settings: relay_core::config::CoreSettings,
    ) -> std::io::Result<Self> {
        let (event_tx, events) = mpsc::sync_channel(2);
        let observed_binding = binding.clone();
        let machine = settings.machine.value;
        let herdr = spawn_reader("dashboard-herdr-refresh", event_tx.clone(), move || {
            let result =
                observe_work_binding(&herdr_cli::RealProcessRunner, &observed_binding, &machine);
            RuntimeRefresh::Observation(result, Instant::now())
        })?;
        let reports = spawn_reader("dashboard-report-refresh", event_tx, move || {
            let result =
                Relay::connect(&settings.url.value, Duration::from_secs(3)).and_then(|relay| {
                    relay
                        .with_prefix(&settings.prefix.value)
                        .work_report(&binding)
                });
            RuntimeRefresh::Report(result, Instant::now())
        })?;
        Ok(Self {
            events,
            stops: vec![herdr, reports],
        })
    }

    /// Stops future observations; dropping the receiver unblocks pending sends.
    pub fn stop(self) {
        for stop in self.stops {
            let _ = stop.try_send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trace: FR-048-AC-5
    #[test]
    fn report_refresh_arrives_while_herdr_read_is_blocked() {
        let (events, received) = mpsc::sync_channel(2);
        let (entered, started) = mpsc::sync_channel(1);
        let (release, released) = mpsc::sync_channel(1);
        let (finished, done) = mpsc::sync_channel(1);
        let herdr = spawn_reader("test-herdr-read", events.clone(), move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
            finished.send(()).unwrap();
            RuntimeRefresh::Observation(Err(OpsError::WorkTargetUnavailable), Instant::now())
        })
        .unwrap();
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let reports = spawn_reader("test-report-read", events, || {
            RuntimeRefresh::Report(Ok(None), Instant::now())
        })
        .unwrap();
        let result = received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(result, RuntimeRefresh::Report(Ok(None), _)));
        herdr.send(()).unwrap();
        reports.send(()).unwrap();
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}
