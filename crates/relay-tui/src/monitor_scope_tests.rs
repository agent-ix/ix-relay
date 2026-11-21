// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! FR-060 Scope hierarchy, visibility, source-state and reuse tests.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use ix_board::{Initiative, IssueId, ProjectId, ScopeSummary};
use ix_trace_rs::trace;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    widgets::TableState,
};
use relay_core::work::WorkScope;

use crate::{
    monitor::{App, View, handle_key},
    monitor_scope::{
        Entry, FILTER_UNAVAILABLE, FocusMode, LINKS_UNAVAILABLE, NO_SELECTED_INITIATIVE,
        ScopeProjection, ScopeState, footer_mode, project,
    },
    monitor_tracker::{
        DashboardView, InitiativeRef, ProjectCompletion, ScopeProgress, TrackerState,
    },
    monitor_ui::{self, tests::flat},
};

fn progress(id: &str, name: &str, done: usize, total: usize) -> ScopeProgress {
    ScopeProgress {
        id: id.into(),
        name: name.into(),
        initiatives: Vec::new(),
        initiative_refs: Vec::new(),
        links_complete: true,
        completion: ProjectCompletion::Unknown,
        status: None,
        remaining_by_stage: BTreeMap::new(),
        ready: 0,
        blocked: 0,
        done,
        total,
    }
}

/// A project related to the named initiative ids, with a completion category.
fn child(id: &str, parents: &[&str], completion: ProjectCompletion) -> ScopeProgress {
    let mut p = progress(id, id, 1, 3);
    p.initiative_refs = parents
        .iter()
        .map(|parent| InitiativeRef {
            id: Some((*parent).into()),
            name: (*parent).into(),
        })
        .collect();
    p.completion = completion;
    p
}

fn view(
    initiatives: Vec<ScopeProgress>,
    projects: Vec<ScopeProgress>,
    supplied: bool,
) -> DashboardView {
    DashboardView {
        scope: ScopeSummary {
            project_ids: BTreeSet::new(),
            issue_ids: BTreeSet::new(),
            leaf_ids: BTreeSet::new(),
            done: 0,
            remaining_by_stage: BTreeMap::new(),
        },
        ticket_roots: Vec::new(),
        initiatives,
        projects,
        tickets: Vec::new(),
        ready_queue: Vec::new(),
        completion_supplied: supplied,
    }
}

fn scope(initiatives: &[&str], projects: &[&str], tickets: &[&str]) -> WorkScope {
    WorkScope {
        initiatives: initiatives.iter().map(|s| (*s).into()).collect(),
        projects: projects.iter().map(|s| (*s).into()).collect(),
        tickets: tickets.iter().map(|s| (*s).into()).collect(),
        ..WorkScope::default()
    }
}

/// Rows as `name`, indented when nested, with marker and label text.
fn outline(view: &DashboardView, projection: &ScopeProjection) -> Vec<String> {
    projection
        .rows
        .iter()
        .map(|row| {
            let name = match row.entry {
                Entry::Initiative(i) => &view.initiatives[i].name,
                Entry::Project(p) => &view.projects[p].name,
            };
            format!(
                "{}{name}{}{}",
                if row.nested { "  " } else { "" },
                if row.shared { " [shared]" } else { "" },
                row.label.map_or_else(String::new, |l| format!(" {l}"))
            )
        })
        .collect()
}

fn terminal_text(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
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

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| monitor_ui::render(frame, app, Instant::now()))
        .unwrap();
    terminal_text(&terminal, width, height)
}

fn press(app: &mut App, code: KeyCode) {
    handle_key(app, KeyEvent::new(code, KeyModifiers::NONE));
}

/// Trace: FR-060-AC-1
#[trace("FR-060-AC-1")]
#[test]
fn projects_nest_under_initiatives_in_model_order_with_shared_markers() {
    use ProjectCompletion::Nonterminal as Active;
    let v = view(
        vec![progress("a", "Alpha", 2, 6), progress("b", "Beta", 3, 9)],
        vec![
            child("shared", &["a", "b"], Active),
            child("only-a", &["a"], Active),
            child("elsewhere", &["z"], Active),
        ],
        true,
    );
    let focus = scope(&["a", "b"], &[], &[]);
    let p = project(&v, Some(&focus), false);
    assert_eq!(
        outline(&v, &p),
        [
            "Alpha",
            "  shared [shared]",
            "  only-a",
            "Beta",
            "  shared [shared]",
            &format!("elsewhere {NO_SELECTED_INITIATIVE}"),
        ]
    );
    assert_eq!(
        outline(&v, &p),
        outline(&v, &project(&v, Some(&focus), false)),
        "the same input renders the same order"
    );
}

