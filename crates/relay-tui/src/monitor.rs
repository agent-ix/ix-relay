// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Read-only team-leader dashboard state for SWM (SWM-21).

use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// One independently refreshed source, retaining its last good snapshot
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
}

/// Main content selected independently of the source status toggle.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Open scope tickets and their assignments.
    #[default]
    Tickets,
    /// All explicit agents, even without a scope or tracker snapshot.
    Agents,
    /// Initiative and project progress within the reported scope.
    Scope,
    /// Structured department progress and source availability.
    Status,
    /// Shared assessments, with severity separate from completeness.
    Health,
    /// Shared current, previous and baseline comparisons.
    Retro,
}

/// Complete dashboard state.
#[derive(Debug, Default)]
pub struct App {
    /// Explicit source-native project selectors, independent of pane identity.
    pub projects: Vec<String>,
    /// Shared department report and comparisons, retained on failed refresh.
    pub department: Source<ix_board::retro::department::Delta>,
    /// The terminal surface's resolved color policy.
    pub color_enabled: bool,
    /// Requested live collection has failed; quitting must retain that outcome.
    pub collection_failed: bool,
    /// Explicit Linear team override; otherwise the bound workspace is the team key.
    pub team: Option<String>,
    /// Native session incarnation this monitor follows.
    pub binding: Option<relay_core::work::WorkBinding>,
    /// Latest independently refreshed Herdr activity.
    pub runtime: Source<relay_ops::work::WorkObservation>,
    /// Latest independently refreshed explicit assignment report.
    pub report: Source<Option<relay_core::work::WorkReport>>,
    /// Once observed, replacement requires an explicit monitor restart.
    pub session_ended: bool,
    /// Stateful table selection and scroll position.
    pub table: ratatui::widgets::TableState,
    /// Current rendered assignment row count.
    pub row_count: usize,
    /// Selected content view.
    pub view: View,
    /// Whether the detail panel is expanded.
    pub details: bool,
    /// Vertical offset within expanded department evidence.
    pub detail_scroll: u16,
    /// Whether source timestamps are visible in the optional status line.
    pub show_status: bool,
    /// Complete authoritative tracker snapshot from Linear.
    pub tracker: Source<crate::monitor_tracker::TrackerState>,
    /// Next scheduled retry after a rate-limited Linear read.
    pub tracker_retry_at: Option<Instant>,
    /// Human-readable identity of the current team lead.
    pub lead_name: String,
    /// Set by `q` or Escape.
    pub should_quit: bool,
    /// Set by `r` to request an immediate refresh.
    pub refresh_now: bool,
}

impl App {
    /// Applies native observations without ever following a replacement binding.
    pub fn apply_runtime_refresh(&mut self, refresh: crate::monitor_runtime::RuntimeRefresh) {
        match refresh {
            crate::monitor_runtime::RuntimeRefresh::Observation(result, read_at) => match result {
                Ok(observation) if !self.session_ended => {
                    if self.binding.as_ref() == Some(&observation.binding) {
                        self.runtime.success(observation, read_at);
                    } else {
                        self.session_ended = true;
                        self.runtime
                            .failure("session ended/replaced · restart to rebind");
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    if matches!(
                        error,
                        relay_ops::OpsError::WorkSessionReplaced
                            | relay_ops::OpsError::WorkAgentAbsent
                    ) {
                        self.session_ended = true;
                    }
                    self.runtime.failure(error.to_string());
                }
            },
            crate::monitor_runtime::RuntimeRefresh::Report(result, read_at) => match result {
                Ok(report) if !self.session_ended => {
                    if report
                        .as_ref()
                        .is_some_and(|report| self.binding.as_ref() != Some(&report.binding))
                    {
                        self.session_ended = true;
                        self.report
                            .failure("session ended/replaced · restart to rebind");
                    } else {
                        self.report.success(report, read_at);
                    }
                }
                Ok(_) => {}
                Err(error) => self.report.failure(error.to_string()),
            },
        }
    }

    /// Applies an independently refreshed Linear tracker result.
    pub fn apply_tracker_refresh(
        &mut self,
        tracker: Result<crate::monitor_tracker::TrackerState, ix_board::Error>,
        now: Instant,
    ) {
        self.collection_failed |= tracker.is_err();
        match tracker {
            Ok(data) => {
                self.tracker_retry_at = None;
                self.tracker.success(data, now);
            }
            Err(ix_board::Error::LinearRateLimited { .. }) => {
                self.tracker_retry_at =
                    now.checked_add(crate::monitor_ops::TRACKER_REFRESH_INTERVAL);
                self.tracker.failure("Linear rate limited");
            }
            Err(error) => {
                self.tracker_retry_at = None;
                self.tracker
                    .failure(crate::monitor_ops::source_diagnostic(&error));
            }
        }
    }
}

/// Handles the dashboard's read-only keyboard controls.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    let selected_before = app.table.selected();
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('r') => app.refresh_now = true,
        KeyCode::Char('d') | KeyCode::Enter => app.details = !app.details,
        KeyCode::Char('s') => app.show_status = !app.show_status,
        KeyCode::Char('t') => select_view(app, View::Tickets),
        KeyCode::Char('a') => select_view(app, View::Agents),
        KeyCode::Char('p') => select_view(app, View::Scope),
        KeyCode::Char('1') => select_view(app, View::Status),
        KeyCode::Char('2') => select_view(app, View::Health),
        KeyCode::Char('3') => select_view(app, View::Retro),
        KeyCode::Right if app.details => {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
        }
        KeyCode::Left if app.details => {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => app.table.select(Some(
            app.table
                .selected()
                .unwrap_or(0)
                .saturating_add(1)
                .min(app.row_count.saturating_sub(1)),
        )),
        KeyCode::Up | KeyCode::Char('k') => app
            .table
            .select(Some(app.table.selected().unwrap_or(0).saturating_sub(1))),
        KeyCode::PageDown => app.table.select(Some(
            app.table
                .selected()
                .unwrap_or(0)
                .saturating_add(5)
                .min(app.row_count.saturating_sub(1)),
        )),
        KeyCode::PageUp => app
            .table
            .select(Some(app.table.selected().unwrap_or(0).saturating_sub(5))),
        _ => {}
    }
    if app.table.selected() != selected_before {
        app.detail_scroll = 0;
    }
}

