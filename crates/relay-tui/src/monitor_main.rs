// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Entry point for the read-only team-leader dashboard (SWM-21).

mod monitor;
mod monitor_department;
mod monitor_department_source;
mod monitor_nav;
mod monitor_ops;
mod monitor_runtime;
#[cfg(test)]
mod monitor_scale_tests;
mod monitor_scope;
#[cfg(test)]
mod monitor_scope_tests;
mod monitor_tracker;
mod monitor_ui;
mod monitor_views;

use clap::Parser;
use ix_cli_kit::{
    exit::Outcome,
    streams::{ColorChoice, DiagnosticFields, Record},
};

#[derive(Parser)]
#[command(
    name = "team-leader-dashboard",
    about = "Monitor an explicit department scope or lead session"
)]
struct Args {
    /// Lead Herdr address, e.g. hub:TEAM:code.
    #[arg(long)]
    lead: Option<String>,
    /// Linear team key; defaults to the bound lead workspace label.
    #[arg(long)]
    team: Option<String>,
    /// Source-native project ID. Repeat to select several projects.
    #[arg(long)]
    project: Vec<String>,
    /// Saved ix-board retro snapshot JSON, snapshots JSONL or delta JSON, with a department envelope.
    #[arg(long, conflicts_with_all = ["team", "project", "lead", "session", "retro_config"])]
    report: Option<std::path::PathBuf>,
    /// Existing ix-board retro configuration JSON containing department policies and sources.
    #[arg(long)]
    retro_config: Option<std::path::PathBuf>,
    /// Native session expected at that address.
    #[arg(long, requires = "lead")]
    session: Option<relay_core::SessionId>,
}

impl Args {
    fn explicit_scope(&self) -> bool {
        self.team.is_some() || !self.project.is_empty()
    }

    fn validate(&self) -> io::Result<()> {
        if self.report.is_some()
            && (self.explicit_scope()
                || self.lead.is_some()
                || self.session.is_some()
                || self.retro_config.is_some())
        {
            return Err(io::Error::other(
                "--report conflicts with live scope, lead/session and --retro-config",
            ));
        }
        if !self.explicit_scope() && self.report.is_none() && self.lead.is_none() {
            return Err(io::Error::other(
                "select --team, --project, --report or --lead; see --help",
            ));
        }
        if self.team.as_ref().is_some_and(|v| v.trim().is_empty())
            || self.project.iter().any(|v| v.trim().is_empty())
            || self.lead.as_ref().is_some_and(|v| v.trim().is_empty())
        {
            return Err(io::Error::other(
                "scope and lead selectors must not be empty",
            ));
        }
        Ok(())
    }
}

