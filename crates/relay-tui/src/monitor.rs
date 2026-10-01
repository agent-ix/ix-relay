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

/// Complete dashboard state.
#[derive(Debug, Default)]
pub struct App {
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
    /// Whether the detail panel is expanded.
    pub details: bool,
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
    /// Applies an independently refreshed Linear tracker result.
    pub fn apply_tracker_refresh(
        &mut self,
        tracker: Result<crate::monitor_tracker::TrackerState, ix_board::Error>,
        now: Instant,
    ) {
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
                self.tracker.failure(error.to_string());
            }
        }
    }
}

/// Handles the dashboard's read-only keyboard controls.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('r') => app.refresh_now = true,
        KeyCode::Char('d') | KeyCode::Enter => app.details = !app.details,
        KeyCode::Char('s') => app.show_status = !app.show_status,
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
}