/// Trace: FR-060-AC-1
#[trace("FR-060-AC-1")]
#[test]
fn rendered_rows_keep_counts_prefixes_and_add_no_heading_rows() {
    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    let snapshot = app.tracker.value.as_mut().unwrap();
    snapshot.projects[0].initiatives.push(Initiative {
        id: "initiative".into(),
        identifier: "I-3".into(),
        name: "Program".into(),
    });
    let mut other = snapshot.projects[0].clone();
    other.id = ProjectId("other".into());
    other.name = "Other".into();
    other.initiatives.clear();
    snapshot.projects.push(other);
    let mut extra = snapshot.issues[2].clone();
    extra.id = IssueId("extra".into());
    extra.identifier = "SWM-99".into();
    extra.project_id = Some(ProjectId("other".into()));
    snapshot.issues.push(extra);
    let report = app.report.value.as_mut().unwrap().as_mut().unwrap();
    report.scope = Some(scope(&["initiative"], &["other"], &[]));
    let text = screen(&mut app, 80, 24);
    let lines: Vec<&str> = text.lines().collect();
    let focus = lines
        .iter()
        .position(|l| l.contains("Focus: not reported"))
        .unwrap();
    assert!(lines[focus + 1].trim().is_empty(), "{text}");
    assert!(lines[focus + 2].contains("Initiative/project"), "{text}");
    assert!(lines[focus + 3].contains("🎯") && lines[focus + 3].contains("Program"));
    assert!(lines[focus + 3].contains("1/2"));
    assert!(lines[focus + 4].contains("📁") && lines[focus + 4].contains("Product"));
    assert!(lines[focus + 4].contains("1/2"));
    // The shared model supplies no link-completeness marker, so a project without a
    // displayed initiative is labeled unavailable, never "(no selected initiative)".
    assert!(
        lines[focus + 5].contains("Other") && lines[focus + 5].contains(LINKS_UNAVAILABLE),
        "{text}"
    );
    assert!(!text.contains(NO_SELECTED_INITIATIVE), "{text}");
    assert!(lines[focus + 6].trim().is_empty(), "{text}");
    assert!(
        !text.contains("rows are the lead's reported focus"),
        "{text}"
    );
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.view, View::Tickets);
}

/// Trace: FR-060-AC-2
#[trace("FR-060-AC-2")]
#[test]
fn missing_partial_and_ambiguous_links_are_never_called_unassigned() {
    use ProjectCompletion::Nonterminal as Active;
    let mut missing = child("missing", &[], Active);
    missing.links_complete = false;
    let mut partial = child("partial", &["a"], Active);
    partial.links_complete = false;
    let mut partial_elsewhere = child("partial-elsewhere", &["z"], Active);
    partial_elsewhere.links_complete = false;
    let mut by_name = child("by-name", &[], Active);
    by_name.initiative_refs = vec![InitiativeRef {
        id: None,
        name: "Dup".into(),
    }];
    let v = view(
        vec![
            progress("d1", "Dup", 0, 1),
            progress("d2", "Dup", 0, 1),
            progress("a", "Alpha", 0, 1),
        ],
        vec![
            missing,
            partial,
            partial_elsewhere,
            by_name,
            child("by-id", &["d1"], Active),
        ],
        true,
    );
    let p = project(&v, Some(&scope(&["a", "d1", "d2"], &[], &[])), false);
    let lines = outline(&v, &p);
    assert_eq!(
        lines,
        [
            "Dup",
            "  by-id",
            "Dup",
            "Alpha",
            "  partial",
            &format!("missing {LINKS_UNAVAILABLE}"),
            &format!("partial-elsewhere {LINKS_UNAVAILABLE}"),
            &format!("by-name {LINKS_UNAVAILABLE}"),
        ]
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains(NO_SELECTED_INITIATIVE) || l.contains("unassigned"))
    );
}

/// Trace: FR-060-AC-3
#[trace("FR-060-AC-3")]
#[test]
fn exactly_one_focus_mode_is_chosen_from_the_reported_scope_only() {
    assert_eq!(
        FocusMode::of(Some(&scope(&["i"], &[], &[]))),
        FocusMode::Focused
    );
    assert_eq!(
        FocusMode::of(Some(&scope(&[], &["p"], &["t"]))),
        FocusMode::Focused
    );
    assert_eq!(
        FocusMode::of(Some(&scope(&[], &[], &[]))),
        FocusMode::WholeTeam
    );
    assert_eq!(FocusMode::of(None), FocusMode::WholeTeam);
    assert_eq!(
        FocusMode::of(Some(&scope(&[], &[], &["t"]))),
        FocusMode::TicketOnly
    );
    assert!(FocusMode::WholeTeam.text().contains("no focus reported"));
    assert!(FocusMode::WholeTeam.text().contains("whole team"));
    assert!(
        FocusMode::TicketOnly
            .text()
            .contains("no initiative/project focus reported")
    );
    assert!(
        !FocusMode::TicketOnly
            .text()
            .contains("lead's reported focus")
    );
}

