// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Rendering for the read-only team-leader dashboard.

use std::time::Instant;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

use crate::monitor::{App, display_text, tracker_is_stale};
use crate::monitor_tracker::{BlockerState, TicketSummary};

const PRIMARY: Color = Color::Rgb(79, 141, 247); // Cloud Command #4f8df7
const FOREGROUND: Color = Color::Rgb(230, 230, 230); // Cloud Command #e6e6e6
const MUTED: Color = Color::Rgb(139, 146, 156); // Cloud Command #8b929c
const BORDER: Color = Color::Rgb(42, 46, 53); // Cloud Command #2a2e35
const SUCCESS: Color = Color::Rgb(87, 201, 138); // Cloud Command #57c98a
const WARNING: Color = Color::Rgb(224, 161, 92); // Cloud Command #e0a15c
const ERROR: Color = Color::Rgb(240, 163, 163); // Cloud Command #f0a3a3

/// Renders only the current lead's assigned team and project tracker state.
pub fn render(frame: &mut Frame, app: &App, now: Instant) {
    let [header, identity, projects, tickets, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(5),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new("◆  TEAM LEADER DASHBOARD  ·  r refresh  q quit")
            .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
        header,
    );
    render_identity(frame, app, now, identity);
    render_projects(frame, app, now, projects);
    render_tickets(frame, app, now, tickets);
    render_footer(frame, app, now, footer);
}

fn render_identity(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let health = lane_health(app, now);
    let health_style = if health.contains("BLOCKED") {
        Style::default().fg(WARNING).add_modifier(Modifier::BOLD)
    } else if health == "NEXT TICKET READY" {
        Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
    } else if health.starts_with("NO READY WORK") {
        Style::default().fg(WARNING).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Lead ", Style::default().fg(MUTED)),
                Span::styled(
                    display_text(&app.lead_name, 36),
                    Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                ),
                Span::raw("   Team "),
                Span::styled(
                    "SWM",
                    Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
                ),
                Span::raw("   Projects ix-board · ix-projects"),
            ]),
            Line::from(vec![
                Span::raw("Lane signal: "),
                Span::styled(health, health_style),
            ]),
        ])
        .style(Style::default().fg(FOREGROUND)),
        area,
    );
}

fn render_projects(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let fresh = app
        .tracker
        .value
        .as_ref()
        .filter(|_| !tracker_is_stale(&app.tracker, now) && app.tracker.error.is_none());
    let rows: Vec<Row> = ["ix-board", "ix-projects"]
        .into_iter()
        .map(|name| {
            let project =
                fresh.and_then(|tracker| tracker.projects.iter().find(|p| p.name == name));
            let values = project.filter(|project| project.available).map_or_else(
                || {
                    vec![
                        name.to_owned(),
                        "unknown".to_owned(),
                        "?".to_owned(),
                        "?".to_owned(),
                    ]
                },
                |project| {
                    vec![
                        display_text(&project.name, 24),
                        display_text(&project.status, 18),
                        project.active_count.to_string(),
                        project.done_count.to_string(),
                    ]
                },
            );
            Row::new(values.into_iter().map(Cell::from))
        })
        .collect();
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Percentage(36),
                Constraint::Percentage(30),
                Constraint::Percentage(17),
                Constraint::Percentage(17),
            ],
        )
        .header(
            Row::new(["Project", "Status", "Active", "Done"])
                .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
        )
        .style(Style::default().fg(FOREGROUND))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title("Assigned projects"),
        ),
        area,
    );
}

fn render_tickets(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let tracker = app
        .tracker
        .value
        .as_ref()
        .filter(|_| !tracker_is_stale(&app.tracker, now) && app.tracker.error.is_none());
    let mut lines = Vec::new();
    if let Some(tracker) = tracker {
        lines.push(Line::from(format!(
            "Current open tickets · {}",
            tracker.current.len()
        )));
        if tracker.current.is_empty() {
            lines.push(Line::from(Span::styled(
                "No open tickets in ix-board or ix-projects",
                Style::default().fg(MUTED),
            )));
        } else {
            for ticket in tracker.current.iter().take(max_ticket_lines(area.height)) {
                lines.push(ticket_line("Current", ticket));
            }
            let omitted = tracker
                .current
                .len()
                .saturating_sub(max_ticket_lines(area.height));
            if omitted > 0 {
                lines.push(Line::from(Span::styled(
                    format!("… and {omitted} more open tickets"),
                    Style::default().fg(MUTED),
                )));
            }
        }
        if let Some(ticket) = tracker.next.as_ref() {
            lines.push(ticket_line("Next", ticket));
        } else {
            lines.push(Line::from(Span::styled(
                "Next: no ready project ticket",
                Style::default().fg(MUTED),
            )));
        }
    } else {
        lines.push(Line::from(
            "Tickets · unknown while Linear data is stale or unavailable",
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(FOREGROUND))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title("Current work and next ready ticket"),
            ),
        area,
    );
}

fn max_ticket_lines(height: u16) -> usize {
    usize::from(height.saturating_sub(5).max(1))
}