use std::{
    io,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{self, Event, KeyEvent, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

use crate::monitor::App;

const TICK: Duration = Duration::from_millis(100);

fn main() -> ExitCode {
    match run() {
        Ok((outcome, diagnostic)) => {
            if let Some(diagnostic) = diagnostic {
                eprintln!("{diagnostic}");
            }
            outcome.into()
        }
        Err(error) => {
            let message = format!("team-leader-dashboard: {error}");
            let record =
                Record::error("DashboardStartup", &message).with_fields(DiagnosticFields {
                    remedy: Some("Check --help and the selected report or source access"),
                    ..DiagnosticFields::default()
                });
            eprintln!(
                "{}",
                monitor::display_text(&record.render_human(false), 4096)
            );
            Outcome::Invalid.into()
        }
    }
}

fn run() -> io::Result<(Outcome, Option<String>)> {
    let args = Args::parse();
    args.validate()?;
    let department_config = args
        .retro_config
        .as_deref()
        .map(|path| {
            monitor_department_source::load_config(path, args.team.as_deref(), &args.project)
        })
        .transpose()?;
    let (mut app, observed_binding) = startup(
        &args,
        department_config.is_some(),
        monitor_department_source::read_report,
        bind_lead,
    )?;
    app.color_enabled = ColorChoice::Auto.decide(
        ix_cli_kit::streams::stdout_is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    );
    let department_worker = department_config
        .map(monitor_department_source::DepartmentWorker::spawn)
        .transpose()?;
    let mut runtime_worker = None;
    if let Some((observed, settings)) = observed_binding {
        match monitor_runtime::RuntimeWorker::spawn(observed.binding.clone(), settings) {
            Ok(worker) => runtime_worker = Some(worker),
            Err(error) => app.runtime.failure(error.to_string()),
        }
        app.runtime.success(observed, Instant::now());
    }
    let tracker_worker = start_tracker(&args, &mut app, monitor_ops::TrackerWorker::spawn);
    // The guard exists before the first terminal mutation, including partial initialization.
    let _guard = TerminalGuard::enter(restore_terminal, || {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)
    })?;
    install_panic_hook();
    let signal = install_signal_handlers()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let mut workers = Workers {
        tracker: tracker_worker,
        department: department_worker,
        runtime: runtime_worker,
    };
    event_loop(
        &mut terminal,
        &mut app,
        &mut workers,
        &mut TerminalKeys,
        &signal,
        args.report.as_deref(),
    )?;
    Ok(quit_outcome(&app))
}

/// Background source workers the event loop drains and stops.
struct Workers {
    tracker: Option<monitor_ops::TrackerWorker>,
    department: Option<monitor_department_source::DepartmentWorker>,
    runtime: Option<monitor_runtime::RuntimeWorker>,
}

/// Where the event loop waits for key presses between draws.
trait KeySource {
    /// Waits at most `wait` for one key press.
    fn next_key(&mut self, wait: Duration) -> io::Result<Option<KeyEvent>>;
}

/// The real terminal's input queue.
struct TerminalKeys;

impl KeySource for TerminalKeys {
    fn next_key(&mut self, wait: Duration) -> io::Result<Option<KeyEvent>> {
        if event::poll(wait)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            return Ok(Some(key));
        }
        Ok(None)
    }
}

/// Draws, drains already-completed source results without waiting, handles one key and
/// quits. The dashboard and its tests both run exactly this loop.
fn event_loop<B, K>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    workers: &mut Workers,
    keys: &mut K,
    signal: &AtomicBool,
    report: Option<&std::path::Path>,
) -> io::Result<()>
where
    B: Backend,
    B::Error: std::error::Error + Send + Sync + 'static,
    K: KeySource,
{
    loop {
        terminal
            .draw(|frame| monitor_ui::render(frame, app, Instant::now()))
            .map_err(io::Error::other)?;
        if let Some(active_worker) = &workers.tracker {
            while let Ok(refresh) = active_worker.events.try_recv() {
                app.apply_tracker_refresh(refresh, Instant::now());
            }
        }
        if let Some(worker) = &workers.department {
            while let Ok(result) = worker.events.try_recv() {
                app.apply_department_refresh(result, Instant::now());
            }
        }
        if let Some(worker) = &workers.runtime {
            while let Ok(refresh) = worker.events.try_recv() {
                app.apply_runtime_refresh(refresh);
            }
        }
        if let Some(key) = keys.next_key(TICK)? {
            monitor::handle_key(app, key);
            if app.refresh_now {
                if let Some(active_worker) = &workers.tracker {
                    active_worker.refresh_now();
                }
                if let Some(worker) = &workers.department {
                    worker.refresh_now();
                }
                if let Some(path) = report {
                    match monitor_department_source::read_report(path) {
                        Ok(report) => app.department.success(report, Instant::now()),
                        Err(error) => app.department.failure(error.to_string()),
                    }
                }
                app.refresh_now = false;
            }
        }
        if signal.load(Ordering::Relaxed) {
            app.should_quit = true;
        }
        if app.should_quit {
            if let Some(worker) = workers.runtime.take() {
                worker.stop();
            }
            if let Some(active_worker) = workers.tracker.take() {
                active_worker.stop();
            }
            return Ok(());
        }
    }
}

fn quit_outcome(app: &App) -> (Outcome, Option<String>) {
    if app.collection_failed {
        let diagnostic = [&app.tracker.error, &app.department.error]
            .into_iter()
            .flatten()
            .map(|error| monitor::display_text(error, 4096))
            .collect::<Vec<_>>()
            .join(" · ");
        (
            Outcome::Partial,
            Some(if diagnostic.is_empty() {
                "Requested live collection failed".into()
            } else {
                diagnostic
            }),
        )
    } else {
        (Outcome::Ok, None)
    }
}