/// Trace: FR-060-AC-3
#[trace("FR-060-AC-3")]
#[test]
fn reported_initiatives_stay_while_fallback_omits_empty_ones_and_all_hidden_explains() {
    use ProjectCompletion::{Nonterminal as Active, Terminal as Done};
    let v = view(
        vec![progress("a", "Alpha", 0, 1), progress("b", "Beta", 0, 1)],
        vec![child("old", &["a"], Done), child("live", &["b"], Active)],
        true,
    );
    let focused = project(&v, Some(&scope(&["a", "b"], &[], &[])), false);
    assert_eq!(outline(&v, &focused), ["Alpha", "Beta", "  live"]);
    assert_eq!(focused.omitted_initiatives, 0);
    let fallback = project(&v, None, false);
    assert_eq!(outline(&v, &fallback), ["Beta", "  live"]);
    assert_eq!(fallback.omitted_initiatives, 1);
    assert!(
        fallback
            .filter_text()
            .unwrap()
            .contains("1 initiative omitted")
    );
    let only = view(vec![], vec![child("old", &[], Done)], true);
    let hidden = project(&only, Some(&scope(&[], &["old"], &[])), false);
    assert!(hidden.rows.is_empty());
    let message = hidden.all_hidden_message().unwrap();
    assert!(message.contains("1 completed/canceled project hidden"));
    assert!(message.contains("press c to show"));
    assert!(
        project(&only, Some(&scope(&[], &["old"], &[])), true)
            .all_hidden_message()
            .is_none()
    );
}

/// Trace: FR-060-AC-4, FR-060-AC-5
#[trace("FR-060-AC-4")]
#[test]
fn only_completed_and_canceled_are_hidden_and_the_others_keep_order_and_progress() {
    use ProjectCompletion::{Nonterminal as Active, Terminal as Done, Unknown};
    let mut all_done = child("active-all-done", &["a"], Active);
    all_done.done = 3;
    all_done.total = 3;
    let v = view(
        vec![progress("a", "Alpha", 4, 9), progress("b", "Beta", 5, 9)],
        vec![
            child("active", &["a"], Active),
            child("completed", &["a", "b"], Done),
            all_done,
            child("canceled", &["b"], Done),
            child("unknown", &["b"], Unknown),
        ],
        true,
    );
    let focus = scope(&["a", "b"], &[], &[]);
    let hiding = project(&v, Some(&focus), false);
    let showing = project(&v, Some(&focus), true);
    let shown = outline(&v, &showing);
    let hidden = outline(&v, &hiding);
    assert_eq!(
        hidden,
        [
            "Alpha",
            "  active",
            "  active-all-done",
            "Beta",
            "  unknown",
        ]
    );
    // Showing only adds rows; the others keep their relative order.
    let mut rest = shown.iter();
    assert!(hidden.iter().all(|row| rest.any(|s| s == row)));
    assert_eq!(shown.len(), 8);
    // The distinct hidden count counts the shared project once.
    assert_eq!(hiding.hidden_projects, 2);
    assert_eq!(showing.hidden_projects, 0);
    let text = hiding.filter_text().unwrap();
    assert!(
        text.contains("2 completed/canceled projects hidden"),
        "{text}"
    );
    assert!(text.contains("initiative progress includes hidden projects"));
    // Every displayed row presents the shared-model counts of its project.
    for row in &hiding.rows {
        if let Entry::Project(p) = row.entry {
            let same = showing
                .rows
                .iter()
                .find(|r| r.key == row.key)
                .map(|r| r.entry);
            assert_eq!(same, Some(Entry::Project(p)));
        }
    }
    // Toggling twice returns to the same rows.
    assert_eq!(outline(&v, &project(&v, Some(&focus), false)), hidden);
}

/// Trace: FR-060-AC-4
#[trace("FR-060-AC-4")]
#[test]
fn c_toggles_only_in_the_scope_view_without_disturbing_existing_keys() {
    let mut app = App::default();
    assert!(!app.show_completed);
    press(&mut app, KeyCode::Char('c'));
    assert!(!app.show_completed, "no effect outside the Scope view");
    press(&mut app, KeyCode::Char('p'));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(!app.show_completed);
    press(&mut app, KeyCode::Char('c'));
    assert!(app.show_completed);
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.view, View::Tickets);
    assert!(app.show_completed, "the mode survives view changes");
    press(&mut app, KeyCode::Char('p'));
    press(&mut app, KeyCode::Char('c'));
    assert!(!app.show_completed);
    for (key, view) in [
        ('1', View::Status),
        ('2', View::Health),
        ('3', View::Retro),
        ('a', View::Agents),
        ('p', View::Scope),
        ('t', View::Tickets),
    ] {
        press(&mut app, KeyCode::Char(key));
        assert_eq!(app.view, view);
    }
    press(&mut app, KeyCode::Char('r'));
    assert!(app.refresh_now);
    press(&mut app, KeyCode::Char('s'));
    assert!(app.show_status);
    press(&mut app, KeyCode::Char('d'));
    assert!(app.details);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.should_quit);
    assert_eq!(footer_mode(None, false), "hiding completed");
    assert_eq!(footer_mode(None, true), "showing completed");
}

