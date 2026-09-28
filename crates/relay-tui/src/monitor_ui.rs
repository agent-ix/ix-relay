// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Rendering for the read-only Herdr activity monitor.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

use crate::monitor::{
    App, BusState, HerdrState, display_text, registration_is_stale, tracker_is_stale,
};
use crate::monitor_tracker::{BlockerState, TicketSummary};

const PRIMARY: Color = Color::Rgb(79, 141, 247); // Cloud Command #4f8df7
const FOREGROUND: Color = Color::Rgb(230, 230, 230); // Cloud Command #e6e6e6
const MUTED: Color = Color::Rgb(139, 146, 156); // Cloud Command #8b929c
const BORDER: Color = Color::Rgb(42, 46, 53); // Cloud Command #2a2e35
const SUCCESS: Color = Color::Rgb(87, 201, 138); // Cloud Command #57c98a
const WARNING: Color = Color::Rgb(224, 161, 92); // Cloud Command #e0a15c
const ERROR: Color = Color::Rgb(240, 163, 163); // Cloud Command #f0a3a3

/// Renders SWM tracker status first, with Herdr and relay context below.
pub fn render(frame: &mut Frame, app: &App, now: Instant) {
    if frame.area().height < 25 {
        render_compact(frame, app, now);
        return;
    }
    render_full(frame, app, now);
}

fn render_full(frame: &mut Frame, app: &App, now: Instant) {
    let [header, identity, projects, tickets, supporting, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(5),
        Constraint::Length(7),
        Constraint::Min(6),
        Constraint::Length(4),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new("◆  SWM WORK TRACKER  ·  r refresh  q quit")
            .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
        header,
    );
    render_identity(frame, app, now, identity);
    render_projects(frame, app, now, projects);
    render_tickets(frame, app, now, tickets);

    let [agents, bus] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(supporting);
    render_agents(frame, app.herdr.value.as_ref(), agents);
    render_bus(frame, app.bus.value.as_ref(), bus);
    render_footer(frame, app, now, footer);
}

fn render_compact(frame: &mut Frame, app: &App, now: Instant) {
    if frame.area().height >= 20 {
        render_compact_with_support(frame, app, now);
        return;
    }
    let [header, identity, projects, tickets, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(4),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new("◆ SWM WORK TRACKER · r refresh · q quit")
            .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
        header,
    );
    render_compact_identity(frame, app, now, identity);
    render_compact_projects(frame, app, now, projects);
    render_compact_tickets(frame, app, now, tickets);
    render_compact_footer(frame, app, now, footer);
}

fn render_compact_with_support(frame: &mut Frame, app: &App, now: Instant) {
    let [header, identity, projects, tickets, supporting, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(4),
        Constraint::Length(4),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new("◆ SWM WORK TRACKER · r refresh · q quit")
            .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
        header,
    );
    render_compact_identity(frame, app, now, identity);
    render_compact_projects(frame, app, now, projects);
    render_compact_tickets(frame, app, now, tickets);
    let [agents, bus] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(supporting);
    render_agents(frame, app.herdr.value.as_ref(), agents);
    render_bus(frame, app.bus.value.as_ref(), bus);
    render_footer(frame, app, now, footer);
}

fn render_compact_identity(frame: &mut Frame, app: &App, now: Instant, area: Rect) {
    let health = lane_health(app, now);
    let health_style = match health.as_str() {
        value if value.contains("BLOCKED") => {
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD)
        }
        "NEXT TICKET READY" => Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
        value if value.starts_with("NO READY WORK") => Style::default().fg(WARNING),
        _ => Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "luna:SWM:code",
                Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" · SWM · "),
            Span::raw("Lane signal "),
            Span::styled(health, health_style),
        ])),
        area,
    );
}

fn render_compact_projects(
    frame: &mut Frame,
    app: &App,
    now: Instant,
    area: ratatui::layout::Rect,
) {
    let fresh = app
        .tracker
        .value
        .as_ref()
        .filter(|_| !tracker_is_stale(&app.tracker, now) && app.tracker.error.is_none());
    let mut lines = vec![Line::from("Projects · status · active · done")];
    for name in ["ix-board", "ix-projects"] {
        let project = fresh.and_then(|tracker| tracker.projects.iter().find(|p| p.name == name));
        let line = project.filter(|project| project.available).map_or_else(
            || format!("{name} · unknown · ? · ?"),
            |project| {
                format!(
                    "{} · {} · {} · {}",
                    display_text(&project.name, 20),
                    display_text(&project.status, 12),
                    project.active_count,
                    project.done_count
                )
            },
        );
        lines.push(Line::from(line));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FOREGROUND)),
        area,
    );
}

