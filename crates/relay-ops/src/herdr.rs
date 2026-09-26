//! The slice of herdr's CLI `investigate` needs, behind a trait.
//!
//! [`RealHerdr`] shells out to the `herdr` binary via `herdr_cli`. A fake
//! implementing [`Herdr`] lets `investigate`'s command wiring be
//! unit-tested without a live herdr session (see `escalate.rs`'s `tests`
//! module). [`RealHerdr`] is itself generic over [`herdr_cli::ProcessRunner`]
//! so its own argv-building can be tested without spawning anything (see this
//! module's `tests`): production code uses [`RealHerdr::new`], which wires up
//! the real [`herdr_cli::RealProcessRunner`].

use std::time::Duration;

use herdr_cli::ProcessRunner;
use relay_core::{PaneId, TabId};
use serde::Deserialize;

pub use herdr_cli::ProcessError;

/// How long `tab_create`/`tab_close` wait before giving up. Generous for a
/// human-invoked command, not a hot loop a hang would repeatedly block.
const HERDR_TIMEOUT: Duration = Duration::from_secs(35);

/// How long `agent_start` asks herdr itself to wait for the forked or fresh
/// session to become interactive-ready, in milliseconds (herdr's own
/// `--timeout` flag; its compiled-in default is 30s, which is routinely too
/// short to resume/fork a Claude Code session on a machine already running
/// several other agents — see AGE-2015's live-test notes). 120s comfortably
/// covers that without approaching herdr's own 300s ceiling.
const AGENT_START_READY_TIMEOUT_MS: &str = "120000";

/// The external process-kill timeout for the `agent_start` call itself.
/// Must exceed [`AGENT_START_READY_TIMEOUT_MS`] (120s) — otherwise this
/// process would kill `herdr agent start` for taking "too long" before
/// herdr's own, longer readiness timeout ever gets to report a clean
/// `timeout` error itself.
const AGENT_START_TIMEOUT: Duration = Duration::from_secs(130);

/// A newly created tab: both ids `investigate` needs — the pane to start an
/// agent in, and the tab to close again if that start fails (see
/// [`Herdr::tab_close`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTab {
    /// The tab's root pane, where `agent_start` runs.
    pub pane_id: PaneId,
    /// The tab itself, for cleanup.
    pub tab_id: TabId,
}

/// The herdr operations `investigate` needs to open a session on a lead's
/// machine, in its cwd.
pub trait Herdr {
    /// `herdr [--machine <machine>] tab create --cwd <cwd> --label <label>
    /// --no-focus`.
    fn tab_create(
        &self,
        machine: Option<&str>,
        cwd: &str,
        label: &str,
    ) -> Result<CreatedTab, ProcessError>;
    /// `herdr [--machine <machine>] agent start <name> --kind <kind> --pane
    /// <pane> [-- <args>...]`. Starts an interactive agent in an existing pane.
    fn agent_start(
        &self,
        machine: Option<&str>,
        name: &str,
        kind: &str,
        pane: &PaneId,
        args: &[&str],
    ) -> Result<(), ProcessError>;
    /// `herdr [--machine <machine>] tab close <tab_id>`. Used to clean up a
    /// tab `tab_create` opened when the `agent_start` that was meant to fill
    /// it fails, so a failed `investigate` doesn't leave a dangling empty tab
    /// on the lead's machine.
    fn tab_close(&self, machine: Option<&str>, tab_id: &TabId) -> Result<(), ProcessError>;
}

#[derive(Deserialize)]
struct TabCreateResult {
    tab: TabRef,
    root_pane: RootPane,
}

#[derive(Deserialize)]
struct TabRef {
    tab_id: TabId,
}

#[derive(Deserialize)]
struct RootPane {
    pane_id: PaneId,
}

/// Shells out to the real `herdr` binary on `PATH`, through `R`
/// ([`herdr_cli::RealProcessRunner`] in production; a recording fake in this
/// module's own tests, to check the exact argv this type builds).
pub struct RealHerdr<R: ProcessRunner = herdr_cli::RealProcessRunner> {
    runner: R,
}

impl RealHerdr<herdr_cli::RealProcessRunner> {
    /// The production `RealHerdr`, spawning real `herdr` calls.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runner: herdr_cli::RealProcessRunner,
        }
    }
}