type ObservedBinding = (
    relay_ops::work::WorkObservation,
    relay_core::config::CoreSettings,
);

fn start_tracker<T>(
    args: &Args,
    app: &mut App,
    spawn: impl FnOnce() -> io::Result<T>,
) -> Option<T> {
    if args.report.is_some() {
        return None;
    }
    match spawn() {
        Ok(worker) => Some(worker),
        Err(error) => {
            app.tracker.failure(error.to_string());
            app.collection_failed = true;
            None
        }
    }
}

fn prepare_app(
    args: &Args,
    read_report: impl FnOnce(&std::path::Path) -> io::Result<ix_board::retro::department::Delta>,
    bind: impl FnOnce(&str, Option<relay_core::SessionId>) -> io::Result<ObservedBinding>,
) -> io::Result<(App, Option<ObservedBinding>)> {
    args.validate()?;
    let mut app = App {
        team: args.team.as_ref().map(|team| team.to_ascii_uppercase()),
        projects: args.project.clone(),
        lead_name: args
            .lead
            .clone()
            .unwrap_or_else(|| "identity unknown".into()),
        view: if args.explicit_scope() || args.report.is_some() {
            monitor::View::Status
        } else {
            monitor::View::Tickets
        },
        department_input: department_input(args, false),
        ..App::default()
    };
    if let Some(path) = &args.report {
        app.department.success(read_report(path)?, Instant::now());
        return Ok((app, None));
    }
    let observed = if let Some(lead) = &args.lead {
        match bind(lead, args.session.clone()) {
            Ok(binding) => {
                app.binding = Some(binding.0.binding.clone());
                Some(binding)
            }
            Err(error) if args.explicit_scope() => {
                app.runtime.failure(error.to_string());
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    Ok((app, observed))
}

/// Builds the dashboard state exactly as a real start does. A missing retro configuration
/// is a state of the Health and Retro views, never a stored source failure.
fn startup(
    args: &Args,
    retro_config_supplied: bool,
    read_report: impl FnOnce(&std::path::Path) -> io::Result<ix_board::retro::department::Delta>,
    bind: impl FnOnce(&str, Option<relay_core::SessionId>) -> io::Result<ObservedBinding>,
) -> io::Result<(App, Option<ObservedBinding>)> {
    let (mut app, observed) = prepare_app(args, read_report, bind)?;
    app.department_input = department_input(args, retro_config_supplied);
    Ok((app, observed))
}

/// How the department report is supplied, from the startup options.
fn department_input(args: &Args, configured: bool) -> monitor::DepartmentInput {
    if args.report.is_some() {
        monitor::DepartmentInput::Saved
    } else if configured {
        monitor::DepartmentInput::Configured
    } else {
        monitor::DepartmentInput::NotConfigured
    }
}

fn bind_lead(
    lead: &str,
    session: Option<relay_core::SessionId>,
) -> io::Result<(
    relay_ops::work::WorkObservation,
    relay_core::config::CoreSettings,
)> {
    let settings = relay_core::config::load_core(Default::default()).map_err(io::Error::other)?;
    let observed = relay_ops::work::resolve_work_target(
        &herdr_cli::RealProcessRunner,
        Some(lead),
        &settings.machine.value,
    )
    .map_err(io::Error::other)?;
    if session.is_some_and(|id| id != observed.binding.session) {
        return Err(io::Error::other(
            "target session changed; explicitly rebind",
        ));
    }
    Ok((observed, settings))
}

fn install_signal_handlers() -> io::Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&flag))?;
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&flag))?;
    Ok(flag)
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

struct TerminalGuard<Restore: FnMut()> {
    restore: Restore,
}

impl<Restore: FnMut()> TerminalGuard<Restore> {
    fn enter(restore: Restore, initialize: impl FnOnce() -> io::Result<()>) -> io::Result<Self> {
        let guard = Self { restore };
        initialize()?;
        Ok(guard)
    }
}

impl<Restore: FnMut()> Drop for TerminalGuard<Restore> {
    fn drop(&mut self) {
        (self.restore)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;
    use std::cell::Cell;

    fn args(values: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("dashboard").chain(values.iter().copied())).unwrap()
    }

