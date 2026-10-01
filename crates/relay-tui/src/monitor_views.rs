// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Agent assignments and scoped program progress, independent content views (FR-048).

use crate::{
    monitor::{App, display_text},
    monitor_tracker::{DashboardView, ScopeProgress},
    monitor_ui::{
        FOREGROUND, MUTED, PRIMARY, SELECTION, WARNING, age_ms, agent_activity, reported_status,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Block, Paragraph, Row, Table, Wrap},
};
use relay_core::work::WorkStatus;
use std::time::Instant;

/// Keeps selection within the visible view and exposes its row count.
pub(crate) fn select_row(app: &mut App, count: usize) -> usize {
    app.row_count = count;
    let selected = app
        .table
        .selected()
        .unwrap_or(0)
        .min(count.saturating_sub(1));
    app.table.select((count > 0).then_some(selected));
    selected
}

/// Shared short-pane behavior: expanded details replace the content table.
pub(crate) fn content_areas(area: Rect, details: bool) -> [Rect; 2] {
    let compact = details && area.height < 12;
    Layout::vertical([
        Constraint::Min(if compact { 0 } else { 3 }),
        Constraint::Length(if compact {
            area.height
        } else if details {
            area.height.saturating_sub(5).min(9)
        } else {
            0
        }),
    ])
    .areas(area)
}

/// Shared detail panel styling.
pub(crate) fn render_details(frame: &mut Frame, text: String, area: Rect) {
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(FOREGROUND))
            .wrap(Wrap { trim: true })
            .block(Block::default().title(Span::styled("Details", Style::default().fg(PRIMARY)))),
        area,
    );
}

/// Shows every explicit assignment without requiring scope or a Linear snapshot.
pub(crate) fn render_agents(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    runtime_fresh: bool,
    now: Instant,
) {
    let selected = select_row(
        app,
        app.report
            .value
            .as_ref()
            .and_then(Option::as_ref)
            .map_or(0, |r| r.agents.len()),
    );
    let Some(report) = app.report.value.as_ref().and_then(Option::as_ref) else {
        frame.render_widget(
            Paragraph::new("Agent assignments: not reported").style(Style::default().fg(MUTED)),
            area,
        );
        return;
    };
    let active = report
        .agents
        .iter()
        .filter(|a| matches!(a.status, WorkStatus::Running | WorkStatus::Waiting))
        .count();
    let finished = report
        .agents
        .iter()
        .filter(|a| matches!(a.status, WorkStatus::Done | WorkStatus::Canceled))
        .count();
    let failed = report
        .agents
        .iter()
        .filter(|a| a.status == WorkStatus::Failed)
        .count();
    let old = app.session_ended
        || app.report.error.is_some()
        || app.report.last_success.is_none_or(|last| {
            now.saturating_duration_since(last)
                >= crate::monitor_runtime::RUNTIME_REFRESH_INTERVAL.saturating_mul(2)
        });
    let title = if old {
        format!(
            "Agents · {} assignments [last snapshot]",
            report.agents.len()
        )
    } else {
        format!("Agents · {active} active · {finished} finished · {failed} failed (reported)")
    };
    let rows = report.agents.iter().map(|agent| {
        let tracker = app.tracker.value.as_ref().and_then(|snapshot| {
            snapshot
                .issues
                .iter()
                .find(|i| i.identifier == agent.ticket.as_str())
        });
        Row::new([
            format!(
                "{}\n{}",
                display_text(&agent.name, 64),
                display_text(&agent.role, 32)
            ),
            format!(
                "{}\n{}",
                agent.ticket,
                tracker.map_or_else(|| "unknown".into(), |i| display_text(&i.state, 64))
            ),
            format!(
                "{}\n{}",
                reported_status(agent.status),
                if agent.pane.is_some() {
                    "pane agent"
                } else {
                    "internal agent"
                }
            ),
            format!(
                "{}\n{} ago",
                agent_activity(app, agent, runtime_fresh),
                age_ms(agent.reported_at_ms)
            ),
        ])
        .height(2)
    });
    let [table_area, detail_area] = content_areas(area, app.details);
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Min(12),
                Constraint::Length(13),
                Constraint::Length(15),
                Constraint::Length(19),
            ],
        )
        .header(
            Row::new(["Agent / role", "Ticket / stage", "Reported", "Activity"])
                .style(Style::default().fg(PRIMARY)),
        )
        .block(Block::default().title(Span::styled(title, Style::default().fg(PRIMARY))))
        .row_highlight_style(Style::default().bg(SELECTION))
        .highlight_symbol("› "),
        table_area,
        &mut app.table,
    );
    if report.agents.is_empty() && !app.details {
        frame.render_widget(
            Paragraph::new(
                "No agent assignments reported. Native subagents are not automatically discovered.",
            )
            .style(Style::default().fg(MUTED))
            .wrap(Wrap { trim: true }),
            table_area,
        );
    }
    if app.details {
        let text = report.agents.get(selected).map_or_else(|| "No agent selected".into(), |agent| {
            let ticket = app.tracker.value.as_ref().and_then(|snapshot| snapshot.issues.iter().find(|i| i.identifier == agent.ticket.as_str()));
            format!("{} · {}\nTicket: {} · {}\nReported: {} · {} ago\nActivity: {}\nAgent ID: {}\n{}",
                display_text(&agent.name, 128), display_text(&agent.role, 64), agent.ticket,
                ticket.map_or_else(|| "Linear data unavailable".into(), |i| display_text(&i.title, 512)),
                reported_status(agent.status), age_ms(agent.reported_at_ms), agent_activity(app, agent, runtime_fresh), display_text(&agent.agent_id, 128),
                agent.pane.as_ref().map_or_else(|| "Internal agent: activity is lead-reported".into(), |pane| format!("Pane: {pane} · Session: {}", agent.session.as_ref().map_or("unknown", |s| s.as_str())))
            )
        });
        render_details(frame, text, detail_area);
    }
}

