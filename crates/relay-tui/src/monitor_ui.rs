// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Compact, scrollable session monitor. Ticket stage and activity stay separate.

use crate::{
    monitor::{App, Source, View, display_text},
    monitor_tracker::{DashboardView, TicketSummary},
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
    if matches!(app.view, View::Status | View::Health | View::Retro) {
        crate::monitor_department::render(frame, app, now);
        return;
    }
    let now_epoch = SystemClock.now_epoch_seconds();
    let unbound = app.binding.is_none();
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
    app.scope.refresh(
        &app.tracker,
        &app.report,
        team.as_deref(),
        app.show_completed,
    );
    let view = app.scope.view();
    let projection = app.scope.projection();
    let scope_error = app.scope.error().map(str::to_owned);
    let snapshot_old = app.tracker.error.is_some()
        || app.report.error.is_some()
        || app.session_ended
        || app
            .tracker
            .value
            .as_ref()
            .is_some_and(|s| snapshot_is_stale(s.fetched_at, now_epoch));
    let scope_view = app.view == View::Scope;
    let summary = if let Some(v) = &view {
        let mut scope_spans = vec![muted("Scope: "), Span::raw(team_label), muted(" · ")];
        if v.projects.len() == 1 && v.ticket_roots.is_empty() && v.initiatives.is_empty() {
            scope_spans.push(Span::raw(display_text(
                &v.projects[0].name,
                frame.area().width.saturating_sub(20).into(),
            )));
        } else {
            scope_spans.push(Span::raw(v.projects.len().to_string()));
            scope_spans.push(muted(" projects"));
            for (count, label) in [
                (v.initiatives.len(), " initiatives"),
                (v.ticket_roots.len(), " ticket roots"),
            ] {
                if count > 0 {
                    scope_spans.push(muted(" · "));
                    scope_spans.push(Span::raw(count.to_string()));
                    scope_spans.push(muted(label));
                }
            }
        }
        if unbound {
            scope_spans.push(muted(" · "));
            scope_spans.push(Span::raw(UNBOUND_SCOPE));
        } else if let Some(p) = &projection
            && !p.mode.text().is_empty()
        {
            scope_spans.push(muted(" · "));
            scope_spans.push(Span::raw(p.mode.text()));
        }
        if snapshot_old {
            scope_spans.push(Span::styled(
                " [last snapshot]",
                Style::default().fg(WARNING),
            ));
        }
        let mut task_spans = vec![
            muted("Tasks: "),
            Span::raw(format!("{}/{}", v.scope.done, v.scope.leaf_ids.len())),
            muted(" done"),
        ];
        // The hidden, omitted, unknown and filter statements precede the variable-length
        // stage list on the existing Tasks line, so the list can never push them out of view.
        if scope_view && let Some(text) = projection.as_ref().and_then(|p| p.filter_text()) {
            task_spans.push(muted(" · "));
            task_spans.push(Span::raw(text));
            if snapshot_old {
                task_spans.push(Span::styled(
                    " [last snapshot]",
                    Style::default().fg(WARNING),
                ));
            }
        }
        for (stage, count) in &v.scope.remaining_by_stage {
            task_spans.push(muted(" · "));
            task_spans.push(Span::raw(count.to_string()));
            task_spans.push(muted(format!(" {}", display_text(stage, 32))));
        }
        vec![Line::from(scope_spans), Line::from(task_spans)]
    } else if let Some(error) = &scope_error {
        vec![Line::from(Span::styled(
            error.clone(),
            Style::default().fg(ERROR),
        ))]
    } else {
        let mut spans = vec![
            muted("Scope: "),
            Span::raw(team_label),
            muted(" · waiting for Linear"),
        ];
        if unbound {
            spans.push(muted(" · "));
            spans.push(Span::raw(UNBOUND_SCOPE));
        }
        vec![Line::from(spans)]
    };
    // Agents lists every reported assignment; it never presents a scope or a scope count.
    let summary = if app.view == View::Agents {
        vec![Line::from(vec![muted("Agents: "), Span::raw(AGENTS_LABEL)])]
    } else {
        summary
    };
    let width = usize::from(frame.area().width.max(1));
    let evidence = evidence_lines(
        &evidence_items(app, now, now_epoch, false, scope_error.as_deref()),
        width,
        ERROR,
        true,
    );
    let evidence_rows = u16::try_from(evidence.len()).unwrap_or(0);
    let controls = footer_controls(
        scope_view
            .then(|| crate::monitor_scope::footer_mode(projection.as_deref(), app.show_completed)),
        width,
    );
    let footer_rows = u16::try_from(controls.len()).unwrap_or(1) + u16::from(app.show_status);
    let focus_rows: u16 = if report.and_then(|r| r.focus.as_ref()).is_some() {
        2
    } else {
        1
    };
    // Wrapped label text gets the rows it needs, bounded only by what the other panels leave.
    let needed: usize = summary.iter().map(|l| wrapped_rows(l, width)).sum();
    let spare = frame
        .area()
        .height
        .saturating_sub(2 + focus_rows + evidence_rows + footer_rows + 3);
    let scope_height = if app.view == View::Agents {
        u16::try_from(needed)
            .unwrap_or(u16::MAX)
            .clamp(1, spare.max(1))
    } else {
        u16::try_from(needed)
            .unwrap_or(u16::MAX)
            .clamp(2, spare.max(2))
    };
    let [
        identity,
        selector,
        scope,
        focus,
        work,
        evidence_area,
        footer,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(scope_height),
        Constraint::Length(focus_rows),
        Constraint::Min(3),
        Constraint::Length(evidence_rows),
        Constraint::Length(footer_rows),
    ])
    .areas(frame.area());
    let runtime_fresh = runtime_is_fresh(app, now);
    let status = current_activity(app, now);
    frame.render_widget(
        Paragraph::new(identity_line(app, now)).style(Style::default().fg(FOREGROUND)),
        identity,
    );
    frame.render_widget(Paragraph::new(selector_line(app.view)), selector);
    frame.render_widget(
        Paragraph::new(evidence).style(Style::default().fg(MUTED)),
        evidence_area,
    );
    frame.render_widget(
        Paragraph::new(summary)
            .style(Style::default().fg(FOREGROUND))
            .wrap(Wrap { trim: true }),
        scope,
    );
    let focus_lines = report.and_then(|r| r.focus.as_ref()).map_or_else(
        || vec![Line::from(vec![muted("Focus: "), muted("not reported")])],
        |f| {
            let mut lines = vec![Line::from(vec![
                muted("Focus: "),
                Span::raw(display_text(&f.summary, 512)),
            ])];
            if !f.tickets.is_empty() {
                lines.push(Line::from(vec![
                    muted("Tickets: "),
                    Span::raw(
                        f.tickets
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]));
            }
            lines
        },
    );
    frame.render_widget(
        Paragraph::new(focus_lines)
            .style(Style::default().fg(FOREGROUND))
            .wrap(Wrap { trim: true }),
        focus,
    );
    match app.view {
        View::Agents => crate::monitor_views::render_agents(frame, app, work, runtime_fresh, now),
        View::Scope => {
            let unavailable = app
                .tracker
                .error
                .clone()
                .or(scope_error.clone())
                .unwrap_or_else(|| "waiting for Linear".into());
            crate::monitor_views::render_scope(
                frame,
                app,
                view.as_deref().zip(projection.as_deref()),
                &unavailable,
                work,
            );
        }
        View::Tickets => {
            if let Some(view) = &view {
                render_work(frame, app, view, work, runtime_fresh);
            } else {
                app.row_count = 0;
                frame.render_widget(Paragraph::new("Tickets unavailable until Linear loads. Agents are available with a.\nNo assignment or focus is inferred from open tickets.").style(Style::default().fg(MUTED)).wrap(Wrap { trim: true }), work);
            }
        }
        View::Status | View::Health | View::Retro => {}
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
        "Activity: {status} · {linear} · Herdr: {} · Reports: {}",
        source_age(&app.runtime, now),
        source_age(&app.report, now)
    );
    let mut footer_lines = Vec::with_capacity(4);
    if app.show_status {
        footer_lines.push(Line::from(display_text(
            &footer_line,
            usize::from(footer.width),
        )));
    }
    footer_lines.extend(controls);
    frame.render_widget(
        Paragraph::new(footer_lines).style(Style::default().fg(MUTED)),
        footer,
    );
    crate::monitor_nav::render_popup(frame, app, work);
}