/// Trace: FR-060-AC-4
#[trace("FR-060-AC-4")]
#[test]
fn selection_follows_the_same_placement_or_clamps_after_rows_change() {
    use ProjectCompletion::{Nonterminal as Active, Terminal as Done};
    let v = view(
        vec![progress("a", "Alpha", 0, 1)],
        vec![
            child("first", &["a"], Done),
            child("second", &["a"], Active),
            child("third", &["a"], Active),
        ],
        true,
    );
    let focus = scope(&["a"], &[], &[]);
    let mut shown = project(&v, Some(&focus), true);
    shown.build = 1;
    let mut state = ScopeState::default();
    let mut table = TableState::default();
    state.restore_selection(&mut table, &shown);
    table.select(Some(3));
    state.remember_selection(&shown, 3);
    let mut hidden = project(&v, Some(&focus), false);
    hidden.build = 2;
    state.restore_selection(&mut table, &hidden);
    assert_eq!(table.selected(), Some(2), "third moved up when first hid");
    state.remember_selection(&hidden, 2);
    let mut empty = project(&view(vec![], vec![], true), None, false);
    empty.build = 3;
    state.restore_selection(&mut table, &empty);
    assert_eq!(
        crate::monitor_views::select_row(&mut App::default(), empty.rows.len()),
        0
    );
}

/// Trace: FR-060-AC-5
#[trace("FR-060-AC-5")]
#[test]
fn initiative_progress_is_the_shared_value_whether_projects_are_hidden_or_not() {
    use ProjectCompletion::{Nonterminal as Active, Terminal as Done};
    let v = view(
        vec![progress("a", "Alpha", 4, 9)],
        vec![child("old", &["a"], Done), child("live", &["a"], Active)],
        true,
    );
    for show in [false, true] {
        let p = project(&v, Some(&scope(&["a"], &[], &[])), show);
        let Entry::Initiative(i) = p.rows[0].entry else {
            panic!("initiative first");
        };
        assert_eq!((v.initiatives[i].done, v.initiatives[i].total), (4, 9));
    }
}

/// Trace: FR-060-AC-6
#[trace("FR-060-AC-6")]
#[test]
fn an_unsupplied_category_hides_nothing_and_says_so() {
    use ProjectCompletion::Unknown;
    let mut named = child("Completed", &["a"], Unknown);
    named.status = Some("Completed".into());
    named.done = 3;
    named.total = 3;
    let mut canceled = child("Canceled", &["a"], ProjectCompletion::Terminal);
    canceled.status = Some("Canceled".into());
    // Even a carried terminal value is ignored while the model supplies no category.
    let v = view(
        vec![progress("a", "Alpha", 0, 1)],
        vec![named, canceled],
        false,
    );
    let p = project(&v, Some(&scope(&["a"], &[], &[])), false);
    assert_eq!(outline(&v, &p), ["Alpha", "  Completed", "  Canceled"]);
    assert_eq!(p.hidden_projects, 0);
    assert_eq!(p.filter_text().as_deref(), Some(FILTER_UNAVAILABLE));
    assert!(p.all_hidden_message().is_none());
    assert_eq!(footer_mode(Some(&p), false), "completed filter unavailable");
    assert_eq!(footer_mode(Some(&p), true), "completed filter unavailable");
}

/// Trace: FR-060-AC-6
#[trace("FR-060-AC-6")]
#[test]
fn pinned_model_renders_unavailable_filter_and_the_key_keeps_its_mode() {
    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    let before = screen(&mut app, 120, 24);
    assert!(before.contains(FILTER_UNAVAILABLE), "{before}");
    assert!(
        before.contains("c completed filter unavailable"),
        "{before}"
    );
    assert!(!before.contains("hiding completed"));
    assert!(!before.contains("hidden"));
    press(&mut app, KeyCode::Char('c'));
    assert!(app.show_completed);
    let after = screen(&mut app, 120, 24);
    assert!(after.contains("completed filter unavailable"));
    assert!(!after.contains("showing completed"));
    let rows = |text: &str| text.lines().filter(|l| l.contains("📁")).count();
    assert_eq!(rows(&before), rows(&after));
    press(&mut app, KeyCode::Char('t'));
    assert!(!screen(&mut app, 120, 24).contains("completed filter unavailable"));
}

/// Trace: FR-060-AC-3
#[trace("FR-060-AC-3")]
#[test]
fn fallback_and_ticket_only_scopes_are_labeled_without_inventing_focus() {
    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    app.report.value.as_mut().unwrap().as_mut().unwrap().scope = None;
    let fallback = screen(&mut app, 100, 24);
    assert!(fallback.contains("no focus reported"), "{fallback}");
    assert!(!fallback.contains("lead's reported focus"));
    assert!(fallback.contains("Product"));
    let report = app.report.value.clone().unwrap();
    let mut ticket_only = report;
    ticket_only.as_mut().unwrap().scope = Some(scope(&[], &[], &["id-12"]));
    app.report.success(ticket_only, Instant::now());
    let text = screen(&mut app, 100, 24);
    assert!(
        text.contains("no initiative/project focus reported"),
        "{text}"
    );
    assert!(!text.contains("lead's reported focus"));
}