    /// Trace: FR-059-AC-1, FR-059-AC-2
    #[trace("FR-059-AC-1", "FR-059-AC-2")]
    #[test]
    fn explicit_scopes_and_saved_input_need_no_native_binding_or_live_factory() {
        for values in [vec!["--team", "SWM"], vec!["--project", "native-project"]] {
            let (app, binding) = prepare_app(
                &args(&values),
                |_| panic!("saved reader called in live mode"),
                |_, _| panic!("native binding called without lead"),
            )
            .unwrap();
            assert_eq!(app.view, monitor::View::Status);
            assert!(app.binding.is_none());
            assert!(binding.is_none());
        }
        let offline = args(&["--report", "supplied.json"]);
        let (mut app, binding) = prepare_app(
            &offline,
            |_| Ok(monitor_department::tests::fixture()),
            |_, _| panic!("Herdr/relay setup called in saved mode"),
        )
        .unwrap();
        let tracker: Option<()> = start_tracker(&offline, &mut app, || {
            panic!("credential/network factory called in saved mode")
        });
        assert!(tracker.is_none());
        assert!(binding.is_none());
        for code in ['1', '2', '3', 'q'] {
            monitor::handle_key(
                &mut app,
                ratatui::crossterm::event::KeyEvent::new(
                    ratatui::crossterm::event::KeyCode::Char(code),
                    ratatui::crossterm::event::KeyModifiers::NONE,
                ),
            );
        }
        assert!(app.should_quit);
        assert!(!app.collection_failed);
        assert_eq!(
            app.department.value.unwrap().current.scope.teams[0].id,
            "SWM"
        );
    }

    /// Trace: FR-059-AC-1
    #[trace("FR-059-AC-1")]
    #[test]
    fn missing_and_ambiguous_requested_leads_keep_explicit_scope_but_refuse_legacy_startup() {
        for reason in ["native agent absent", "ambiguous native process"] {
            let scoped = args(&["--lead", "host:workspace:code", "--team", "SWM"]);
            let (app, binding) = prepare_app(
                &scoped,
                |_| panic!("saved reader called"),
                |_, _| Err(io::Error::other(reason)),
            )
            .unwrap();
            assert_eq!(app.view, monitor::View::Status);
            assert_eq!(app.runtime.error.as_deref(), Some(reason));
            assert!(app.binding.is_none());
            assert!(binding.is_none());
            let legacy = args(&["--lead", "host:workspace:code"]);
            assert!(
                prepare_app(
                    &legacy,
                    |_| panic!("saved reader called"),
                    |_, _| Err(io::Error::other(reason))
                )
                .is_err()
            );
        }
    }

    /// Trace: FR-059-AC-2
    #[trace("FR-059-AC-2")]
    #[test]
    fn invalid_options_refuse_before_source_access() {
        let absent = args(&[]);
        assert!(
            prepare_app(
                &absent,
                |_| panic!("saved access"),
                |_, _| panic!("binding access")
            )
            .is_err()
        );
        for flag in ["--team", "--project", "--lead", "--retro-config"] {
            assert!(
                Args::try_parse_from(["dashboard", "--report", "file", flag, "value"]).is_err()
            );
        }
        assert!(Args::try_parse_from(["dashboard", "--session", "invalid"]).is_err());
    }

    /// Trace: FR-059-AC-6
    #[trace("FR-059-AC-6")]
    #[test]
    fn terminal_guard_restores_after_quit_initialization_and_render_failures() {
        let restored = Cell::new(0);
        {
            let _guard =
                TerminalGuard::enter(|| restored.set(restored.get() + 1), || Ok(())).unwrap();
        }
        assert_eq!(restored.get(), 1);
        let failed = TerminalGuard::enter(
            || restored.set(restored.get() + 1),
            || Err(io::Error::other("alternate screen init failed")),
        );
        assert!(failed.is_err());
        assert_eq!(restored.get(), 2);
        let render = || -> io::Result<()> {
            let _guard = TerminalGuard::enter(|| restored.set(restored.get() + 1), || Ok(()))?;
            Err(io::Error::other("render failed"))
        };
        assert!(render().is_err());
        assert_eq!(restored.get(), 3);
    }

    use crate::monitor_ui::tests::{flat, frame_text};
    use ratatui::{backend::TestBackend, crossterm::event::KeyCode};

