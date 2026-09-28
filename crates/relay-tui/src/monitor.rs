// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Read-only team-leader dashboard state for SWM (SWM-21).

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

pub(crate) const TRACKER_STALE_AFTER: Duration = Duration::from_secs(90);

/// A tracker snapshot that has not refreshed within this duration is stale.
pub const STALE_AFTER: Duration = Duration::from_secs(15);

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

    /// Whether this source has never succeeded or has exceeded the stale age.
    #[must_use]
    pub fn is_stale(&self, now: Instant) -> bool {
        self.last_success
            .is_none_or(|last| now.saturating_duration_since(last) > STALE_AFTER)
    }
}

/// Complete dashboard state.
#[derive(Debug, Default)]
pub struct App {
    /// Selected ix-board and ix-projects tracker state from Linear.
    pub tracker: Source<crate::monitor_tracker::TrackerState>,
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
        tracker: Result<crate::monitor_tracker::TrackerState, String>,
        now: Instant,
    ) {
        match tracker {
            Ok(data) => self.tracker.success(data, now),
            Err(error) => self.tracker.failure(error),
        }
    }
}

/// Whether tracker state is stale or has never loaded.
#[must_use]
pub fn tracker_is_stale(source: &Source<impl Sized>, now: Instant) -> bool {
    source
        .last_success
        .is_none_or(|last| now.saturating_duration_since(last) > TRACKER_STALE_AFTER)
}

/// Handles the dashboard's read-only keyboard controls.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('r') => app.refresh_now = true,
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
    use std::time::{Duration, Instant};

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    #[test]
    fn source_retains_last_good_value_and_marks_stale_after_threshold() {
        let started = Instant::now();
        let mut source = Source::default();
        source.success("good", started);
        source.failure("Linear unavailable");
        assert_eq!(source.value, Some("good"));
        assert_eq!(source.error.as_deref(), Some("Linear unavailable"));
        assert!(!source.is_stale(started + STALE_AFTER));
        assert!(source.is_stale(started + STALE_AFTER + Duration::from_millis(1)));
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