fn ticket_line(label: &str, ticket: &TicketSummary) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(PRIMARY)),
        Span::styled(
            display_text(&ticket.identifier, 16),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " · {} · {} · {} · {}",
            display_text(&ticket.state, 14),
            display_text(&ticket.project, 18),
            ticket.priority,
            blocker_line(&ticket.blockers)
        )),
        Span::raw(format!(" · {}", display_text(&ticket.title, 64))),
    ])
}

fn blocker_line(blockers: &BlockerState) -> String {
    match blockers {
        BlockerState::Unblocked => "no blockers".to_owned(),
        BlockerState::Blocked(ids) => format!("blocked by {}", ids.join(", ")),
        BlockerState::Unknown(ids) => format!("blockers unknown: {}", ids.join(", ")),
    }
}

fn lane_health(app: &App, now: Instant) -> String {
    if tracker_is_stale(&app.tracker, now) || app.tracker.error.is_some() {
        return "UNKNOWN · tracker data stale or unavailable".to_owned();
    }
    let Some(tracker) = app.tracker.value.as_ref() else {
        return "UNKNOWN · waiting for tracker data".to_owned();
    };
    if tracker.projects.iter().any(|project| !project.available) {
        return "UNKNOWN · assigned project missing".to_owned();
    }
    if tracker
        .current
        .iter()
        .any(|ticket| matches!(ticket.blockers, BlockerState::Blocked(_)))
    {
        return "OPEN WORK BLOCKED".to_owned();
    }
    if tracker.next.is_some() {
        "NEXT TICKET READY".to_owned()
    } else {
        "NO READY WORK".to_owned()
    }
}

fn render_footer(frame: &mut Frame, app: &App, now: Instant, area: ratatui::layout::Rect) {
    let freshness = match app.tracker.last_success {
        Some(last) if !tracker_is_stale(&app.tracker, now) => {
            format!(
                "Linear: fresh {}s ago",
                now.saturating_duration_since(last).as_secs()
            )
        }
        Some(last) => format!(
            "Linear: STALE {}s ago",
            now.saturating_duration_since(last).as_secs()
        ),
        None => "Linear: no data".to_owned(),
    };
    let error = app
        .tracker
        .error
        .as_deref()
        .map_or_else(String::new, |error| {
            format!(" · {}", display_text(error, 72))
        });
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(freshness, Style::default().fg(MUTED)),
            Span::styled(error, Style::default().fg(ERROR)),
        ]))
        .wrap(Wrap { trim: true }),
        area,
    );
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::monitor_tracker::{ProjectSummary, TrackerState};

    fn rendered(app: &App, width: u16, height: u16, now: Instant) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app, now)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let width = usize::from(buffer.area.width);
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn dashboard_focuses_on_assigned_projects_work_and_freshness() {
        let now = Instant::now();
        let mut app = App {
            lead_name: "luna:SWM:code".into(),
            ..App::default()
        };
        app.tracker.success(
            TrackerState {
                projects: vec![
                    ProjectSummary {
                        name: "ix-board".into(),
                        status: "Started".into(),
                        active_count: 2,
                        done_count: 1,
                        available: true,
                    },
                    ProjectSummary {
                        name: "ix-projects".into(),
                        status: "Paused".into(),
                        active_count: 1,
                        done_count: 3,
                        available: true,
                    },
                ],
                current: vec![TicketSummary {
                    identifier: "SWM-21".into(),
                    title: "Team Leader Dashboard".into(),
                    project: "ix-relay".into(),
                    state: "Coding".into(),
                    priority: "High".into(),
                    blockers: BlockerState::Blocked(vec!["SWM-20".into()]),
                }],
                next: Some(TicketSummary {
                    identifier: "SWM-22".into(),
                    title: "Next project work".into(),
                    project: "ix-board".into(),
                    state: "Specced".into(),
                    priority: "Medium".into(),
                    blockers: BlockerState::Unblocked,
                }),
            },
            now,
        );
        let content = rendered(&app, 120, 30, now);
        assert!(content.contains("TEAM LEADER DASHBOARD"));
        assert!(content.contains("luna:SWM:code"));
        assert!(content.contains("ix-board"));
        assert!(content.contains("ix-projects"));
        assert!(content.contains("SWM-21"));
        assert!(content.contains("SWM-22"));
        assert!(content.contains("blocked by SWM-20"));
        assert!(content.contains("Linear: fresh"));
        assert!(!content.contains("Herdr sessions"));
        assert!(!content.contains("Relay registrations"));
        assert!(!content.contains("Recent bus messages"));
        assert!(!content.contains("Agent Status"));
    }

    #[test]
    fn dashboard_shows_tracker_errors_and_unknown_lane_state() {
        let now = Instant::now();
        let mut app = App {
            lead_name: "swm-lead".into(),
            ..App::default()
        };
        app.tracker.failure("Linear API unavailable");
        let content = rendered(&app, 100, 20, now);
        assert!(content.contains("UNKNOWN"));
        assert!(content.contains("Linear API unavailable"));
        assert!(content.contains("ix-board"));
        assert!(!content.contains("session"));
        assert!(!content.contains("registrations"));
    }
}
