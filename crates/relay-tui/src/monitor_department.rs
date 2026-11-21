// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Read-only projection of the shared department assessment (FR-059).

use crate::{
    monitor::{App, DepartmentInput, View, display_text},
    monitor_ui::{
        FOREGROUND, MUTED, SELECTION, WARNING, evidence_items, evidence_lines, footer_controls,
        identity_line, selector_line, wrapped_rows,
    },
    monitor_views::{content_areas, select_row},
};
use ix_board::{
    Clock, SystemClock,
    retro::department::{
        CLASSES, Comparison, ComparisonState, Delta, Measurement, Route, Scope, Severity, Value,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Row, Table, Wrap},
};
use std::time::Instant;

/// Fixed statement relating the department population to the lead's reported focus.
const DEPARTMENT_RELATION: &str =
    "Department scope, chosen independently of the lead's reported focus; see Scope (p).";
/// Relay-local orientation for Status; it defines none of the Status contents.
const STATUS_PURPOSE: &str =
    "Status: progress, blockers and lane evidence for the selected department scope.";

struct DisplayRow {
    name: String,
    state: String,
    value: String,
    detail: String,
    severity: Option<Severity>,
}

fn row(name: &str, state: &str, value: String, detail: String) -> DisplayRow {
    DisplayRow {
        name: display_text(name, 160),
        state: display_text(state, 160),
        value: display_text(&value, 1024),
        detail,
        severity: None,
    }
}

fn safe(raw: &str) -> String {
    display_text(raw, 4096)
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Red => "red",
        Severity::Amber => "amber",
        Severity::Unknown => "unknown",
        Severity::Green => "green",
    }
}

fn severity_color(severity: Option<Severity>, enabled: bool) -> Color {
    if !enabled {
        return Color::Reset;
    }
    match severity {
        Some(Severity::Red) => Color::LightRed,
        Some(Severity::Amber) => WARNING,
        Some(Severity::Green) => Color::Green,
        Some(Severity::Unknown) | None => FOREGROUND,
    }
}

