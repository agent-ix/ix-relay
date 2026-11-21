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
    /// Compact classified summary, when the source supplies one.
    pub error_summary: Option<String>,
    /// Incremented whenever `success` replaces the value; lets derived results be reused.
    pub revision: u64,
}

impl<T> Default for Source<T> {
    fn default() -> Self {
        Self {
            value: None,
            last_success: None,
            error: None,
            error_summary: None,
            revision: 0,
        }
    }
}

impl<T> Source<T> {
    /// Replaces the last good value and clears any earlier error.
    pub fn success(&mut self, value: T, now: Instant) {
        self.revision = self.revision.wrapping_add(1);
        self.value = Some(value);
        self.last_success = Some(now);
        self.error = None;
        self.error_summary = None;
    }

    /// Keeps the last good value but exposes the newest failure.
    pub fn failure(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.error_summary = None;
    }

    /// Retains a compact summary alongside the complete actionable diagnostic.
    pub fn failure_with_summary(&mut self, summary: impl Into<String>, error: impl Into<String>) {
        self.failure(error);
        self.error_summary = Some(summary.into());
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

impl View {
    /// Every view in the fixed order the View selector lists them.
    pub const ALL: [View; 6] = [
        View::Tickets,
        View::Agents,
        View::Scope,
        View::Status,
        View::Health,
        View::Retro,
    ];

    /// Name shown in the View selector and dropdown.
    pub const fn label(self) -> &'static str {
        match self {
            View::Tickets => "Tickets",
            View::Agents => "Agents",
            View::Scope => "Scope",
            View::Status => "Status",
            View::Health => "Health",
            View::Retro => "Retro",
        }
    }

    /// Compatibility shortcut that selects this view without the dropdown.
    pub const fn key(self) -> char {
        match self {
            View::Tickets => 't',
            View::Agents => 'a',
            View::Scope => 'p',
            View::Status => '1',
            View::Health => '2',
            View::Retro => '3',
        }
    }
}

/// The one popup that may be open over the work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popup {
    /// View dropdown; `highlight` indexes `View::ALL` and `offset` is its first visible entry.
    Dropdown {
        /// Highlighted entry.
        highlight: usize,
        /// First visible entry, clamped when drawn.
        offset: usize,
    },
    /// Help popup scrolled by `scroll` lines, clamped when drawn.
    Help {
        /// First visible help line.
        scroll: u16,
    },
}

/// How the department report is supplied; decides the Health and Retro empty states.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DepartmentInput {
    /// Live startup without a retro configuration.
    #[default]
    NotConfigured,
    /// Live startup with a retro configuration; the first read may be pending.
    Configured,
    /// A saved report supplied the model.
    Saved,
}

/// Text shared by the binding fence and the evidence rows when a session is replaced.
pub(crate) const SESSION_REPLACED: &str = "session ended/replaced · restart to rebind";

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
    /// Whether source failures replace the department rows for full inspection.
    pub show_diagnostics: bool,
    /// Whether source timestamps are visible in the optional status line.
    pub show_status: bool,
    /// Complete authoritative tracker snapshot from Linear.
    pub tracker: Source<crate::monitor_tracker::TrackerState>,
    /// Next scheduled retry after a rate-limited Linear read.
    pub tracker_retry_at: Option<Instant>,
    /// Typed state of the latest tracker attempt: clear, a snapshot not certified fresh with no
    /// failure claimed, or the typed cause of the failed attempt.
    pub tracker_status: crate::monitor_ops::TrackerStatus,
    /// Human-readable identity of the current team lead.
    pub lead_name: String,
    /// Set by `q` or Escape.
    pub should_quit: bool,
    /// Set by `r` to request an immediate refresh.
    pub refresh_now: bool,
    /// Whether the Scope view shows completed and canceled projects; process-local.
    pub show_completed: bool,
    /// Reused shared summary, Scope projection and Scope selection memory.
    pub scope: crate::monitor_scope::ScopeState,
    /// Open popup, if any; at most one at a time.
    pub popup: Option<Popup>,
    /// How the department report is supplied.
    pub department_input: DepartmentInput,
}

impl App {
    /// Retains shared department evidence on failure without rewriting its timestamps.
    pub fn apply_department_refresh(
        &mut self,
        result: Result<ix_board::retro::department::Delta, String>,
        now: Instant,
    ) {
        match result {
            Ok(report) => self.department.success(report, now),
            Err(error) => {
                self.collection_failed = true;
                self.department.failure(error);
            }
        }
    }

