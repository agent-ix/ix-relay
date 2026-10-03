//! The slice of herdr's CLI `relay-watcher` needs, behind a trait.
//!
//! [`RealHerdr`] shells out to the real `herdr` binary via `herdr_cli`. A fake
//! implementing [`Herdr`] lets `watch`'s delivery decision be unit-tested
//! without a live herdr session.
//!
//! The process-running and JSON-envelope plumbing (typed errors, run with a
//! timeout and kill) lives in `herdr-cli`, shared with `agent-msg`'s own
//! `Herdr` trait for the different slice of commands it needs. This module
//! keeps only what's specific to `relay-watcher`: which three herdr commands
//! it calls, and how to decode their results. Typing a batch in is the
//! [`Deliverer`] seam ([`crate::deliver`]), which [`RealHerdr`] implements.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

pub use herdr_cli::ProcessError;
use relay_core::{MachineName, PaneId, Place, Registration, SessionId};
use relay_ops::{find::MachineView, herdr_check::HerdrCheck};
use serde::Deserialize;

use crate::deliver::{DeliverError, Deliverer};

/// How long to wait for one `herdr` invocation before killing it and
/// reporting [`herdr_cli::ProcessError::Timeout`]. A hung herdr call would
/// otherwise block every other lead this process watches, forever.
const HERDR_CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Metadata is display-only and best-effort; never let it hold delivery for a
/// full agent-operation timeout.
const SIDEBAR_REPORT_TIMEOUT: Duration = Duration::from_millis(500);
/// Keep best-effort sidebar work from accumulating without bound when Herdr is slow.
const MAX_SIDEBAR_REPORTS_IN_FLIGHT: usize = 4;

/// A lead's state as herdr's `agent_status` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    /// Ready for input; the agent has not produced output since its last turn.
    Idle,
    /// Ready for input; the agent finished a turn and is waiting.
    Done,
    /// Mid-turn.
    Working,
    /// Waiting on an approval or question dialog.
    Blocked,
    /// Present but herdr could not classify it confidently.
    Unknown,
}

impl AgentStatus {
    fn parse(raw: &str) -> Self {
        match raw {
            "idle" => Self::Idle,
            "done" => Self::Done,
            "working" => Self::Working,
            "blocked" => Self::Blocked,
            // A future herdr version could add a status we don't know yet;
            // treat it like `unknown` rather than fail the tick.
            _ => Self::Unknown,
        }
    }
}

/// What `herdr agent get` reported for one pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentGet {
    /// The lead's current state.
    pub status: AgentStatus,
    /// herdr's monotonic counter for this pane; bumps on every state change.
    /// Used to detect a lead starting a turn between the readiness check and
    /// the prompt.
    pub state_change_seq: u64,
    /// The harness session id occupying the pane right now
    /// (`agent_session.value`; for Claude this equals the hook's
    /// `session_id`), when herdr reports one. Compared against the
    /// registration's own `session_id` before typing anything in, so a pane
    /// whose agent was restarted under a stale registration is never typed
    /// into (AGE-2013's PR review, finding 4).
    pub session_id: Option<SessionId>,
}

/// What one read of herdr's layout shows for this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    /// The place of every live agent whose workspace and tab herdr named.
    pub places: HashMap<PaneId, Place>,
    /// Every pane herdr lists an agent in, with the session its harness reported
    /// (`None`: none yet).
    pub sessions: HashMap<PaneId, Option<SessionId>>,
    /// The whole read, for the metrics snapshot (FR-045): the same herdr answer
    /// the two maps above come from, so publishing it costs no herdr call.
    /// `None` from an implementation with no real herdr behind it.
    pub view: Option<MachineView>,
}

/// Reads herdr's layout through `runner`: the [`Layout`] and how many list
/// entries this binary could not decode and left out. The one place herdr's three
/// listings become a [`Layout`], so a test can replay a real herdr's output
/// through the same code the watcher runs.
pub(crate) fn read_layout(
    runner: impl herdr_cli::ProcessRunner,
    machine: &MachineName,
    timeout: Duration,
) -> Result<(Layout, usize), ProcessError> {
    let view = relay_ops::find::gather_machine_view(
        &relay_ops::find::RealFindHerdr::with_runner(runner),
        None,
        machine,
        timeout,
    )?;
    Ok((
        Layout {
            places: relay_ops::find::places(&view),
            sessions: relay_ops::find::sessions(&view),
            view: Some(view.clone()),
        },
        view.dropped,
    ))
}

