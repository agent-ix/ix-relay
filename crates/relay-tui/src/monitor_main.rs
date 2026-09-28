// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Herdr pane entry point for the read-only activity monitor (SWM-21).

mod monitor;
mod monitor_ops;
mod monitor_tracker;
mod monitor_ui;

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
    let settings = match relay_core::config::load_core(relay_core::config::CoreFlags::default()) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("herdr-monitor: {error}");
            return ExitCode::FAILURE;
        }
    };
    match run(settings) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("herdr-monitor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(settings: relay_core::config::CoreSettings) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    install_panic_hook();
    let _guard = TerminalGuard;
    let signal = install_signal_handlers()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let pane_id = std::env::var("HERDR_PANE_ID").ok();
    let mut worker = Some(monitor_ops::Worker::spawn(settings, pane_id)?);
    let mut tracker_worker = Some(monitor_ops::TrackerWorker::spawn()?);
    let mut app = App::default();
    loop {
        terminal.draw(|frame| monitor_ui::render(frame, &app, Instant::now()))?;
        if let Some(active_worker) = &worker {
            while let Ok(refresh) = active_worker.events.try_recv() {
                app.apply_refresh(refresh.herdr, refresh.bus, Instant::now());
            }
        }
        if let Some(active_worker) = &tracker_worker {
            while let Ok(refresh) = active_worker.events.try_recv() {
                app.apply_tracker_refresh(refresh, Instant::now());
            }
        }
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            monitor::handle_key(&mut app, key);
            if app.refresh_now {
                if let Some(active_worker) = &worker {
                    active_worker.refresh_now();
                }
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
            if let Some(active_worker) = worker.take() {
                active_worker.stop();
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
