// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Read-only display state for the Herdr activity monitor (SWM-21).

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use serde::Deserialize;

/// How long a successfully read source remains fresh without another update.
pub const STALE_AFTER: Duration = Duration::from_secs(15);
/// A registration not refreshed for this long may be abandoned or offline.
pub const REGISTRATION_STALE_AFTER_MS: i64 = 90_000;

/// Selected details for the current Herdr pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HerdrState {
    /// The current monitor pane's location, when Herdr exported it.
    pub current: Option<CurrentPane>,
    /// Agent sessions visible in the local Herdr snapshot.
    pub agents: Vec<HerdrAgent>,
}

/// Identity of the pane hosting the monitor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CurrentPane {
    /// Stable Herdr pane id, such as `w1:p2`.
    pub pane_id: String,
    /// Human-readable workspace label, if present.
    pub workspace: Option<String>,
    /// Human-readable tab label, if present.
    pub tab: Option<String>,
    /// Terminal title, if present.
    pub terminal_title: Option<String>,
    /// Agent session occupying this pane, if present.
    pub agent: Option<String>,
    /// Agent session identifier, when Herdr exposes one.
    pub session: Option<String>,
    /// Herdr's current status for the pane.
    pub status: String,
}

/// A local Herdr agent session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HerdrAgent {
    /// Harness name, e.g. `claude` or `codex`.
    pub agent: String,
    /// Herdr-reported activity state.
    pub status: String,
    /// Session identifier when Herdr provides one.
    pub session: Option<String>,
    /// Pane identifier.
    pub pane_id: String,
    /// Workspace label where available.
    pub workspace: Option<String>,
    /// Tab label where available.
    pub tab: Option<String>,
    /// Terminal title, falling back to its stripped form.
    pub terminal_title: Option<String>,
    /// Session working directory.
    pub cwd: Option<String>,
}

/// Current bus registrations and the most recent retained message entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BusState {
    /// Registered team leads, sorted by their bus name as returned by `Relay::agents`.
    pub registrations: Vec<BusAgent>,
    /// Unreadable registration records returned by the bus store.
    pub skipped_registrations: Vec<String>,
    /// Recent messages, oldest first within the retained display window.
    pub messages: Vec<BusMessage>,
}

/// One ix-relay registration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BusAgent {
    /// Lead name on the bus.
    pub name: String,
    /// Harness kind.
    pub kind: String,
    /// Machine name.
    pub machine: String,
    /// Verified Herdr address, when available.
    pub place: Option<String>,
    /// Working directory, when reported.
    pub cwd: Option<String>,
    /// Unix milliseconds of the last registration or watcher observation.
    pub last_seen_ms: i64,
}

/// A recent message that was retained in the relay activity log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BusMessage {
    /// Sender's lead name.
    pub from: String,
    /// Message text, sanitized and bounded for terminal display.
    pub text: String,
    /// Unix milliseconds from Redis's stream id.
    pub sent_at_ms: i64,
}

/// A single independently refreshed source, retaining its last good snapshot
/// while reporting an error or stale age.
#[derive(Debug, Clone)]
pub struct Source<T> {
    /// Last successful value, retained through later failures.
    pub value: Option<T>,
    /// Time of the last successful refresh on this process.
    pub last_success: Option<Instant>,
    /// Most recent refresh error.
    pub error: Option<String>,
}

impl<T> Default for Source<T> {
    fn default() -> Self {
        Self {
            value: None,
            last_success: None,
            error: None,
        }
    }
}

impl<T> Source<T> {
    /// Replaces the last good value and clears any earlier error.
    pub fn success(&mut self, value: T, now: Instant) {
        self.value = Some(value);
        self.last_success = Some(now);
        self.error = None;
    }