fn render_compact_tickets(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let tracker = app
        .tracker
        .value
        .as_ref()
        .filter(|_| !tracker_is_stale(&app.tracker, now) && app.tracker.error.is_none());
    let empty = if tracker.is_some() {
        "none found"
    } else {
        "unknown · waiting for fresh Linear data"
    };
    let current = tracker.map_or(&[][..], |state| state.current.as_slice());
    let omitted = current.len().saturating_sub(2);
    let suffix = if omitted > 0 {
        format!(" (+{omitted})")
    } else {
        String::new()
    };
    let mut lines = vec![Line::from(format!(
        "Tickets · state · priority · blockers · {} open{suffix}",
        current.len()
    ))];
    if current.is_empty() {
        lines.push(Line::from(format!("Current: {empty}")));
    } else {
        for ticket in current.iter().take(2) {
            lines.push(Line::from(format!(
                "Current: {} · {} · {} · {}",
                display_text(&ticket.identifier, 14),
                display_text(&ticket.state, 10),
                ticket.priority,
                blocker_line(&ticket.blockers)
            )));
        }
    }
    if let Some(ticket) = tracker.and_then(|state| state.next.as_ref()) {
        lines.push(Line::from(format!(
            "Next: {} · {} · {} · {}",
            display_text(&ticket.identifier, 14),
            display_text(&ticket.state, 10),
            ticket.priority,
            blocker_line(&ticket.blockers)
        )));
    } else {
        lines.push(Line::from(format!("Next: {empty}")));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(FOREGROUND)),
        area,
    );
}

fn render_compact_footer(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let pane = app
        .herdr
        .value
        .as_ref()
        .and_then(|state| state.current.as_ref())
        .map(|pane| {
            format!(
                "{} {}",
                display_text(&pane.pane_id, 12),
                display_text(&pane.status, 10)
            )
        })
        .unwrap_or_else(|| "unavailable".to_owned());
    let tracker_age = app.tracker.last_success.map_or_else(
        || "no data".to_owned(),
        |last| {
            let age = now.saturating_duration_since(last).as_secs();
            if tracker_is_stale(&app.tracker, now) {
                format!("STALE {age}s ago")
            } else {
                format!("ok {age}s ago")
            }
        },
    );
    let tracker_error = app
        .tracker
        .error
        .as_deref()
        .map_or_else(String::new, |error| {
            format!(" · {}", display_text(error, 68))
        });
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "Herdr: {} · Bus: {}",
                short_source_status(&app.herdr, now),
                short_source_status(&app.bus, now)
            )),
            Line::from(vec![
                Span::styled(format!("Linear: {tracker_age}"), Style::default().fg(MUTED)),
                Span::styled(tracker_error, Style::default().fg(ERROR)),
            ]),
            Line::from(format!("Herdr pane: {pane}")),
            Line::from("Drift history unavailable · only registered agents appear"),
        ])
        .style(Style::default().fg(MUTED))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn short_source_status<T>(source: &crate::monitor::Source<T>, now: Instant) -> String {
    let Some(last) = source.last_success else {
        return source.error.as_deref().map_or_else(
            || "no data".to_owned(),
            |error| format!("unavailable: {}", display_text(error, 32)),
        );
    };
    if source.is_stale(now) {
        format!("STALE {}s", now.saturating_duration_since(last).as_secs())
    } else if let Some(error) = &source.error {
        format!("unavailable: {}", display_text(error, 32))
    } else {
        format!("ok {}s", now.saturating_duration_since(last).as_secs())
    }
}

fn render_identity(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let health = lane_health(app, now);
    let health_style = match health.as_str() {
        value if value.contains("BLOCKED") => {
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD)
        }
        "NEXT TICKET READY" => Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
        value if value.starts_with("NO READY WORK") => Style::default().fg(WARNING),
        _ => Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Lead ", Style::default().fg(MUTED)),
                Span::styled(
                    "luna:SWM:code",
                    Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                ),
                Span::raw("   Team "),
                Span::styled(
                    "SWM",
                    Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw("Lane signal: "),
                Span::styled(health, health_style),
                Span::raw("  ·  based on Linear project, ticket and direct blocker state"),
            ]),
        ]),
        area,
    );
}