impl Default for RealHerdr<herdr_cli::RealProcessRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ProcessRunner> RealHerdr<R> {
    /// Argv for `tab create`, *before* the optional `--machine` prefix
    /// (applied once, at the call site, via [`herdr_cli::with_machine`]).
    fn tab_create_args<'a>(cwd: &'a str, label: &'a str) -> [&'a str; 7] {
        [
            "tab",
            "create",
            "--cwd",
            cwd,
            "--label",
            label,
            "--no-focus",
        ]
    }

    fn agent_start_args<'a>(
        name: &'a str,
        kind: &'a str,
        pane: &'a str,
        args: &[&'a str],
    ) -> Vec<&'a str> {
        let mut full = vec![
            "agent",
            "start",
            name,
            "--kind",
            kind,
            "--pane",
            pane,
            "--timeout",
            AGENT_START_READY_TIMEOUT_MS,
        ];
        if !args.is_empty() {
            full.push("--");
            full.extend_from_slice(args);
        }
        full
    }

    fn tab_close_args(tab_id: &str) -> Vec<&str> {
        vec!["tab", "close", tab_id]
    }
}

impl<R: ProcessRunner> Herdr for RealHerdr<R> {
    fn tab_create(
        &self,
        machine: Option<&str>,
        cwd: &str,
        label: &str,
    ) -> Result<CreatedTab, ProcessError> {
        let inner = Self::tab_create_args(cwd, label);
        let args = herdr_cli::with_machine(machine, &inner);
        let result: TabCreateResult = herdr_cli::call(&self.runner, "herdr", &args, HERDR_TIMEOUT)?;
        Ok(CreatedTab {
            pane_id: result.root_pane.pane_id,
            tab_id: result.tab.tab_id,
        })
    }

    fn agent_start(
        &self,
        machine: Option<&str>,
        name: &str,
        kind: &str,
        pane: &PaneId,
        args: &[&str],
    ) -> Result<(), ProcessError> {
        let built = Self::agent_start_args(name, kind, pane.as_str(), args);
        let full = herdr_cli::with_machine(machine, &built);
        herdr_cli::call::<serde_json::Value>(&self.runner, "herdr", &full, AGENT_START_TIMEOUT)
            .map(|_| ())
    }