/// Fixed Scope-line statement while no lead is bound; it never implies a reported focus.
const UNBOUND_SCOPE: &str = "no lead bound · rows are the whole team";
/// Population statement for Agents, which is not limited by scope (FR-048-AC-6).
const AGENTS_LABEL: &str = "every reported assignment is listed, not limited by scope";
/// The most rows the evidence area may use, counting a "not shown" line.
const MAX_EVIDENCE_ROWS: usize = 4;

/// One source state shown outside any popup. The label is never truncated.
pub(crate) struct Evidence {
    severity: EvidenceSeverity,
    label: String,
    detail: String,
}

/// How urgent an evidence row is; the declaration order is the display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EvidenceSeverity {
    /// A failed source or a replaced binding.
    Failure,
    /// A rate-limit countdown.
    RateLimit,
    /// A stale source or retained last snapshot.
    Stale,
    /// A plain source fact.
    Fact,
}

fn item(
    severity: EvidenceSeverity,
    label: impl Into<String>,
    detail: impl Into<String>,
) -> Evidence {
    Evidence {
        severity,
        label: label.into(),
        detail: detail.into(),
    }
}

/// Source evidence, highest severity first: failures and a replaced binding, then a
/// rate-limit countdown, stale evidence and finally plain source facts.
///
/// Department views use the compact classified summaries; lead views keep the full text.
pub(crate) fn evidence_items(
    app: &App,
    now: Instant,
    now_epoch: i64,
    department_view: bool,
    scope_error: Option<&str>,
) -> Vec<Evidence> {
    let text = |summary: &Option<String>, error: &str| {
        display_text(
            if department_view {
                summary.as_deref().unwrap_or(error)
            } else {
                error
            },
            4096,
        )
    };
    let mut items = Vec::new();
    if app.session_ended {
        items.push(item(
            EvidenceSeverity::Failure,
            "Binding: session replaced",
            "restart to rebind",
        ));
    }
    if department_view {
        if let Some(error) = &app.department.error {
            items.push(item(
                EvidenceSeverity::Failure,
                "Department failed",
                text(&app.department.error_summary, error),
            ));
        }
    } else if let Some(error) = scope_error {
        items.push(item(
            EvidenceSeverity::Failure,
            "Scope failed",
            display_text(error, 4096),
        ));
    }
    if let Some(error) = &app.tracker.error {
        if let Some(deadline) = app.tracker_retry_at {
            let seconds = deadline.saturating_duration_since(now).as_secs();
            items.push(item(
                EvidenceSeverity::RateLimit,
                format!(
                    "Tracker rate-limited (next attempt in {}m {:02}s)",
                    seconds / 60,
                    seconds % 60
                ),
                display_text(error, 4096),
            ));
        } else {
            items.push(item(
                EvidenceSeverity::Failure,
                "Tracker failed",
                text(&app.tracker.error_summary, error),
            ));
        }
    } else if let Some(snapshot) = &app.tracker.value
        && snapshot_is_stale(snapshot.fetched_at, now_epoch)
    {
        items.push(item(
            EvidenceSeverity::Stale,
            "Tracker stale",
            format!(
                "snapshot {} old",
                duration(now_epoch.saturating_sub(snapshot.fetched_at).max(0))
            ),
        ));
    }
    if app.tracker_freshness_unconfirmed && app.tracker.error.is_none() {
        items.push(item(
            EvidenceSeverity::Stale,
            "Tracker freshness unconfirmed",
            crate::monitor_ops::FRESHNESS_UNCONFIRMED_DETAIL,
        ));
    }
    if let Some(error) = app
        .runtime
        .error
        .as_ref()
        .filter(|error| error.as_str() != crate::monitor::SESSION_REPLACED)
    {
        items.push(item(
            EvidenceSeverity::Failure,
            if department_view {
                "Binding / Herdr failed"
            } else {
                "Herdr failed"
            },
            text(&app.runtime.error_summary, error),
        ));
    }
    if let Some(error) = app
        .report
        .error
        .as_ref()
        .filter(|error| error.as_str() != crate::monitor::SESSION_REPLACED)
    {
        items.push(item(
            EvidenceSeverity::Failure,
            "Reports failed",
            text(&app.report.error_summary, error),
        ));
    }
    if department_view {
        if let Some(delta) = &app.department.value {
            let stale = app.department.error.is_some();
            items.push(item(
                if stale {
                    EvidenceSeverity::Stale
                } else {
                    EvidenceSeverity::Fact
                },
                if stale {
                    "Department evidence · last snapshot"
                } else {
                    "Department evidence"
                },
                crate::monitor_department::evidence_windows(delta),
            ));
        }
        if let Some(snapshot) = &app.tracker.value {
            items.push(item(
                EvidenceSeverity::Fact,
                "Tracker evidence",
                format!("epoch {}", snapshot.fetched_at),
            ));
        }
    }
    items.sort_by_key(|evidence| evidence.severity);
    items
}