    /// Keeps the last good value but exposes the newest failure.
    pub fn failure(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    /// Whether this source has never succeeded or has exceeded the stale age.
    #[must_use]
    pub fn is_stale(&self, now: Instant) -> bool {
        self.last_success
            .is_none_or(|last| now.saturating_duration_since(last) > STALE_AFTER)
    }
}

/// Whether a bus registration's last observation is old enough to be marked
/// stale in the registrations table.
#[must_use]
pub fn registration_is_stale(last_seen_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(last_seen_ms) > REGISTRATION_STALE_AFTER_MS
}

/// Complete monitor application state.
#[derive(Debug, Default)]
pub struct App {
    /// Herdr source state.
    pub herdr: Source<HerdrState>,
    /// Relay bus source state.
    pub bus: Source<BusState>,
    /// Set by `q` or Escape.
    pub should_quit: bool,
    /// Set by `r` to request an immediate refresh.
    pub refresh_now: bool,
}

impl App {
    /// Applies one independently successful/failed refresh for both sources.
    pub fn apply_refresh(
        &mut self,
        herdr: Result<HerdrState, String>,
        bus: Result<BusState, String>,
        now: Instant,
    ) {
        match herdr {
            Ok(data) => self.herdr.success(data, now),
            Err(error) => self.herdr.failure(error),
        }
        match bus {
            Ok(data) => self.bus.success(data, now),
            Err(error) => self.bus.failure(error),
        }
    }
}

/// Handles the monitor's read-only keyboard controls.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('r') => app.refresh_now = true,
        _ => {}
    }
}

/// Decodes Herdr's structured `herdr api snapshot` payload into the fields
/// used by the monitor. Unknown fields are ignored because the Herdr snapshot
/// grows as Herdr adds state; the selected field types still fail closed when
/// their wire types change.
#[cfg(test)]
fn decode_herdr_snapshot(raw: &str, current_pane_id: Option<&str>) -> Result<HerdrState, String> {
    let result: HerdrResult = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    Ok(result.into_state(current_pane_id))
}

impl HerdrResult {
    /// Converts the typed structured API result to the fields rendered by the
    /// monitor.
    pub(crate) fn into_state(self, current_pane_id: Option<&str>) -> HerdrState {
        let snapshot = self.snapshot;
        let workspaces = snapshot.workspaces;
        let tabs = snapshot.tabs;

        let workspace_label = |id: &str| {
            workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == id)
                .map(|workspace| workspace.label.clone())
                .filter(|label| !label.is_empty())
        };
        let tab_label = |id: &str| {
            tabs.iter()
                .find(|tab| tab.tab_id == id)
                .map(|tab| tab.label.clone())
                .filter(|label| !label.is_empty())
        };

        let current = current_pane_id.and_then(|pane_id| {
            let pane = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id)?;
            let agent = snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id);
            Some(CurrentPane {
                pane_id: pane.pane_id.clone(),
                workspace: workspace_label(&pane.workspace_id),
                tab: tab_label(&pane.tab_id),
                terminal_title: pane.display_title(),
                agent: agent.map(|agent| agent.agent.clone()),
                session: agent
                    .and_then(|agent| agent.agent_session.as_ref())
                    .and_then(|session| session.value.clone()),
                status: pane.agent_status.clone(),
            })
        });

        let agents = snapshot
            .agents
            .into_iter()
            .map(|agent| HerdrAgent {
                agent: agent.agent,
                status: agent.agent_status,
                session: agent.agent_session.and_then(|session| session.value),
                workspace: workspace_label(&agent.workspace_id),
                tab: tab_label(&agent.tab_id),
                pane_id: agent.pane_id,
                terminal_title: agent.terminal_title.or(agent.terminal_title_stripped),
                cwd: agent.cwd,
            })
            .collect();
        HerdrState { current, agents }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct HerdrResult {
    snapshot: HerdrSnapshot,
}

#[derive(Debug, Deserialize)]
struct HerdrSnapshot {
    #[serde(default)]
    agents: Vec<HerdrAgentRecord>,
    #[serde(default)]
    panes: Vec<HerdrPaneRecord>,
    #[serde(default)]
    tabs: Vec<HerdrTabRecord>,
    #[serde(default)]
    workspaces: Vec<HerdrWorkspaceRecord>,
}

#[derive(Debug, Deserialize)]
struct HerdrAgentRecord {
    #[serde(default)]
    agent: String,
    #[serde(default)]
    agent_status: String,
    #[serde(default)]
    agent_session: Option<HerdrSessionRecord>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    pane_id: String,
    #[serde(default)]
    tab_id: String,
    #[serde(default)]
    terminal_title: Option<String>,
    #[serde(default)]
    terminal_title_stripped: Option<String>,
    #[serde(default)]
    workspace_id: String,
}

