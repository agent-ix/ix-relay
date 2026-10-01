// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Compact, scrollable session monitor. Ticket stage and activity stay separate.

use crate::{
    monitor::{App, Source, View, display_text},
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

pub(crate) const PRIMARY: Color = Color::Rgb(79, 141, 247);
pub(crate) const FOREGROUND: Color = Color::Rgb(230, 230, 230);
pub(crate) const MUTED: Color = Color::Rgb(139, 146, 156);
pub(crate) const WARNING: Color = Color::Rgb(224, 161, 92);
pub(crate) const SELECTION: Color = Color::Rgb(31, 41, 55);
const ERROR: Color = Color::Rgb(240, 163, 163);

/// Renders one bound session, retaining visibly labeled snapshots on source failure.
pub fn render(frame: &mut Frame, app: &mut App, now: Instant) {
    let now_epoch = SystemClock.now_epoch_seconds();
    let report = app.report.value.as_ref().and_then(Option::as_ref);
    let team = app.team.clone().or_else(|| {
        app.binding.as_ref().and_then(|binding| {
            binding
                .address
                .parse::<relay_core::Address>()
                .ok()
                .map(|address| address.workspace().to_ascii_uppercase())
        })
    });
    let whole_team = report.and_then(|r| r.scope.as_ref()).is_none_or(|scope| {
        scope.initiatives.is_empty() && scope.projects.is_empty() && scope.tickets.is_empty()
    });
    let team_label = format!(
        "{}{}",
        team.as_deref().unwrap_or("unknown team"),
        if whole_team { " team" } else { "" }
    );
    let computed = app
        .tracker
        .value
        .as_ref()
        .zip(team.as_deref())
        .map(|(snapshot, team)| summarize(snapshot, report, team))
        .transpose();
    let (view, scope_error) = match computed {
        Ok(view) => (view, None),
        Err(error) => (None, Some(error.to_string())),
    };
    let [identity, tabs, scope, focus, work, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(if report.and_then(|r| r.focus.as_ref()).is_some() {
            2
        } else {
            1
        }),
        Constraint::Min(3),
        Constraint::Length(if app.show_status { 2 } else { 1 }),
    ])
    .areas(frame.area());
    let runtime_fresh = fresh(
        &app.runtime,
        now,
        crate::monitor_runtime::RUNTIME_REFRESH_INTERVAL.saturating_mul(2),
    );
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
    let mut tab_spans = Vec::new();
    for (key, label, view) in [
        ("t", "Tickets", View::Tickets),
        ("a", "Agents", View::Agents),
        ("p", "Scope", View::Scope),
    ] {
        tab_spans.push(Span::styled(key, Style::default().fg(PRIMARY)));
        tab_spans.push(Span::styled(
            format!(" {label}  "),
            Style::default()
                .fg(if app.view == view { FOREGROUND } else { MUTED })
                .add_modifier(if app.view == view {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(tab_spans)), tabs);
    let snapshot_old = app.tracker.error.is_some()
        || app.report.error.is_some()
        || app.session_ended
        || app
            .tracker
            .value
            .as_ref()
            .is_some_and(|s| snapshot_is_stale(s.fetched_at, now_epoch));
    let summary = view.as_ref().map_or_else(
        || {
            scope_error
                .clone()
                .unwrap_or_else(|| format!("Scope: {team_label} · waiting for Linear"))
        },
        |v| {
            let stage = v
                .scope
                .remaining_by_stage
                .iter()
                .map(|(stage, count)| format!("{count} {}", display_text(stage, 32)))
                .collect::<Vec<_>>()
                .join(" · ");
            let names =
                if v.projects.len() == 1 && v.ticket_roots.is_empty() && v.initiatives.is_empty() {
                    display_text(
                        &v.projects[0].name,
                        frame.area().width.saturating_sub(20).into(),
                    )
                } else {
                    let mut groups = vec![format!("{} projects", v.projects.len())];
                    if !v.initiatives.is_empty() {
                        groups.push(format!("{} initiatives", v.initiatives.len()));
                    }
                    if !v.ticket_roots.is_empty() {
                        groups.push(format!("{} ticket roots", v.ticket_roots.len()));
                    }
                    groups.join(" · ")
                };
            format!(
                "Scope: {} · {}{}\nTasks: {}/{} done{}{}",
                team_label,
                names,
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
                "Focus: {}{}",
                display_text(&f.summary, 512),
                if f.tickets.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\nTickets: {}",
                        f.tickets
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            )
        },
    );
    frame.render_widget(
        Paragraph::new(focus_line)
            .style(Style::default().fg(MUTED))
            .wrap(Wrap { trim: true }),
        focus,
    );
    match app.view {
        View::Agents => crate::monitor_views::render_agents(frame, app, work, runtime_fresh, now),
        View::Scope => crate::monitor_views::render_scope(frame, app, view.as_ref(), work),
        View::Tickets => {
            if let Some(view) = &view {
                render_work(frame, app, view, work, runtime_fresh);
            } else {
                app.row_count = 0;
                frame.render_widget(Paragraph::new("Tickets unavailable until Linear loads. Agents are available with a.\nNo assignment or focus is inferred from open tickets.").style(Style::default().fg(MUTED)).wrap(Wrap { trim: true }), work);
            }
        }
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
                if snapshot_is_stale(s.fetched_at, now_epoch) {
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
    let tracker_error = app.tracker.error.as_ref().map(|error| {
        app.tracker_retry_at.map_or_else(
            || error.clone(),
            |deadline| {
                let seconds = deadline.saturating_duration_since(now).as_secs();
                format!(
                    "{error} · next attempt in {}m {:02}s",
                    seconds / 60,
                    seconds % 60
                )
            },
        )
    });
    let errors = [
        scope_error.as_deref(),
        tracker_error.as_deref(),
        app.runtime.error.as_deref(),
        app.report.error.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let controls = if errors.is_empty() {
        Line::from(vec![
            Span::styled("↑↓", Style::default().fg(PRIMARY)),
            Span::raw(" select · "),
            Span::styled("d", Style::default().fg(PRIMARY)),
            Span::raw(" details · "),
            Span::styled("r", Style::default().fg(PRIMARY)),
            Span::raw(" refresh · "),
            Span::styled("q", Style::default().fg(PRIMARY)),
            Span::raw(" quit · "),
            Span::styled("s", Style::default().fg(PRIMARY)),
            Span::raw(" status"),
        ])
    } else {
        Line::from(Span::styled(
            display_text(&errors, usize::from(footer.width)),
            Style::default().fg(ERROR),
        ))
    };
    let mut footer_lines = Vec::with_capacity(2);
    if app.show_status {
        footer_lines.push(Line::from(display_text(
            &footer_line,
            usize::from(footer.width),
        )));
    }
    footer_lines.push(controls);
    frame.render_widget(
        Paragraph::new(footer_lines).style(Style::default().fg(MUTED)),
        footer,
    );
}

fn snapshot_is_stale(fetched_at: i64, now_epoch: i64) -> bool {
    let age = now_epoch.saturating_sub(fetched_at);
    u64::try_from(age).unwrap_or(0) >= crate::monitor_ops::TRACKER_REFRESH_INTERVAL.as_secs()
}

fn fresh<T>(source: &Source<T>, now: Instant, max_age: std::time::Duration) -> bool {
    source.error.is_none()
        && source
            .last_success
            .is_some_and(|last| now.saturating_duration_since(last) < max_age)
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

pub(crate) fn age_ms(timestamp: i64) -> String {
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

pub(crate) fn reported_status(status: WorkStatus) -> &'static str {
    match status {
        WorkStatus::Running => "running",
        WorkStatus::Waiting => "waiting",
        WorkStatus::Done => "stage done",
        WorkStatus::Failed => "failed",
        WorkStatus::Canceled => "canceled",
    }
}

pub(crate) fn agent_activity(app: &App, agent: &AgentWork, runtime_fresh: bool) -> String {
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
                    "Outside focus · "
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
    let selected = crate::monitor_views::select_row(app, rows.len());
    let [table_area, detail_area] = crate::monitor_views::content_areas(area, app.details);
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
    .row_highlight_style(Style::default().bg(SELECTION))
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
        crate::monitor_views::render_details(frame, text, detail_area);
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
    let initiatives = view
        .initiatives
        .iter()
        .map(|p| format!("{} {}/{}", display_text(&p.name, 64), p.done, p.total))
        .collect::<Vec<_>>()
        .join(" · ");
    format!(
        "{}: {}\nProject: {} · Stage: {} · Priority: {}\nBlockers: {}\n{}{}",
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
        display_text(&projects, 512),
        if initiatives.is_empty() {
            String::new()
        } else {
            format!("\nInitiatives (scoped): {initiatives}")
        }
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
    /// Trace: FR-048-AC-4, FR-048-AC-5
    #[test]
    fn source_status_is_opt_in_and_errors_remain_visible_when_hidden() {
        let mut app = app();
        assert!(!app.show_status);
        let hidden = screen(&mut app, 90, 15);
        assert!(!hidden.contains("Linear: updated"));
        assert!(!hidden.contains("Herdr:"));
        assert!(hidden.contains("s status"));
        crate::monitor::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        let shown = screen(&mut app, 90, 15);
        assert!(shown.contains("Linear: updated"));
        assert!(shown.contains("Herdr:"));
        assert!(shown.contains("Reports:"));
        crate::monitor::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        app.tracker.failure("Linear unavailable");
        let hidden = screen(&mut app, 90, 15);
        assert!(hidden.contains("Linear unavailable"));
        assert!(!hidden.contains("Herdr:"));
    }

    /// Trace: FR-048-AC-4
    #[test]
    fn footer_highlights_only_hotkeys_and_preserves_error_color() {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(69, 15)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, Instant::now()))
            .unwrap();
        let mut x = 0;
        for (text, color) in [
            ("↑↓", PRIMARY),
            (" select · ", MUTED),
            ("d", PRIMARY),
            (" details · ", MUTED),
            ("r", PRIMARY),
            (" refresh · ", MUTED),
            ("q", PRIMARY),
            (" quit · ", MUTED),
            ("s", PRIMARY),
            (" status", MUTED),
        ] {
            for character in text.chars() {
                let cell = &terminal.backend().buffer()[(x, 14)];
                assert_eq!(cell.symbol(), character.to_string());
                assert_eq!(cell.fg, color, "footer column {x}");
                x += 1;
            }
        }
        app.tracker.failure("offline");
        terminal
            .draw(|frame| render(frame, &mut app, Instant::now()))
            .unwrap();
        for x in 0..7 {
            assert_eq!(terminal.backend().buffer()[(x, 14)].fg, ERROR);
        }
    }

    /// Trace: FR-048-AC-5
    #[test]
    fn runtime_observations_remain_fresh_between_minute_polls() {
        let now = Instant::now();
        let mut source = Source::default();
        source.success("observed", now);
        let interval = crate::monitor_runtime::RUNTIME_REFRESH_INTERVAL;
        assert_eq!(interval, std::time::Duration::from_secs(60));
        assert!(fresh(
            &source,
            now + std::time::Duration::from_secs(45),
            interval.saturating_mul(2)
        ));
        assert!(!fresh(
            &source,
            now + std::time::Duration::from_secs(120),
            interval.saturating_mul(2)
        ));
        source.failure("unavailable");
        assert!(!fresh(&source, now, interval.saturating_mul(2)));
    }

    /// Trace: FR-048-AC-5
    #[test]
    fn rate_limit_countdown_advances_and_cached_progress_remains_visible() {
        let mut app = app();
        let now = Instant::now();
        app.apply_tracker_refresh(
            Err(ix_board::Error::LinearRateLimited {
                retry_after: std::time::Duration::from_secs(3600),
            }),
            now,
        );
        assert_eq!(
            app.tracker_retry_at,
            now.checked_add(std::time::Duration::from_secs(1800))
        );
        let mut terminal = Terminal::new(TestBackend::new(90, 15)).unwrap();
        for (elapsed, expected) in [(0, "30m 00s"), (65, "28m 55s"), (1800, "0m 00s")] {
            terminal
                .draw(|frame| {
                    render(
                        frame,
                        &mut app,
                        now + std::time::Duration::from_secs(elapsed),
                    )
                })
                .unwrap();
            let line = (0..90)
                .map(|x| terminal.backend().buffer()[(x, 14)].symbol())
                .collect::<String>();
            assert!(line.contains("Linear rate limited"), "{line}");
            assert!(line.contains(expected), "{line}");
            assert!(!line.contains("3599"));
        }
        assert!(screen(&mut app, 90, 15).contains("Tasks: 1/2 done"));
        let snapshot = app.tracker.value.as_ref().unwrap().clone();
        app.apply_tracker_refresh(Ok(snapshot), now);
        assert!(app.tracker_retry_at.is_none());
        assert!(app.tracker.error.is_none());
        assert!(!snapshot_is_stale(9_400, 10_000));
        assert!(snapshot_is_stale(8_199, 10_000));
    }

    /// Trace: FR-048-AC-3, FR-048-AC-4
    #[test]
    fn initiative_progress_counts_only_scoped_leaves_once_across_projects() {
        let mut app = app();
        let snapshot = app.tracker.value.as_mut().unwrap();
        let initiative = ix_board::Initiative {
            id: "initiative".into(),
            name: "Program".into(),
        };
        snapshot.projects[0].initiatives.push(initiative.clone());
        let mut other = snapshot.projects[0].clone();
        other.id = ProjectId("other".into());
        other.name = "Other".into();
        snapshot.projects.push(other);
        let mut extra = snapshot.issues[2].clone();
        extra.id = IssueId("extra".into());
        extra.identifier = "SWM-99".into();
        extra.project_id = Some(ProjectId("other".into()));
        snapshot.issues.push(extra);
        let report = app.report.value.as_mut().unwrap().as_mut().unwrap();
        report
            .scope
            .as_mut()
            .unwrap()
            .initiatives
            .push("initiative".into());
        let view = summarize(snapshot, Some(report), "SWM").unwrap();
        assert_eq!(view.initiatives.len(), 1);
        assert_eq!(
            (view.initiatives[0].done, view.initiatives[0].total),
            (1, 3)
        );
        app.report
            .value
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .scope
            .as_mut()
            .unwrap()
            .projects
            .push("other".into());
        let view = summarize(
            app.tracker.value.as_ref().unwrap(),
            app.report.value.as_ref().unwrap().as_ref(),
            "SWM",
        )
        .unwrap();
        assert_eq!(
            (view.initiatives[0].done, view.initiatives[0].total),
            (1, 3)
        );
        assert_eq!(view.projects.len(), 2);
        app.details = true;
        assert!(screen(&mut app, 90, 25).contains("Initiatives (scoped): Program 1/3"));
    }

    /// Trace: FR-048-AC-6
    #[test]
    fn agents_are_visible_without_scope_or_linear_and_details_follow_selection() {
        let mut app = app();
        let report = app.report.value.as_mut().unwrap().as_mut().unwrap();
        report.scope = None;
        report.focus = Some(relay_core::work::WorkFocus {
            summary: "Review credentials".into(),
            tickets: vec!["SWM-12".parse().unwrap()],
            reported_at_ms: 100,
        });
        report.agents = (0..12)
            .map(|n| AgentWork {
                agent_id: format!("native-{n}"),
                name: format!("worker-{n:02}"),
                ticket: "SWM-12".parse().unwrap(),
                role: "coder".into(),
                status: if n == 0 {
                    WorkStatus::Done
                } else {
                    WorkStatus::Running
                },
                pane: None,
                session: None,
                reported_at_ms: 100,
            })
            .collect();
        app.tracker.value = None;
        crate::monitor::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        let first = screen(&mut app, 69, 15);
        assert!(first.contains("worker-00"), "{first}");
        assert!(first.contains("SWM-12"));
        assert!(first.contains("stage done"));
        assert!(first.contains("11 active"));
        assert!(first.contains("Focus: Review credentials"));
        assert!(first.contains("Tickets: SWM-12"));
        assert_eq!(app.row_count, 12);
        for _ in 0..11 {
            crate::monitor::handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert!(screen(&mut app, 69, 15).contains("worker-11"));
        app.details = true;
        let detail = screen(&mut app, 69, 15);
        assert!(detail.contains("native-11"), "{detail}");
        assert!(detail.contains("Linear data unavailable"));
        app.details = false;
        app.report.failure("Redis unavailable");
        let stale = screen(&mut app, 69, 15);
        assert!(stale.contains("last snapshot"));
        assert!(stale.contains("Redis unavailable"));
        crate::monitor::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );
        assert_eq!(app.table.selected(), None);
        assert_eq!(app.table.offset(), 0);
    }

    /// Trace: FR-048-AC-7
    #[test]
    fn scope_view_shows_group_counts_and_remaining_stage_details() {
        let mut app = app();
        let snapshot = app.tracker.value.as_mut().unwrap();
        snapshot.projects[0].initiatives.push(ix_board::Initiative {
            id: "program".into(),
            name: "Program".into(),
        });
        snapshot.issues[2]
            .blocked_by
            .push(IssueId("missing".into()));
        let mut ready = snapshot.issues[2].clone();
        ready.id = IssueId("ready".into());
        ready.identifier = "SWM-20".into();
        ready.state = "Specced".into();
        ready.state_type = WorkflowStateType::Unstarted;
        ready.blocked_by.clear();
        snapshot.issues.push(ready);
        let report = app.report.value.as_mut().unwrap().as_mut().unwrap();
        report
            .scope
            .as_mut()
            .unwrap()
            .initiatives
            .push("program".into());
        let view = summarize(snapshot, Some(report), "SWM").unwrap();
        for progress in [&view.projects[0], &view.initiatives[0]] {
            assert_eq!(
                (
                    progress.done,
                    progress.total,
                    progress.ready,
                    progress.blocked
                ),
                (1, 3, 1, 1)
            );
            assert_eq!(progress.remaining_by_stage.get("Review"), Some(&1));
            assert_eq!(progress.remaining_by_stage.get("Specced"), Some(&1));
        }
        crate::monitor::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );
        let scope = screen(&mut app, 69, 15);
        assert!(scope.contains("Program"), "{scope}");
        assert!(scope.contains("Product"));
        assert!(scope.contains("Blocked"));
        assert_eq!(app.row_count, 2);
        crate::monitor::handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.details = true;
        let detail = screen(&mut app, 69, 15);
        assert!(detail.contains("Project: Product"), "{detail}");
        assert!(detail.contains("Project status: Started"));
        assert!(detail.contains("Ready: 1 · Blocked: 1"));
        assert!(detail.contains("1 Review · 1 Specced"));
        app.details = false;
        let snapshot = app.tracker.value.as_mut().unwrap();
        let template = snapshot.projects[0].clone();
        let scope = app
            .report
            .value
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .scope
            .as_mut()
            .unwrap();
        for n in 0..20 {
            let mut project = template.clone();
            project.id = ProjectId(format!("project-{n}"));
            project.name = format!("Project-{n:02}");
            project.initiatives.clear();
            scope.projects.push(project.id.0.clone());
            snapshot.projects.push(project);
        }
        screen(&mut app, 69, 15);
        for _ in 0..21 {
            crate::monitor::handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert!(screen(&mut app, 69, 15).contains("Project-19"));
        crate::monitor::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        );
        assert_eq!(app.view, View::Tickets);
        assert_eq!(app.table.selected(), None);
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
        assert!(compact.contains("Scope: SWM · Product"));
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
        assert!(screen(&mut app, 69, 15).contains("Scope: SWM team · Product"));
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
        let view = summarize(snapshot, Some(report), "SWM").unwrap();
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
    /// Trace: FR-048-AC-9
    #[test]
    fn team_scope_defaults_and_project_focus_exclude_foreign_work() {
        let mut app = app();
        let snapshot = app.tracker.value.as_mut().unwrap();
        snapshot.projects[0].initiatives.push(ix_board::Initiative {
            id: "parent".into(),
            name: "Parent initiative".into(),
        });
        let mut foreign = snapshot.issues[2].clone();
        foreign.id = IssueId("foreign".into());
        foreign.identifier = "AGE-1".into();
        foreign.team_key = TeamKey("AGE".into());
        let mut loose = snapshot.issues[2].clone();
        loose.id = IssueId("loose".into());
        loose.identifier = "SWM-99".into();
        loose.project_id = None;
        snapshot.issues.extend([foreign, loose]);
        let report = app.report.value.as_mut().unwrap().as_mut().unwrap();
        let focused = summarize(snapshot, Some(report), "SWM").unwrap();
        assert_eq!(focused.scope.leaf_ids.len(), 2);
        assert!(focused.initiatives.is_empty());
        assert!(
            !focused
                .tickets
                .iter()
                .any(|t| t.identifier == "AGE-1" || t.identifier == "SWM-99")
        );
        report.scope = Some(WorkScope {
            tickets: vec!["foreign".into()],
            ..Default::default()
        });
        let foreign_focus = summarize(snapshot, Some(report), "SWM").unwrap();
        assert!(foreign_focus.scope.issue_ids.is_empty());
        assert!(foreign_focus.ticket_roots.is_empty());
        report.scope = None;
        let whole = summarize(snapshot, Some(report), "SWM").unwrap();
        assert_eq!(whole.scope.leaf_ids.len(), 3);
        assert!(
            whole
                .tickets
                .iter()
                .any(|t| t.identifier == "SWM-99" && t.in_scope)
        );
        assert!(!whole.tickets.iter().any(|t| t.identifier == "AGE-1"));
        app.report.value = Some(None);
        let rendered = screen(&mut app, 69, 15);
        assert!(rendered.contains("Scope: SWM"), "{rendered}");
        assert!(rendered.contains("Tasks: 1/3"));
        assert!(rendered.contains("Focus: not reported"));
        app.team = Some("AGE".into());
        let overridden = screen(&mut app, 90, 15);
        assert!(overridden.contains("Scope: AGE team"));
        assert!(overridden.contains("Tasks: 0/1"));
        assert!(!overridden.contains("SWM-99"));
        app.team = Some("UNKNOWN".into());
        let failed = screen(&mut app, 90, 15);
        assert!(failed.contains("UNKNOWN"), "{failed}");
        assert!(!failed.contains("Tasks:"));
        app.team = None;
        app.tracker.value = None;
        let waiting = screen(&mut app, 90, 15);
        assert!(waiting.contains("Scope: SWM team · waiting for Linear"));
        assert!(!waiting.contains("Scope: not reported"));
    }
}