/// Trace: FR-060-AC-7
#[trace("FR-060-AC-7")]
#[test]
fn source_states_keep_rows_and_never_show_a_zero_count_without_a_model() {
    let mut app = App {
        view: View::Scope,
        ..App::default()
    };
    press(&mut app, KeyCode::Char('c'));
    assert!(app.show_completed);
    let waiting = screen(&mut app, 100, 24);
    assert!(
        waiting.contains("Scope progress unavailable: waiting for Linear"),
        "{waiting}"
    );
    assert!(!waiting.contains("hidden"));
    assert!(!waiting.contains("📁"));
    app.apply_tracker_refresh(Err(ix_board::Error::MissingApiKey), Instant::now());
    let failed = screen(&mut app, 120, 24);
    assert!(failed.contains("Scope progress unavailable"), "{failed}");
    assert!(!failed.contains("hidden"));

    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    let good = screen(&mut app, 120, 24);
    assert!(good.contains("📁") && good.contains(FILTER_UNAVAILABLE));
    app.apply_tracker_refresh(Err(ix_board::Error::MissingApiKey), Instant::now());
    let retained = screen(&mut app, 120, 24);
    assert!(retained.contains("📁"), "{retained}");
    assert!(retained.contains(FILTER_UNAVAILABLE));
    assert!(retained.contains("[last snapshot]"), "{retained}");
}

/// Trace: FR-060-AC-8, FR-060-AC-9
#[trace("FR-060-AC-8")]
#[test]
fn shared_summary_is_reused_until_source_report_or_scope_changes() {
    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let mut draw = |app: &mut App| {
        terminal
            .draw(|frame| monitor_ui::render(frame, app, Instant::now()))
            .unwrap();
        (app.scope.summary_computations, app.scope.projection_builds)
    };
    assert_eq!(draw(&mut app), (1, 1));
    assert_eq!(draw(&mut app), (1, 1), "an idle draw changes neither");
    for code in [
        KeyCode::Down,
        KeyCode::Char('j'),
        KeyCode::PageDown,
        KeyCode::Up,
    ] {
        press(&mut app, code);
        assert_eq!(draw(&mut app), (1, 1), "{code:?}");
    }
    for code in [
        KeyCode::Char('t'),
        KeyCode::Char('a'),
        KeyCode::Char('p'),
        KeyCode::Char('d'),
    ] {
        press(&mut app, code);
        assert_eq!(draw(&mut app), (1, 1), "{code:?}");
    }
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(draw(&mut app), (1, 2), "a toggle adds one projection build");
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(draw(&mut app), (1, 3));
    app.tracker.failure("offline");
    assert_eq!(draw(&mut app), (1, 3), "a failed refresh keeps the result");
    let report = app.report.value.clone().unwrap();
    app.report.success(report, Instant::now());
    assert_eq!(draw(&mut app), (2, 4), "a report change adds one of each");
    let snapshot: TrackerState = app.tracker.value.clone().unwrap();
    app.apply_tracker_refresh(
        Ok(crate::monitor_ops::TrackerRead {
            snapshot,
            freshness: None,
        }),
        Instant::now(),
    );
    assert_eq!(draw(&mut app), (3, 5), "a source change adds one of each");
    app.team = Some("SWM".into());
    assert_eq!(draw(&mut app), (3, 5), "the same effective scope is reused");
    app.team = Some("AGE".into());
    assert_eq!(
        draw(&mut app).0,
        4,
        "a team change recomputes the summary once"
    );
    assert_eq!(draw(&mut app).0, 4);
}

/// Trace: FR-060-AC-9
#[trace("FR-060-AC-9")]
#[test]
fn a_changed_report_scope_updates_the_displayed_rows() {
    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    let snapshot = app.tracker.value.as_mut().unwrap();
    let mut other = snapshot.projects[0].clone();
    other.id = ProjectId("other".into());
    other.name = "Other".into();
    snapshot.projects.push(other);
    let mut extra = snapshot.issues[2].clone();
    extra.id = IssueId("extra".into());
    extra.identifier = "SWM-99".into();
    extra.project_id = Some(ProjectId("other".into()));
    snapshot.issues.push(extra);
    app.scope.invalidate();
    assert!(!screen(&mut app, 100, 24).contains("Other"));
    let mut report = app.report.value.clone().unwrap();
    report.as_mut().unwrap().scope = Some(scope(&[], &["project", "other"], &[]));
    app.report.success(report, Instant::now());
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("Other"), "{text}");
    assert_eq!(app.row_count, 2);
}

/// Real shared-model summary: one initiative with projects in every category.
fn categorized_app() -> App {
    use ix_board::{ProjectStatusCategory as Category, WorkflowStateType};
    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    let initiative = Initiative {
        id: "i1".into(),
        identifier: "I-1".into(),
        name: "Program".into(),
    };
    let snapshot = app.tracker.value.as_mut().unwrap();
    let project_template = snapshot.projects[0].clone();
    let issue_template = snapshot.issues[2].clone();
    snapshot.projects.clear();
    snapshot.issues.clear();
    let rows = [
        ("Live", Category::Started, WorkflowStateType::Started),
        ("Retired", Category::Completed, WorkflowStateType::Completed),
        ("Dropped", Category::Canceled, WorkflowStateType::Started),
        ("Mystery", Category::Unknown, WorkflowStateType::Started),
        (
            "Finished tasks",
            Category::Started,
            WorkflowStateType::Completed,
        ),
    ];
    for (n, (name, category, state)) in rows.into_iter().enumerate() {
        let mut project = project_template.clone();
        project.id = ProjectId(format!("p{n}"));
        project.name = name.into();
        project.status_category = category;
        project.initiatives = vec![initiative.clone()];
        let mut issue = issue_template.clone();
        issue.id = IssueId(format!("leaf-{n}"));
        issue.identifier = format!("SWM-{}", 200 + n);
        issue.project_id = Some(project.id.clone());
        issue.state_type = state;
        snapshot.projects.push(project);
        snapshot.issues.push(issue);
    }
    app.report.value.as_mut().unwrap().as_mut().unwrap().scope = Some(scope(&["i1"], &[], &[]));
    app.scope.invalidate();
    app
}