/// The herdr operations `relay-watcher` needs.
pub trait Herdr {
    /// Reads a pane's agent state. `Ok(None)` means herdr reports no agent in
    /// that pane (exited, replaced, or never one there).
    fn get(&self, pane: &PaneId) -> Result<Option<AgentGet>, ProcessError>;
    /// Shows a desktop notification so the operator notices something needs them.
    fn notify(&self, title: &str, body: &str) -> Result<(), ProcessError>;
    /// Publishes a short-lived, display-only ix-relay token for a pane. It
    /// never changes Herdr's semantic agent state or lifecycle authority.
    fn report_relay_sidebar(&self, _pane: &PaneId, _value: &str, _ttl_ms: u64) {
        // Implementations without a Herdr metadata API can safely omit the badge.
    }
    /// Verifies the registration and publishes its metadata. Real implementations
    /// should schedule this outside the watcher delivery loop.
    fn report_relay_sidebar_for_registration(
        &self,
        registration: &relay_core::Registration,
        pane: &PaneId,
        value: &str,
        ttl_ms: u64,
    ) where
        Self: Sized,
    {
        crate::sidebar::report_if_verified(
            self,
            registration,
            pane,
            value,
            Duration::from_millis(ttl_ms),
        );
    }
    /// Where every live agent on this machine sits and which session it hosts,
    /// keyed by pane id, so the watcher can keep registered
    /// `server:workspace[:tab[:pane]]` addresses true and follow a session to the pane
    /// it moved to. The default reports none: an implementation that cannot say
    /// leaves every registration as it is.
    fn layout(&self, _machine: &MachineName) -> Result<Layout, ProcessError> {
        Ok(Layout::default())
    }
    /// Whether this herdr is a version, and answers in a shape, ix-relay can
    /// work with (FR-036). The default does not look: an implementation with no
    /// real herdr behind it has nothing to check.
    fn check(&self) -> HerdrCheck {
        HerdrCheck::NotChecked
    }
    /// The check as a job the caller may run on another thread, so a hung herdr
    /// costs the watcher's loop nothing. The default runs [`Herdr::check`] now
    /// and hands back its answer: right for an implementation with no real herdr.
    fn check_job(&self) -> CheckJob {
        let found = self.check();
        Box::new(move || found)
    }
    /// Whether a herdr call's output failed to decode since this was last asked,
    /// clearing the answer: the cue to run [`Herdr::check`] again. The default
    /// has no herdr to fail.
    fn take_decode_failure(&self) -> bool {
        false
    }
    /// How many entries of herdr's last layout listing this binary could not
    /// decode (see [`Herdr::layout`]): a typed count, not a log line, for the
    /// watcher to report once per episode. The default has no herdr to drop any.
    fn dropped_entries(&self) -> usize {
        0
    }
}

/// A herdr check to run off the watcher's loop.
pub type CheckJob = Box<dyn FnOnce() -> HerdrCheck + Send>;

/// How long each herdr call of the version and shape check may take. Shorter
/// than [`HERDR_CALL_TIMEOUT`]: the check makes up to three, and runs on the
/// watcher's own thread.
const CHECK_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the whole check may take: it runs on its own thread, so this only
/// bounds how long a hung herdr keeps a check outstanding.
const CHECK_BUDGET: Duration = Duration::from_secs(10);

/// Shells out to the real `herdr` binary on `PATH`. Clones share what they have
/// noticed: the watcher's delivery side and its check side are one herdr.
#[derive(Debug, Clone, Default)]
pub struct RealHerdr {
    /// Set when a call's output did not decode; read by [`Herdr::take_decode_failure`].
    decode_failed: Arc<AtomicBool>,
    /// The entries the last layout listing dropped; read by [`Herdr::dropped_entries`].
    dropped: Arc<AtomicUsize>,
    /// Bounds background metadata work independently of watcher ticks.
    sidebar_reports_in_flight: Arc<Mutex<std::collections::HashSet<PaneId>>>,
}

fn reserve_sidebar_report(
    in_flight: &Mutex<std::collections::HashSet<PaneId>>,
    pane: &PaneId,
) -> bool {
    let mut in_flight = in_flight
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if in_flight.len() >= MAX_SIDEBAR_REPORTS_IN_FLIGHT || !in_flight.insert(pane.clone()) {
        return false;
    }
    true
}

impl RealHerdr {
    /// `result`, after remembering that a call's output did not decode, for the
    /// herdr check to re-run on.
    fn note_if_undecodable<T>(&self, result: Result<T, ProcessError>) -> Result<T, ProcessError> {
        if matches!(result, Err(ProcessError::BadOutput { .. })) {
            self.decode_failed.store(true, Ordering::Relaxed);
        }
        result
    }
}

