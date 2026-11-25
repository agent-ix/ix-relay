// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Read-only shared report access, outside the terminal input thread (FR-059).

use ix_board::retro::{self, department::Delta};
use std::{
    io,
    path::Path,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Duration,
};

/// Opens supported shared snapshot JSON, history JSONL or delta JSON without collectors.
pub(crate) fn read_report(path: &Path) -> io::Result<Delta> {
    let report = retro::read_saved_report(path, None)
        .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?;
    match report {
        retro::SavedReport::Record(record) => record
            .department
            .map(|current| retro::department::delta(&current, None, None)),
        retro::SavedReport::Delta(report) => report.department,
    }
    .ok_or_else(|| {
        io::Error::other(format!(
            "{}: department report unavailable in saved input",
            path.display()
        ))
    })
}

/// Validates policy scope without accessing live sources or changing configured populations.
pub(crate) fn load_config(
    path: &Path,
    team: Option<&str>,
    projects: &[String],
) -> io::Result<retro::RetroConfig> {
    let config = retro::RetroConfig::load(path).map_err(io::Error::other)?;
    let scope = &config
        .department
        .as_ref()
        .ok_or_else(|| io::Error::other("--retro-config must contain a department configuration"))?
        .scope;
    let configured_teams: std::collections::BTreeSet<_> = scope
        .teams
        .iter()
        .map(|s| s.id.to_ascii_uppercase())
        .collect();
    let selected_teams: std::collections::BTreeSet<_> =
        team.into_iter().map(str::to_ascii_uppercase).collect();
    let configured_projects: std::collections::BTreeSet<_> =
        scope.projects.iter().map(|s| s.id.as_str()).collect();
    let selected_projects: std::collections::BTreeSet<_> =
        projects.iter().map(String::as_str).collect();
    if configured_teams != selected_teams || configured_projects != selected_projects {
        return Err(io::Error::other(
            "--retro-config department team/project population must exactly match --team and --project selectors",
        ));
    }
    Ok(config)
}

/// One independently refreshed shared department collection.
pub(crate) struct DepartmentWorker {
    /// Bounded results from the shared collector.
    pub(crate) events: Receiver<Result<Delta, String>>,
    commands: SyncSender<()>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl DepartmentWorker {
    /// Uses existing approved SDK source factories; no retro history is appended.
    pub(crate) fn spawn(config: retro::RetroConfig) -> io::Result<Self> {
        Self::spawn_with_read(move || collect(&config))
    }

    pub(crate) fn spawn_with_read(
        mut read: impl FnMut() -> Result<Delta, String> + Send + 'static,
    ) -> io::Result<Self> {
        let (events_tx, events) = mpsc::sync_channel(1);
        let (commands, commands_rx) = mpsc::sync_channel(1);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_stop = std::sync::Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("department-report-refresh".into())
            .spawn(move || {
                while !worker_stop.load(std::sync::atomic::Ordering::Acquire) {
                    let result = read();
                    // The queue bounds pending results. Dropping the UI receiver on
                    // shutdown releases a blocked send without waiting for the source.
                    if events_tx.send(result).is_err() {
                        break;
                    }
                    if commands_rx
                        .recv_timeout(Duration::from_secs(30 * 60))
                        .is_err_and(|error| error == mpsc::RecvTimeoutError::Disconnected)
                    {
                        break;
                    }
                }
            })?;
        // The worker's stop flag and bounded channel own shutdown; an in-flight
        // SDK request is deliberately detached so it cannot block terminal quit.
        drop(handle);
        Ok(Self {
            events,
            commands,
            stop,
        })
    }

    /// Coalesces manual refresh requests without blocking terminal input.
    pub(crate) fn refresh_now(&self) {
        let _ = self.commands.try_send(());
    }
}

impl Drop for DepartmentWorker {
    fn drop(&mut self) {
        // The read can be blocked in an SDK request. Shutdown never joins it.
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let _ = self.commands.try_send(());
    }
}

fn collect(config: &retro::RetroConfig) -> Result<Delta, String> {
    let linear = retro::live::LiveLinear::from_env()
        .map_err(|error| crate::monitor_ops::source_diagnostic(&error))?;
    let agents = retro::live::LiveAgents::start(config.herdr_teams.clone());
    let sources = retro::Sources {
        linear: &linear,
        github: &retro::live::LiveGithub,
        agents: &agents,
        gate_log: &retro::live::FileGateLog,
        relay: &retro::live::RedisCliRelay,
        local: &retro::live::SystemLocal,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock is before the UTC epoch")?
        .as_secs_f64();
    let frame = retro::Frame {
        now,
        window_secs: retro::parse_window(retro::DEFAULT_WINDOW)
            .map_err(|error| error.to_string())?,
        end_exclusive: false,
    };
    collect_from_sources(config, frame, &sources)
}

fn collect_from_sources(
    config: &retro::RetroConfig,
    frame: retro::Frame,
    sources: &retro::Sources<'_>,
) -> Result<Delta, String> {
    let record = retro::collect_snapshot(config, frame, sources, None, None)
        .map_err(|error| error.to_string())?;
    let current = record
        .department
        .ok_or("department configuration unavailable")?;
    Ok(retro::department::delta(&current, None, None))
}

#[cfg(test)]
mod auth_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn fixture_record() -> retro::Record {
        retro::Record {
            schema_version: 2,
            kind: retro::RecordKind::Snapshot,
            taken_at: "2026-11-19T00:00:00Z".into(),
            team: "SWM".into(),
            window: "24h".into(),
            metrics: Default::default(),
            department: Some(crate::monitor_department::tests::fixture().current),
        }
    }

    /// Trace: FR-059-AC-2, FR-059-AC-3
    #[trace("FR-059-AC-2", "FR-059-AC-3")]
    #[test]
    fn supplied_report_uses_shared_decoder_and_never_rewrites_input() {
        let path = std::env::temp_dir().join(format!(
            "swm165-report-{}-{}.json",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let record = fixture_record();
        let bytes = serde_json::to_vec(&record).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let report = read_report(&path).unwrap();
        assert_eq!(report.current, record.department.unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let missing_baseline = &report.measurements["wip_active"].since_baseline;
        assert_eq!(
            missing_baseline.state,
            retro::department::ComparisonState::Unavailable
        );
        assert!(missing_baseline.reference.is_none());
        for bytes in [
            b"not json".to_vec(),
            serde_json::to_vec(&retro::Record {
                schema_version: 900,
                ..fixture_record()
            })
            .unwrap(),
        ] {
            std::fs::write(&path, &bytes).unwrap();
            let error = read_report(&path).unwrap_err().to_string();
            assert!(error.contains(path.to_str().unwrap()));
            assert!(error.contains("invalid") || error.contains("unsupported"));
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::remove_file(path).unwrap();
    }

    /// Trace: FR-059-AC-6
    #[trace("FR-059-AC-6")]
    #[test]
    fn blocked_department_read_does_not_hold_quit() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = DepartmentWorker::spawn_with_read(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            finished_tx.send(()).unwrap();
            Err("fixture unavailable".into())
        })
        .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let stopper = thread::spawn(move || {
            drop(worker);
            stopped_tx.send(()).unwrap();
        });
        let stopped_before_release = stopped_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        release_tx.send(()).unwrap();
        stopper.join().unwrap();
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            stopped_before_release,
            "department quit waited for the blocked read"
        );
    }
}