/// Trace: FR-060-AC-4, FR-060-AC-5
#[trace("FR-060-AC-4")]
#[test]
fn supplied_categories_hide_only_terminal_projects_from_a_real_summary() {
    let mut app = categorized_app();
    let hiding = screen(&mut app, 130, 30);
    for shown in ["Live", "Mystery", "Finished tasks"] {
        assert!(hiding.contains(shown), "{shown}\n{hiding}");
    }
    for hidden in ["Retired", "Dropped"] {
        assert!(!hidden_row(&hiding, hidden), "{hidden}\n{hiding}");
    }
    assert!(
        hiding.contains("2 completed/canceled projects hidden"),
        "{hiding}"
    );
    assert!(hiding.contains("hiding completed"), "{hiding}");
    assert!(!hiding.contains(FILTER_UNAVAILABLE));
    let summaries = app.scope.summary_computations;
    press(&mut app, KeyCode::Char('c'));
    let showing = screen(&mut app, 130, 30);
    for shown in ["Live", "Retired", "Dropped", "Mystery", "Finished tasks"] {
        assert!(showing.contains(shown), "{shown}\n{showing}");
    }
    assert!(showing.contains("showing completed"), "{showing}");
    assert_eq!(
        app.scope.summary_computations, summaries,
        "no recomputation"
    );
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(
        screen(&mut app, 130, 30),
        hiding,
        "the toggle returns to the same rows"
    );
}

fn hidden_row(text: &str, name: &str) -> bool {
    text.lines()
        .any(|line| line.contains("📁") && line.contains(name))
}

/// Trace: FR-060-AC-2
#[trace("FR-060-AC-2")]
#[test]
fn projects_outside_a_displayed_initiative_are_never_called_unassigned_without_a_marker() {
    let mut app = monitor_ui::tests::app();
    app.view = View::Scope;
    let text = screen(&mut app, 120, 24);
    assert!(text.contains(LINKS_UNAVAILABLE), "{text}");
    assert!(!text.contains(NO_SELECTED_INITIATIVE) && !text.contains("unassigned"));
}

/// The text FR-060 requires in the Scope view at the 80x24 target, whatever the state.
fn assert_scope_essentials(text: &str, footer_mode: &str) {
    let lines: Vec<&str> = text.lines().collect();
    let focus = lines
        .iter()
        .position(|l| l.contains("Focus:"))
        .unwrap_or_else(|| panic!("no Focus line\n{text}"));
    assert!(lines[focus + 1].trim().is_empty(), "blank row\n{text}");
    assert!(lines[focus + 2].contains("Initiative/project"), "{text}");
    assert!(
        flat(text).contains(&format!("c {footer_mode}")),
        "footer key and mode\n{text}"
    );
}

fn assert_count_statements(text: &str) {
    let flat = flat(text);
    for needed in [
        "2 completed/canceled projects hidden",
        "initiative progress includes hidden projects",
        "1 project with unknown status retained",
    ] {
        assert!(flat.contains(needed), "{needed}\n{text}");
    }
}

/// Trace: FR-060-AC-4, FR-060-AC-5
#[trace("FR-060-AC-5")]
#[test]
fn hidden_count_and_mixed_unknown_statements_fit_80x24() {
    let mut app = categorized_app();
    let text = screen(&mut app, 80, 24);
    assert_scope_essentials(&text, "hiding completed");
    assert_count_statements(&text);
    for shown in ["Live", "Mystery", "Finished tasks"] {
        assert!(hidden_row(&text, shown), "{shown}\n{text}");
    }
    press(&mut app, KeyCode::Char('c'));
    let showing = screen(&mut app, 80, 24);
    assert_scope_essentials(&showing, "showing completed");
    assert!(!showing.contains("hidden"), "{showing}");
    assert!(flat(&showing).contains("1 project with unknown status retained"));
}

/// Trace: FR-060-AC-6
#[trace("FR-060-AC-6")]
#[test]
fn an_all_unknown_scope_is_unavailable_at_80x24_and_a_real_summary_hides_nothing() {
    let mut app = categorized_app();
    for project in &mut app.tracker.value.as_mut().unwrap().projects {
        project.status_category = ix_board::ProjectStatusCategory::Unknown;
    }
    app.scope.invalidate();
    let text = screen(&mut app, 80, 24);
    assert_scope_essentials(&text, "completed filter unavailable");
    assert!(text.contains(FILTER_UNAVAILABLE), "{text}");
    for name in ["Live", "Retired", "Dropped", "Mystery", "Finished tasks"] {
        assert!(hidden_row(&text, name), "{name}\n{text}");
    }
    assert!(!text.contains("hidden") && !text.contains("hiding completed"));
    assert!(!text.contains("unknown status retained"), "{text}");
}