#[derive(Debug, Deserialize)]
struct GetResult {
    agent: AgentInfo,
}

#[derive(Debug, Deserialize)]
struct AgentInfo {
    agent_status: String,
    state_change_seq: u64,
    #[serde(default)]
    agent_session: Option<relay_ops::find::ListedSession>,
}

/// herdr's code for "no agent in that pane" (`herdr agent get`). Reported as
/// `Ok(None)`, not an error, per AGE-2013: it means skip the lead, not fail
/// the tick.
const AGENT_NOT_FOUND: &str = "agent_not_found";

impl Herdr for RealHerdr {
    fn get(&self, pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
        let result = herdr_cli::call::<GetResult>(
            &herdr_cli::RealProcessRunner,
            "herdr",
            &["agent", "get", pane.as_str()],
            HERDR_CALL_TIMEOUT,
        );
        match self.note_if_undecodable(result) {
            Ok(GetResult { agent }) => Ok(Some(AgentGet {
                status: AgentStatus::parse(&agent.agent_status),
                state_change_seq: agent.state_change_seq,
                // A value that is not a session id cannot be the registered one.
                session_id: agent.agent_session.and_then(|s| s.session_id()),
            })),
            Err(ProcessError::Reported { code, .. }) if code == AGENT_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn layout(&self, machine: &MachineName) -> Result<Layout, ProcessError> {
        let (layout, dropped) = self.note_if_undecodable(read_layout(
            herdr_cli::RealProcessRunner,
            machine,
            HERDR_CALL_TIMEOUT,
        ))?;
        self.dropped.store(dropped, Ordering::Relaxed);
        Ok(layout)
    }

    fn check(&self) -> HerdrCheck {
        relay_ops::herdr_check::check_within(
            &herdr_cli::RealProcessRunner,
            CHECK_CALL_TIMEOUT,
            CHECK_BUDGET,
        )
    }

    fn check_job(&self) -> CheckJob {
        Box::new(|| {
            relay_ops::herdr_check::check_within(
                &herdr_cli::RealProcessRunner,
                CHECK_CALL_TIMEOUT,
                CHECK_BUDGET,
            )
        })
    }

    fn take_decode_failure(&self) -> bool {
        self.decode_failed.swap(false, Ordering::Relaxed)
    }

    fn dropped_entries(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), ProcessError> {
        herdr_cli::call::<serde_json::Value>(
            &herdr_cli::RealProcessRunner,
            "herdr",
            &["notification", "show", title, "--body", body],
            HERDR_CALL_TIMEOUT,
        )
        .map(|_| ())
    }

    fn report_relay_sidebar(&self, pane: &PaneId, value: &str, ttl_ms: u64) {
        let token = format!("{}={value}", crate::sidebar::TOKEN);
        let ttl = ttl_ms.to_string();
        let _ = herdr_cli::call::<serde_json::Value>(
            &herdr_cli::RealProcessRunner,
            "herdr",
            &[
                "pane",
                "report-metadata",
                pane.as_str(),
                "--source",
                "agent-ix:ix-relay",
                "--token",
                &token,
                "--ttl-ms",
                &ttl,
            ],
            SIDEBAR_REPORT_TIMEOUT,
        );
    }

    fn report_relay_sidebar_for_registration(
        &self,
        registration: &Registration,
        pane: &PaneId,
        value: &str,
        ttl_ms: u64,
    ) {
        if !reserve_sidebar_report(&self.sidebar_reports_in_flight, pane) {
            return;
        }
        let herdr = self.clone();
        let registration = registration.clone();
        let report_pane = pane.clone();
        let value = value.to_owned();
        let in_flight = Arc::clone(&self.sidebar_reports_in_flight);
        let tracked_pane = pane.clone();
        let started = thread::Builder::new()
            .name("relay-sidebar-metadata".into())
            .spawn(move || {
                struct Release(Arc<Mutex<std::collections::HashSet<PaneId>>>, PaneId);
                impl Drop for Release {
                    fn drop(&mut self) {
                        self.0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&self.1);
                    }
                }
                let _release = Release(in_flight, tracked_pane);
                crate::sidebar::report_if_verified(
                    &herdr,
                    &registration,
                    &report_pane,
                    &value,
                    Duration::from_millis(ttl_ms),
                );
            });
        if started.is_err() {
            self.sidebar_reports_in_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(pane);
        }
    }
}

/// The one transport today: `herdr agent prompt`.
impl Deliverer for RealHerdr {
    fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), DeliverError> {
        herdr_cli::call::<serde_json::Value>(
            &herdr_cli::RealProcessRunner,
            "herdr",
            &["agent", "prompt", pane.as_str(), text],
            HERDR_CALL_TIMEOUT,
        )
        .map(|_| ())
        .map_err(DeliverError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trace: FR-052-AC-4
    #[test]
    fn sidebar_reporting_reserves_at_most_four_background_slots() {
        let in_flight = Mutex::new(std::collections::HashSet::new());
        let pane: PaneId = "w1:p1".parse().unwrap();
        for _ in 0..MAX_SIDEBAR_REPORTS_IN_FLIGHT {
            let pane: PaneId = format!("w1:p{}", in_flight.lock().unwrap().len() + 1)
                .parse()
                .unwrap();
            assert!(reserve_sidebar_report(&in_flight, &pane));
        }
        assert!(!reserve_sidebar_report(&in_flight, &pane));
        in_flight.lock().unwrap().remove(&"w1:p1".parse().unwrap());
        assert!(reserve_sidebar_report(&in_flight, &pane));
    }

    #[test]
    fn parses_all_known_statuses_and_falls_back_on_unknown() {
        for (raw, expect) in [
            ("idle", AgentStatus::Idle),
            ("done", AgentStatus::Done),
            ("working", AgentStatus::Working),
            ("blocked", AgentStatus::Blocked),
            ("unknown", AgentStatus::Unknown),
            ("something-future-herdr-adds", AgentStatus::Unknown),
        ] {
            assert_eq!(AgentStatus::parse(raw), expect);
        }
    }

    #[test]
    fn decodes_a_real_agent_get_result_including_session_id() {
        let raw = br#"{"result":{"agent":{"agent":"claude",
            "agent_session":{"agent":"claude","kind":"id","source":"herdr:claude",
            "value":"32eb5177-ab07-44ea-8363-7a93ad06b7a2"},
            "agent_status":"blocked","state_change_seq":251,"pane_id":"wE:p1"},
            "type":"agent_info"}}"#;
        let result: GetResult = herdr_cli::decode("herdr", raw, b"", true).unwrap();
        assert_eq!(result.agent.agent_status, "blocked");
        assert_eq!(result.agent.state_change_seq, 251);
        assert_eq!(
            result.agent.agent_session.map(|s| s.value),
            Some("32eb5177-ab07-44ea-8363-7a93ad06b7a2".to_owned())
        );
    }

    #[test]
    fn decodes_a_real_agent_get_result_with_no_session() {
        // Some agent kinds/states may not report agent_session at all.
        let raw = br#"{"result":{"agent":{"agent_status":"idle","state_change_seq":1}}}"#;
        let result: GetResult = herdr_cli::decode("herdr", raw, b"", true).unwrap();
        assert!(result.agent.agent_session.is_none());
    }

    /// Captured live: `herdr agent get w999:p999`, which exits 1 and writes
    /// the error envelope to *stderr*, not stdout (AGE-2013's PR review,
    /// finding 2).
    #[test]
    fn decodes_a_real_agent_not_found_stderr_envelope() {
        let stderr = br#"{"error":{"code":"agent_not_found","message":"agent target w999:p999 not found"},"id":"cli:agent:get"}"#;
        let err = herdr_cli::decode::<GetResult>("herdr", b"", stderr, false).unwrap_err();
        assert!(matches!(&err, ProcessError::Reported { code, .. } if code == AGENT_NOT_FOUND));
    }

    /// herdr reports a session as `id` or `path`: only an id is a session id, so
    /// a pane that reports a path shows no session (AGE-2033 review, finding 3).
    ///
    /// Tracing: TC-043-02
    /// ACs: FR-043-AC-2
    #[test]
    fn tc_043_02_a_path_session_in_agent_get_is_no_session() {
        let get = |kind: &str| {
            let raw = format!(
                r#"{{"result":{{"agent":{{"agent_session":{{"kind":"{kind}","value":"sess-1"}},
                "agent_status":"idle","state_change_seq":1}}}}}}"#
            );
            let result: GetResult = herdr_cli::decode("herdr", raw.as_bytes(), b"", true).unwrap();
            result.agent.agent_session.and_then(|s| s.session_id())
        };
        assert_eq!(get("id"), Some("sess-1".parse().unwrap()));
        assert_eq!(get("path"), None);
    }
}