fn truncate_to(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for character in text.chars() {
        let w = Span::raw(character.to_string()).width();
        if used + w > width {
            break;
        }
        used += w;
        out.push(character);
    }
    out
}

/// Evidence rows: each label intact, free-form detail truncated with a marker, and a count
/// of the sources not shown when more have evidence than fit.
///
/// `failure` colours failures; with `colored` false every row uses the terminal's default
/// colour, as the department views do when color is disabled.
pub(crate) fn evidence_lines(
    items: &[Evidence],
    width: usize,
    failure: Color,
    colored: bool,
) -> Vec<Line<'static>> {
    let shown = if items.len() > MAX_EVIDENCE_ROWS {
        MAX_EVIDENCE_ROWS - 1
    } else {
        items.len()
    };
    let mut lines: Vec<Line<'static>> = items[..shown]
        .iter()
        .map(|evidence| {
            let color = if !colored {
                Color::Reset
            } else {
                match evidence.severity {
                    EvidenceSeverity::Failure => failure,
                    EvidenceSeverity::RateLimit | EvidenceSeverity::Stale => WARNING,
                    EvidenceSeverity::Fact => MUTED,
                }
            };
            let label = Span::styled(
                evidence.label.clone(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            );
            if evidence.detail.is_empty() {
                return Line::from(label);
            }
            let lead = format!(": {}", evidence.detail);
            let room = width.saturating_sub(label.width());
            let detail = if Span::raw(lead.clone()).width() <= room {
                lead
            } else if room > 3 {
                format!("{}…", truncate_to(&lead, room - 1))
            } else {
                String::new()
            };
            Line::from(vec![
                label,
                Span::styled(detail, Style::default().fg(color)),
            ])
        })
        .collect();
    if items.len() > shown {
        lines.push(Line::from(Span::styled(
            format!("+{} more sources not shown", items.len() - shown),
            Style::default().fg(if colored { MUTED } else { Color::Reset }),
        )));
    }
    lines
}

fn runtime_is_fresh(app: &App, now: Instant) -> bool {
    fresh(
        &app.runtime,
        now,
        crate::monitor_runtime::RUNTIME_REFRESH_INTERVAL.saturating_mul(2),
    )
}

/// Current activity wording for the identity row and the optional status line.
fn current_activity(app: &App, now: Instant) -> &'static str {
    if app.session_ended {
        crate::monitor::SESSION_REPLACED
    } else if !runtime_is_fresh(app, now) {
        "activity unknown"
    } else {
        app.runtime
            .value
            .as_ref()
            .map_or("activity unknown", |r| activity(&r.status))
    }
}

/// The monitored lead, identical in every view. Without a bound lead it says so plainly
/// and keeps any binding diagnostic.
pub(crate) fn identity_line(app: &App, now: Instant) -> Line<'static> {
    let name = Span::styled(
        display_text(&app.lead_name, 40),
        Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
    );
    let Some(binding) = &app.binding else {
        let mut spans = vec![name, muted(" · "), Span::raw("no lead bound")];
        if let Some(error) = &app.runtime.error {
            spans.push(muted(" · "));
            spans.push(Span::styled(
                display_text(error, 120),
                Style::default().fg(ERROR),
            ));
        }
        return Line::from(spans);
    };
    Line::from(vec![
        name,
        muted(" · "),
        Span::raw(display_text(&binding.kind, 40)),
        muted(" · "),
        Span::raw(activity_marker(current_activity(app, now))),
    ])
}

/// The one View selector shown in every view: its label, the current view and its key.
pub(crate) fn selector_line(view: View) -> Line<'static> {
    Line::from(vec![
        muted("View: "),
        Span::styled(
            view.label(),
            Style::default().fg(FOREGROUND).add_modifier(Modifier::BOLD),
        ),
        muted(" ▾  "),
        Span::styled("v", Style::default().fg(PRIMARY)),
        muted(" opens the list"),
    ])
}