/// Trace: FR-060-AC-7
#[trace("FR-060-AC-7")]
#[test]
fn error_with_last_snapshot_keeps_rows_counts_footer_mode_and_the_error_at_80x24() {
    let mut app = categorized_app();
    app.apply_tracker_refresh(Err(ix_board::Error::MissingApiKey), Instant::now());
    let text = screen(&mut app, 80, 24);
    assert_scope_essentials(&text, "hiding completed");
    assert_count_statements(&text);
    let flat_text = flat(&text);
    let counted = flat_text.find("projects hidden").unwrap();
    assert!(
        flat_text[counted..].contains("[last snapshot]"),
        "the count is labeled last snapshot\n{text}"
    );
    let error = crate::monitor::display_text(app.tracker.error.as_ref().unwrap(), 20);
    assert!(text.contains(&error), "the error stays visible\n{text}");
    assert!(hidden_row(&text, "Live"), "{text}");
}

/// Trace: FR-060-AC-7
#[trace("FR-060-AC-7")]
#[test]
fn an_aged_snapshot_and_an_ended_session_label_the_count_at_80x24() {
    let mut aged = categorized_app();
    aged.tracker.value.as_mut().unwrap().fetched_at -= 2 * 60 * 60;
    let mut ended = categorized_app();
    ended.session_ended = true;
    let mut fresh = categorized_app();
    let fresh_text = screen(&mut fresh, 80, 24);
    assert!(!fresh_text.contains("[last snapshot]"), "{fresh_text}");
    for (name, app) in [("aged", &mut aged), ("ended", &mut ended)] {
        let text = screen(app, 80, 24);
        assert_scope_essentials(&text, "hiding completed");
        assert_count_statements(&text);
        let flat_text = flat(&text);
        let counted = flat_text.find("projects hidden").unwrap();
        assert!(
            flat_text[counted..].contains("[last snapshot]"),
            "{name}: the count is labeled\n{text}"
        );
        assert!(hidden_row(&text, "Live"), "{name}\n{text}");
    }
}

/// Trace: FR-060-AC-4
#[trace("FR-060-AC-4")]
#[test]
fn many_long_stage_names_cannot_push_the_essential_text_out_at_80x24() {
    let mut app = categorized_app();
    let snapshot = app.tracker.value.as_mut().unwrap();
    let template = snapshot.issues[0].clone();
    for n in 0..8 {
        let mut issue = template.clone();
        issue.id = IssueId(format!("stage-{n}"));
        issue.identifier = format!("SWM-{}", 900 + n);
        issue.state = format!("Awaiting external vendor review {n:02}");
        issue.state_name = issue.state.clone();
        issue.state_type = ix_board::WorkflowStateType::Started;
        snapshot.issues.push(issue);
    }
    app.scope.invalidate();
    app.apply_tracker_refresh(Err(ix_board::Error::MissingApiKey), Instant::now());
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Tasks:"), "{text}");
    assert_scope_essentials(&text, "hiding completed");
    assert_count_statements(&text);
    assert!(text.contains("[last snapshot]"), "{text}");
    assert!(
        hidden_row(&text, "Live") && hidden_row(&text, "Mystery"),
        "{text}"
    );
}

/// Distinct hidden and unknown counts come from the projection, never placements.
#[test]
fn unknown_projects_are_counted_once_and_only_while_the_category_is_supplied() {
    use ProjectCompletion::{Nonterminal as Active, Unknown};
    let v = view(
        vec![progress("a", "Alpha", 0, 1), progress("b", "Beta", 0, 1)],
        vec![
            child("shared-unknown", &["a", "b"], Unknown),
            child("live", &["a"], Active),
        ],
        true,
    );
    let p = project(&v, Some(&scope(&["a", "b"], &[], &[])), false);
    assert_eq!(p.unknown_projects, 1);
    assert!(
        p.filter_text()
            .unwrap()
            .contains("1 project with unknown status retained")
    );
    let unsupplied = view(vec![], vec![child("x", &[], Unknown)], false);
    let p = project(&unsupplied, None, false);
    assert_eq!(p.unknown_projects, 0);
    assert_eq!(p.filter_text().as_deref(), Some(FILTER_UNAVAILABLE));
}

fn selected_line(text: &str) -> &str {
    text.lines()
        .find(|l| l.contains("› "))
        .unwrap_or_else(|| panic!("no selected row\n{text}"))
}

fn without_projects(app: &App, names: &[&str]) -> TrackerState {
    let mut snapshot = app.tracker.value.clone().unwrap();
    let ids: Vec<ProjectId> = snapshot
        .projects
        .iter()
        .filter(|p| names.contains(&p.name.as_str()))
        .map(|p| p.id.clone())
        .collect();
    snapshot.projects.retain(|p| !ids.contains(&p.id));
    snapshot
        .issues
        .retain(|i| i.project_id.as_ref().is_none_or(|id| !ids.contains(id)));
    snapshot
}