fn render_projects(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let stale = tracker_is_stale(&app.tracker, now) || app.tracker.error.is_some();
    let rows = app
        .tracker
        .value
        .as_ref()
        .map(|tracker| {
            tracker
                .projects
                .iter()
                .map(|project| {
                    if stale || !project.available {
                        Row::new([project.name.as_str(), "unknown", "?", "?"])
                    } else {
                        Row::new(vec![
                            display_text(&project.name, 24),
                            display_text(&project.status, 12),
                            project.active_count.to_string(),
                            project.done_count.to_string(),
                        ])
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            ["ix-board", "ix-projects"]
                .into_iter()
                .map(|name| Row::new([name, "unknown", "?", "?"]))
                .collect()
        });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Min(20),
                Constraint::Length(14),
                Constraint::Length(9),
                Constraint::Length(8),
            ],
        )
        .style(Style::default().fg(FOREGROUND))
        .header(
            Row::new(["Project", "Status", "Active", "Done"])
                .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title("Project tracker"),
        ),
        area,
    );
}

fn render_tickets(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let [current, next] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let stale = tracker_is_stale(&app.tracker, now) || app.tracker.error.is_some();
    let tracker = app.tracker.value.as_ref().filter(|_| !stale);
    render_ticket_list(
        frame,
        "Current open tickets",
        tracker.map_or(&[][..], |value| value.current.as_slice()),
        current,
    );
    render_ticket(
        frame,
        "Next · ix-board ready queue",
        tracker.and_then(|value| value.next.as_ref()),
        if tracker.is_some() {
            "no ready ticket"
        } else {
            "unknown · waiting for a fresh Linear snapshot"
        },
        next,
    );
}

fn render_ticket(
    frame: &mut Frame,
    title: &str,
    ticket: Option<&TicketSummary>,
    empty_message: &str,
    area: ratatui::layout::Rect,
) {
    let lines = ticket.map_or_else(
        || vec![Line::from(empty_message)],
        |ticket| {
            vec![
                Line::from(vec![
                    Span::styled(
                        display_text(&ticket.identifier, 16),
                        Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!("  {}", display_text(&ticket.title, 52))),
                ]),
                Line::from(format!(
                    "{} · {} · priority {}",
                    display_text(&ticket.project, 20),
                    display_text(&ticket.state, 12),
                    ticket.priority
                )),
                Line::from(blocker_line(&ticket.blockers)),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(FOREGROUND))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(title),
            ),
        area,
    );
}

fn render_ticket_list(
    frame: &mut Frame,
    title: &str,
    tickets: &[TicketSummary],
    area: ratatui::layout::Rect,
) {
    let mut lines = Vec::new();
    for ticket in tickets.iter().take(2) {
        lines.push(Line::from(format!(
            "{} · {} · priority {}",
            display_text(&ticket.identifier, 14),
            display_text(&ticket.state, 10),
            ticket.priority
        )));
        lines.push(Line::from(format!(
            "{} · {}",
            display_text(&ticket.title, 38),
            blocker_line(&ticket.blockers)
        )));
    }
    if tickets.len() > 2 {
        lines.push(Line::from(format!(
            "+{} more open ticket(s)",
            tickets.len() - 2
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from("none found"));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(FOREGROUND))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(title),
            ),
        area,
    );
}

fn blocker_line(blockers: &BlockerState) -> String {
    match blockers {
        BlockerState::Unblocked => "Blockers: unblocked".to_owned(),
        BlockerState::Blocked(ids) => format!("Blocked by: {}", display_text(&ids.join(", "), 72)),
        BlockerState::Unknown(ids) => {
            format!("Blockers: unknown ({})", display_text(&ids.join(", "), 60))
        }
    }
}

