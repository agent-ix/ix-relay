// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Bounded large-scope scenario that drives the real dashboard render, key handler and
//! tracker worker.
//!
//! The scenario is ignored by default so it only runs when an optimized build is
//! requested explicitly. It asserts no wall-time limit; it prints the timings so the
//! same fixture can be recorded before and after a change. The counter assertions are
//! the FR-060-AC-8 contract.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use ix_board::{Initiative, IssueId, Project, ProjectId, ProjectStatusCategory, WorkflowStateType};
use ix_trace_rs::trace;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

use crate::{
    monitor::{self, App, View},
    monitor_tracker::{TrackerState, summarize},
    monitor_ui,
};

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
    let statuses = [
        ("Started", ProjectStatusCategory::Started),
        ("Paused", ProjectStatusCategory::Paused),
        ("Completed", ProjectStatusCategory::Completed),
        ("Canceled", ProjectStatusCategory::Canceled),
        ("Backlog", ProjectStatusCategory::Backlog),
    ];
    snapshot.projects.clear();
    snapshot.issues.clear();
    for p in 0..PROJECTS {
        let project_id = ProjectId(format!("project-{p:02}"));
        let mut project: Project = project_template.clone();
        project.id = project_id.clone();
        project.name = format!("Project {p:02}");
        let (status, category) = statuses[p % statuses.len()];
        project.status = status.into();
        project.status_category = category;
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
    app.view = View::Scope;
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

fn counters(app: &App) -> (usize, usize) {
    (app.scope.summary_computations, app.scope.projection_builds)
}

fn draw(app: &mut App, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| monitor_ui::render(frame, app, Instant::now()))
        .expect("draw");
}

/// Trace: FR-060-AC-8
#[trace("FR-060-AC-8")]
#[test]
#[ignore = "bounded large-scope scenario; run with --release --ignored --nocapture"]
fn large_scope_event_sequence_with_pending_source_read() {
    let mut app = large_scope_app();
    let snapshot = app.tracker.value.clone().expect("snapshot");
    let view = summarize(
        &snapshot,
        app.report.value.as_ref().and_then(Option::as_ref),
        "SWM",
    )
    .expect("large fixture resolves");
    assert_eq!(view.initiatives.len(), INITIATIVES);
    assert_eq!(view.projects.len(), PROJECTS);
    assert!(view.scope.leaf_ids.len() >= 2_000);

    drop(app);

    let events = scripted_events();
    assert!(events.len() > 200);
    for (width, height) in [(80_u16, 24_u16), (200, 60)] {
        // A fresh app per size, so the cold summary and projection build are timed too.
        let mut app = large_scope_app();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let started = Instant::now();
        monitor::handle_key(&mut app, key(KeyCode::Char('p')));
        draw(&mut app, &mut terminal);
        let cold = started.elapsed();
        assert_eq!(counters(&app), (1, 1), "the cold draw builds once");
        let mut draws = 1_usize;
        for code in &events {
            let before = counters(&app);
            let in_scope = app.view == View::Scope;
            monitor::handle_key(&mut app, key(*code));
            // The run loop draws after each event and again on the idle tick.
            draw(&mut app, &mut terminal);
            draw(&mut app, &mut terminal);
            draws += 2;
            let after = counters(&app);
            let toggled = in_scope && *code == KeyCode::Char('c');
            assert_eq!(
                (after.0 - before.0, after.1 - before.1),
                (0, usize::from(toggled)),
                "{code:?}"
            );
            if app.should_quit {
                break;
            }
        }
        let total = started.elapsed();
        assert!(app.should_quit, "the sequence ends with quit");
        eprintln!(
            "scale-scenario {width}x{height}: events={} draws={draws} cold_ms={:.1} steady_ms={:.1} total_ms={:.1}",
            events.len(),
            cold.as_secs_f64() * 1_000.0,
            total.saturating_sub(cold).as_secs_f64() * 1_000.0,
            total.as_secs_f64() * 1_000.0,
        );
    }
}

/// Releases a held source read when dropped, and when the watchdog fires.
struct HeldRead {
    release: mpsc::Sender<()>,
    fired: Arc<AtomicBool>,
    cancel: Option<mpsc::Sender<()>>,
}