/// Trace: FR-060-AC-4
#[trace("FR-060-AC-4")]
#[test]
fn pressing_c_and_shrinking_refreshes_keep_the_selected_placement_through_render() {
    let mut app = categorized_app();
    press(&mut app, KeyCode::Char('c'));
    screen(&mut app, 100, 24);
    for _ in 0..3 {
        press(&mut app, KeyCode::Char('j'));
    }
    let text = screen(&mut app, 100, 24);
    assert!(selected_line(&text).contains("Live"), "{text}");
    assert_eq!(app.table.selected(), Some(3));
    // Hiding removes one row above Live: the selection follows the placement.
    press(&mut app, KeyCode::Char('c'));
    let text = screen(&mut app, 100, 24);
    assert!(selected_line(&text).contains("Live"), "{text}");
    assert_eq!(app.table.selected(), Some(2));
    press(&mut app, KeyCode::Char('c'));
    let text = screen(&mut app, 100, 24);
    assert!(selected_line(&text).contains("Live"), "{text}");
    assert_eq!(app.table.selected(), Some(3));
    // A shrinking refresh above the selection moves it up with its placement.
    press(&mut app, KeyCode::Char('j'));
    let text = screen(&mut app, 100, 24);
    assert!(selected_line(&text).contains("Mystery"), "{text}");
    let snapshot = without_projects(&app, &["Finished tasks"]);
    app.apply_tracker_refresh(
        Ok(crate::monitor_ops::TrackerRead {
            snapshot,
            freshness: None,
        }),
        Instant::now(),
    );
    let text = screen(&mut app, 100, 24);
    assert!(selected_line(&text).contains("Mystery"), "{text}");
    assert_eq!(app.table.selected(), Some(3));
    // Removing the selected placement clamps to the last remaining row.
    press(&mut app, KeyCode::Char('j'));
    let text = screen(&mut app, 100, 24);
    assert!(selected_line(&text).contains("Retired"), "{text}");
    let snapshot = without_projects(&app, &["Retired"]);
    app.apply_tracker_refresh(
        Ok(crate::monitor_ops::TrackerRead {
            snapshot,
            freshness: None,
        }),
        Instant::now(),
    );
    let text = screen(&mut app, 100, 24);
    assert!(selected_line(&text).contains("Mystery"), "{text}");
    assert_eq!(app.table.selected(), Some(3));
}

/// Trace: FR-060-AC-4
#[trace("FR-060-AC-4")]
#[test]
fn a_view_switch_reset_is_not_undone_by_the_remembered_placement() {
    let mut app = categorized_app();
    press(&mut app, KeyCode::Char('c'));
    screen(&mut app, 100, 24);
    for _ in 0..4 {
        press(&mut app, KeyCode::Char('j'));
    }
    assert!(selected_line(&screen(&mut app, 100, 24)).contains("Mystery"));
    press(&mut app, KeyCode::Char('t'));
    // The scope rebuilds while another view is shown.
    let report = app.report.value.clone().unwrap();
    app.report.success(report, Instant::now());
    screen(&mut app, 100, 24);
    press(&mut app, KeyCode::Char('p'));
    let text = screen(&mut app, 100, 24);
    assert_eq!(app.table.selected(), Some(0), "{text}");
    assert!(selected_line(&text).contains("Program"), "{text}");
}

#[trace("FR-065-AC-5")]
#[test]
fn rate_limit_replaced_session_stale_source_and_active_filter_fit_at_80x24() {
    let evidence_rows = |text: &str, labels: &[&str]| {
        text.lines()
            .filter(|line| labels.iter().any(|label| line.contains(label)))
            .count()
    };
    let footer_holds = |text: &str| {
        let footer = text.lines().last().unwrap();
        footer.contains("v View") && footer.contains("F1 Help") && footer.contains("c hiding")
    };
    // A rate-limit countdown, a failed source and a replaced session, with the filter active.
    let mut limited = categorized_app();
    limited.apply_tracker_refresh(
        Err(ix_board::Error::LinearRateLimited {
            retry_after: std::time::Duration::from_secs(3600),
        }),
        Instant::now(),
    );
    limited.report.failure("Redis unavailable");
    limited.session_ended = true;
    let text = screen(&mut limited, 80, 24);
    let labels = [
        "Binding: session replaced",
        "Reports failed",
        "Tracker rate-limited (next attempt in",
    ];
    for label in labels {
        assert!(text.contains(label), "{label}\n{text}");
    }
    assert!(evidence_rows(&text, &labels) <= 4, "{text}");
    assert_count_statements(&text);
    assert!(footer_holds(&text), "{text}");
    // A stale snapshot and a failed source, with the filter active.
    let mut stale = categorized_app();
    stale.tracker.value.as_mut().unwrap().fetched_at -= 2 * 60 * 60;
    stale.runtime.failure("Herdr unreachable");
    let text = screen(&mut stale, 80, 24);
    let labels = ["Tracker stale", "Herdr failed"];
    for label in labels {
        assert!(text.contains(label), "{label}\n{text}");
    }
    assert!(evidence_rows(&text, &labels) <= 4, "{text}");
    assert_count_statements(&text);
    assert!(footer_holds(&text), "{text}");
    assert!(hidden_row(&text, "Live"), "{text}");
}