    /// Scripted key presses; `None` is an idle tick that lets the loop drain workers. An
    /// optional hook runs once, when the loop asks for key number `hook_at`.
    struct Script {
        keys: std::vec::IntoIter<Option<KeyCode>>,
        calls: usize,
        hook_at: usize,
        hook: Option<Box<dyn FnOnce()>>,
    }

    impl Script {
        fn new(keys: Vec<Option<KeyCode>>) -> Self {
            Self {
                keys: keys.into_iter(),
                calls: 0,
                hook_at: usize::MAX,
                hook: None,
            }
        }

        fn with_hook(mut self, hook_at: usize, hook: impl FnOnce() + 'static) -> Self {
            self.hook_at = hook_at;
            self.hook = Some(Box::new(hook));
            self
        }
    }

    impl KeySource for Script {
        fn next_key(&mut self, _wait: Duration) -> io::Result<Option<KeyEvent>> {
            if self.calls == self.hook_at
                && let Some(hook) = self.hook.take()
            {
                hook();
            }
            self.calls += 1;
            Ok(self
                .keys
                .next()
                .flatten()
                .map(|code| KeyEvent::new(code, ratatui::crossterm::event::KeyModifiers::NONE)))
        }
    }

    /// Opens and closes both popups and presses keys they do not own (including `q` and
    /// `r`), then idles and quits. The last four entries are three idle ticks and `q`.
    fn popup_keys() -> Vec<Option<KeyCode>> {
        use KeyCode::{Char, Down, Esc, F, PageDown};
        vec![
            Some(Char('2')),
            Some(Char('v')),
            Some(Char('q')),
            Some(Char('r')),
            Some(Down),
            Some(Esc),
            Some(F(1)),
            Some(Char('q')),
            Some(Char('r')),
            Some(PageDown),
            Some(Char('?')),
            Some(Char('3')),
            Some(Char('v')),
            Some(Esc),
            Some(F(1)),
            Some(Esc),
            None,
            None,
            None,
            Some(Char('q')),
        ]
    }

    fn files(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut found: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect();
        found.sort();
        found
    }

    /// Writes a saved snapshot whose department section lacks the Health and Retro fields.
    fn write_saved_report(path: &std::path::Path) {
        let mut department = monitor_department::tests::fixture().current;
        department.classes.clear();
        department.measurements.clear();
        department.loops.clear();
        let record = ix_board::retro::Record {
            schema_version: 2,
            kind: ix_board::retro::RecordKind::Snapshot,
            taken_at: "2026-11-19T00:00:00Z".into(),
            team: "SWM".into(),
            window: "24h".into(),
            metrics: Default::default(),
            department: Some(department),
        };
        std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    }

