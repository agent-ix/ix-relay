// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Entry point for the read-only team-leader dashboard (SWM-21).

mod monitor;
mod monitor_ops;
mod monitor_runtime;
mod monitor_tracker;
mod monitor_ui;
mod monitor_views;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "team-leader-dashboard",
    about = "Monitor one explicitly assigned lead session"
)]
struct Args {
    /// Lead Herdr address, e.g. hub:TEAM:code.
    #[arg(long)]
    lead: String,
    /// Linear team key; defaults to the bound lead workspace label.
    #[arg(long)]
    team: Option<String>,
    /// Native session expected at that address.
    #[arg(long)]
    session: Option<relay_core::SessionId>,
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
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

use crate::monitor::App;

const TICK: Duration = Duration::from_millis(100);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("team-leader-dashboard: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let args = Args::parse();
    let settings = relay_core::config::load_core(Default::default()).map_err(io::Error::other)?;
    let observed = relay_ops::work::resolve_work_target(
        &herdr_cli::RealProcessRunner,
        Some(&args.lead),
        &settings.machine.value,
    )
    .map_err(io::Error::other)?;
    if args
        .session
        .is_some_and(|id| id != observed.binding.session)
    {
        return Err(io::Error::other(
            "target session changed; explicitly rebind",
        ));
    }
    let mut runtime_worker = Some(monitor_runtime::RuntimeWorker::spawn(
        observed.binding.clone(),
        settings,
    )?);
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    install_panic_hook();
    let _guard = TerminalGuard;
    let signal = install_signal_handlers()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let mut tracker_worker = Some(monitor_ops::TrackerWorker::spawn()?);
    let mut app = App {
        team: args.team.map(|team| team.to_ascii_uppercase()),
        lead_name: args.lead,
        binding: Some(observed.binding.clone()),
        ..App::default()
    };
    app.runtime.success(observed, Instant::now());
    loop {
        terminal.draw(|frame| monitor_ui::render(frame, &mut app, Instant::now()))?;
        if let Some(active_worker) = &tracker_worker {
            while let Ok(refresh) = active_worker.events.try_recv() {
                app.apply_tracker_refresh(refresh, Instant::now());
            }
        }
        if let Some(worker) = &runtime_worker {
            while let Ok(refresh) = worker.events.try_recv() {
                match refresh {
                    monitor_runtime::RuntimeRefresh::Observation(result, read_at) => match result {
                        Ok(observation) if !app.session_ended => {
                            app.runtime.success(observation, read_at)
                        }
                        Ok(_) => {}
                        Err(error) => {
                            if matches!(
                                error,
                                relay_ops::OpsError::WorkSessionReplaced
                                    | relay_ops::OpsError::WorkAgentAbsent
                            ) {
                                app.session_ended = true;
                            }
                            app.runtime.failure(error.to_string());
                        }
                    },
                    monitor_runtime::RuntimeRefresh::Report(result, read_at) => match result {
                        Ok(report) => app.report.success(report, read_at),
                        Err(error) => app.report.failure(error.to_string()),
                    },
                }
            }
        }
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            monitor::handle_key(&mut app, key);
            if app.refresh_now {
                if let Some(active_worker) = &tracker_worker {
                    active_worker.refresh_now();
                }
                app.refresh_now = false;
            }
        }
        if signal.load(Ordering::Relaxed) {
            app.should_quit = true;
        }
        if app.should_quit {
            if let Some(worker) = runtime_worker.take() {
                worker.stop();
            }
            if let Some(active_worker) = tracker_worker.take() {
                active_worker.stop();
            }
            return Ok(());
        }
    }
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

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}