    fn tab_close(&self, machine: Option<&str>, tab_id: &TabId) -> Result<(), ProcessError> {
        let built = Self::tab_close_args(tab_id.as_str());
        let full = herdr_cli::with_machine(machine, &built);
        herdr_cli::call::<serde_json::Value>(&self.runner, "herdr", &full, HERDR_TIMEOUT)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use herdr_cli::Output;

    use super::*;

    #[test]
    fn tab_create_args_places_cwd_and_label() {
        assert_eq!(
            RealHerdr::<herdr_cli::RealProcessRunner>::tab_create_args("/work", "esc-e7"),
            [
                "tab",
                "create",
                "--cwd",
                "/work",
                "--label",
                "esc-e7",
                "--no-focus"
            ]
        );
    }

    #[test]
    fn agent_start_args_include_the_double_dash_separator_only_with_args() {
        assert_eq!(
            RealHerdr::<herdr_cli::RealProcessRunner>::agent_start_args(
                "esc-e7-1",
                "claude",
                "w1:p2",
                &[]
            ),
            vec![
                "agent",
                "start",
                "esc-e7-1",
                "--kind",
                "claude",
                "--pane",
                "w1:p2",
                "--timeout",
                "120000"
            ]
        );
        assert_eq!(
            RealHerdr::<herdr_cli::RealProcessRunner>::agent_start_args(
                "esc-e7-1",
                "claude",
                "w1:p2",
                &["--resume", "sid-1", "--fork-session"],
            ),
            vec![
                "agent",
                "start",
                "esc-e7-1",
                "--kind",
                "claude",
                "--pane",
                "w1:p2",
                "--timeout",
                "120000",
                "--",
                "--resume",
                "sid-1",
                "--fork-session"
            ]
        );
    }

    struct RecordingRunner {
        calls: RefCell<Vec<(String, Vec<String>, Duration)>>,
        response: Output,
    }

    impl RecordingRunner {
        fn ok(json: &str) -> Self {
            Self {
                calls: RefCell::default(),
                response: Output {
                    status_code: 0,
                    success: true,
                    stdout: json.as_bytes().to_vec(),
                    stderr: Vec::new(),
                },
            }
        }
    }

    impl ProcessRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            timeout: Duration,
        ) -> Result<Output, ProcessError> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter().map(|s| (*s).to_owned()).collect(),
                timeout,
            ));
            Ok(self.response.clone())
        }
    }

    /// The exact argv `RealHerdr::tab_create` sends to the real `herdr`
    /// binary, for both the local (no `--machine`) and remote cases —
    /// exercising the same argv-building code the production path uses, with
    /// no process spawned.
    #[test]
    fn real_herdr_tab_create_sends_exactly_the_documented_argv_local_and_remote() {
        let runner = RecordingRunner::ok(
            r#"{"result":{"tab":{"tab_id":"w1:t2"},"root_pane":{"pane_id":"w1:p3"}}}"#,
        );
        let herdr = RealHerdr { runner };
        let created = herdr.tab_create(None, "/work/lead", "esc-e7").unwrap();
        assert_eq!(created.pane_id.as_str(), "w1:p3");
        assert_eq!(created.tab_id.as_str(), "w1:t2");
        assert_eq!(
            herdr.runner.calls.borrow()[0].1,
            vec![
                "tab",
                "create",
                "--cwd",
                "/work/lead",
                "--label",
                "esc-e7",
                "--no-focus"
            ]
        );

        let runner = RecordingRunner::ok(
            r#"{"result":{"tab":{"tab_id":"w1:t2"},"root_pane":{"pane_id":"w1:p3"}}}"#,
        );
        let herdr = RealHerdr { runner };
        herdr
            .tab_create(Some("hub"), "/work/lead", "esc-e7")
            .unwrap();
        assert_eq!(
            herdr.runner.calls.borrow()[0].1,
            vec![
                "--machine",
                "hub",
                "tab",
                "create",
                "--cwd",
                "/work/lead",
                "--label",
                "esc-e7",
                "--no-focus"
            ]
        );
    }

    #[test]
    fn real_herdr_agent_start_sends_exactly_the_documented_argv_and_a_longer_deadline() {
        let runner = RecordingRunner::ok(r#"{"result":{}}"#);
        let herdr = RealHerdr { runner };
        herdr
            .agent_start(
                None,
                "esc-e7-1",
                "claude",
                &"w1:p3".parse().unwrap(),
                &["--resume", "sid-1", "--fork-session"],
            )
            .unwrap();
        assert_eq!(
            herdr.runner.calls.borrow()[0].1,
            vec![
                "agent",
                "start",
                "esc-e7-1",
                "--kind",
                "claude",
                "--pane",
                "w1:p3",
                "--timeout",
                "120000",
                "--",
                "--resume",
                "sid-1",
                "--fork-session"
            ]
        );
        // Our own process-kill deadline must exceed the 120s we asked herdr
        // itself to wait, or we'd kill the call before herdr's own, longer
        // timeout ever gets a chance to report cleanly.
        assert_eq!(herdr.runner.calls.borrow()[0].2, AGENT_START_TIMEOUT);
        assert!(AGENT_START_TIMEOUT > Duration::from_millis(120_000));
    }

    /// `tab_create`/`tab_close` are unaffected by the longer `agent_start`
    /// deadline — they keep the original, shorter [`HERDR_TIMEOUT`].
    #[test]
    fn tab_create_and_tab_close_keep_the_original_shorter_timeout() {
        let runner = RecordingRunner::ok(
            r#"{"result":{"tab":{"tab_id":"w1:t2"},"root_pane":{"pane_id":"w1:p3"}}}"#,
        );
        let herdr = RealHerdr { runner };
        herdr.tab_create(None, "/work", "esc-e7").unwrap();
        assert_eq!(herdr.runner.calls.borrow()[0].2, HERDR_TIMEOUT);

        let runner = RecordingRunner::ok(r#"{"result":{}}"#);
        let herdr = RealHerdr { runner };
        herdr.tab_close(None, &"w1:t2".parse().unwrap()).unwrap();
        assert_eq!(herdr.runner.calls.borrow()[0].2, HERDR_TIMEOUT);
    }

    #[test]
    fn real_herdr_tab_close_sends_exactly_the_documented_argv() {
        let runner = RecordingRunner::ok(r#"{"result":{}}"#);
        let herdr = RealHerdr { runner };
        herdr
            .tab_close(Some("node-b"), &"w1:t2".parse().unwrap())
            .unwrap();
        assert_eq!(
            herdr.runner.calls.borrow()[0].1,
            vec!["--machine", "node-b", "tab", "close", "w1:t2"]
        );
    }
}
