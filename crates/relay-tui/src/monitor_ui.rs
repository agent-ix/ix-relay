// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Compact, scrollable session monitor. Ticket stage and activity stay separate.

use crate::{
    monitor::{App, Source, display_text},
    monitor_tracker::{DashboardView, TicketSummary, summarize},
};
use ix_board::{Clock, SystemClock};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Cell, Paragraph, Row, Table, Wrap},
};
use relay_core::work::{AgentWork, WorkStatus};
use std::time::Instant;

const PRIMARY: Color = Color::Rgb(79, 141, 247);
const FOREGROUND: Color = Color::Rgb(230, 230, 230);
const MUTED: Color = Color::Rgb(139, 146, 156);
const WARNING: Color = Color::Rgb(224, 161, 92);
const ERROR: Color = Color::Rgb(240, 163, 163);

/// Renders one bound session, retaining visibly labeled snapshots on source failure.
pub fn render(frame: &mut Frame, app: &mut App, now: Instant) {
    let [identity, scope, focus, work, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    let runtime_fresh = fresh(&app.runtime, now, 10);
    let status = if app.session_ended {
        "session ended/replaced · restart to rebind"
    } else if !runtime_fresh {
        "activity unknown"
    } else {
        app.runtime
            .value
            .as_ref()
            .map_or("activity unknown", |r| activity(&r.status))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                display_text(&app.lead_name, 40),
                Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                " · {} · {status}",
                app.binding.as_ref().map_or("unknown", |b| b.kind.as_str())
            )),
        ]))
        .style(Style::default().fg(FOREGROUND)),
        identity,
    );
    let report = app.report.value.as_ref().and_then(Option::as_ref);
    let computed = app
        .tracker
        .value
        .as_ref()
        .zip(report)
        .map(|(snapshot, report)| summarize(snapshot, report))
        .transpose();
    let (view, scope_error) = match computed {
        Ok(view) => (view.flatten(), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let snapshot_old = app.tracker.error.is_some()
        || app.report.error.is_some()
        || app.session_ended
        || app.tracker.value.as_ref().is_some_and(|s| {
            SystemClock.now_epoch_seconds().saturating_sub(s.fetched_at)
                >= ix_board::CACHE_TTL_SECONDS
        });
    let summary = view.as_ref().map_or_else(
        || {
            scope_error.clone().unwrap_or_else(|| {
                if report.and_then(|r| r.scope.as_ref()).is_none() {
                    "Scope: not reported".into()
                } else {
                    "Scope: waiting for Linear".into()
                }
            })
        },
        |v| {
            let stage = v
                .scope
                .remaining_by_stage
                .iter()
                .map(|(stage, count)| format!("{count} {}", display_text(stage, 32)))
                .collect::<Vec<_>>()
                .join(" · ");
            format!(
                "Scope: {}{}\nTasks: {}/{} done{}{}",
                v.labels
                    .iter()
                    .map(|label| display_text(label, 128))
                    .collect::<Vec<_>>()
                    .join(", "),
                if snapshot_old { " [last snapshot]" } else { "" },
                v.scope.done,
                v.scope.leaf_ids.len(),
                if stage.is_empty() { "" } else { " · " },
                stage
            )
        },
    );
    frame.render_widget(
        Paragraph::new(summary)
            .style(Style::default().fg(FOREGROUND))
            .wrap(Wrap { trim: true }),
        scope,
    );
    let focus_line = report.and_then(|r| r.focus.as_ref()).map_or_else(
        || "Focus: not reported".into(),
        |f| {
            format!(
                "Focus: {} · reported {} ago",
                display_text(&f.summary, 512),
                age_ms(f.reported_at_ms)
            )
        },
    );
    frame.render_widget(
        Paragraph::new(focus_line)
            .style(Style::default().fg(MUTED))
            .wrap(Wrap { trim: true }),
        focus,
    );
    if let Some(view) = &view {
        render_work(frame, app, view, work, runtime_fresh);
    } else {
        app.row_count = 0;
        frame.render_widget(Paragraph::new("Ticket/agent assignments unavailable until scope and Linear load.\nNo assignment or focus is inferred from open tickets.").style(Style::default().fg(MUTED)).wrap(Wrap { trim: true }), work);
    }
    let linear = app.tracker.value.as_ref().map_or_else(
        || "Linear: no data".into(),
        |s| {
            let seconds = SystemClock
                .now_epoch_seconds()
                .saturating_sub(s.fetched_at)
                .max(0);
            format!(
                "Linear: {}{} ago",
                if seconds >= ix_board::CACHE_TTL_SECONDS {
                    "stale "
                } else {
                    "updated "
                },
                duration(seconds)
            )
        },
    );
    let footer_line = format!(
        "{linear} · Herdr: {} · Reports: {}",
        source_age(&app.runtime, now),
        source_age(&app.report, now)
    );
    let errors = [
        scope_error.as_deref(),
        app.tracker.error.as_deref(),
        app.runtime.error.as_deref(),
        app.report.error.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let controls = if errors.is_empty() {
        "↑↓ select · d details · r refresh · q quit".to_owned()
    } else {
        display_text(&errors, usize::from(footer.width))
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(display_text(&footer_line, usize::from(footer.width))),
            Line::from(Span::styled(
                controls,
                Style::default().fg(if errors.is_empty() { PRIMARY } else { ERROR }),
            )),
        ])
        .style(Style::default().fg(MUTED)),
        footer,
    );
}

