// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Read-only projection of the shared department assessment (FR-059).

use crate::{
    monitor::{App, View, display_text},
    monitor_ui::{FOREGROUND, MUTED, PRIMARY, SELECTION, WARNING},
    monitor_views::{content_areas, select_row},
};
use ix_board::retro::department::{
    CLASSES, Comparison, ComparisonState, Delta, Measurement, Route, Scope, Severity, Value,
};
use ix_cli_kit::streams::{DiagnosticFields, Record, Severity as DiagnosticSeverity};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Paragraph, Row, Table, Wrap},
};

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
    let mut rows = vec![row("Plan / progress / blockers", "unavailable", "No structured tracker snapshot".into(), "Saved department reports do not supply a tracker plan or leaf-task progress. Select live scope to read structured tracker data.".into())];
    if let Some(snapshot) = &app.tracker.value {
        match crate::monitor_tracker::summarize_department(
            snapshot,
            app.team.as_deref(),
            &app.projects,
        ) {
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

/// Draws shared results without collecting or assessing evidence.
pub(crate) fn render(frame: &mut Frame, app: &mut App) {
    let [header, tabs, source, body, diagnostics, controls] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let selected_scope = app.department.value.as_ref().map_or_else(
        || {
            format!(
                "team:{} · projects:{}",
                app.team.as_deref().unwrap_or("unknown"),
                app.projects.join(", ")
            )
        },
        |delta| scope_text(&delta.current.scope),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "Department · {}\n{}",
            safe(&app.lead_name),
            safe(&selected_scope)
        ))
        .style(Style::default().fg(FOREGROUND)),
        header,
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}1 Status  {}2 Health  {}3 Retro  t Tickets  a Agents  p Scope",
            if app.view == View::Status { "›" } else { " " },
            if app.view == View::Health { "›" } else { " " },
            if app.view == View::Retro { "›" } else { " " }
        ))
        .style(Style::default().fg(PRIMARY)),
        tabs,
    );
    let department_source = app.department.value.as_ref().map_or_else(
        || "Department source: unavailable; no successful report".into(),
        |delta| {
            let windows = delta
                .current
                .measurements
                .values()
                .map(|metric| {
                    format!(
                        "({}, {}] age:{}",
                        safe(&metric.window_start),
                        safe(&metric.window_end),
                        metric
                            .source_age_seconds
                            .map_or_else(|| "unknown".into(), |age| format!("{age}s"))
                    )
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" · ");
            format!(
                "Evidence: {windows}{}",
                if app.department.error.is_some() {
                    " · last snapshot"
                } else {
                    ""
                }
            )
        },
    );
    let source_text = app.tracker.value.as_ref().map_or_else(
        || department_source.clone(),
        |snapshot| {
            format!(
                "Tracker evidence at epoch {}\n{department_source}",
                snapshot.fetched_at
            )
        },
    );
    frame.render_widget(
        Paragraph::new(source_text)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(MUTED)),
        source,
    );
    let rows = match app.view {
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
            Paragraph::new("Unavailable: no shared department report"),
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
    let errors = [
        &app.department.error,
        &app.tracker.error,
        &app.runtime.error,
        &app.report.error,
    ]
    .into_iter()
    .flatten()
    .map(|v| safe(v))
    .collect::<Vec<_>>()
    .join(" · ");
    let diagnostic = Record::error("DepartmentSource", &errors).with_fields(DiagnosticFields {
        reason: Some("source_unavailable"),
        remedy: Some("Inspect source access or open a saved report with --report"),
        ..DiagnosticFields::default()
    });
    let color = match diagnostic.severity {
        DiagnosticSeverity::Error => Color::LightRed,
        DiagnosticSeverity::Warning => WARNING,
    };
    frame.render_widget(
        Paragraph::new(diagnostic.render_human(false))
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(if app.color_enabled {
                color
            } else {
                Color::Reset
            })),
        diagnostics,
    );
    frame.render_widget(
        Paragraph::new(Line::from(
            "↑↓ select · d details · ←→ scroll evidence · r refresh · q quit",
        )),
        controls,
    );
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
                        metrics: BTreeMap::new(),
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
                loops: BTreeMap::new(),
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
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn key(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Trace: FR-059-AC-3, FR-059-AC-4
    #[test]
    fn shared_fixture_and_serialized_model_project_the_same_values_and_evidence() {
        let model = fixture();
        let json = serde_json::to_vec(&model).unwrap();
        let decoded: Delta = serde_json::from_slice(&json).unwrap();
        for view in [View::Status, View::Health, View::Retro] {
            let mut typed = App {
                view,
                ..App::default()
            };
            typed.department.success(model.clone(), Instant::now());
            let mut serialized = App {
                view,
                ..App::default()
            };
            serialized
                .department
                .success(decoded.clone(), Instant::now());
            for (width, height) in [(80, 24), (120, 40)] {
                assert_eq!(
                    screen(&mut typed, width, height),
                    screen(&mut serialized, width, height)
                );
            }
        }
        let health = health_rows(&decoded);
        assert_eq!(health.len(), 10);
        assert_eq!(health[0].state, "red / incomplete");
        assert!(health[0].detail.contains("Assign a reviewer"));
        assert!(
            health[0]
                .detail
                .contains("partial relation page / unreachable host")
        );
        assert!(health[1].detail.contains("unrouted"));
        let retro = retro_rows(&decoded);
        assert!(retro[0].detail.contains("query_changed"));
        assert!(retro[0].detail.contains("missing baseline"));
        assert!(retro[0].detail.contains("linear:issue:fixture-17"));
        assert!(retro[0].detail.contains("17 issues"));
    }

    /// Trace: FR-059-AC-4
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
        app.table.select(Some(99));
        screen(&mut app, 120, 40);
        assert_eq!(app.table.selected(), Some(0));
    }

    /// Trace: FR-059-AC-5
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
        assert!(output.contains("q quit"));
        key(&mut app, KeyCode::Right);
        assert_eq!(app.detail_scroll, 1);
    }

    /// Trace: FR-059-AC-3
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
}
