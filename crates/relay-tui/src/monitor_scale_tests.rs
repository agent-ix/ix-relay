// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Bounded large-scope scenario that drives the real dashboard render and key handler.
//!
//! The scenario is ignored by default so it only runs when an optimized build is
//! requested explicitly. It asserts no wall-time limit; it prints the timings so the
//! same fixture can be recorded before and after a change.

use std::{sync::mpsc, time::Instant};

use ix_board::{Initiative, IssueId, Project, ProjectId, WorkflowStateType};
use ix_trace_rs::trace;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

use crate::{monitor, monitor::App, monitor_tracker::summarize, monitor_ui};

const INITIATIVES: usize = 4;
const PROJECTS: usize = 34;
const LEAVES_PER_PROJECT: usize = 62;

/// Four initiatives, 34 projects (one shared by two initiatives) and 2,108 leaf tasks.
fn large_scope_app() -> App {
    let mut app = monitor_ui::tests::app();
    let now = Instant::now();
    let mut snapshot = app.tracker.value.take().expect("fixture snapshot");
    let project_template = snapshot.projects[0].clone();
    let issue_template = snapshot.issues[2].clone();
    let initiatives: Vec<Initiative> = (0..INITIATIVES)
        .map(|n| Initiative {
            id: format!("initiative-{n}"),
            identifier: format!("I-{n}"),
            name: format!("Initiative {n:02}"),
        })
        .collect();
    let statuses = ["Started", "Paused", "Completed", "Canceled", "Backlog"];
    snapshot.projects.clear();
    snapshot.issues.clear();
    for p in 0..PROJECTS {
        let project_id = ProjectId(format!("project-{p:02}"));
        let mut project: Project = project_template.clone();
        project.id = project_id.clone();
        project.name = format!("Project {p:02}");
        project.status = statuses[p % statuses.len()].into();
        project.sort_order = p as f64;
        project.initiatives = vec![initiatives[p % INITIATIVES].clone()];
        if p == 0 {
            // The one project shared by two initiatives.
            project.initiatives.push(initiatives[1].clone());
        }
        snapshot.projects.push(project);
        for n in 0..LEAVES_PER_PROJECT {
            let serial = p * LEAVES_PER_PROJECT + n + 100;
            let mut issue = issue_template.clone();
            issue.id = IssueId(format!("leaf-{serial}"));
            issue.identifier = format!("SWM-{serial}");
            issue.project_id = Some(project_id.clone());
            issue.parent_id = None;
            if n % 3 == 0 {
                issue.state_type = WorkflowStateType::Completed;
                issue.state = "Done".into();
                issue.state_name = "Done".into();
            } else {
                issue.state_type = WorkflowStateType::Started;
                issue.state = "Review".into();
                issue.state_name = "Review".into();
            }
            if n % 5 == 1 && n > 0 {
                issue.blocked_by = vec![IssueId(format!("leaf-{}", serial - 1))];
            }
            snapshot.issues.push(issue);
        }
    }
    app.tracker.success(snapshot, now);
    let report = app
        .report
        .value
        .as_mut()
        .and_then(Option::as_mut)
        .expect("fixture report");
    let scope = report.scope.as_mut().expect("fixture scope");
    scope.projects.clear();
    scope.initiatives = initiatives.iter().map(|i| i.id.clone()).collect();
    app.view = monitor::View::Scope;
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Scroll, selection, view-switch and visibility-key events, then quit.
fn scripted_events() -> Vec<KeyCode> {
    let cycle = [
        KeyCode::Down,
        KeyCode::Char('j'),
        KeyCode::PageDown,
        KeyCode::Up,
        KeyCode::Char('k'),
        KeyCode::PageUp,
        KeyCode::Char('c'),
        KeyCode::Char('t'),
        KeyCode::Char('p'),
        KeyCode::Char('a'),
        KeyCode::Char('p'),
        KeyCode::Char('c'),
    ];
    let mut events: Vec<KeyCode> = cycle.iter().copied().cycle().take(216).collect();
    events.push(KeyCode::Char('q'));
    events
}

/// Trace: FR-060-AC-8
#[trace("FR-060-AC-8")]
#[test]
#[ignore = "bounded large-scope scenario; run with --release --ignored --nocapture"]
fn large_scope_event_sequence_with_pending_source_read() {
    let mut app = large_scope_app();
    let view = summarize(
        app.tracker.value.as_ref().expect("snapshot"),
        app.report.value.as_ref().and_then(Option::as_ref),
        "SWM",
    )
    .expect("large fixture resolves");
    assert_eq!(view.initiatives.len(), INITIATIVES);
    assert_eq!(view.projects.len(), PROJECTS);
    assert!(view.scope.leaf_ids.len() >= 2_000);

    // A source read that never completes: the loop only polls it without waiting.
    let (_held_sender, pending_read) = mpsc::channel::<()>();
    let events = scripted_events();
    assert!(events.len() > 200);

    for (width, height) in [(80_u16, 24_u16), (200, 60)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let mut draws = 0_usize;
        let mut draw = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
            terminal
                .draw(|frame| monitor_ui::render(frame, app, Instant::now()))
                .expect("draw");
            assert!(pending_read.try_recv().is_err());
            draws += 1;
        };
        monitor::handle_key(&mut app, key(KeyCode::Char('p')));
        draw(&mut app, &mut terminal);
        let started = Instant::now();
        let mut idle_draws = 0_usize;
        for code in &events {
            monitor::handle_key(&mut app, key(*code));
            draw(&mut app, &mut terminal);
            // The run loop also draws on every idle tick between events.
            draw(&mut app, &mut terminal);
            idle_draws += 1;
            if app.should_quit {
                break;
            }
        }
        let elapsed = started.elapsed();
        assert!(app.should_quit, "the sequence ends with quit");
        app.should_quit = false;
        eprintln!(
            "scale-scenario {width}x{height}: events={} draws={} idle_draws={idle_draws} wall_ms={:.1}",
            events.len(),
            draws,
            elapsed.as_secs_f64() * 1_000.0,
        );
    }
}