fn select_view(app: &mut App, view: View) {
    if app.view != view {
        app.view = view;
        app.table = Default::default();
        app.row_count = 0;
        app.detail_scroll = 0;
    }
}

/// Removes terminal control characters and bounds untrusted tracker text.
#[must_use]
pub fn display_text(raw: &str, max_chars: usize) -> String {
    let safe = relay_core::text::sanitise_line(raw);
    safe.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    #[test]
    fn source_retains_last_good_value_and_reports_latest_error() {
        let started = Instant::now();
        let mut source = Source::default();
        source.success("good", started);
        source.failure("Linear unavailable");
        assert_eq!(source.value, Some("good"));
        assert_eq!(source.error.as_deref(), Some("Linear unavailable"));
    }

    #[test]
    fn controls_quit_and_request_tracker_refresh() {
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
    fn tracker_text_is_sanitized_and_bounded() {
        assert_eq!(display_text("hi\u{1b}[31mthere\nagain", 8), "hi�[31mt");
    }

    /// Trace: FR-059-AC-7
    #[test]
    fn replacement_cannot_change_attribution_and_department_data_survives() {
        let now = Instant::now();
        let original = relay_core::work::WorkBinding {
            machine: "luna".parse().unwrap(),
            pane: "w1:p1".parse().unwrap(),
            session: "session-a".parse().unwrap(),
            process_id: 42,
            kind: "codex".into(),
            address: "luna:SWM:code".into(),
        };
        let mut replacement = original.clone();
        replacement.session = "session-b".parse().unwrap();
        replacement.process_id = 43;
        let mut app = App {
            binding: Some(original.clone()),
            ..App::default()
        };
        app.department
            .success(crate::monitor_department::tests::fixture(), now);
        app.apply_runtime_refresh(crate::monitor_runtime::RuntimeRefresh::Observation(
            Ok(relay_ops::work::WorkObservation {
                binding: original.clone(),
                status: "idle".into(),
                panes: Vec::new(),
            }),
            now,
        ));
        app.apply_runtime_refresh(crate::monitor_runtime::RuntimeRefresh::Observation(
            Err(relay_ops::OpsError::WorkSessionReplaced),
            now,
        ));
        app.apply_runtime_refresh(crate::monitor_runtime::RuntimeRefresh::Observation(
            Ok(relay_ops::work::WorkObservation {
                binding: replacement.clone(),
                status: "working".into(),
                panes: Vec::new(),
            }),
            now,
        ));
        app.apply_runtime_refresh(crate::monitor_runtime::RuntimeRefresh::Report(
            Ok(Some(relay_core::work::WorkReport {
                binding: replacement,
                scope: None,
                focus: None,
                agents: Vec::new(),
            })),
            now,
        ));
        assert!(app.session_ended);
        assert_eq!(app.binding, Some(original.clone()));
        assert_eq!(app.runtime.value.as_ref().unwrap().binding, original);
        assert!(app.report.value.is_none());
        assert!(app.department.value.is_some());
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
        );
        assert_eq!(app.view, View::Health);
    }
}