    /// Team selector shared with lead views, without narrowing project-only requests.
    pub fn department_team(&self) -> Option<String> {
        self.team.clone().or_else(|| {
            if !self.projects.is_empty() {
                return None;
            }
            self.binding
                .as_ref()
                .and_then(|binding| binding.address.parse::<relay_core::Address>().ok())
                .map(|address| address.workspace().to_ascii_uppercase())
        })
    }
    /// Applies native observations without ever following a replacement binding.
    pub fn apply_runtime_refresh(&mut self, refresh: crate::monitor_runtime::RuntimeRefresh) {
        match refresh {
            crate::monitor_runtime::RuntimeRefresh::Observation(result, read_at) => match result {
                Ok(observation) if !self.session_ended => {
                    if self.binding.as_ref() == Some(&observation.binding) {
                        self.runtime.success(observation, read_at);
                    } else {
                        self.session_ended = true;
                        self.runtime.failure(SESSION_REPLACED);
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
                        self.report.failure(SESSION_REPLACED);
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
        tracker: Result<crate::monitor_ops::TrackerRead, ix_board::Error>,
        now: Instant,
    ) {
        use crate::monitor_ops::{
            Provenance, TrackerStatus,
            quota::{Cause, Notice},
        };
        self.collection_failed |= tracker.is_err();
        match tracker {
            Ok(read) => {
                // A snapshot kept after a failed refresh keeps its source timestamp; the
                // failure shows beside it until a refresh succeeds. Unconfirmed provenance
                // is not a failure and only marks the snapshot as not certified fresh.
                let provenance = read.provenance();
                self.tracker.success(read.snapshot, now);
                self.tracker_retry_at = None;
                self.tracker_status = match provenance {
                    Provenance::Current => TrackerStatus::Clear,
                    Provenance::Unconfirmed => TrackerStatus::Unconfirmed,
                    Provenance::RefreshFailed { cause, notice } => {
                        self.collection_failed = true;
                        self.hold_tracker(cause, notice, now);
                        TrackerStatus::Failed(cause)
                    }
                };
            }
            Err(error) => match Cause::of_error(&error) {
                Some(cause) => {
                    let notice = Notice::new(cause, error.quota_decision(), false);
                    self.hold_tracker(cause, notice, now);
                    self.tracker_status = TrackerStatus::Failed(cause);
                }
                None => {
                    self.tracker_retry_at = None;
                    self.tracker_status = TrackerStatus::Failed(Cause::SourceFailed);
                    self.tracker.failure_with_summary(
                        crate::monitor_ops::source_summary(&error),
                        crate::monitor_ops::source_diagnostic(&error),
                    );
                }
            },
        }
    }

    /// Shows the notice for a failed tracker attempt. Only a hold to wait out schedules the
    /// next attempt and coalesces manual refreshes; no other cause invents a cooldown.
    fn hold_tracker(
        &mut self,
        cause: crate::monitor_ops::quota::Cause,
        notice: crate::monitor_ops::quota::Notice,
        now: Instant,
    ) {
        self.tracker_retry_at = if cause.waits() {
            now.checked_add(crate::monitor_ops::TRACKER_REFRESH_INTERVAL)
        } else {
            None
        };
        self.tracker
            .failure_with_summary(notice.summary, notice.detail);
    }
}

/// Handles the dashboard's read-only keyboard controls.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    // An open popup receives every key and acts only on the keys it owns.
    if let Some(popup) = app.popup {
        crate::monitor_nav::handle_popup_key(app, popup, key);
        return;
    }
    let selected_before = app.table.selected();
    match key.code {
        KeyCode::Char('v') if key.modifiers.is_empty() => {
            app.popup = Some(Popup::Dropdown {
                highlight: View::ALL.iter().position(|v| *v == app.view).unwrap_or(0),
                offset: 0,
            });
        }
        KeyCode::F(1) | KeyCode::Char('?') => app.popup = Some(Popup::Help { scroll: 0 }),
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('r') => app.refresh_now = true,
        KeyCode::Char('d') | KeyCode::Enter => app.details = !app.details,
        KeyCode::Char('s') => app.show_status = !app.show_status,
        KeyCode::Char('e') => {
            app.show_diagnostics = !app.show_diagnostics;
            app.table = Default::default();
            app.scope.forget_selection();
            app.detail_scroll = 0;
        }
        KeyCode::Char('t') => select_view(app, View::Tickets),
        KeyCode::Char('a') => select_view(app, View::Agents),
        KeyCode::Char('p') => select_view(app, View::Scope),
        KeyCode::Char('c') if app.view == View::Scope && key.modifiers.is_empty() => {
            app.show_completed = !app.show_completed;
        }
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

pub(crate) fn select_view(app: &mut App, view: View) {
    if app.view != view {
        app.view = view;
        app.table = Default::default();
        app.scope.forget_selection();
        app.row_count = 0;
        app.detail_scroll = 0;
        app.show_diagnostics = false;
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
    use ix_trace_rs::trace;
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
    #[trace("FR-059-AC-7")]
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