fn lane_health(app: &App, now: Instant) -> String {
    if tracker_is_stale(&app.tracker, now) || app.tracker.error.is_some() {
        return "UNKNOWN · Linear unavailable or stale".to_owned();
    }
    let Some(tracker) = app.tracker.value.as_ref() else {
        return "UNKNOWN · waiting for Linear".to_owned();
    };
    if tracker.projects.iter().any(|project| !project.available) {
        return "UNKNOWN · project data missing".to_owned();
    }
    if tracker
        .current
        .iter()
        .any(|ticket| matches!(ticket.blockers, BlockerState::Unknown(_)))
    {
        return "UNKNOWN · blocker reference missing".to_owned();
    }
    if tracker.next.is_some() {
        "NEXT TICKET READY".to_owned()
    } else if tracker
        .current
        .iter()
        .any(|ticket| matches!(ticket.blockers, BlockerState::Blocked(_)))
    {
        "OPEN WORK BLOCKED".to_owned()
    } else if tracker
        .projects
        .iter()
        .all(|project| project.status.eq_ignore_ascii_case("Paused"))
    {
        "NO READY WORK · projects paused".to_owned()
    } else {
        "NO READY WORK".to_owned()
    }
}

fn render_agents(frame: &mut Frame, state: Option<&HerdrState>, area: ratatui::layout::Rect) {
    let [host_area, sessions_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(3)]).areas(area);
    let host = state.and_then(|state| state.current.as_ref()).map_or_else(
        || vec![Line::from("Current pane unavailable")],
        |pane| {
            vec![Line::from(format!(
                "Current: {} · {} · {} · {} · {}",
                display_text(&pane.pane_id, 12),
                display_text(pane.agent.as_deref().unwrap_or("no agent"), 12),
                display_text(&pane.status, 10),
                display_text(pane.session.as_deref().unwrap_or("no session"), 16),
                display_text(pane.terminal_title.as_deref().unwrap_or("untitled"), 24)
            ))]
        },
    );
    frame.render_widget(
        Paragraph::new(host)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(FOREGROUND)),
        host_area,
    );
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
            .border_style(Style::default().fg(BORDER))
            .title("Herdr sessions"),
    );
    frame.render_widget(table, sessions_area);
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
        .style(Style::default().fg(FOREGROUND))
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
                    Style::default().fg(PRIMARY),
                ),
                Span::raw(format!(": {}", display_text(&message.text, 72))),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(messages).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title("Recent bus messages"),
        ),
        messages_area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let herdr_state = source_label("Herdr", &app.herdr, now);
    let bus_state = source_label("Bus", &app.bus, now);
    let tracker_freshness = match app.tracker.last_success {
        Some(last) if !tracker_is_stale(&app.tracker, now) => {
            format!(
                "Linear: ok {}s ago",
                now.saturating_duration_since(last).as_secs()
            )
        }
        Some(last) => format!(
            "Linear: STALE {}s ago",
            now.saturating_duration_since(last).as_secs()
        ),
        None => "Linear: no data".to_owned(),
    };
    let tracker_error = app
        .tracker
        .error
        .as_deref()
        .map_or_else(String::new, |error| {
            format!(" · unavailable: {}", display_text(error, 64))
        });
    let notes = "Collaboration subagents appear only if registered in Herdr/relay; ix-board drift history is not available here.";
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
            Line::from(vec![
                Span::styled(tracker_freshness, Style::default().fg(MUTED)),
                Span::styled(tracker_error, Style::default().fg(ERROR)),
            ]),
            Line::from(partial),
            Line::from(notes),
        ])
        .style(Style::default().fg(MUTED))
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
    use crate::monitor_tracker::TrackerState;

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

        app.tracker.success(
            TrackerState {
                projects: vec![
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-board".into(),
                        status: "Started".into(),
                        active_count: 2,
                        done_count: 1,
                        available: true,
                    },
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-projects".into(),
                        status: "Paused".into(),
                        active_count: 1,
                        done_count: 3,
                        available: true,
                    },
                ],
                current: vec![TicketSummary {
                    identifier: "SWM-21".into(),
                    title: "Monitor pane".into(),
                    project: "ix-relay".into(),
                    state: "Coding".into(),
                    priority: "High".into(),
                    blockers: BlockerState::Unblocked,
                }],
                next: Some(TicketSummary {
                    identifier: "SWM-22".into(),
                    title: "Next work".into(),
                    project: "ix-board".into(),
                    state: "Specced".into(),
                    priority: "Medium".into(),
                    blockers: BlockerState::Unblocked,
                }),
            },
            now,
        );
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
        assert!(content.contains("luna:SWM:code"));
        assert!(content.contains("SWM WORK TRACKER"));
        assert!(content.contains("ix-board"));
        assert!(content.contains("SWM-21"));
        assert!(content.contains("SWM-22"));
        assert!(content.contains("session-7"));
        assert!(content.contains("Implement monitor"));
        assert!(content.contains("swm-lead"));
        assert!(content.contains("STALE"));
        assert!(content.contains("unreadable registration(s)"));
        assert!(content.contains("starting first draft"));
        assert!(content.contains("unavailable: Redis unavailable"));
    }

    #[test]
    fn compact_view_shows_project_counts_and_review_tickets_in_a_short_pane() {
        let now = Instant::now();
        let mut app = App::default();
        app.tracker.success(
            TrackerState {
                projects: vec![
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-board".into(),
                        status: "Started".into(),
                        active_count: 0,
                        done_count: 4,
                        available: true,
                    },
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-projects".into(),
                        status: "Started".into(),
                        active_count: 3,
                        done_count: 4,
                        available: true,
                    },
                ],
                current: vec![
                    TicketSummary {
                        identifier: "SWM-12".into(),
                        title: "Review first change".into(),
                        project: "ix-projects".into(),
                        state: "Review".into(),
                        priority: "High".into(),
                        blockers: BlockerState::Unblocked,
                    },
                    TicketSummary {
                        identifier: "SWM-13".into(),
                        title: "Review second change".into(),
                        project: "ix-projects".into(),
                        state: "Review".into(),
                        priority: "Medium".into(),
                        blockers: BlockerState::Unblocked,
                    },
                ],
                next: Some(TicketSummary {
                    identifier: "SWM-8".into(),
                    title: "Next ready ticket".into(),
                    project: "ix-board".into(),
                    state: "Specced".into(),
                    priority: "Urgent".into(),
                    blockers: BlockerState::Unblocked,
                }),
            },
            now,
        );
        let mut terminal = Terminal::new(TestBackend::new(103, 22)).unwrap();
        terminal.draw(|frame| render(frame, &app, now)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let width = usize::from(buffer.area.width);
        let content = buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<String>();
        assert!(content.contains("Lane signal NEXT TICKET READY"));
        assert!(content.contains("ix-board · Started · 0 · 4"));
        assert!(content.contains("ix-projects · Started · 3 · 4"));
        assert!(content.contains("SWM-12 · Review · High"));
        assert!(content.contains("SWM-13 · Review · Medium"));
        assert!(content.contains("Next: SWM-8 · Specced · Urgent"));
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
        app.tracker.success(
            TrackerState {
                projects: vec![
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-board".into(),
                        status: "Started".into(),
                        active_count: 0,
                        done_count: 0,
                        available: true,
                    },
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-projects".into(),
                        status: "Started".into(),
                        active_count: 0,
                        done_count: 0,
                        available: true,
                    },
                ],
                ..TrackerState::default()
            },
            now,
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
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

    #[test]
    fn lane_health_uses_unknown_for_missing_stale_or_failed_tracker_data() {
        let now = Instant::now();
        let mut app = App::default();
        assert!(lane_health(&app, now).starts_with("UNKNOWN"));

        app.tracker.success(
            TrackerState {
                projects: vec![
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-board".into(),
                        status: "Started".into(),
                        active_count: 1,
                        done_count: 0,
                        available: true,
                    },
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-projects".into(),
                        status: "Started".into(),
                        active_count: 0,
                        done_count: 0,
                        available: false,
                    },
                ],
                ..TrackerState::default()
            },
            now,
        );
        assert!(lane_health(&app, now).starts_with("UNKNOWN"));

        app.tracker.success(
            TrackerState {
                projects: vec![
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-board".into(),
                        status: "Started".into(),
                        active_count: 1,
                        done_count: 0,
                        available: true,
                    },
                    crate::monitor_tracker::ProjectSummary {
                        name: "ix-projects".into(),
                        status: "Started".into(),
                        active_count: 0,
                        done_count: 0,
                        available: true,
                    },
                ],
                current: vec![TicketSummary {
                    blockers: BlockerState::Blocked(vec!["SWM-3".to_owned()]),
                    ..TicketSummary::default()
                }],
                ..TrackerState::default()
            },
            now,
        );
        assert_eq!(lane_health(&app, now), "OPEN WORK BLOCKED");
        assert!(
            lane_health(
                &app,
                now + crate::monitor::TRACKER_STALE_AFTER + std::time::Duration::from_secs(1)
            )
            .starts_with("UNKNOWN")
        );
        app.tracker.failure("Linear API unavailable");
        assert!(lane_health(&app, now).starts_with("UNKNOWN"));
    }
}
