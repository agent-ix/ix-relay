//! `relay-tui`: the operator's escalation inbox. A ratatui app that runs in a herdr
//! pane on node-a, listing escalations from the ix-relay bus and letting them
//! investigate or resolve them without touching `agent-msg` directly
//! (AGE-2015).
//!
//! All the actual escalation orchestration — investigate (fork or fresh
//! through herdr), resolve (validate, notify the lead, post to Linear), and
//! post-linear — is `relay_ops::escalate`, shared with `agent-msg`. This
//! binary is the terminal UI and the background workers (`ops.rs`) that
//! drive it; see `app.rs` for the pure state/update logic and `ui.rs` for
//! rendering.

mod app;
mod notify;
mod ops;
mod ui;

use std::{
    io,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use clap::Parser;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

use crate::{
    app::App,
    ops::{UiEvent, Workers},
};

/// How long one event-loop tick waits for a key press before redrawing
/// anyway — keeps the status line and the list responsive to background
/// refreshes even with no input.
const TICK: Duration = Duration::from_millis(100);

/// How long a confirmed quit waits for the action worker to finish
/// whatever it's already doing before exiting anyway.
const QUIT_WAIT: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(
    name = "relay-tui",
    version,
    about = "The operator's escalation inbox: investigate and resolve ix-relay escalations."
)]
struct Cli {
    /// Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`,
    /// then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`.
    #[arg(long)]
    url: Option<String>,
    /// Redis key prefix. Defaults come from `~/.ix/relay/config.toml`, then
    /// `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`].
    #[arg(long)]
    prefix: Option<String>,
    /// The human inbox `resolve` notifies/signs as. Defaults come from
    /// `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`.
    #[arg(long)]
    human: Option<String>,
    /// Fire a `herdr notification show` once per newly arrived waiting
    /// escalation (never for the backlog already waiting when `relay-tui`
    /// starts — see `app.rs`'s new-arrival detection). Off by default: on a
    /// machine with no desktop-notification support, this would just be a
    /// failing herdr call every time a new escalation shows up.
    #[arg(long)]
    notify: bool,
}

fn core_flags(cli: &Cli) -> relay_core::config::CoreFlags {
    relay_core::config::CoreFlags {
        url: cli.url.clone(),
        prefix: cli.prefix.clone(),
        machine: None,
        human: cli.human.clone(),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let settings = match relay_core::config::load_core(core_flags(&cli)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("relay-tui: {e}");
            return ExitCode::FAILURE;
        }
    };

    match run(cli.notify, settings) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("relay-tui: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Sets up the terminal, runs the event loop, and restores the terminal on
/// the way out — including on panic, via [`install_panic_hook`], and on
/// early return, via [`TerminalGuard`]'s `Drop`. Whichever fires first,
/// The operator's shell is never left in raw mode with the alternate screen up.
///
/// Background-thread panics never reach that same panic hook: `ops.rs`
/// catches each one where it happens (one poll tick, one action) and
/// reports it as an ordinary event instead, so the hook only ever fires for
/// a genuine failure on *this*, the UI thread — never while this loop is
/// still drawing on what the hook already restored to a plain scrollback.
fn run(notify_on_new: bool, settings: relay_core::config::CoreSettings) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    install_panic_hook();
    let _guard = TerminalGuard;

    let term_signal = install_signal_handlers()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let workers = ops::spawn(settings);
    let mut app = App::new(notify_on_new);

    loop {
        terminal.draw(|f| ui::render(f, &app))?;
        drain_ui_events(&workers, &mut app);
        check_workers_alive(&workers, &mut app);

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(action) = app::handle_key(&mut app, key)
        {
            let _ = workers.actions.send(action);
        }

        if term_signal.load(Ordering::Relaxed) {
            app.should_quit = true;
        }

        if app.should_quit {
            // Dropping the sender lets the action worker's `for action in
            // rx` loop end as soon as whatever it's already doing finishes
            // (see ops.rs's `action_loop`) — nothing more will ever be
            // dispatched, so there's no reason to keep it open.
            drop(workers.actions);
            wait_for_worker(&workers.action_handle, QUIT_WAIT);
            return Ok(());
        }
    }
}

/// Registers `SIGTERM` and `SIGHUP` to set a flag the event loop checks
/// every tick, instead of taking Rust's default action for either (an
/// immediate, uncontrolled process exit that would skip [`TerminalGuard`]
/// entirely and leave the terminal in raw mode / the alternate screen).
/// `signal_hook::flag::register` is signal-safe by construction — the
/// signal handler it installs behind the scenes only ever does one atomic
/// store — so this needs no `unsafe` of its own.
fn install_signal_handlers() -> io::Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&flag))?;
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&flag))?;
    Ok(flag)
}

/// Polls `handle` for up to `timeout`, so a confirmed quit gives an
/// in-flight action (e.g. a Linear post already underway) a chance to
/// finish rather than being cut off mid-call by the process exiting out
/// from under it. Gives up and returns once `timeout` elapses either way —
/// `relay-tui` exits regardless, since the user already confirmed it twice
/// (see `app.rs`'s quit-confirmation).
fn wait_for_worker(handle: &thread::JoinHandle<()>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
}

/// Applies every [`UiEvent`] waiting on `workers.events` without blocking,
/// updating `app` and dispatching `--notify` actions for any escalation
/// that just arrived as newly waiting.
fn drain_ui_events(workers: &Workers, app: &mut App) {
    while let Ok(event) = workers.events.try_recv() {
        match event {
            UiEvent::Refresh(Ok(data)) => {
                let new_arrivals = app.apply_refresh(data);
                if app.notify_on_new {
                    for e in new_arrivals {
                        let _ = workers.actions.send(app::Action::Notify {
                            id: e.id,
                            ticket: e.filing.ticket.to_string(),
                            lead: e.filing.lead.to_string(),
                        });
                    }
                }
            }
            UiEvent::Refresh(Err(e)) => {
                app.status = format!("refresh error: {e}");
            }
            UiEvent::ActionDone { id, message } => {
                if let Some(id) = &id {
                    app.clear_in_flight(id);
                }
                app.status = message;
            }
        }
    }
}

/// A worker thread finishing on its own — not through the normal shutdown
/// path this module drives — means something escaped `ops.rs`'s own
/// `catch_unwind` around each unit of work (a second panic while unwinding,
/// an abort, ...). Reports it exactly like an `UiEvent::WorkerDied` the
/// thread didn't get the chance to send itself.
fn check_workers_alive(workers: &Workers, app: &mut App) {
    if app.worker_died.is_none() && workers.poll_handle.is_finished() {
        app.mark_worker_died("poller", "exited unexpectedly");
    }
    if app.worker_died.is_none() && workers.action_handle.is_finished() {
        app.mark_worker_died("action worker", "exited unexpectedly");
    }
}

/// Installs a panic hook that restores the terminal (raw mode off, back to
/// the main screen) before handing off to the default hook, so a panic's
/// backtrace prints to a normal, scrollable terminal instead of vanishing
/// into (or corrupting) the alternate screen.
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

/// Restores the terminal when dropped — covers every early return from
/// [`run`] (a draw or event error), not only a clean fall-through.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}