#[derive(Debug, Deserialize)]
struct HerdrSessionRecord {
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HerdrPaneRecord {
    #[serde(default)]
    agent_status: String,
    #[serde(default)]
    pane_id: String,
    #[serde(default)]
    tab_id: String,
    #[serde(default)]
    terminal_title: Option<String>,
    #[serde(default)]
    terminal_title_stripped: Option<String>,
    #[serde(default)]
    workspace_id: String,
}

impl HerdrPaneRecord {
    fn display_title(&self) -> Option<String> {
        self.terminal_title
            .clone()
            .or_else(|| self.terminal_title_stripped.clone())
    }
}

#[derive(Debug, Deserialize)]
struct HerdrTabRecord {
    #[serde(default)]
    label: String,
    #[serde(default)]
    tab_id: String,
}

#[derive(Debug, Deserialize)]
struct HerdrWorkspaceRecord {
    #[serde(default)]
    label: String,
    #[serde(default)]
    workspace_id: String,
}

/// Removes terminal control characters and bounds untrusted bus/Herdr text.
#[must_use]
pub fn display_text(raw: &str, max_chars: usize) -> String {
    let safe = relay_core::text::sanitise_line(raw);
    safe.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    const SNAPSHOT: &str = r#"{
      "snapshot": {
        "agents": [{"agent":"codex","agent_status":"working","agent_session":{"value":"session-7"},"cwd":"/work","pane_id":"w2:p3","tab_id":"w2:t1","workspace_id":"w2","terminal_title":"Build monitor"}],
        "panes": [{"agent_status":"working","pane_id":"w2:p3","tab_id":"w2:t1","workspace_id":"w2","terminal_title":"Build monitor"}],
        "tabs": [{"label":"tools","tab_id":"w2:t1"}],
        "workspaces": [{"label":"SWM","workspace_id":"w2"}],
        "future_field": true
      },
      "type": "session_snapshot"
    }"#;

    #[test]
    fn structured_snapshot_resolves_current_pane_and_agent_context() {
        let state = decode_herdr_snapshot(SNAPSHOT, Some("w2:p3")).unwrap();
        assert_eq!(
            state.current.as_ref().unwrap().workspace.as_deref(),
            Some("SWM")
        );
        assert_eq!(
            state.current.as_ref().unwrap().tab.as_deref(),
            Some("tools")
        );
        assert_eq!(
            state.current.as_ref().unwrap().agent.as_deref(),
            Some("codex")
        );
        assert_eq!(
            state.current.as_ref().unwrap().session.as_deref(),
            Some("session-7")
        );
        assert_eq!(state.agents[0].session.as_deref(), Some("session-7"));
        assert_eq!(
            state.agents[0].terminal_title.as_deref(),
            Some("Build monitor")
        );
    }

    #[test]
    fn missing_pane_identity_is_explicit_in_data() {
        let state = decode_herdr_snapshot(SNAPSHOT, Some("w9:p9")).unwrap();
        assert!(state.current.is_none());
        assert_eq!(state.agents.len(), 1);
    }

    #[test]
    fn source_retains_last_good_value_and_marks_stale_after_threshold() {
        let started = Instant::now();
        let mut source = Source::default();
        source.success("good", started);
        source.failure("connection refused");
        assert_eq!(source.value, Some("good"));
        assert_eq!(source.error.as_deref(), Some("connection refused"));
        assert!(!source.is_stale(started + STALE_AFTER));
        assert!(source.is_stale(started + STALE_AFTER + Duration::from_millis(1)));
    }

    #[test]
    fn controls_quit_and_request_refresh() {
        let mut app = App::default();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        );
        assert!(app.refresh_now);
        handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.should_quit);
    }

    #[test]
    fn display_text_strips_controls_and_bounds_length() {
        assert_eq!(display_text("hi\u{1b}[31mthere\nagain", 8), "hi�[31mt");
    }

    #[test]
    fn registration_staleness_handles_old_and_future_timestamps() {
        assert!(registration_is_stale(1_000, 92_000));
        assert!(!registration_is_stale(10_000, 10_000));
        assert!(!registration_is_stale(20_000, 10_000));
    }
}