    #[trace("FR-065-AC-10")]
    #[test]
    fn saved_report_popups_never_reach_the_loops_reader_or_any_adapter() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.json");
        write_saved_report(&path);
        let before = files(directory.path());
        let opened = args(&["--report", path.to_str().unwrap()]);
        let run = |keys: Vec<Option<KeyCode>>| {
            // The loop's refresh path calls the real saved reader, so the department
            // revision counts every read. Herdr, credential and network adapters are
            // unreachable from the loop by construction: it is given no worker at all.
            let (mut app, binding) = startup(
                &opened,
                false,
                monitor_department_source::read_report,
                |_, _| panic!("Herdr or relay setup called in saved mode"),
            )
            .unwrap();
            let tracker: Option<()> = start_tracker(&opened, &mut app, || {
                panic!("credential or network factory called in saved mode")
            });
            assert!(binding.is_none() && tracker.is_none());
            assert_eq!(app.department.revision, 1, "the one startup read");
            let mut workers = Workers {
                tracker: None,
                department: None,
                runtime: None,
            };
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            event_loop(
                &mut terminal,
                &mut app,
                &mut workers,
                &mut Script::new(keys),
                &AtomicBool::new(false),
                Some(&path),
            )
            .unwrap();
            (app, flat(&frame_text(&terminal)))
        };
        let (app, frame) = run(popup_keys());
        assert!(app.should_quit && app.popup.is_none());
        assert_eq!(app.view, monitor::View::Retro);
        assert_eq!(app.department.revision, 1, "no saved re-read happened");
        assert!(app.department.error.is_none() && app.tracker.error.is_none());
        assert!(!app.collection_failed);
        assert_eq!(files(directory.path()), before, "no file written");
        assert!(
            frame.contains("Fields absent: this saved report does not supply the Retro fields"),
            "{frame}"
        );
        assert!(!frame.to_lowercase().contains("authentication"), "{frame}");
        let (outcome, diagnostic) = quit_outcome(&app);
        assert!(matches!(outcome, Outcome::Ok) && diagnostic.is_none());
        // Control: the same loop does re-read when `r` is pressed with no popup open.
        let (control, _) = run(vec![Some(KeyCode::Char('r')), Some(KeyCode::Char('q'))]);
        assert_eq!(control.department.revision, 2);
        assert_eq!(files(directory.path()), before);
    }

    /// What one live run of the production loop left behind.
    struct LiveRun {
        app: App,
        tracker_calls: usize,
        department_calls: usize,
        frame: String,
        before: Vec<(String, Vec<u8>)>,
        after: Vec<(String, Vec<u8>)>,
    }

    /// Runs the real startup and loop with a tracker worker and, when `configured`, a
    /// department worker, both on injected reads that block until released. `released`
    /// lets them finish when the loop asks for key `hook_at`, then waits long enough for a
    /// refresh queued earlier to start a second read.
    fn run_live(configured: bool, released: Option<usize>, keys: Vec<Option<KeyCode>>) -> LiveRun {
        use std::sync::{atomic::AtomicUsize, mpsc};
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("retro-config.json"), b"{}").unwrap();
        let before = files(directory.path());
        let live = args(&["--team", "SWM"]);
        let (mut app, binding) = startup(
            &live,
            configured,
            |_| panic!("saved reader called in live mode"),
            |_, _| panic!("native binding called without a lead"),
        )
        .unwrap();
        assert!(binding.is_none());
        let snapshot = crate::monitor_ui::tests::app()
            .tracker
            .value
            .clone()
            .unwrap();
        let tracker_calls = Arc::new(AtomicUsize::new(0));
        let department_calls = Arc::new(AtomicUsize::new(0));
        let (tracker_entered, tracker_entry) = mpsc::channel();
        let (tracker_done, tracker_finished) = mpsc::channel();
        let (tracker_release, tracker_held) = mpsc::channel::<()>();
        let (department_entered, department_entry) = mpsc::channel();
        let (department_done, department_finished) = mpsc::channel();
        let (department_release, department_held) = mpsc::channel::<()>();
        let calls = Arc::clone(&tracker_calls);
        let tracker = start_tracker(&live, &mut app, move || {
            monitor_ops::TrackerWorker::spawn_with_read(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                let _ = tracker_entered.send(());
                let _ = tracker_held.recv();
                let _ = tracker_done.send(());
                Ok(monitor_ops::TrackerRead {
                    snapshot: snapshot.clone(),
                    freshness: None,
                })
            })
        })
        .expect("tracker worker");
        tracker_entry
            .recv_timeout(Duration::from_secs(5))
            .expect("the tracker read is held in flight");
        let department = configured.then(|| {
            let calls = Arc::clone(&department_calls);
            let worker = monitor_department_source::DepartmentWorker::spawn_with_read(move || {
                calls.fetch_add(1, Ordering::SeqCst);
                let _ = department_entered.send(());
                let _ = department_held.recv();
                let _ = department_done.send(());
                Ok(monitor_department::tests::fixture())
            })
            .unwrap();
            department_entry
                .recv_timeout(Duration::from_secs(5))
                .expect("the department read is held in flight");
            worker
        });
        let mut workers = Workers {
            tracker: Some(tracker),
            department,
            runtime: None,
        };
        let mut script = Script::new(keys);
        if let Some(hook_at) = released {
            let (tracker_go, department_go) = (tracker_release.clone(), department_release.clone());
            script = script.with_hook(hook_at, move || {
                let _ = tracker_go.send(());
                tracker_finished
                    .recv_timeout(Duration::from_secs(5))
                    .expect("the released tracker read finishes");
                if configured {
                    let _ = department_go.send(());
                    department_finished
                        .recv_timeout(Duration::from_secs(5))
                        .expect("the released department read finishes");
                }
                std::thread::sleep(Duration::from_millis(300));
            });
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        event_loop(
            &mut terminal,
            &mut app,
            &mut workers,
            &mut script,
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        let run = LiveRun {
            tracker_calls: tracker_calls.load(Ordering::SeqCst),
            department_calls: department_calls.load(Ordering::SeqCst),
            frame: flat(&frame_text(&terminal)),
            after: files(directory.path()),
            before,
            app,
        };
        // Let any read still held finish; nothing was waiting on it.
        drop((tracker_release, department_release));
        run
    }

    #[trace("FR-065-AC-10")]
    #[test]
    fn live_popups_start_no_read_whether_the_reads_are_held_or_finish_during_the_script() {
        for configured in [false, true] {
            let keys = popup_keys();
            let hook_at = keys.len() - 4;
            // Held for the whole script: the loop never waits for the pending reads.
            let held = run_live(configured, None, keys);
            assert!(held.app.should_quit && held.app.popup.is_none());
            assert_eq!(held.tracker_calls, 1);
            assert_eq!(held.department_calls, usize::from(configured));
            assert!(held.app.tracker.value.is_none() && held.app.tracker.error.is_none());
            assert!(held.app.department.value.is_none() && held.app.department.error.is_none());
            assert_eq!(held.before, held.after, "no file or history written");
            assert!(!held.frame.to_lowercase().contains("authentication"));
            assert!(!held.frame.contains("Department failed"));
            if configured {
                assert!(
                    held.frame
                        .contains("Waiting for the first read of the retro configuration"),
                    "{}",
                    held.frame
                );
            } else {
                assert!(
                    held.frame.contains("--retro-config <file>"),
                    "{}",
                    held.frame
                );
            }
            // Released before the end: a refresh that leaked from a popup would have been
            // queued on the workers and would now start a second read.
            let done = run_live(configured, Some(hook_at), popup_keys());
            assert_eq!(done.tracker_calls, 1, "configured={configured}");
            assert_eq!(done.department_calls, usize::from(configured));
            assert_eq!(done.app.tracker.revision, 1);
            assert!(done.app.tracker.value.is_some() && done.app.tracker.error.is_none());
            assert_eq!(done.app.department.revision, u64::from(configured));
            assert_eq!(done.app.department.value.is_some(), configured);
            assert!(!done.app.collection_failed && !done.app.refresh_now);
            assert_eq!(done.before, done.after, "no file or history written");
            assert!(!done.frame.to_lowercase().contains("authentication"));
            assert!(matches!(quit_outcome(&done.app).0, Outcome::Ok));
        }
    }

    #[trace("FR-065-AC-10")]
    #[test]
    fn a_refresh_key_outside_any_popup_does_start_a_second_read_in_the_same_harness() {
        // Control for the test above: `r` with no popup open queues a refresh, and the
        // released reads then run again. If this ever stops holding, the oracle is blind.
        use KeyCode::Char;
        let keys = vec![
            Some(Char('2')),
            Some(Char('r')),
            None,
            None,
            None,
            Some(Char('q')),
        ];
        let run = run_live(true, Some(2), keys);
        assert_eq!(run.tracker_calls, 2);
        assert_eq!(run.department_calls, 2);
    }

    #[trace("FR-065-AC-10")]
    #[test]
    fn startup_maps_the_options_to_how_the_department_report_is_supplied() {
        use monitor::DepartmentInput::{Configured, NotConfigured, Saved};
        let unreachable_read = |_: &std::path::Path| -> io::Result<_> { panic!("no read") };
        let live = args(&["--team", "SWM"]);
        for (configured, expected) in [(false, NotConfigured), (true, Configured)] {
            let (app, _) = startup(&live, configured, unreachable_read, |_, _| panic!()).unwrap();
            assert_eq!(app.department_input, expected);
            assert!(app.department.error.is_none(), "never a stored failure");
        }
        let saved = args(&["--report", "supplied.json"]);
        let (app, _) = startup(
            &saved,
            false,
            |_| Ok(monitor_department::tests::fixture()),
            |_, _| panic!(),
        )
        .unwrap();
        assert_eq!(app.department_input, Saved);
    }
}