fn value_text(value: Option<&Value>) -> String {
    match value {
        None => "unavailable".into(),
        Some(Value::Scalar(value)) => value.to_string(),
        Some(Value::Keyed(values)) => values
            .iter()
            .map(|(key, value)| format!("{}={value}", safe(key)))
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::Durations(values)) => values
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}: count={} median={} p90={}",
                    safe(key),
                    value.count,
                    value.median,
                    value.p90
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::SourceObservation(values)) => values
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}={} [{}; {}]",
                    safe(key),
                    safe(&value.category),
                    safe(&value.source_schema_version),
                    safe(&value.observation_ref)
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn measurement_detail(metric: &Measurement) -> String {
    let coverage = metric.classification_coverage.as_ref().map_or_else(
        || "unavailable".into(),
        |coverage| {
            format!(
                "{} classified / {} total · {:?}",
                coverage.classified, coverage.total, coverage.basis
            )
        },
    );
    format!(
        "{}: {} {}\nStatus: {:?} · {}\nWindow: ({}, {}] · {}s\nSource age: {} · population: {} · denominator: {}\nClassification coverage: {}\nQuery: {}\nEvidence: {}",
        safe(&metric.id),
        value_text(metric.value.as_ref()),
        safe(&metric.unit),
        metric.status,
        metric
            .reason
            .as_deref()
            .map_or_else(|| "available".into(), safe),
        safe(&metric.window_start),
        safe(&metric.window_end),
        metric.window_seconds,
        metric
            .source_age_seconds
            .map_or_else(|| "unknown".into(), |v| format!("{v}s")),
        metric
            .population
            .map_or_else(|| "unknown".into(), |v| v.to_string()),
        metric
            .denominator
            .map_or_else(|| "unknown".into(), |v| v.to_string()),
        coverage,
        safe(&metric.query_identity),
        metric
            .evidence_refs
            .iter()
            .map(|v| safe(v))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn comparison_text(comparison: &Comparison) -> String {
    let state = match comparison.state {
        ComparisonState::Changed => "changed",
        ComparisonState::Same => "same",
        ComparisonState::QueryChanged => "query_changed",
        ComparisonState::Unavailable => "unavailable",
    };
    format!(
        "{state}: current {}; reference {}; changes [{}]; {}",
        value_text(comparison.current.as_ref()),
        value_text(comparison.reference.as_ref()),
        comparison
            .changes
            .iter()
            .map(|(key, value)| format!("{}={value}", safe(key)))
            .collect::<Vec<_>>()
            .join(", "),
        comparison.reason.as_deref().map_or_else(String::new, safe),
    )
}

fn scope_text(scope: &Scope) -> String {
    [
        ("team", &scope.teams),
        ("project", &scope.projects),
        ("lane", &scope.lanes),
        ("host", &scope.hosts),
        ("repo", &scope.repositories),
    ]
    .into_iter()
    .flat_map(|(kind, selectors)| {
        selectors.iter().map(move |selector| {
            format!("{kind}:{} ({})", safe(&selector.id), safe(&selector.label))
        })
    })
    .collect::<Vec<_>>()
    .join(" · ")
}

fn health_rows(delta: &Delta) -> Vec<DisplayRow> {
    CLASSES
        .iter()
        .map(|id| {
            let Some(class) = delta.current.classes.get(*id) else {
                return row(
                    id,
                    "unknown / incomplete",
                    "unavailable".into(),
                    "Class absent from shared report".into(),
                );
            };
            let route = match &class.route {
                Route::Routed(route) => format!(
                    "Owner: {} · Action: {}",
                    safe(&route.owner),
                    safe(&route.action)
                ),
                Route::Unrouted => "unrouted".into(),
            };
            let mut detail = format!(
                "{}: {} / {}\n{route}\nSource age: {}\n",
                safe(id),
                severity_label(class.summary.severity),
                if class.summary.complete {
                    "complete"
                } else {
                    "incomplete"
                },
                class
                    .source_age_seconds
                    .map_or_else(|| "unknown".into(), |v| format!("{v}s"))
            );
            for (id, reason) in &class.summary.missing {
                detail.push_str(&format!("Missing {}: {}\n", safe(id), safe(reason)));
            }
            for (id, assessment) in &class.metrics {
                detail.push_str(&format!(
                    "{}: {} / {} · {}\n",
                    safe(id),
                    severity_label(assessment.summary.severity),
                    if assessment.summary.complete {
                        "complete"
                    } else {
                        "incomplete"
                    },
                    assessment.reason.as_deref().map_or_else(String::new, safe)
                ));
                if let Some(metric) = delta.current.measurements.get(id) {
                    detail.push_str(&measurement_detail(metric));
                    detail.push('\n');
                }
                if let Some(comparisons) = delta.measurements.get(id) {
                    detail.push_str(&format!(
                        "Previous: {}\nBaseline: {}\n",
                        comparison_text(&comparisons.since_previous),
                        comparison_text(&comparisons.since_baseline)
                    ));
                }
            }
            let mut result = row(
                id,
                &format!(
                    "{} / {}",
                    severity_label(class.summary.severity),
                    if class.summary.complete {
                        "complete"
                    } else {
                        "incomplete"
                    }
                ),
                route,
                detail,
            );
            result.severity = Some(class.summary.severity);
            result
        })
        .collect()
}

fn retro_rows(delta: &Delta) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    for (id, metric) in &delta.current.measurements {
        let detail = delta.measurements.get(id).map_or_else(
            || {
                format!(
                    "{}\nPrevious: unavailable\nBaseline: unavailable",
                    measurement_detail(metric)
                )
            },
            |comparisons| {
                format!(
                    "{}\nPrevious: {}\nBaseline: {}",
                    measurement_detail(metric),
                    comparison_text(&comparisons.since_previous),
                    comparison_text(&comparisons.since_baseline)
                )
            },
        );
        rows.push(row(
            id,
            &format!("{:?}", metric.status).to_lowercase(),
            value_text(metric.value.as_ref()),
            detail,
        ));
    }
    for (id, report) in &delta.current.loops {
        let mut detail = format!(
            "{}: {:?} · {}",
            safe(id),
            report.status,
            report.reason.as_deref().map_or_else(String::new, safe)
        );
        for finding in &report.findings {
            detail.push_str(&format!(
                "\n({}, {}] {}\nParticipants: {}\nEvents: {}",
                safe(&finding.start),
                safe(&finding.end),
                safe(&finding.deduplication_key),
                finding
                    .participants
                    .iter()
                    .map(|v| safe(v))
                    .collect::<Vec<_>>()
                    .join(", "),
                finding
                    .event_ids
                    .iter()
                    .map(|v| safe(v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        rows.push(row(
            id,
            &format!("{:?}", report.status).to_lowercase(),
            report
                .reason
                .as_deref()
                .map_or_else(|| format!("{} findings", report.findings.len()), safe),
            detail,
        ));
    }
    rows
}

fn status_rows(app: &App) -> Vec<DisplayRow> {
    let team = app.department_team();
    let mut rows = vec![row("Plan / progress / blockers", "unavailable", "No structured tracker snapshot".into(), "Saved department reports do not supply a tracker plan or leaf-task progress. Select live scope to read structured tracker data.".into())];
    if let Some(snapshot) = &app.tracker.value {
        match crate::monitor_tracker::summarize_department(snapshot, team.as_deref(), &app.projects)
        {
            Ok(view) => {
                rows.clear();
                rows.push(row(
                    "Scope progress",
                    "structured",
                    format!("{}/{} done", view.scope.done, view.scope.leaf_ids.len()),
                    "Progress uses the authoritative structured scope result.".into(),
                ));
                for project in &view.projects {
                    rows.push(row(&project.name, project.status.as_deref().unwrap_or("unknown"), format!("{}/{} done · {} ready · {} blocked", project.done, project.total, project.ready, project.blocked), format!("Plan: {}\nStages: {:?}\nNext owned action: unavailable without explicit assignment", safe(&project.name), project.remaining_by_stage)));
                }
                for ticket in &view.tickets {
                    rows.push(row(
                        &ticket.identifier,
                        &ticket.state,
                        safe(&ticket.title),
                        format!(
                            "Project: {}\nBlockers: {}\nAssignment: unknown",
                            safe(&ticket.project),
                            ticket
                                .blockers
                                .iter()
                                .map(|v| safe(v))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                }
            }
            Err(error) => rows[0].detail = safe(&error.to_string()),
        }
    }
    let activity = if app.session_ended {
        "session ended/replaced"
    } else if crate::monitor_ui::fresh(
        &app.runtime,
        std::time::Instant::now(),
        crate::monitor_runtime::RUNTIME_REFRESH_INTERVAL.saturating_mul(2),
    ) {
        app.runtime
            .value
            .as_ref()
            .map_or("unknown", |observation| observation.status.as_str())
    } else {
        "unknown"
    };
    rows.push(row("Bound activity", activity, if app.binding.is_some() { "See Agents (a) for verified observations".into() } else { "No verified native identity".into() }, "Restart explicitly to rebind a replaced native session. Department scope remains independent.".into()));
    let focus = app
        .report
        .value
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|report| report.focus.as_ref());
    rows.push(row("Next owned action", if focus.is_some() { "reported" } else { "unavailable" }, focus.map_or_else(|| "No explicit focus reported".into(), |focus| safe(&focus.summary)), "Owned actions require an explicit report; open tickets alone do not establish an assignment.".into()));
    if let Some(delta) = &app.department.value {
        for id in [
            "wip_active",
            "wip_capacity_ratio",
            "host_slots_used",
            "host_slots_free",
            "lane_execution_state",
        ] {
            if let Some(metric) = delta.current.measurements.get(id) {
                rows.push(row(
                    id,
                    &format!("{:?}", metric.status).to_lowercase(),
                    value_text(metric.value.as_ref()),
                    measurement_detail(metric),
                ));
            } else {
                rows.push(row(
                    id,
                    "unavailable",
                    "Field absent from saved/shared report".into(),
                    "No value is inferred for an absent measurement.".into(),
                ));
            }
        }
    } else {
        rows.push(row(
            "Capacity / lane activity",
            "unavailable",
            "No department report".into(),
            "Awaiting shared collection; capacity and liveness are not inferred.".into(),
        ));
    }
    rows
}

fn diagnostic_rows(app: &App) -> Vec<DisplayRow> {
    [
        (
            "Department",
            &app.department.error_summary,
            &app.department.error,
        ),
        ("Tracker", &app.tracker.error_summary, &app.tracker.error),
        (
            "Binding / Herdr",
            &app.runtime.error_summary,
            &app.runtime.error,
        ),
        ("Reports", &app.report.error_summary, &app.report.error),
    ]
    .into_iter()
    .filter_map(|(label, summary, error)| {
        error.as_ref().map(|error| {
            row(
                label,
                "unavailable",
                summary.as_deref().map_or_else(|| safe(error), safe),
                safe(error),
            )
        })
    })
    .collect()
}

/// Draws shared results without collecting or assessing evidence.
pub(crate) fn render(frame: &mut Frame, app: &mut App, now: Instant) {
    let now_epoch = SystemClock.now_epoch_seconds();
    let width = usize::from(frame.area().width.max(1));
    // Department evidence keeps its existing policy: light red failures, and the terminal's
    // default colour when color is disabled.
    let evidence = evidence_lines(
        &evidence_items(app, now, now_epoch, true, None),
        width,
        Color::LightRed,
        app.color_enabled,
    );
    let evidence_rows = u16::try_from(evidence.len()).unwrap_or(0);
    let controls = footer_controls(None, width);
    let footer_rows = u16::try_from(controls.len()).unwrap_or(1);
    let label = label_lines(app);
    let needed: usize = label.iter().map(|line| wrapped_rows(line, width)).sum();
    let spare = frame
        .area()
        .height
        .saturating_sub(2 + evidence_rows + footer_rows + 3);
    let label_rows = u16::try_from(needed)
        .unwrap_or(u16::MAX)
        .clamp(1, spare.max(1));
    let [identity, selector, label_area, body, evidence_area, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(label_rows),
        Constraint::Min(3),
        Constraint::Length(evidence_rows),
        Constraint::Length(footer_rows),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new(identity_line(app, now)).style(Style::default().fg(FOREGROUND)),
        identity,
    );
    frame.render_widget(Paragraph::new(selector_line(app.view)), selector);
    frame.render_widget(
        Paragraph::new(label)
            .style(Style::default().fg(FOREGROUND))
            .wrap(Wrap { trim: true }),
        label_area,
    );
    frame.render_widget(
        Paragraph::new(evidence).style(Style::default().fg(MUTED)),
        evidence_area,
    );
    render_content(frame, app, body);
    frame.render_widget(
        Paragraph::new(controls).style(Style::default().fg(MUTED)),
        footer,
    );
    crate::monitor_nav::render_popup(frame, app, body);
}

fn muted(text: impl Into<std::borrow::Cow<'static, str>>) -> Span<'static> {
    Span::styled(text, Style::default().fg(MUTED))
}

/// Source windows and ages of the shared report, age first so the evidence row can be
/// truncated at its end without hiding the age.
pub(crate) fn evidence_windows(delta: &Delta) -> String {
    delta
        .current
        .measurements
        .values()
        .map(|metric| {
            format!(
                "age:{} ({}, {}]",
                metric
                    .source_age_seconds
                    .map_or_else(|| "unknown".into(), |age| format!("{age}s")),
                safe(&metric.window_start),
                safe(&metric.window_end),
            )
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Names the department selection that decides the evidence population. It reads only the
/// selectors the dashboard was given or the saved report; it computes no population.
fn department_selection(app: &App) -> String {
    if app.department_input == DepartmentInput::Saved {
        let scope = app
            .department
            .value
            .as_ref()
            .map(|delta| scope_text(&delta.current.scope))
            .filter(|text| !text.is_empty());
        return scope.map_or_else(
            || "saved report".to_owned(),
            |text| format!("saved report · {}", safe(&text)),
        );
    }
    if app.team.is_some() || !app.projects.is_empty() {
        let mut parts = Vec::new();
        if let Some(team) = &app.team {
            parts.push(format!("team {}", safe(team)));
        }
        if !app.projects.is_empty() {
            parts.push(format!(
                "projects {}",
                app.projects
                    .iter()
                    .map(|project| safe(project))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        return parts.join(" · ");
    }
    format!(
        "team {} derived from lead workspace · all projects",
        app.department_team()
            .as_deref()
            .map_or_else(|| "unknown".to_owned(), safe)
    )
}

/// Population label, its fixed relation to the lead's focus and, in Status, its purpose.
fn label_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            muted("Department selection: "),
            Span::raw(department_selection(app)),
        ]),
        Line::from(muted(DEPARTMENT_RELATION)),
    ];
    if app.view == View::Status {
        lines.push(Line::from(Span::raw(STATUS_PURPOSE)));
    }
    lines
}

/// The one concise message for Health or Retro when no values can be shown, chosen from
/// their own fields. Not configured and Waiting are not source failures.
fn empty_message(app: &App) -> Option<String> {
    let name = match app.view {
        View::Health => "Health",
        View::Retro => "Retro",
        _ => return None,
    };
    let saved_lacks = || {
        format!(
            "Not configured: this saved report does not supply the {name} fields. \
             Open a report that contains them with --report <file>."
        )
    };
    match &app.department.value {
        Some(delta) => {
            let lacking = if app.view == View::Health {
                delta.current.classes.is_empty()
            } else {
                delta.current.measurements.is_empty() && delta.current.loops.is_empty()
            };
            lacking.then(|| {
                if app.department_input == DepartmentInput::Saved {
                    saved_lacks()
                } else {
                    format!("The retained report supplies no {name} fields; nothing is shown.")
                }
            })
        }
        None => Some(match (&app.department.error, app.department_input) {
            (Some(error), _) => format!(
                "{name} unavailable: {}. No retained report exists, so no values are shown.",
                safe(app.department.error_summary.as_deref().unwrap_or(error))
            ),
            (None, DepartmentInput::Saved) => saved_lacks(),
            (None, DepartmentInput::Configured) => format!(
                "Waiting for the first read of the retro configuration. \
                 No {name} values are shown until it returns."
            ),
            (None, DepartmentInput::NotConfigured) => format!(
                "Not configured: {name} needs a department retro configuration. Start with \
                 --retro-config <file> whose department team and project population matches \
                 the selection shown above ({}).{}",
                department_selection(app),
                if app.view == View::Retro {
                    " Previous and baseline comparisons also need a retained snapshot history."
                } else {
                    ""
                }
            ),
        }),
    }
}

/// Draws the rows, details or empty state of the current department view inside `body`.
fn render_content(frame: &mut Frame, app: &mut App, body: Rect) {
    if !app.show_diagnostics
        && let Some(message) = empty_message(app)
    {
        select_row(app, 0);
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(FOREGROUND))
                .wrap(Wrap { trim: true }),
            body,
        );
        return;
    }
    let rows = if app.show_diagnostics {
        diagnostic_rows(app)
    } else {
        match app.view {
            View::Status => status_rows(app),
            View::Health => app
                .department
                .value
                .as_ref()
                .map_or_else(Vec::new, health_rows),
            View::Retro => app
                .department
                .value
                .as_ref()
                .map_or_else(Vec::new, retro_rows),
            View::Tickets | View::Agents | View::Scope => Vec::new(),
        }
    };
    let selected = select_row(app, rows.len());
    let [table_area, detail_area] = content_areas(body, app.details);
    let table = Table::new(
        rows.iter().map(|row| {
            Row::new([row.name.clone(), row.state.clone(), row.value.clone()])
                .style(Style::default().fg(severity_color(row.severity, app.color_enabled)))
        }),
        [
            Constraint::Percentage(32),
            Constraint::Percentage(27),
            Constraint::Percentage(41),
        ],
    )
    .header(Row::new([
        "Measure / class",
        "State / completeness",
        "Value / owner action",
    ]))
    .row_highlight_style(Style::default().bg(SELECTION))
    .highlight_symbol("› ");
    frame.render_stateful_widget(table, table_area, &mut app.table);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.show_diagnostics {
                "No source failures"
            } else {
                "No rows in the shared department report"
            }),
            table_area,
        );
    }
    if app.details {
        let text = rows
            .get(selected)
            .map_or("No row selected", |row| row.detail.as_str());
        frame.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .scroll((app.detail_scroll, 0))
                .block(Block::default().title("Evidence / action · ←→ scroll")),
            detail_area,
        );
    }
}

#[cfg(test)]
/// Shared deterministic display fixtures and behavioral terminal checks.
pub(crate) mod tests {
    use super::*;
    use crate::monitor::handle_key;
    use ix_board::retro::{
        Direction, Status,
        department::{
            self, ClassAssessment, MetricComparisons, Parameters, Report, Selector, Summary,
        },
    };
    use ix_trace_rs::trace;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };
    use std::{
        collections::{BTreeMap, BTreeSet},
        time::Instant,
    };

    /// A breached but incomplete report, with incompatible and absent comparisons.
    pub(crate) fn fixture() -> Delta {
        let scope = Scope {
            teams: vec![Selector {
                id: "SWM".into(),
                label: "Department".into(),
            }],
            ..Scope::default()
        };
        let metric = Measurement {
            id: "wip_active".into(),
            catalog_version: department::CATALOG_VERSION.into(),
            status: Status::Ok,
            value: Some(Value::Scalar(17.0)),
            reason: None,
            unit: "issues".into(),
            direction: Direction::Down,
            scope: scope.clone(),
            query_identity: "fixture-current".into(),
            window_start: "2026-11-18T00:00:00Z".into(),
            window_end: "2026-11-19T00:00:00Z".into(),
            window_seconds: 86400,
            source_age_seconds: Some(7200.0),
            population: Some(17),
            denominator: None,
            denominators: BTreeMap::new(),
            query_parameters: Parameters::default(),
            evidence_refs: vec!["linear:issue:fixture-17".into()],
            classification_coverage: None,
            operational_parameters: None,
        };
        let classes = CLASSES
            .iter()
            .map(|id| {
                (
                    (*id).into(),
                    ClassAssessment {
                        id: (*id).into(),
                        required: BTreeSet::from(["wip_active".into()]),
                        optional: BTreeSet::new(),
                        metrics: BTreeMap::from([(
                            "wip_active".into(),
                            department::MetricAssessment {
                                policy: None,
                                keys: BTreeMap::from([(
                                    String::new(),
                                    department::KeyAssessment {
                                        value: Some(17.0),
                                        severity: Severity::Red,
                                        reason: None,
                                    },
                                )]),
                                summary: Summary {
                                    severity: Severity::Red,
                                    complete: false,
                                    missing: BTreeMap::from([(
                                        "host_observation".into(),
                                        "partial relation page / unreachable host".into(),
                                    )]),
                                },
                                reason: Some("partial evidence".into()),
                            },
                        )]),
                        summary: Summary {
                            severity: Severity::Red,
                            complete: false,
                            missing: BTreeMap::from([(
                                "host_observation".into(),
                                "partial relation page / unreachable host".into(),
                            )]),
                        },
                        source_age_seconds: Some(7200.0),
                        route: if *id == "flow_wip" {
                            Route::Routed(department::OwnerAction {
                                owner: "planner".into(),
                                action: "Assign a reviewer".into(),
                            })
                        } else {
                            Route::Unrouted
                        },
                    },
                )
            })
            .collect();
        let previous = Comparison {
            state: ComparisonState::QueryChanged,
            current_identity: "fixture-current".into(),
            reference_identity: Some("fixture-previous".into()),
            current: metric.value.clone(),
            reference: Some(Value::Scalar(2.0)),
            changes: BTreeMap::new(),
            reason: Some("query_changed".into()),
        };
        let baseline = Comparison {
            state: ComparisonState::Unavailable,
            current_identity: "fixture-current".into(),
            reference_identity: None,
            current: metric.value.clone(),
            reference: None,
            changes: BTreeMap::new(),
            reason: Some("missing baseline".into()),
        };
        Delta {
            current: Report {
                version: 1,
                catalog_version: department::CATALOG_VERSION.into(),
                policy_id: "fixture-policy".into(),
                config_identity: "fixture-config".into(),
                scope,
                observations: Vec::new(),
                measurements: BTreeMap::from([("wip_active".into(), metric)]),
                loops: BTreeMap::from([(
                    "state_bounce_loop".into(),
                    department::LoopReport {
                        detector_id: "state_bounce_loop".into(),
                        status: Status::Ok,
                        reason: None,
                        findings: vec![department::LoopFinding {
                            event_ids: vec!["state-event-1".into(), "state-event-2".into()],
                            start: "2026-11-18T01:00:00Z".into(),
                            end: "2026-11-18T02:00:00Z".into(),
                            participants: BTreeSet::from(["SWM-165".into()]),
                            deduplication_key: "fixture-loop-165".into(),
                        }],
                    },
                )]),
                classes,
                rollup: Summary {
                    severity: Severity::Red,
                    complete: false,
                    missing: BTreeMap::from([("host_observation".into(), "unreachable".into())]),
                },
            },
            measurements: BTreeMap::from([(
                "wip_active".into(),
                MetricComparisons {
                    since_previous: previous,
                    since_baseline: baseline,
                },
            )]),
        }
    }

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, app, Instant::now()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn key(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn painted_details(app: &mut App, width: u16, height: u16) -> String {
        app.details = true;
        let mut pages = Vec::new();
        for offset in 0..64 {
            app.detail_scroll = offset;
            pages.push(screen(app, width, height));
        }
        pages
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Trace: FR-059-AC-3, FR-059-AC-4
    #[trace("FR-059-AC-3", "FR-059-AC-4")]
    #[test]
    fn shared_fixture_and_serialized_model_project_the_same_values_and_evidence() {
        let model = fixture();
        let json = serde_json::to_vec(&model).unwrap();
        let decoded: Delta = serde_json::from_slice(&json).unwrap();
        for source in [model, decoded] {
            for (width, height) in [(80, 24), (120, 40)] {
                let metric = &source.current.measurements["wip_active"];
                let Value::Scalar(number) = metric.value.as_ref().unwrap() else {
                    panic!("fixture must carry scalar source value");
                };
                let mut app = App {
                    view: View::Status,
                    ..App::default()
                };
                app.department.success(source.clone(), Instant::now());
                let status = screen(&mut app, width, height)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                assert!(
                    status.contains(&format!("{} ok {number}", metric.id)),
                    "{status}"
                );
                key(&mut app, KeyCode::Char('2'));
                let health = screen(&mut app, width, height)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let class = &source.current.classes["flow_wip"];
                assert!(
                    health.contains(&format!("{} red / incomplete", class.id)),
                    "{health}"
                );
                assert!(health.contains("unrouted"));
                let detail = painted_details(&mut app, width, height);
                let Route::Routed(route) = &class.route else {
                    panic!("fixture must carry an owner action");
                };
                assert!(detail.contains(&route.owner));
                assert!(detail.contains(&route.action));
                for reason in class.summary.missing.values() {
                    assert!(detail.contains(reason), "{detail}");
                }
                assert!(detail.contains("wip_active: red / incomplete"));
                assert!(detail.contains(&format!("{number} {}", metric.unit)));
                for reference in &metric.evidence_refs {
                    assert!(detail.contains(reference));
                }
                key(&mut app, KeyCode::Char('3'));
                let detail = painted_details(&mut app, width, height);
                let comparisons = &source.measurements[&metric.id];
                assert!(detail.contains("Previous: query_changed"));
                assert!(detail.contains(comparisons.since_previous.reason.as_deref().unwrap()));
                assert!(detail.contains("Baseline: unavailable"));
                assert!(detail.contains(comparisons.since_baseline.reason.as_deref().unwrap()));
                for reference in &metric.evidence_refs {
                    assert!(detail.contains(reference));
                }
                app.details = false;
                screen(&mut app, width, height);
                key(&mut app, KeyCode::Down);
                let detail = painted_details(&mut app, width, height);
                for finding in &source.current.loops["state_bounce_loop"].findings {
                    assert!(detail.contains(&finding.deduplication_key));
                    assert!(detail.contains(&finding.start));
                    assert!(detail.contains(&finding.end));
                    for event in &finding.event_ids {
                        assert!(detail.contains(event));
                    }
                    for participant in &finding.participants {
                        assert!(detail.contains(participant));
                    }
                }
            }
        }
    }

    /// Trace: FR-059-AC-4
    #[trace("FR-059-AC-4")]
    #[test]
    fn navigation_preserves_scope_reaches_last_class_and_clamps_after_refresh() {
        let mut app = App {
            team: Some("SWM".into()),
            projects: vec!["native-project".into()],
            ..App::default()
        };
        app.department.success(fixture(), Instant::now());
        for (code, view) in [
            ('1', View::Status),
            ('2', View::Health),
            ('3', View::Retro),
            ('2', View::Health),
        ] {
            app.table.select(Some(8));
            app.detail_scroll = 50;
            key(&mut app, KeyCode::Char(code));
            assert_eq!(app.view, view);
            assert_eq!(app.table.selected(), None);
            assert_eq!(app.detail_scroll, 0);
            assert_eq!(app.projects, ["native-project"]);
            assert_eq!(app.team.as_deref(), Some("SWM"));
        }
        screen(&mut app, 80, 24);
        for _ in 0..20 {
            key(&mut app, KeyCode::Down);
        }
        let output = screen(&mut app, 80, 24);
        assert_eq!(app.table.selected(), Some(9));
        assert!(output.contains("comms_overhead"));
        key(&mut app, KeyCode::Char('3'));
        let mut shortened = fixture();
        shortened.current.loops.clear();
        app.department.success(shortened, Instant::now());
        app.table.select(Some(99));
        screen(&mut app, 120, 40);
        assert_eq!(app.table.selected(), Some(0));
    }

    /// Trace: FR-059-AC-5
    #[trace("FR-059-AC-5")]
    #[test]
    fn failed_refresh_retains_evidence_age_and_visible_error_with_status_hidden() {
        let mut app = App {
            view: View::Retro,
            ..App::default()
        };
        let model = fixture();
        app.department.success(model.clone(), Instant::now());
        app.department
            .failure("unreachable host · partial relation page");
        let output = screen(&mut app, 80, 24);
        assert_eq!(app.department.value, Some(model));
        assert!(!app.show_status);
        assert!(output.contains("7200s"));
        assert!(output.contains("last snapshot"));
        assert!(output.contains("unreachable host"));
        assert!(output.contains("partial relation page"));
    }

    /// Trace: FR-059-AC-3, FR-059-AC-4
    #[trace("FR-059-AC-3", "FR-059-AC-4")]
    #[test]
    fn categorical_evidence_is_literal_and_untrusted_text_cannot_inject_controls() {
        let mut model = fixture();
        let metric = model.current.measurements.get_mut("wip_active").unwrap();
        metric.value = Some(Value::SourceObservation(BTreeMap::from([(
            "lane".into(),
            department::SourceObservation {
                category: "login_blocked\u{1b}[2J\nunknown".repeat(100),
                source_schema_version: "v1".into(),
                observation_ref: "pane:evidence".into(),
            },
        )])));
        let detail = measurement_detail(metric);
        assert!(detail.contains("login_blocked"));
        assert!(detail.contains("pane:evidence"));
        assert!(!detail.contains('\u{1b}'));
        let mut app = App {
            view: View::Retro,
            details: true,
            ..App::default()
        };
        app.department.success(model, Instant::now());
        app.department.failure("source unavailable");
        let output = screen(&mut app, 80, 24);
        assert!(output.contains("source unavailable"));
        assert!(output.contains("q Quit"));
        key(&mut app, KeyCode::Right);
        assert_eq!(app.detail_scroll, 1);
    }

    /// Trace: FR-059-AC-3
    #[trace("FR-059-AC-3")]
    #[test]
    fn classified_subset_coverage_is_displayed_without_recalculating_the_rate() {
        let mut model = fixture();
        let metric = model.current.measurements.get_mut("wip_active").unwrap();
        metric.classification_coverage = Some(department::ClassificationCoverage {
            classified: 2,
            total: 8,
            basis: department::CoverageBasis::LabeledSubset,
        });
        let detail = measurement_detail(metric);
        assert!(detail.contains("2 classified / 8 total"));
        assert!(detail.contains("LabeledSubset"));
        assert!(detail.contains("17 issues"));
    }

    #[trace("FR-059-AC-1", "FR-059-AC-4", "FR-059-AC-6")]
    #[test]
    fn simultaneous_source_failures_have_visible_summaries_and_complete_details() {
        let mut app = App {
            view: View::Status,
            ..App::default()
        };
        app.department.failure("Department health/retro unavailable: supply --retro-config with department policies matching the selected scope, or open --report");
        app.apply_tracker_refresh(Err(ix_board::Error::MissingApiKey), Instant::now());
        app.runtime.failure(format!(
            "ambiguous requested lead; {}",
            "native identity must remain unknown. ".repeat(12)
        ));
        for view in ['1', '2', '3'] {
            key(&mut app, KeyCode::Char(view));
            let output = screen(&mut app, 80, 24);
            assert!(output.contains("Department failed:"), "{output}");
            assert!(output.contains("Tracker failed:"), "{output}");
            assert!(output.contains("ix-board auth login"), "{output}");
            assert!(
                output.contains("Binding / Herdr failed: ambiguous requested lead"),
                "{output}"
            );
            assert!(output.contains("F1 Help"), "{output}");
        }
        key(&mut app, KeyCode::Char('e'));
        screen(&mut app, 80, 24);
        let expected = [
            app.department.error.clone().unwrap(),
            app.tracker.error.clone().unwrap(),
            app.runtime.error.clone().unwrap(),
        ];
        for (index, diagnostic) in expected.iter().enumerate() {
            app.table.select(Some(index));
            app.detail_scroll = 0;
            let detail = painted_details(&mut app, 80, 24);
            assert!(
                detail.contains(diagnostic),
                "missing complete diagnostic: {detail}"
            );
        }
    }

    #[trace("FR-059-AC-1", "FR-048-AC-9")]
    #[test]
    fn lead_status_uses_bound_team_and_project_only_scope_does_not_inherit_it() {
        let mut app = crate::monitor_ui::tests::app();
        let expected =
            crate::monitor_tracker::summarize(app.tracker.value.as_ref().unwrap(), None, "SWM")
                .unwrap();
        assert!(!expected.scope.leaf_ids.is_empty());
        key(&mut app, KeyCode::Char('1'));
        let progress = format!(
            "{}/{} done",
            expected.scope.done,
            expected.scope.leaf_ids.len()
        );
        assert!(screen(&mut app, 80, 24).contains(&progress));
        assert_eq!(app.department_team().as_deref(), Some("SWM"));
        app.binding.as_mut().unwrap().address = "luna:OTHER:code".into();
        app.projects = vec!["project".into()];
        assert!(app.department_team().is_none());
        assert!(screen(&mut app, 80, 24).contains(&progress));
        app.team = Some("SWM".into());
        assert_eq!(app.department_team().as_deref(), Some("SWM"));
    }

    /// Trace: FR-065-AC-12
    #[trace("FR-065-AC-12")]
    #[test]
    fn department_evidence_keeps_the_existing_color_policy() {
        for enabled in [false, true] {
            let mut app = App {
                view: View::Status,
                color_enabled: enabled,
                ..App::default()
            };
            app.department.failure("source unavailable");
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut app, Instant::now()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let row = (0..24)
                .find(|y| {
                    (0..80)
                        .map(|x| buffer[(x, *y)].symbol())
                        .collect::<String>()
                        .starts_with("Department failed")
                })
                .expect("the failure evidence row");
            let expected = if enabled {
                Color::LightRed
            } else {
                Color::Reset
            };
            // Both the label and the detail follow the policy.
            for x in [0, 20] {
                assert_eq!(buffer[(x, row)].fg, expected, "color_enabled={enabled}");
            }
        }
    }

    fn flat(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn labels(app: &App) -> String {
        label_lines(app)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Trace: FR-065-AC-7
    #[trace("FR-065-AC-7")]
    #[test]
    fn department_views_name_their_selection_and_state_the_fixed_relation() {
        let mut explicit = App {
            team: Some("SWM".into()),
            projects: vec!["native-project".into()],
            ..App::default()
        };
        let mut derived = crate::monitor_ui::tests::app();
        let mut saved = App {
            department_input: DepartmentInput::Saved,
            ..App::default()
        };
        saved.department.success(fixture(), Instant::now());
        for (name, app, selection) in [
            (
                "explicit",
                &mut explicit,
                "team SWM · projects native-project",
            ),
            (
                "derived",
                &mut derived,
                "team SWM derived from lead workspace · all projects",
            ),
            ("saved", &mut saved, "saved report · team:SWM (Department)"),
        ] {
            for view in [View::Status, View::Health, View::Retro] {
                app.view = view;
                let text = labels(app);
                assert!(
                    text.contains(&format!("Department selection: {selection}")),
                    "{name}\n{text}"
                );
                assert!(text.contains(DEPARTMENT_RELATION), "{name}\n{text}");
                let lower = text.to_lowercase();
                for word in ["equal", "same", "narrower", "subset"] {
                    assert!(!lower.contains(word), "{name} {word}\n{text}");
                }
                for (width, height) in [(80, 24), (120, 40)] {
                    let drawn = flat(&screen(app, width, height));
                    assert!(drawn.contains(DEPARTMENT_RELATION), "{name}\n{drawn}");
                    assert!(drawn.contains(&format!("Department selection: {selection}")));
                }
            }
        }
    }

    /// Trace: FR-065-AC-8
    #[trace("FR-065-AC-8")]
    #[test]
    fn status_states_its_purpose_without_hiding_evidence_or_rows() {
        for (width, height) in [(80, 24), (120, 40)] {
            let mut app = App {
                view: View::Status,
                ..App::default()
            };
            app.department.success(fixture(), Instant::now());
            app.department.failure("source unavailable");
            app.runtime.failure("ambiguous requested lead");
            let text = flat(&screen(&mut app, width, height));
            assert!(text.contains(STATUS_PURPOSE), "{text}");
            assert!(text.contains("Department failed:"), "{text}");
            assert!(text.contains("Binding / Herdr failed:"), "{text}");
            assert!(text.contains("Bound activity"), "{text}");
            for view in [View::Health, View::Retro] {
                app.view = view;
                let other = flat(&screen(&mut app, width, height));
                assert!(!other.contains(STATUS_PURPOSE), "{view:?}");
            }
        }
    }

    /// Trace: FR-065-AC-9
    #[trace("FR-065-AC-9")]
    #[test]
    fn health_and_retro_show_one_concise_state_and_never_a_failure_or_a_value() {
        for view in [View::Health, View::Retro] {
            let name = if view == View::Health {
                "Health"
            } else {
                "Retro"
            };
            let mut live = App {
                view,
                team: Some("SWM".into()),
                ..App::default()
            };
            let text = flat(&screen(&mut live, 80, 24));
            assert_eq!(text.matches("Not configured").count(), 1, "{text}");
            assert!(text.contains("--retro-config <file>"), "{text}");
            assert!(text.contains("team SWM"), "{text}");
            assert_eq!(
                text.contains("retained snapshot history"),
                view == View::Retro
            );
            assert!(diagnostic_rows(&live).is_empty());
            assert!(live.department.error.is_none());
            let lower = text.to_lowercase();
            for forbidden in [
                "failed",
                "authentication",
                "complete",
                "green",
                "measure / class",
            ] {
                assert!(!lower.contains(forbidden), "{forbidden}\n{text}");
            }

            let mut saved = App {
                view,
                department_input: DepartmentInput::Saved,
                ..App::default()
            };
            let mut lacking = fixture();
            lacking.current.classes.clear();
            lacking.current.measurements.clear();
            lacking.current.loops.clear();
            saved.department.success(lacking, Instant::now());
            let text = flat(&screen(&mut saved, 80, 24));
            assert!(
                text.contains(&format!("does not supply the {name} fields")),
                "{text}"
            );
            assert!(text.contains("--report <file>"), "{text}");
            assert!(diagnostic_rows(&saved).is_empty());

            let mut waiting = App {
                view,
                department_input: DepartmentInput::Configured,
                ..App::default()
            };
            let text = flat(&screen(&mut waiting, 80, 24));
            assert!(
                text.contains("Waiting for the first read of the retro configuration"),
                "{text}"
            );
            assert!(!text.to_lowercase().contains("failed"), "{text}");
            assert!(diagnostic_rows(&waiting).is_empty());

            let mut failed = App {
                view,
                department_input: DepartmentInput::Configured,
                ..App::default()
            };
            failed.department.failure("source unreachable");
            let text = flat(&screen(&mut failed, 80, 24));
            assert!(
                text.contains(&format!("{name} unavailable: source unreachable")),
                "{text}"
            );
            assert!(text.contains("Department failed:"), "{text}");
            assert!(!text.contains("Measure / class"), "{text}");

            let mut stale = App {
                view,
                department_input: DepartmentInput::Configured,
                ..App::default()
            };
            stale.department.success(fixture(), Instant::now());
            stale.department.failure("source unreachable");
            let text = flat(&screen(&mut stale, 80, 24));
            assert!(text.contains("last snapshot"), "{text}");
            assert!(text.contains("Measure / class"), "{text}");
        }
    }

    /// State-only companion to the production event-loop fixtures in `monitor_main`.
    #[test]
    fn popups_over_a_not_configured_view_leave_source_state_untouched() {
        let mut app = App {
            view: View::Health,
            ..App::default()
        };
        for open in [KeyCode::Char('v'), KeyCode::F(1)] {
            key(&mut app, open);
            screen(&mut app, 80, 24);
            key(&mut app, KeyCode::Esc);
        }
        assert!(app.popup.is_none() && !app.should_quit);
        assert!(!app.refresh_now && !app.collection_failed);
        assert!(app.department.value.is_none() && app.department.error.is_none());
        assert_eq!(app.department.revision, 0);
        assert!(app.tracker.error.is_none() && app.runtime.error.is_none());
        assert_eq!(app.department_input, DepartmentInput::NotConfigured);
    }
}