fn fresh<T>(source: &Source<T>, now: Instant, max_seconds: u64) -> bool {
    source.error.is_none()
        && source
            .last_success
            .is_some_and(|last| now.saturating_duration_since(last).as_secs() < max_seconds)
}

fn source_age<T>(source: &Source<T>, now: Instant) -> String {
    let age = source.last_success.map_or_else(
        || "no data".into(),
        |last| format!("{}s ago", now.saturating_duration_since(last).as_secs()),
    );
    if source.error.is_some() {
        format!("error ({age})")
    } else {
        age
    }
}

fn duration(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h", seconds / 3600)
    }
}

fn age_ms(timestamp: i64) -> String {
    duration(
        SystemClock
            .now_epoch_seconds()
            .saturating_sub(timestamp / 1000)
            .max(0),
    )
}

fn activity(status: &str) -> &str {
    match status {
        "working" => "working",
        "blocked" => "waiting for input",
        "idle" | "done" => "ready for input",
        _ => "activity unknown",
    }
}

fn reported_status(status: WorkStatus) -> &'static str {
    match status {
        WorkStatus::Running => "running",
        WorkStatus::Waiting => "waiting",
        WorkStatus::Done => "stage done",
        WorkStatus::Failed => "failed",
        WorkStatus::Canceled => "canceled",
    }
}

fn agent_activity(app: &App, agent: &AgentWork, runtime_fresh: bool) -> String {
    if app.session_ended {
        return "last report".into();
    }
    if let Some(pane) = &agent.pane {
        if !runtime_fresh {
            return "activity unknown".into();
        }
        let observed = app.runtime.value.as_ref().and_then(|r| {
            r.panes
                .iter()
                .find(|p| &p.pane == pane && p.session == agent.session)
        });
        return observed.map_or_else(
            || "session unavailable".into(),
            |p| activity(&p.status).into(),
        );
    }
    format!("reported {}", reported_status(agent.status))
}

fn render_work(
    frame: &mut Frame,
    app: &mut App,
    view: &DashboardView,
    area: Rect,
    runtime_fresh: bool,
) {
    let mut rows = Vec::new();
    let mut tickets = Vec::new();
    for ticket in &view.tickets {
        let agents: Vec<Option<&AgentWork>> = if ticket.agents.is_empty() {
            vec![None]
        } else {
            ticket.agents.iter().map(Some).collect()
        };
        for agent in agents {
            let work = format!(
                "{}{}",
                if ticket.in_scope {
                    ""
                } else {
                    "OUT OF SCOPE · "
                },
                display_text(&ticket.title, 160)
            );
            let agent_cell = agent.map_or_else(
                || "unknown\nnot reported".into(),
                |a| {
                    format!(
                        "{}\n{}",
                        display_text(&a.name, 32),
                        display_text(&a.role, 32)
                    )
                },
            );
            let activity_cell = agent.map_or_else(
                || {
                    if ticket.ready {
                        "ready in queue".into()
                    } else {
                        "not reported".into()
                    }
                },
                |a| {
                    format!(
                        "{}\n{} ago",
                        agent_activity(app, a, runtime_fresh),
                        age_ms(a.reported_at_ms)
                    )
                },
            );
            rows.push(
                Row::new([
                    Cell::from(format!(
                        "{}\n{}",
                        display_text(&ticket.identifier, 20),
                        display_text(&ticket.state, 20)
                    )),
                    Cell::from(work),
                    Cell::from(agent_cell),
                    Cell::from(activity_cell),
                ])
                .height(2)
                .style(Style::default().fg(if ticket.in_scope { FOREGROUND } else { WARNING })),
            );
            tickets.push(ticket);
        }
    }
    app.row_count = rows.len();
    let selected = app
        .table
        .selected()
        .unwrap_or(0)
        .min(rows.len().saturating_sub(1));
    app.table.select((!rows.is_empty()).then_some(selected));
    let compact_details = app.details && area.height < 12;
    let [table_area, detail_area] = Layout::vertical([
        Constraint::Min(if compact_details { 0 } else { 3 }),
        Constraint::Length(if compact_details {
            area.height
        } else if app.details {
            area.height.saturating_sub(5).min(9)
        } else {
            0
        }),
    ])
    .areas(area);
    let title = format!(
        "Tickets / agents · {} rows · {} ready",
        rows.len(),
        view.ready_queue.len()
    );
    let title = if let Some(next) = view.ready_queue.first() {
        format!("{title} · next {}", display_text(next, 20))
    } else {
        title
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(12),
            Constraint::Length(16),
            Constraint::Length(19),
        ],
    )
    .header(
        Row::new(["Ticket/stage", "Work", "Agent/role", "Activity"])
            .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().title(Line::from(Span::styled(
        title,
        Style::default().fg(PRIMARY),
    ))))
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("› ");
    frame.render_stateful_widget(table, table_area, &mut app.table);
    if rows_empty(view) {
        frame.render_widget(
            Paragraph::new("No open tickets or agent assignments in this scope.")
                .style(Style::default().fg(MUTED)),
            table_area,
        );
    }
    if app.details {
        let text = tickets.get(selected).map_or_else(
            || "No ticket selected".into(),
            |ticket| ticket_details(view, ticket),
        );
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(FOREGROUND))
                .wrap(Wrap { trim: true })
                .block(
                    Block::default().title(Span::styled("Details", Style::default().fg(PRIMARY))),
                ),
            detail_area,
        );
    }
}