/// Gives each initiative and project its own scrollable progress row.
pub(crate) fn render_scope(
    frame: &mut Frame,
    app: &mut App,
    view: Option<&DashboardView>,
    area: Rect,
) {
    let [_, area] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let Some(view) = view else {
        select_row(app, 0);
        frame.render_widget(
            Paragraph::new("Scope progress unavailable until scope and Linear load.")
                .style(Style::default().fg(MUTED))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    };
    let entries: Vec<(&str, &str, &ScopeProgress)> = view
        .initiatives
        .iter()
        .map(|p| ("Initiative", "🎯", p))
        .chain(view.projects.iter().map(|p| ("Project", "📁", p)))
        .collect();
    let selected = select_row(app, entries.len());
    let rows = entries.iter().map(|(_, icon, progress)| {
        Row::new([
            format!("{icon} {}", display_text(&progress.name, 128)),
            format!("{}/{}", progress.done, progress.total),
            progress.total.saturating_sub(progress.done).to_string(),
            progress.ready.to_string(),
            progress.blocked.to_string(),
        ])
    });
    let [table_area, detail_area] = content_areas(area, app.details);
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Min(12),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(6),
                Constraint::Length(7),
            ],
        )
        .header(
            Row::new([
                "Initiative/project",
                "Progress",
                "Remaining",
                "Ready",
                "Blocked",
            ])
            .style(Style::default().fg(PRIMARY)),
        )
        .row_highlight_style(Style::default().bg(SELECTION))
        .highlight_symbol("› "),
        table_area,
        &mut app.table,
    );
    if entries.is_empty() && !app.details {
        frame.render_widget(
            Paragraph::new("Ticket-only scope has no associated initiative or project.")
                .style(Style::default().fg(WARNING)),
            table_area,
        );
    }
    if app.details {
        let text = entries.get(selected).map_or_else(|| "No initiative or project selected".into(), |(kind, _, progress)| {
            let stages = progress.remaining_by_stage.iter().map(|(stage, count)| format!("{count} {}", display_text(stage, 64))).collect::<Vec<_>>().join(" · ");
            format!("{kind}: {}\n{}{}{}\nTasks: {}/{} done · {} remaining\nReady: {} · Blocked: {}\nRemaining stages: {}\nCounts include only scoped leaf tasks; groups may overlap.",
                display_text(&progress.name, 128), if progress.status.is_some() { "Project status: " } else { "" }, progress.status.as_deref().map_or_else(String::new, |s| display_text(s, 64)), if progress.status.is_some() { " (Linear)" } else { "Progress across scoped projects" },
                progress.done, progress.total, progress.total.saturating_sub(progress.done), progress.ready, progress.blocked, if stages.is_empty() { "none" } else { &stages })
                + &if progress.initiatives.is_empty() { String::new() } else { format!("\nInitiatives: {}", progress.initiatives.iter().map(|name| display_text(name, 128)).collect::<Vec<_>>().join(", ")) }
        });
        render_details(frame, text, detail_area);
    }
}