/// The single controls row: `v View` and `F1 Help` always, `q Quit` when width allows, and
/// in Scope the `c` key with its mode, which is never dropped: it joins the row when it
/// fits and otherwise takes a line of its own.
pub(crate) fn footer_controls(
    scope_mode: Option<&'static str>,
    width: usize,
) -> Vec<Line<'static>> {
    let hint = |key: &'static str, label: &'static str| {
        vec![
            Span::styled(key, Style::default().fg(PRIMARY)),
            muted(format!(" {label}")),
        ]
    };
    let separator = || muted(" · ");
    let mut core = hint("v", "View");
    core.push(separator());
    core.extend(hint("F1", "Help"));
    let mut with_quit = core.clone();
    with_quit.push(separator());
    with_quit.extend(hint("q", "Quit"));
    let toggle = scope_mode.map(|mode| {
        vec![
            Span::styled("c", Style::default().fg(PRIMARY)),
            muted(format!(" {mode}")),
        ]
    });
    let fits = |spans: &[Span<'static>], extra: Option<&[Span<'static>]>| {
        let mut width_used = Line::from(spans.to_vec()).width();
        if let Some(extra) = extra {
            width_used += 3 + Line::from(extra.to_vec()).width();
        }
        width_used <= width
    };
    let join = |spans: &[Span<'static>], toggle: &[Span<'static>]| {
        let mut joined = spans.to_vec();
        joined.push(separator());
        joined.extend(toggle.to_vec());
        Line::from(joined)
    };
    match toggle {
        None => vec![Line::from(if fits(with_quit.as_slice(), None) {
            with_quit
        } else {
            core
        })],
        Some(toggle) => {
            if fits(with_quit.as_slice(), Some(toggle.as_slice())) {
                vec![join(with_quit.as_slice(), toggle.as_slice())]
            } else if fits(core.as_slice(), Some(toggle.as_slice())) {
                vec![join(core.as_slice(), toggle.as_slice())]
            } else {
                vec![Line::from(core), Line::from(toggle)]
            }
        }
    }
}

/// Rows a line occupies when word-wrapped to `width`, counting over-long words split.
pub(crate) fn wrapped_rows(line: &Line, width: usize) -> usize {
    let width = width.max(1);
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let mut rows = 1;
    let mut used = 0;
    for word in text.split_whitespace() {
        let w = Span::raw(word).width();
        if used == 0 {
            used = w;
        } else if used + 1 + w <= width {
            used += 1 + w;
            continue;
        } else {
            rows += 1;
            used = w;
        }
        if used > width {
            let extra = used.div_ceil(width) - 1;
            rows += extra;
            used -= extra * width;
        }
    }
    rows
}

fn snapshot_is_stale(fetched_at: i64, now_epoch: i64) -> bool {
    let age = now_epoch.saturating_sub(fetched_at);
    u64::try_from(age).unwrap_or(0) >= crate::monitor_ops::TRACKER_REFRESH_INTERVAL.as_secs()
}

pub(crate) fn fresh<T>(source: &Source<T>, now: Instant, max_age: std::time::Duration) -> bool {
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

/// The muted style used for descriptions and separators, shared by every view and popup.
pub(crate) fn muted<'a>(text: impl Into<std::borrow::Cow<'a, str>>) -> Span<'a> {
    Span::styled(text, Style::default().fg(MUTED))
}

fn activity_marker(status: &str) -> &str {
    match status {
        "ready for input" => "○",
        "waiting for input" => "◇",
        _ => status,
    }
}