fn rows_empty(view: &DashboardView) -> bool {
    view.tickets.is_empty()
}

fn ticket_details(view: &DashboardView, ticket: &TicketSummary) -> String {
    let projects = view
        .projects
        .iter()
        .map(|p| format!("{} {}/{} done", p.name, p.done, p.total))
        .collect::<Vec<_>>()
        .join(" · ");
    let blockers = if ticket.blockers.is_empty() {
        "none".into()
    } else {
        ticket
            .blockers
            .iter()
            .map(|id| display_text(id, 80))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{}: {}\nProject: {} · Stage: {} · Priority: {}\nBlockers: {}\n{}",
        display_text(&ticket.identifier, 32),
        display_text(&ticket.title, 512),
        display_text(&ticket.project, 128),
        display_text(&ticket.state, 64),
        ticket
            .priority
            .to_string()
            .parse::<ix_board::ProjectPriority>()
            .map_or_else(
                |_| ticket.priority.to_string(),
                |priority| priority.label().to_owned()
            ),
        blockers,
        display_text(&projects, 512)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_board::{
        Issue, IssueId, Project, ProjectId, TeamKey, WorkflowStateType, WorkspaceSnapshot,
    };
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };
    use relay_core::work::{WorkBinding, WorkReport, WorkScope};

    fn app() -> App {
        let now = Instant::now();
        let binding = WorkBinding {
            machine: "luna".parse().unwrap(),
            pane: "w1:p1".parse().unwrap(),
            session: "session-a".parse().unwrap(),
            process_id: 42,
            kind: "codex".into(),
            address: "luna:SWM:code".into(),
        };
        let project = Project {
            id: ProjectId("project".into()),
            identifier: None,
            name: "Product".into(),
            description: String::new(),
            content: None,
            status: "Started".into(),
            priority: 2,
            sort_order: 0.0,
            team_keys: vec![TeamKey("SWM".into())],
            milestones: vec![],
            initiatives: vec![],
        };
        let issue = |n, category, parent: Option<&str>| Issue {
            id: IssueId(format!("id-{n}")),
            identifier: format!("SWM-{n}"),
            title: "Shared Rust settings and OS credential store".into(),
            description: None,
            state: if category == WorkflowStateType::Completed {
                "Done".into()
            } else {
                "Review".into()
            },
            state_type: category,
            parent_id: parent.map(|id| IssueId(id.into())),
            team_key: TeamKey("SWM".into()),
            project_id: Some(ProjectId("project".into())),
            priority: 2.0,
            sort_order: 0.0,
            blocked_by: vec![],
        };
        let mut app = App {
            binding: Some(binding.clone()),
            lead_name: "luna:SWM:code".into(),
            ..Default::default()
        };
        app.tracker.success(
            WorkspaceSnapshot {
                projects: vec![project],
                issues: vec![
                    issue(1, WorkflowStateType::Completed, None),
                    issue(2, WorkflowStateType::Completed, Some("id-1")),
                    issue(12, WorkflowStateType::Started, None),
                ],
                fetched_at: SystemClock.now_epoch_seconds(),
            },
            now,
        );
        app.report.success(
            Some(WorkReport {
                binding,
                scope: Some(WorkScope {
                    projects: vec!["project".into()],
                    ..Default::default()
                }),
                focus: None,
                agents: vec![],
            }),
            now,
        );
        app
    }
    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, app, Instant::now()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Trace: FR-048-AC-3, FR-048-AC-4, FR-048-AC-5
    #[test]
    fn compact_monitor_counts_leaves_and_details_fit_the_actual_short_pane() {
        let mut app = app();
        let compact = screen(&mut app, 69, 15);
        assert!(compact.contains("Tasks: 1/2 done"));
        assert!(compact.contains("SWM-12"));
        assert!(compact.contains("unknown"));
        assert!(!compact.contains("unassigned"));
        assert!(compact.contains("Focus: not reported"));
        app.details = true;
        let details = screen(&mut app, 69, 15);
        assert!(details.contains("OS credential store"));
        assert!(details.contains("Priority: High"));
        assert!(details.contains("Blockers: none"));
        assert!(details.contains("Product 1/2 done"));
        app.tracker.failure("offline");
        let stale = screen(&mut app, 69, 15);
        assert!(stale.contains("last snapshot"));
        assert!(stale.contains("Tasks: 1/2 done"));
        app.report.value = None;
        assert!(screen(&mut app, 69, 15).contains("Scope: not reported"));
    }

    /// Trace: FR-048-AC-3, FR-048-AC-4
    #[test]
    fn next_ready_ticket_keeps_tracker_order_rather_than_identifier_order() {
        let mut app = app();
        let snapshot = app.tracker.value.as_mut().unwrap();
        let ticket = snapshot.issues.last_mut().unwrap();
        ticket.state = "Specced".into();
        ticket.state_type = WorkflowStateType::Unstarted;
        ticket.priority = 1.0;
        let mut other = ticket.clone();
        other.id = IssueId("id-10".into());
        other.identifier = "SWM-10".into();
        other.priority = 2.0;
        snapshot.issues.push(other);
        let report = app.report.value.as_ref().unwrap().as_ref().unwrap();
        let view = summarize(snapshot, report).unwrap().unwrap();
        assert_eq!(view.ready_queue, ["SWM-12", "SWM-10"]);
        let rendered = screen(&mut app, 69, 15);
        assert!(rendered.contains("2 ready · next SWM-12"));
        assert!(rendered.contains("ready in queue"));
        assert!(rendered.contains("unknown"));
        assert!(!rendered.contains("unassigned"));
    }

    /// Trace: FR-048-AC-4
    #[test]
    fn pane_activity_requires_the_reported_native_session() {
        let mut app = app();
        let binding = app.binding.clone().unwrap();
        let now = Instant::now();
        app.runtime.success(
            relay_ops::work::WorkObservation {
                binding,
                status: "working".into(),
                panes: vec![relay_ops::work::PaneActivity {
                    pane: "w1:p2".parse().unwrap(),
                    session: Some("replacement".parse().unwrap()),
                    status: "working".into(),
                }],
            },
            now,
        );
        let agent = AgentWork {
            agent_id: "review".into(),
            name: "reviewer".into(),
            ticket: "SWM-12".parse().unwrap(),
            role: "reviewer".into(),
            status: WorkStatus::Done,
            pane: Some("w1:p2".parse().unwrap()),
            session: Some("original".parse().unwrap()),
            reported_at_ms: 100,
        };
        assert_eq!(agent_activity(&app, &agent, true), "session unavailable");
        app.runtime.value.as_mut().unwrap().panes[0].session = agent.session.clone();
        assert_eq!(agent_activity(&app, &agent, true), "working");
        assert_eq!(agent_activity(&app, &agent, false), "activity unknown");
        app.report
            .value
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .agents
            .push(agent);
        let rendered = screen(&mut app, 77, 15);
        assert!(
            rendered.contains("Review"),
            "agent stage completion does not complete the Linear ticket"
        );
    }

    /// Trace: FR-048-AC-4
    #[test]
    fn multiple_internal_agents_are_reported_and_can_be_scrolled() {
        let mut app = app();
        let report = app.report.value.as_mut().unwrap().as_mut().unwrap();
        report.agents = (0..12)
            .map(|n| AgentWork {
                agent_id: n.to_string(),
                name: format!("worker-{n:02}"),
                ticket: "SWM-12".parse().unwrap(),
                role: "reviewer".into(),
                status: WorkStatus::Running,
                pane: None,
                session: None,
                reported_at_ms: 100,
            })
            .collect();
        let first = screen(&mut app, 77, 15);
        assert!(first.contains("worker-00"));
        assert!(first.contains("reported running"));
        for _ in 0..11 {
            crate::monitor::handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        let last = screen(&mut app, 77, 15);
        assert!(last.contains("worker-11"));
        assert!(!last.contains("worker-00"));
        app.session_ended = true;
        assert!(screen(&mut app, 77, 15).contains("last report"));
    }
}