impl HeldRead {
    /// The watchdog only bounds a hang: it releases the read after `limit`, which a
    /// non-blocking loop never needs.
    fn watch(release: mpsc::Sender<()>, limit: Duration) -> Self {
        let fired = Arc::new(AtomicBool::new(false));
        let (cancel, cancelled) = mpsc::channel::<()>();
        let (watch_release, watch_fired) = (release.clone(), Arc::clone(&fired));
        thread::spawn(move || {
            if cancelled.recv_timeout(limit).is_err() {
                watch_fired.store(true, Ordering::SeqCst);
                let _ = watch_release.send(());
            }
        });
        Self {
            release,
            fired,
            cancel: Some(cancel),
        }
    }
}

impl Drop for HeldRead {
    fn drop(&mut self) {
        drop(self.cancel.take());
        let _ = self.release.send(());
    }
}

/// Scripted input for the production event loop, with a planned mid-sequence release of
/// the held source read.
struct ScriptedKeys {
    events: Vec<KeyCode>,
    next: usize,
    release_at: usize,
    release: mpsc::Sender<()>,
    read_finished: mpsc::Receiver<()>,
    released: bool,
    idle_after_release: usize,
    fired: Arc<AtomicBool>,
}

impl crate::KeySource for ScriptedKeys {
    fn next_key(&mut self, _wait: Duration) -> std::io::Result<Option<KeyEvent>> {
        if self.next == self.release_at && !self.released {
            // Everything before this point ran while the read was still pending.
            assert!(
                !self.fired.load(Ordering::SeqCst),
                "input waited for the pending source read"
            );
            self.released = true;
            self.release.send(()).expect("release the read");
            self.read_finished
                .recv_timeout(Duration::from_secs(10))
                .expect("the released read finishes");
            thread::sleep(Duration::from_millis(50));
        }
        if self.released && self.idle_after_release < 3 {
            // Idle ticks let the loop drain the delivered result.
            self.idle_after_release += 1;
            return Ok(None);
        }
        let code = self.events.get(self.next).copied();
        self.next += 1;
        Ok(code.map(key))
    }
}

/// Trace: FR-060-AC-8, FR-060-AC-9
#[trace("FR-060-AC-8")]
#[test]
fn production_event_loop_never_waits_for_a_pending_tracker_read() {
    let mut app = large_scope_app();
    let snapshot = app.tracker.value.clone().expect("snapshot");
    // The real tracker worker with an injected read that stays pending until released.
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let refreshed: TrackerState = snapshot;
    let worker = crate::monitor_ops::TrackerWorker::spawn_with_read(move |_| {
        let _ = entered_tx.send(());
        let _ = release_rx.recv();
        let _ = finished_tx.send(());
        Ok(refreshed.clone())
    })
    .expect("worker");
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the read is in flight");
    let held = HeldRead::watch(release_tx.clone(), Duration::from_secs(60));

    let mut events = scripted_events();
    events.truncate(events.len() - 1);
    // A manual refresh while the read is pending must also return immediately.
    events.insert(100, KeyCode::Char('r'));
    let release_at = events.len();
    events.push(KeyCode::Char('c'));
    events.push(KeyCode::Char('q'));
    let toggles = events.iter().filter(|c| **c == KeyCode::Char('c')).count();
    let mut keys = ScriptedKeys {
        events,
        next: 0,
        release_at,
        release: release_tx,
        read_finished: finished_rx,
        released: false,
        idle_after_release: 0,
        fired: Arc::clone(&held.fired),
    };
    let mut workers = crate::Workers {
        tracker: Some(worker),
        department: None,
        runtime: None,
    };
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    let signal = AtomicBool::new(false);
    app.view = View::Scope;
    let before = counters(&app);
    crate::event_loop(&mut terminal, &mut app, &mut workers, &mut keys, &signal, None)
        .expect("the loop quits");
    assert!(keys.released, "the whole script ran before the read finished");
    assert!(
        !held.fired.load(Ordering::SeqCst),
        "the loop waited for the pending read until the watchdog released it"
    );
    assert!(app.should_quit);
    assert!(workers.tracker.is_none(), "quit stops the worker");
    // The cold draw and the one delivered source change compute the summary; every build
    // beyond those is exactly one toggle, each pressed while the Scope view was shown.
    let after = counters(&app);
    assert_eq!(after.0 - before.0, 2, "cold draw plus the delivered refresh");
    assert_eq!(
        after.1 - before.1,
        2 + toggles,
        "a projection build per summary and per toggle"
    );
}