fn activity(status: &str) -> &'static str {
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
/// Existing populated tracker and binding fixtures for terminal regressions.
pub(crate) mod tests {
    use super::*;
    use crate::monitor_tracker::summarize;
    use ix_board::{
        Issue, IssueId, Project, ProjectId, TeamKey, WorkflowStateType, WorkspaceSnapshot,
    };
    use ix_trace_rs::trace;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };
    use relay_core::work::{WorkBinding, WorkReport, WorkScope};

    /// Bound SWM lead with a populated structured tracker snapshot.
    pub(crate) fn app() -> App {
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
            updated_at: None,
            content: None,
            status: "Started".into(),
            status_name: None,
            // A record without category evidence, as cached before categories were kept.
            status_category: ix_board::ProjectStatusCategory::Unknown,
            priority: 2,
            sort_order: 0.0,
            target_date: None,
            team_keys: vec![TeamKey("SWM".into())],
            milestones: vec![],
            initiatives: vec![],
        };
        let issue = |n, category, parent: Option<&str>| {
            let state = if category == WorkflowStateType::Completed {
                "Done"
            } else {
                "Review"
            };
            Issue {
                id: IssueId(format!("id-{n}")),
                identifier: format!("SWM-{n}"),
                title: "Shared Rust settings and OS credential store".into(),
                description: None,
                state: state.into(),
                state_name: state.into(),
                state_type: category,
                parent_id: parent.map(|id| IssueId(id.into())),
                team_key: TeamKey("SWM".into()),
                assignee_id: None,
                assignee_name: None,
                project_id: Some(ProjectId("project".into())),
                project_milestone_id: None,
                priority: 2.0,
                sort_order: 0.0,
                blocked_by: vec![],
                created_at: None,
                updated_at: None,
                attachment_urls: vec![],
                attachments_complete: true,
            }
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
        assert!(hidden.contains("v View") && hidden.contains("F1 Help"));
        assert!(!hidden.contains("s status"), "shortcuts live in help");
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
            ("v", PRIMARY),
            (" View", MUTED),
            (" · ", MUTED),
            ("F1", PRIMARY),
            (" Help", MUTED),
            (" · ", MUTED),
            ("q", PRIMARY),
            (" Quit", MUTED),
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
        // The error is an evidence row above the one-row footer, which keeps its hints.
        for x in 0..7 {
            assert_eq!(terminal.backend().buffer()[(x, 13)].fg, ERROR);
        }
        assert_eq!(terminal.backend().buffer()[(0, 14)].fg, PRIMARY);
    }

    /// Trace: FR-048-AC-4
    #[test]
    fn header_distinguishes_labels_values_tabs_and_input_activity() {
        let mut app = app();
        app.report.value.as_mut().unwrap().as_mut().unwrap().focus =
            Some(relay_core::work::WorkFocus {
                summary: "Review credentials".into(),
                tickets: vec!["SWM-12".parse().unwrap()],
                reported_at_ms: 100,
            });
        app.runtime.success(
            relay_ops::work::WorkObservation {
                binding: app.binding.clone().unwrap(),
                status: "idle".into(),
                panes: vec![],
            },
            Instant::now(),
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 18)).unwrap();
        for (view, tab) in [(View::Tickets, "Tickets"), (View::Scope, "Scope")] {
            app.view = view;
            terminal
                .draw(|frame| render(frame, &mut app, Instant::now()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            for (y, pieces) in [
                (
                    0,
                    vec![
                        ("luna:SWM:code", PRIMARY),
                        (" · ", MUTED),
                        ("codex", FOREGROUND),
                        (" · ", MUTED),
                        ("○", FOREGROUND),
                    ],
                ),
                (
                    2,
                    vec![
                        ("Scope: ", MUTED),
                        ("SWM", FOREGROUND),
                        (" · ", MUTED),
                        ("Product", FOREGROUND),
                    ],
                ),
                (
                    3,
                    if view == View::Scope {
                        // The fixture's model supplies no status category, so the Scope
                        // view states that between the done count and the stage list.
                        vec![
                            ("Tasks: ", MUTED),
                            ("1/2", FOREGROUND),
                            (" done · ", MUTED),
                            (crate::monitor_scope::FILTER_UNAVAILABLE, FOREGROUND),
                            (" · ", MUTED),
                            ("1", FOREGROUND),
                            (" Review", MUTED),
                        ]
                    } else {
                        vec![
                            ("Tasks: ", MUTED),
                            ("1/2", FOREGROUND),
                            (" done · ", MUTED),
                            ("1", FOREGROUND),
                            (" Review", MUTED),
                        ]
                    },
                ),
                (
                    4,
                    vec![("Focus: ", MUTED), ("Review credentials", FOREGROUND)],
                ),
                (5, vec![("Tickets: ", MUTED), ("SWM-12", FOREGROUND)]),
            ] {
                let mut x = 0;
                for (text, color) in pieces {
                    for character in text.chars() {
                        let cell = &buffer[(x, y)];
                        assert_eq!(cell.symbol(), character.to_string(), "header {x},{y}");
                        assert_eq!(cell.fg, color, "header {x},{y}");
                        x += 1;
                    }
                }
            }
            // The one View selector replaces the persistent tab strip.
            let mut x = 0;
            for (text, color) in [
                ("View: ", MUTED),
                (tab, FOREGROUND),
                (" ▾  ", MUTED),
                ("v", PRIMARY),
                (" opens the list", MUTED),
            ] {
                for character in text.chars() {
                    let cell = &buffer[(x, 1)];
                    assert_eq!(cell.symbol(), character.to_string(), "selector {x}");
                    assert_eq!(cell.fg, color, "selector {x}");
                    x += 1;
                }
            }
        }
        for (state, marker, description) in [
            ("idle", "○", "ready for input"),
            ("done", "○", "ready for input"),
            ("blocked", "◇", "waiting for input"),
        ] {
            app.runtime.value.as_mut().unwrap().status = state.into();
            app.show_status = true;
            let visible = screen(&mut app, 120, 18);
            assert!(visible.lines().next().unwrap().contains(marker));
            assert!(visible.contains(&format!("Activity: {description}")));
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
                .map(|x| terminal.backend().buffer()[(x, 13)].symbol())
                .collect::<String>();
            assert!(line.contains("Linear rate limited"), "{line}");
            assert!(line.contains(expected), "{line}");
            assert!(!line.contains("3599"));
        }
        assert!(screen(&mut app, 90, 15).contains("Tasks: 1/2 done"));
        let snapshot = app.tracker.value.as_ref().unwrap().clone();
        app.apply_tracker_refresh(
            Ok(crate::monitor_ops::TrackerRead::unreported(snapshot)),
            now,
        );
        assert!(app.tracker_retry_at.is_none());
        assert!(app.tracker.error.is_none());
        assert!(!snapshot_is_stale(9_400, 10_000));
        assert!(snapshot_is_stale(8_199, 10_000));
    }

    /// A tracker read whose service reports the snapshot stale with the given typed fields;
    /// `failure_reason` carries text that must never reach the screen.
    fn stale_read(
        app: &App,
        age_seconds: Option<i64>,
        rate_limit_until: Option<i64>,
    ) -> crate::monitor_ops::TrackerRead {
        let snapshot = app.tracker.value.as_ref().unwrap().clone();
        crate::monitor_ops::TrackerRead {
            freshness: Some(ix_board::CacheFreshness {
                last_successful_sync: Some(snapshot.fetched_at),
                last_full_reconciliation: None,
                stale: true,
                age_seconds,
                rate_limit_until,
                failure_reason: Some("transport detail".into()),
            }),
            snapshot,
        }
    }

    /// A snapshot the service kept after a failed refresh is two minutes old, far under the
    /// thirty-minute stale label, yet the failure and the last-snapshot label show at once and
    /// the raw failure cause stays out of the display.
    #[trace("FR-059-AC-5", "FR-064-AC-10")]
    #[test]
    fn a_retained_snapshot_shows_the_failed_refresh_while_under_thirty_minutes_old() {
        let mut app = app();
        let fetched_at = app.tracker.value.as_ref().unwrap().fetched_at;
        let read = stale_read(&app, Some(120), None);
        app.apply_tracker_refresh(Ok(read), Instant::now());
        assert_eq!(app.tracker.value.as_ref().unwrap().fetched_at, fetched_at);
        let text = screen(&mut app, 90, 15);
        assert!(text.contains("Tracker failed"), "{text}");
        assert!(text.contains("last snapshot"), "{text}");
        assert!(text.contains("Tasks: 1/2 done"), "{text}");
        assert!(!text.contains("transport detail"), "{text}");
        assert!(!text.contains("Tracker stale"), "{text}");
        assert!(app.collection_failed);
    }

    /// A stale snapshot with no read age and no cooldown may be a successful read of a
    /// snapshot stored without read-start evidence. It is shown as unconfirmed, not as a failed
    /// refresh: no failure row, no remedy, no Partial exit; the source time is kept and it is
    /// not certified fresh. The next read the service reports clean clears the notice.
    #[trace("FR-059-AC-5", "FR-064-AC-10")]
    #[test]
    fn an_unconfirmed_snapshot_is_shown_as_unconfirmed_not_as_a_failed_refresh() {
        let mut app = app();
        let fetched_at = app.tracker.value.as_ref().unwrap().fetched_at;
        let read = stale_read(&app, None, None);
        app.apply_tracker_refresh(Ok(read), Instant::now());
        assert_eq!(app.tracker.value.as_ref().unwrap().fetched_at, fetched_at);
        assert!(app.tracker.error.is_none() && !app.collection_failed);
        assert_eq!(crate::quit_outcome(&app).0, ix_cli_kit::exit::Outcome::Ok);
        let text = screen(&mut app, 90, 15);
        assert!(text.contains("Tracker freshness unconfirmed"), "{text}");
        assert!(text.contains("Tasks: 1/2 done"), "{text}");
        for failure in ["Tracker failed", "refresh failed", "transport detail"] {
            assert!(!text.contains(failure), "{failure}\n{text}");
        }

        let clean = crate::monitor_ops::TrackerRead::unreported(
            app.tracker.value.as_ref().unwrap().clone(),
        );
        app.apply_tracker_refresh(Ok(clean), Instant::now());
        assert!(!app.tracker_freshness_unconfirmed);
        let text = screen(&mut app, 90, 15);
        assert!(!text.contains("Tracker freshness unconfirmed"), "{text}");
    }

    /// An unconfirmed age does not hide a cooldown the service did report.
    #[trace("FR-048-AC-5")]
    #[test]
    fn a_reported_cooldown_is_recognized_without_a_read_age() {
        let mut app = app();
        let read = stale_read(&app, None, Some(4_000));
        app.apply_tracker_refresh(Ok(read), Instant::now());
        assert!(app.tracker_retry_at.is_some());
        assert!(app.collection_failed);
        assert!(!app.tracker_freshness_unconfirmed);
        let text = screen(&mut app, 90, 15);
        assert!(text.contains("Tracker rate-limited"), "{text}");
    }

    /// Trace: FR-048-AC-3, FR-048-AC-4
    #[test]
    fn initiative_progress_counts_only_scoped_leaves_once_across_projects() {
        let mut app = app();
        let snapshot = app.tracker.value.as_mut().unwrap();
        let initiative = ix_board::Initiative {
            id: "initiative".into(),
            identifier: "I-3".into(),
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
            identifier: "I-1".into(),
            name: "Program".into(),
        });
        snapshot.issues[2]
            .blocked_by
            .push(IssueId("missing".into()));
        let mut ready = snapshot.issues[2].clone();
        ready.id = IssueId("ready".into());
        ready.identifier = "SWM-20".into();
        ready.state = "Specced".into();
        ready.state_name = "Specced".into();
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
        assert!(scope.contains("📁"), "{scope}");
        let lines: Vec<_> = scope.lines().collect();
        let focus_row = lines
            .iter()
            .position(|line| line.contains("Focus: not reported"))
            .unwrap();
        let header_row = lines
            .iter()
            .position(|line| line.contains("Initiative/project"))
            .unwrap();
        assert_eq!(header_row, focus_row + 2, "{scope}");
        assert!(lines[focus_row + 1].trim().is_empty());
        assert!(!scope.contains("counts within reported scope"));
        let initiative_row = lines
            .iter()
            .position(|line| line.contains("🎯") && line.contains("Program"))
            .unwrap();
        assert!(
            lines[initiative_row + 1].contains("📁")
                && lines[initiative_row + 1].contains("Product"),
            "{scope}"
        );
        assert!(lines[initiative_row].contains("1/3"));
        assert!(lines[initiative_row + 1].contains("1/3"));
        assert!(
            !lines
                .iter()
                .any(|line| matches!(line.trim(), "Project" | "Initiative" | "done" | "remaining"))
        );
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
        app.scope.invalidate();
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

    /// The drawn cells of a test terminal, one line per row.
    pub(crate) fn frame_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whitespace-normalized text, so a statement wrapped across rows is one statement.
    pub(crate) fn flat(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// One frame of the production renderer at `width` by `height`.
    pub(crate) fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, app, Instant::now()))
            .unwrap();
        frame_text(&terminal)
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
        ticket.state_name = "Specced".into();
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
            identifier: "I-2".into(),
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

    #[trace("FR-065-AC-5")]
    #[test]
    fn every_view_has_the_same_compact_footer_selector_and_no_tab_strip() {
        for (width, height) in [(80_u16, 24_u16), (120, 40)] {
            for view in View::ALL {
                let mut app = app();
                app.view = view;
                let text = screen(&mut app, width, height);
                let footer = text.lines().last().unwrap();
                assert!(
                    footer.contains("v View") && footer.contains("F1 Help"),
                    "{view:?}\n{text}"
                );
                for strip in [
                    "t Tickets",
                    "a Agents",
                    "p Scope",
                    "1 Status",
                    "2 Health",
                    "3 Retro",
                ] {
                    assert!(!text.contains(strip), "{view:?} {strip}\n{text}");
                }
                let selector = text.lines().nth(1).unwrap();
                assert!(
                    selector.starts_with(&format!("View: {}", view.label())),
                    "{text}"
                );
            }
        }
    }

    #[trace("FR-065-AC-5")]
    #[test]
    fn evidence_rows_keep_labels_intact_and_bound_themselves_at_80x24() {
        let mut app = app();
        app.apply_tracker_refresh(
            Err(ix_board::Error::LinearRateLimited {
                retry_after: std::time::Duration::from_secs(3600),
            }),
            Instant::now(),
        );
        app.runtime
            .failure(format!("Herdr unreachable {}", "detail ".repeat(60)));
        app.report.failure("Redis unavailable");
        app.session_ended = true;
        let labels = [
            "Binding: session replaced",
            "Herdr failed",
            "Reports failed",
            "Tracker rate-limited (next attempt in",
        ];
        let text = screen(&mut app, 80, 24);
        for label in labels {
            assert!(text.contains(label), "{label}\n{text}");
        }
        assert!(
            text.contains('…'),
            "detail is truncated with a marker\n{text}"
        );
        let footer = text.lines().last().unwrap();
        assert!(footer.contains("v View") && footer.contains("F1 Help"));
        let rows = text
            .lines()
            .filter(|line| labels.iter().any(|label| line.contains(label)))
            .count();
        assert!(rows <= 4, "{text}");
        // More sources than rows: highest severity first and a count of those not shown.
        app.team = Some("UNKNOWN".into());
        let crowded = screen(&mut app, 80, 24);
        assert!(crowded.contains("Scope failed"), "{crowded}");
        assert!(crowded.contains("+2 more sources not shown"), "{crowded}");
        assert!(crowded.lines().last().unwrap().contains("F1 Help"));
        // Below the supported size the dashboard truncates without panic.
        for (width, height) in [(40_u16, 10_u16), (60, 16)] {
            screen(&mut app, width, height);
        }
    }

    #[trace("FR-065-AC-6")]
    #[test]
    fn identity_row_is_the_same_in_all_six_views() {
        let verified = app();
        let mut replaced = app();
        replaced.session_ended = true;
        replaced.runtime.failure(crate::monitor::SESSION_REPLACED);
        let mut ambiguous = app();
        ambiguous.binding = None;
        ambiguous.report.value = Some(None);
        ambiguous.lead_name = "host:workspace:code".into();
        ambiguous.team = Some("SWM".into());
        ambiguous.runtime.failure("ambiguous requested lead");
        let mut explicit = app();
        explicit.binding = None;
        explicit.report.value = Some(None);
        explicit.lead_name = "identity unknown".into();
        explicit.team = Some("SWM".into());
        let mut saved = App {
            department_input: crate::monitor::DepartmentInput::Saved,
            lead_name: "identity unknown".into(),
            ..App::default()
        };
        saved
            .department
            .success(crate::monitor_department::tests::fixture(), Instant::now());
        // Each case with the exact text its identity row must carry in every view.
        for (name, mut app, expected) in [
            ("verified", verified, "luna:SWM:code · codex"),
            (
                "replaced",
                replaced,
                "luna:SWM:code · codex · session ended/replaced · restart to rebind",
            ),
            (
                "ambiguous",
                ambiguous,
                "host:workspace:code · no lead bound · ambiguous requested lead",
            ),
            ("explicit", explicit, "identity unknown · no lead bound"),
            ("saved", saved, "identity unknown · no lead bound"),
        ] {
            for (width, height) in [(80_u16, 24_u16), (120, 40)] {
                let mut rows = Vec::new();
                for view in View::ALL {
                    app.view = view;
                    rows.push(
                        screen(&mut app, width, height)
                            .lines()
                            .next()
                            .unwrap()
                            .trim_end()
                            .to_owned(),
                    );
                }
                assert!(
                    rows.iter().all(|row| row == &rows[0]),
                    "{name} {width}x{height}: {rows:?}"
                );
                assert!(
                    rows[0].starts_with(expected),
                    "{name} {width}x{height}: {:?}",
                    rows[0]
                );
            }
        }
    }

    #[trace("FR-065-AC-7")]
    #[test]
    fn lead_views_name_their_real_population() {
        let mut agents = app();
        agents.view = View::Agents;
        let text = screen(&mut agents, 200, 30);
        assert!(
            text.contains("Agents: every reported assignment is listed, not limited by scope"),
            "{text}"
        );
        assert!(
            !text.contains("Scope:") && !text.contains("Tasks:"),
            "{text}"
        );
        let mut focused = app();
        let text = screen(&mut focused, 200, 30);
        assert!(text.contains("Scope: SWM · Product"));
        assert!(!text.contains("focus reported") && !text.contains("no lead bound"));
        let mut whole = app();
        whole.report.value.as_mut().unwrap().as_mut().unwrap().scope = None;
        let mut tickets = app();
        tickets
            .report
            .value
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .scope = Some(WorkScope {
            tickets: vec!["id-12".into()],
            ..Default::default()
        });
        let mut unbound = app();
        unbound.binding = None;
        unbound.team = Some("SWM".into());
        unbound.report.value = Some(None);
        for (app, expected) in [
            (&mut whole, "no focus reported · rows are the whole team"),
            (&mut tickets, "no initiative/project focus reported"),
            (&mut unbound, "no lead bound · rows are the whole team"),
        ] {
            for view in [View::Tickets, View::Scope] {
                app.view = view;
                let text = screen(app, 200, 30);
                assert!(text.contains(expected), "{view:?} {expected}\n{text}");
            }
        }
        unbound.view = View::Agents;
        assert!(screen(&mut unbound, 200, 30).contains("no lead bound"));
    }

    #[trace("FR-065-AC-11")]
    #[test]
    fn help_stays_inside_the_work_area_and_a_shrinking_refresh_clamps_selection() {
        let mut app = app();
        app.view = View::Agents;
        let agent = |n: usize| AgentWork {
            agent_id: n.to_string(),
            name: format!("worker-{n:02}"),
            ticket: "SWM-12".parse().unwrap(),
            role: "coder".into(),
            status: WorkStatus::Running,
            pane: None,
            session: None,
            reported_at_ms: 100,
        };
        app.report.value.as_mut().unwrap().as_mut().unwrap().agents = (0..12).map(agent).collect();
        screen(&mut app, 80, 24);
        for _ in 0..11 {
            crate::monitor::handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        screen(&mut app, 80, 24);
        assert_eq!(app.table.selected(), Some(11));
        crate::monitor::handle_key(&mut app, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        app.runtime.failure("Herdr unreachable");
        app.session_ended = true;
        app.apply_tracker_refresh(
            Err(ix_board::Error::LinearRateLimited {
                retry_after: std::time::Duration::from_secs(3600),
            }),
            Instant::now(),
        );
        app.report.value.as_mut().unwrap().as_mut().unwrap().agents = (0..3).map(agent).collect();
        let text = screen(&mut app, 80, 24);
        assert_eq!(app.table.selected(), Some(2), "clamped while help is open");
        let lines: Vec<&str> = text.lines().collect();
        let help = lines.iter().position(|l| l.contains("Help · F1")).unwrap();
        let evidence = lines
            .iter()
            .position(|l| l.contains("Binding: session replaced"))
            .unwrap();
        assert!(help > 3 && help < evidence, "{text}");
        for label in ["Herdr failed", "Tracker rate-limited"] {
            assert!(text.contains(label), "{label}\n{text}");
        }
        assert!(lines.last().unwrap().contains("v View"));
        crate::monitor::handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.popup.is_none() && !app.should_quit);
        assert_eq!(app.table.selected(), Some(2));
    }

    #[trace("FR-065-AC-12")]
    #[test]
    fn key_hints_use_the_primary_blue_and_descriptions_and_separators_are_muted() {
        fn row_of(terminal: &Terminal<TestBackend>, needle: &str) -> u16 {
            let buffer = terminal.backend().buffer();
            (0..buffer.area.height)
                .find(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, *y)].symbol())
                        .collect::<String>()
                        .contains(needle)
                })
                .unwrap_or_else(|| panic!("no row contains {needle}"))
        }
        let draw = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| render(frame, app, Instant::now()))
                .unwrap();
            terminal
        };
        for view in View::ALL {
            let mut app = app();
            app.view = view;
            let terminal = draw(&mut app);
            let buffer = terminal.backend().buffer();
            // Identity row keeps its existing lead-name colour.
            assert_eq!(buffer[(0, 0)].fg, PRIMARY, "{view:?}");
            // Selector: "View: " muted, the key blue, its description muted.
            let key_x = 6 + u16::try_from(view.label().len()).unwrap() + 4;
            assert_eq!(buffer[(0, 1)].fg, MUTED, "{view:?}");
            assert_eq!(buffer[(key_x, 1)].symbol(), "v", "{view:?}");
            assert_eq!(buffer[(key_x, 1)].fg, PRIMARY, "{view:?}");
            assert_eq!(buffer[(key_x + 1, 1)].fg, MUTED, "{view:?}");
            // Footer "v View · F1 Help": keys blue, descriptions and separator muted.
            for (x, symbol, color) in [
                (0, "v", PRIMARY),
                (2, "V", MUTED),
                (7, "·", MUTED),
                (9, "F", PRIMARY),
                (10, "1", PRIMARY),
                (12, "H", MUTED),
            ] {
                assert_eq!(buffer[(x, 23)].symbol(), symbol, "{view:?} footer {x}");
                assert_eq!(buffer[(x, 23)].fg, color, "{view:?} footer {x}");
            }
        }
        let mut app = app();
        app.popup = Some(crate::monitor::Popup::Dropdown {
            highlight: 0,
            offset: 0,
        });
        let terminal = draw(&mut app);
        let y = row_of(&terminal, "(current)");
        let buffer = terminal.backend().buffer();
        assert_eq!((buffer[(3, y)].symbol(), buffer[(3, y)].fg), ("t", PRIMARY));
        assert_eq!((buffer[(6, y)].symbol(), buffer[(6, y)].fg), ("T", MUTED));
        app.popup = Some(crate::monitor::Popup::Help { scroll: 0 });
        let terminal = draw(&mut app);
        let y = row_of(&terminal, "open the View selector");
        let buffer = terminal.backend().buffer();
        assert_eq!((buffer[(3, y)].symbol(), buffer[(3, y)].fg), ("v", PRIMARY));
        assert_eq!((buffer[(6, y)].symbol(), buffer[(6, y)].fg), ("o", MUTED));
    }

    #[trace("FR-065-AC-12")]
    #[test]
    fn rate_limit_and_stale_rows_use_the_existing_warning_palette() {
        fn label_color(app: &mut App, label: &str) -> Color {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| render(frame, app, Instant::now()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let row = (0..24)
                .find(|y| {
                    (0..80)
                        .map(|x| buffer[(x, *y)].symbol())
                        .collect::<String>()
                        .starts_with(label)
                })
                .unwrap_or_else(|| panic!("no evidence row starts with {label}"));
            buffer[(0, row)].fg
        }
        // Lead views: failures keep the error colour; rate-limit and stale rows are amber.
        let mut limited = app();
        limited.apply_tracker_refresh(
            Err(ix_board::Error::LinearRateLimited {
                retry_after: std::time::Duration::from_secs(3600),
            }),
            Instant::now(),
        );
        limited.runtime.failure("Herdr unreachable");
        assert_eq!(label_color(&mut limited, "Tracker rate-limited"), WARNING);
        assert_eq!(label_color(&mut limited, "Herdr failed"), ERROR);
        let mut stale = app();
        stale.tracker.value.as_mut().unwrap().fetched_at -= 2 * 60 * 60;
        assert_eq!(label_color(&mut stale, "Tracker stale"), WARNING);
        // Department views, colour enabled: failure light red, last snapshot amber, fact muted.
        let mut department = app();
        department.view = View::Status;
        department.color_enabled = true;
        department
            .department
            .success(crate::monitor_department::tests::fixture(), Instant::now());
        department.department.failure("source unavailable");
        assert_eq!(
            label_color(&mut department, "Department failed"),
            Color::LightRed
        );
        assert_eq!(
            label_color(&mut department, "Department evidence · last snapshot"),
            WARNING
        );
        assert_eq!(label_color(&mut department, "Tracker evidence"), MUTED);
    }
}
