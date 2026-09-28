// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Rendering for the read-only Herdr activity monitor.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

use crate::monitor::{App, BusState, HerdrState, display_text, registration_is_stale};

/// Renders the current pane, Herdr agent sessions, bus registrations and
/// recent bus activity without performing I/O.
pub fn render(frame: &mut Frame, app: &App, now: Instant) {
    let [header, current, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new("Herdr activity monitor  ·  r refresh  q quit"),
        header,
    );
    render_current(frame, app.herdr.value.as_ref(), current);

    let [agents, bus] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(body);
    render_agents(frame, app.herdr.value.as_ref(), agents);
    render_bus(frame, app.bus.value.as_ref(), bus);
    render_footer(frame, app, now, footer);
}

fn render_current(frame: &mut Frame, state: Option<&HerdrState>, area: ratatui::layout::Rect) {
    let lines = state
        .and_then(|state| state.current.as_ref())
        .map_or_else(
            || {
                vec![
                    Line::from(
                        "Current pane: unavailable (HERDR_PANE_ID missing or pane absent from snapshot)",
                    ),
                    Line::from("Session and activity unavailable"),
                ]
            },
            |pane| {
                let location = [pane.workspace.as_deref(), pane.tab.as_deref()]
                    .into_iter()
                    .flatten()
                    .map(|part| display_text(part, 16))
                    .collect::<Vec<_>>()
                    .join("/");
                let title = pane.terminal_title.as_deref().unwrap_or("untitled");
                let agent = pane.agent.as_deref().unwrap_or("no agent");
                let session = pane.session.as_deref().unwrap_or("no session id");
                vec![
                    Line::from(format!(
                        "Current: {} {} · {} · {}",
                        display_text(&pane.pane_id, 16),
                        if location.is_empty() { "" } else { "@" },
                        location,
                        display_text(&pane.status, 10)
                    )),
                    Line::from(format!(
                        "Session: {} {} · {}",
                        display_text(agent, 12),
                        display_text(session, 16),
                        display_text(title, 32)
                    )),
                ]
            },
        );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_agents(frame: &mut Frame, state: Option<&HerdrState>, area: ratatui::layout::Rect) {
    let rows: Vec<Row> = state
        .into_iter()
        .flat_map(|state| state.agents.iter())
        .map(|agent| {
            let location = [agent.workspace.as_deref(), agent.tab.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("/");
            Row::new(vec![
                Cell::from(display_text(&agent.agent, 12)),
                Cell::from(display_text(&agent.status, 9)),
                Cell::from(display_text(&agent.pane_id, 9)),
                Cell::from(display_text(agent.session.as_deref().unwrap_or(""), 14)),
                Cell::from(display_text(&location, 14)),
                Cell::from(display_text(
                    agent.terminal_title.as_deref().unwrap_or(""),
                    34,
                )),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new([
            "Agent",
            "Status",
            "Pane",
            "Session",
            "Workspace/Tab",
            "Terminal title",
        ])
        .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Herdr sessions"),
    );
    frame.render_widget(table, area);
}

fn render_bus(frame: &mut Frame, state: Option<&BusState>, area: ratatui::layout::Rect) {
    let now_ms = unix_ms();
    let [registrations_area, messages_area] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let registration_rows: Vec<Row> = state
        .into_iter()
        .flat_map(|state| state.registrations.iter())
        .map(|agent| {
            Row::new(vec![
                Cell::from(display_text(&agent.name, 16)),
                Cell::from(display_text(&agent.kind, 8)),
                Cell::from(display_text(&agent.machine, 12)),
                Cell::from(display_text(
                    agent.place.as_deref().unwrap_or("unplaced"),
                    22,
                )),
                Cell::from(if registration_is_stale(agent.last_seen_ms, now_ms) {
                    "STALE"
                } else {
                    "recent"
                }),
            ])
        })
        .collect();
    frame.render_widget(
        Table::new(
            registration_rows,
            [
                Constraint::Length(16),
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Min(10),
                Constraint::Length(7),
            ],
        )
        .header(
            Row::new(["Lead", "Kind", "Machine", "Herdr place", "Seen"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(Block::default().borders(Borders::ALL).title(format!(
            "Relay registrations ({} unreadable)",
            state.map_or(0, |bus| bus.skipped_registrations.len())
        ))),
        registrations_area,
    );

    let messages: Vec<Line> = state
        .into_iter()
        .flat_map(|state| state.messages.iter().rev())
        .take(8)
        .map(|message| {
            Line::from(vec![
                Span::styled(
                    display_text(&message.from, 12),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(format!(": {}", display_text(&message.text, 72))),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(messages).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recent bus messages"),
        ),
        messages_area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let herdr_state = source_label("Herdr", &app.herdr, now);
    let bus_state = source_label("Bus", &app.bus, now);
    let notes = "Collaboration subagents appear only if registered in Herdr/relay.";
    let partial = app
        .bus
        .value
        .as_ref()
        .filter(|bus| !bus.skipped_registrations.is_empty())
        .map(|bus| {
            format!(
                "Bus partial: {} unreadable registration(s): {}",
                bus.skipped_registrations.len(),
                bus.skipped_registrations
                    .iter()
                    .take(2)
                    .map(|label| display_text(label, 48))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("{herdr_state}  |  {bus_state}")),
            Line::from(partial),
            Line::from(notes),
        ])
        .style(Style::default().fg(Color::Gray))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn source_label<T>(name: &str, source: &crate::monitor::Source<T>, now: Instant) -> String {
    let freshness = match source.last_success {
        Some(last) => format!("ok {}s ago", now.saturating_duration_since(last).as_secs()),
        None => "no data".to_owned(),
    };
    let freshness = if source.is_stale(now) {
        format!("STALE {freshness}")
    } else {
        freshness
    };
    let error = source
        .error
        .as_deref()
        .map(|error| format!(" · unavailable: {}", display_text(error, 64)))
        .unwrap_or_default();
    format!("{name}: {freshness}{error}")
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::monitor::{BusAgent, BusMessage, CurrentPane, HerdrAgent};

    #[test]
    fn rendering_includes_current_session_registrations_messages_and_error_state() {
        let now = Instant::now();
        let mut app = App::default();
        app.herdr.success(
            HerdrState {
                current: Some(CurrentPane {
                    pane_id: "w1:p2".into(),
                    workspace: Some("SWM".into()),
                    tab: Some("build".into()),
                    terminal_title: Some("Implement monitor".into()),
                    agent: Some("codex".into()),
                    session: Some("session-7".into()),
                    status: "working".into(),
                }),
                agents: vec![HerdrAgent {
                    agent: "codex".into(),
                    status: "working".into(),
                    pane_id: "w1:p2".into(),
                    ..HerdrAgent::default()
                }],
            },
            now,
        );
        app.bus.success(
            BusState {
                registrations: vec![BusAgent {
                    name: "swm-lead".into(),
                    kind: "codex".into(),
                    machine: "luna".into(),
                    place: Some("SWM/build".into()),
                    ..BusAgent::default()
                }],
                skipped_registrations: vec!["bad-entry: invalid JSON".into()],
                messages: vec![BusMessage {
                    from: "swm-lead".into(),
                    text: "starting first draft".into(),
                    sent_at_ms: 1,
                }],
            },
            now,
        );
        app.bus.failure("Redis unavailable");

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|frame| render(frame, &app, now)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let width = usize::from(buffer.area.width);
        let content = buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<String>();
        assert!(content.contains("Current: w1:p2"));
        assert!(content.contains("session-7"));
        assert!(content.contains("Implement monitor"));
        assert!(content.contains("swm-lead"));
        assert!(content.contains("STALE"));
        assert!(content.contains("unreadable registration(s)"));
        assert!(content.contains("starting first draft"));
        assert!(content.contains("unavailable: Redis unavailable"));
    }

    #[test]
    fn current_pane_herdr_fields_are_sanitized_before_rendering() {
        let now = Instant::now();
        let mut app = App::default();
        app.herdr.success(
            HerdrState {
                current: Some(CurrentPane {
                    pane_id: "w1:p2\u{1b}[2J".into(),
                    workspace: Some("SWM\u{202e}fake".into()),
                    tab: Some("build\nforged".into()),
                    terminal_title: Some("safe title".into()),
                    agent: Some("codex\u{1b}[31m".into()),
                    session: Some("session-7".into()),
                    status: "working\u{202e}blocked".into(),
                }),
                agents: Vec::new(),
            },
            now,
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();
        terminal.draw(|frame| render(frame, &app, now)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let width = usize::from(buffer.area.width);
        let content = buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<String>();
        assert!(!content.contains('\u{1b}'));
        assert!(!content.contains('\u{202e}'));
        assert!(content.contains("w1:p2"));
        assert!(content.contains("SWM"));
        assert!(content.contains("working"));
    }
}
