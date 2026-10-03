// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Searchable cross-agent Herdr directory (SWM-44).

use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};
use serde::Deserialize;

const TICK: Duration = Duration::from_millis(100);
const MAX_REPORT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Snapshot {
    rows: Vec<AgentRow>,
    machines: Vec<MachineState>,
    complete: bool,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct AgentRow {
    address: Option<String>,
    bus_name: Option<String>,
    aliases: Vec<String>,
    machine: String,
    workspace_label: Option<String>,
    tab_label: Option<String>,
    pane_id: String,
    session_name: Option<String>,
    herdr_status: String,
    on_bus: bool,
    last_seen_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct MachineState {
    machine: String,
    herdr: String,
    watcher: String,
    last_tick_ms: Option<i64>,
    detail: Option<String>,
}

#[derive(Default)]
struct App {
    snapshot: Option<Snapshot>,
    query: String,
    selected: usize,
    error: Option<String>,
    refreshing: bool,
    copy_notice: Option<String>,
    message_lines: Option<Vec<String>>,
    verified_lead: Option<String>,
    reply_sender: Option<String>,
    reply_draft: Option<String>,
}

enum RefreshEvent {
    Complete(Result<Snapshot, String>),
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("relay-directory: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let (events_tx, events_rx) = mpsc::channel();
    let (refresh_tx, refresh_rx) = mpsc::channel();
    let _refresh_worker = thread::spawn(move || refresh_loop(refresh_rx, events_tx));
    let mut app = App::default();
    let _ = refresh_tx.send(());

    loop {
        terminal.draw(|frame| render(frame, &app))?;
        apply_refreshes(&events_rx, &mut app);
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && handle_key(key.code, &mut app, &refresh_tx)
        {
            break;
        }
    }
    drop(refresh_tx);
    Ok(())
}

fn refresh_loop(receiver: Receiver<()>, events: Sender<RefreshEvent>) {
    while receiver.recv().is_ok() {
        let result = load_snapshot();
        if events.send(RefreshEvent::Complete(result)).is_err() {
            break;
        }
    }
}

fn load_snapshot() -> Result<Snapshot, String> {
    let mut child = Command::new("agent-msg")
        .args(["find", "--report-json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start agent-msg: {error}"))?;
    let mut output = Vec::new();
    let Some(mut stdout) = child.stdout.take() else {
        return Err("agent-msg did not provide output".into());
    };
    let read_result = stdout
        .by_ref()
        .take(MAX_REPORT_BYTES.saturating_add(1))
        .read_to_end(&mut output);
    if read_result.is_err() || u64::try_from(output.len()).unwrap_or(u64::MAX) > MAX_REPORT_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return Err("agent-msg directory snapshot exceeded 2 MiB".into());
    }
    let status = child
        .wait()
        .map_err(|error| format!("could not wait for agent-msg: {error}"))?;
    if !status.success() {
        return Err("agent-msg could not read the relay directory".into());
    }
    serde_json::from_slice(&output).map_err(|error| format!("invalid directory snapshot: {error}"))
}

fn apply_refreshes(receiver: &Receiver<RefreshEvent>, app: &mut App) {
    while let Ok(event) = receiver.try_recv() {
        let RefreshEvent::Complete(result) = event;
        match result {
            Ok(snapshot) => {
                app.snapshot = Some(snapshot);
                app.error = None;
                app.refreshing = false;
                app.clamp_selection();
            }
            Err(error) => {
                app.error = Some(error);
                app.refreshing = false;
            }
        }
    }
}

impl App {
    fn filtered(&self) -> Vec<&AgentRow> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        snapshot
            .rows
            .iter()
            .filter(|row| matches_query(row, &self.query))
            .collect()
    }

    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.filtered().len().saturating_sub(1));
    }
}

fn matches_query(row: &AgentRow, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    [
        row.address.as_deref(),
        row.bus_name.as_deref(),
        Some(row.machine.as_str()),
        row.workspace_label.as_deref(),
        row.tab_label.as_deref(),
        Some(row.pane_id.as_str()),
        row.session_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(&query))
        || row
            .aliases
            .iter()
            .any(|value| value.to_lowercase().contains(&query))
}

fn verified_address(row: &AgentRow) -> Option<&str> {
    row.on_bus.then_some(row.address.as_deref()).flatten()
}

fn handle_key(code: KeyCode, app: &mut App, refresh: &Sender<()>) -> bool {
    match code {
        KeyCode::Esc | KeyCode::Char('q') if app.reply_draft.is_none() => return true,
        KeyCode::F(5) if app.reply_draft.is_none() => {
            if !app.refreshing && refresh.send(()).is_ok() {
                app.refreshing = true;
            }
        }
        KeyCode::Char('m') if app.query.is_empty() && app.reply_draft.is_none() => {
            if app.message_lines.is_none() {
                let (lines, lead, sender) = load_message_lines();
                app.message_lines = Some(lines);
                app.verified_lead = lead;
                app.reply_sender = sender;
            } else {
                app.message_lines = None;
                app.reply_draft = None;
            }
        }
        KeyCode::Char('r') if app.message_lines.is_some() && app.reply_draft.is_none() => {
            if app.verified_lead.is_some() && app.reply_sender.is_some() {
                app.reply_draft = Some(String::new());
            } else {
                app.copy_notice =
                    Some("reply unavailable: no verified lead or pending sender".into());
            }
        }
        KeyCode::Esc if app.reply_draft.is_some() => app.reply_draft = None,
        KeyCode::Enter if app.reply_draft.is_some() => {
            let draft = app.reply_draft.take().unwrap_or_default();
            match (&app.verified_lead, &app.reply_sender) {
                (Some(lead), Some(sender)) if !draft.trim().is_empty() => {
                    app.copy_notice = Some(send_verified_reply(lead, sender, &draft));
                }
                _ => app.copy_notice = Some("reply cancelled: message is empty".into()),
            }
        }
        KeyCode::Backspace if app.reply_draft.is_some() => {
            app.reply_draft.as_mut().map(String::pop);
        }
        KeyCode::Char(character) if app.reply_draft.is_some() && !character.is_control() => {
            if let Some(draft) = &mut app.reply_draft
                && draft.len().saturating_add(character.len_utf8()) <= relay_core::MAX_MESSAGE_BYTES
            {
                draft.push(character);
            }
        }
        KeyCode::Char('y') if app.reply_draft.is_none() => {
            if let Some(address) = app
                .filtered()
                .get(app.selected)
                .and_then(|row| verified_address(row))
            {
                let encoded = base64(address.as_bytes());
                print!("\x1b]52;c;{encoded}\x07");
                let _ = io::stdout().flush();
                app.copy_notice = Some(format!("copied {address}"));
            } else {
                app.copy_notice = Some("this live pane has no verified relay address".into());
            }
        }
        KeyCode::Up => app.selected = app.selected.saturating_sub(1),
        KeyCode::Down => {
            app.selected = app
                .selected
                .saturating_add(1)
                .min(app.filtered().len().saturating_sub(1));
        }
        KeyCode::Backspace => {
            app.query.pop();
            app.selected = 0;
        }
        KeyCode::Char(character) if !character.is_control() && app.query.chars().count() < 80 => {
            app.query.push(character);
            app.selected = 0;
        }
        _ => {}
    }
    false
}

fn render(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    if let Some(lines) = &app.message_lines {
        let mut content = if let Some(draft) = &app.reply_draft {
            format!(
                "Reply to {}\n\n{}\n\nEnter send · Esc cancel · max {} bytes",
                app.reply_sender.as_deref().unwrap_or("unknown"),
                clean(draft),
                relay_core::MAX_MESSAGE_BYTES
            )
        } else if lines.is_empty() {
            "No recent or pending messages".into()
        } else {
            lines.join("\n\n")
        };
        if let Some(notice) = &app.copy_notice {
            content.push_str(&format!("\n\n{}", clean(notice)));
        }
        frame.render_widget(
            Paragraph::new(content)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title(
                    "ix-relay · messages · r: reply to oldest pending · m: leads · q: close",
                )),
            area,
        );
        return;
    }
    let [title_area, search_area, rows_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(2),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new("ix-relay · online leads (Herdr · all coding agents)"),
        title_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Search: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(clean(&app.query)),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        search_area,
    );

    let rows = app.filtered();
    let selected = app.selected;
    let machine_state = app.snapshot.as_ref().map(|snapshot| &snapshot.machines);
    let mut table_rows = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let state = machine_state.and_then(|machines| {
                machines
                    .iter()
                    .find(|machine| machine.machine == row.machine)
            });
            let relay = row_relay_state(row, state);
            let where_at = format!(
                "{}/{} · {}",
                row.workspace_label
                    .as_deref()
                    .map(clean)
                    .unwrap_or_else(|| "?".into()),
                row.tab_label
                    .as_deref()
                    .map(clean)
                    .unwrap_or_else(|| "?".into()),
                clean(&row.machine)
            );
            let name = row
                .bus_name
                .as_deref()
                .map(clean)
                .or_else(|| row.session_name.as_deref().map(clean))
                .unwrap_or_else(|| "unregistered".into());
            let age = relative_age(row.last_seen_ms);
            let style = if index == selected {
                Style::default().bg(Color::Rgb(31, 41, 55)).fg(Color::White)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(name),
                Cell::from(relay),
                Cell::from(clean(&row.herdr_status)),
                Cell::from(where_at),
                Cell::from(age),
                Cell::from(clean(&row.address.clone().unwrap_or_default())),
            ])
            .style(style)
        })
        .collect::<Vec<_>>();
    if table_rows.is_empty() {
        table_rows.push(Row::new([
            "No matching live Herdr leads",
            "",
            "",
            "",
            "",
            "",
        ]));
    }
    let table = Table::new(
        table_rows,
        [
            Constraint::Percentage(18),
            Constraint::Percentage(17),
            Constraint::Percentage(12),
            Constraint::Percentage(25),
            Constraint::Length(9),
            Constraint::Percentage(28),
        ],
    )
    .header(
        Row::new(["Lead", "Relay", "Activity", "Location", "Seen", "Address"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(table, rows_area);

    let mut footer = vec![Span::raw(
        "type to search · ↑/↓ select · y copy verified address · F5 refresh · q close",
    )];
    if app.refreshing {
        footer.push(Span::styled(
            " · refreshing…",
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(snapshot) = &app.snapshot {
        if !snapshot.complete {
            footer.push(Span::styled(
                " · results incomplete; unavailable machines remain unknown",
                Style::default().fg(Color::Yellow),
            ));
        }
        if snapshot.truncated {
            footer.push(Span::styled(
                " · showing first 500 live matches",
                Style::default().fg(Color::Yellow),
            ));
        }
        if let Some(states) = machine_state {
            let summary = states
                .iter()
                .map(|state| {
                    format!(
                        "{} {}/{} seen {}",
                        clean(&state.machine),
                        clean(&state.herdr),
                        clean(&state.watcher),
                        relative_age(state.last_tick_ms),
                    ) + &state
                        .detail
                        .as_deref()
                        .map(|detail| format!(" · {}", clean(detail)))
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("  ");
            if !summary.is_empty() {
                footer.push(Span::raw(format!("\n{summary}")));
            }
        }
    }
    if let Some(error) = &app.error {
        footer.push(Span::styled(
            format!(" · {}", clean(error)),
            Style::default().fg(Color::Red),
        ));
    }
    if let Some(notice) = &app.copy_notice {
        footer.push(Span::raw(format!(" · {}", clean(notice))));
    }
    frame.render_widget(
        Paragraph::new(Line::from(footer)).block(Block::default()),
        footer_area,
    );
}

fn load_message_lines() -> (Vec<String>, Option<String>, Option<String>) {
    let history = Command::new("agent-msg")
        .args(["log", "--since", "1h", "--max", "50", "--json"])
        .output();
    let status = Command::new("agent-msg")
        .args(["status", "--json"])
        .output();
    let Ok(status) = status else {
        return (
            vec!["Focused lead could not be verified; pending preview unavailable".into()],
            None,
            None,
        );
    };
    let identity: serde_json::Value = match serde_json::from_slice(&status.stdout) {
        Ok(v) => v,
        Err(_) => {
            return (
                vec!["Focused lead could not be verified; pending preview unavailable".into()],
                None,
                None,
            );
        }
    };
    let Some(name) = identity
        .get("lead")
        .filter(|v| v.get("state").and_then(serde_json::Value::as_str) == Some("verified"))
        .and_then(|v| v.get("name"))
        .and_then(serde_json::Value::as_str)
    else {
        return (
            vec![
                "Focused Herdr pane is not a verified relay lead; pending preview unavailable"
                    .into(),
            ],
            None,
            None,
        );
    };
    let pending = Command::new("agent-msg")
        .args(["inbox", "--peek", "--max", "50", "--as", name])
        .output();
    let (Ok(history), Ok(pending)) = (history, pending) else {
        return (vec!["Message view unavailable".into()], None, None);
    };
    if !history.status.success() || !pending.status.success() {
        return (
            vec!["Message view unavailable; relay read failed".into()],
            None,
            None,
        );
    }
    let history: serde_json::Value = match serde_json::from_slice(&history.stdout) {
        Ok(v) => v,
        Err(_) => {
            return (
                vec!["Message history response was invalid".into()],
                None,
                None,
            );
        }
    };
    let pending: serde_json::Value = match serde_json::from_slice(&pending.stdout) {
        Ok(v) => v,
        Err(_) => {
            return (
                vec!["Inbox preview response was invalid".into()],
                None,
                None,
            );
        }
    };
    let sender = pending
        .get("messages")
        .and_then(serde_json::Value::as_array)
        .and_then(|messages| messages.first())
        .and_then(|message| message.get("from"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let mut lines = Vec::new();
    for (section, value, is_pending) in [
        ("RECENT", history.get("messages"), false),
        ("PENDING (not removed)", pending.get("messages"), true),
    ] {
        lines.push(section.to_owned());
        if let Some(messages) = value.and_then(serde_json::Value::as_array) {
            for message in messages.iter().take(50) {
                let get = |key: &str| {
                    message
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("?")
                };
                let timestamp = message
                    .get("sent_at_ms")
                    .and_then(serde_json::Value::as_i64)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into());
                let badge = if is_pending {
                    if message
                        .get("redelivered")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    {
                        " · REDELIVERED".to_owned()
                    } else {
                        " · pending".to_owned()
                    }
                } else {
                    format!(" · redelivery {}", clean(get("redelivery_state")))
                };
                let body = display_body(get("text"));
                lines.push(format!(
                    "{} → {} · {}{}\n{}",
                    clean(get("from")),
                    clean(get("to")),
                    timestamp,
                    badge,
                    body
                ));
            }
        }
    }
    (lines, Some(name.to_owned()), sender)
}

fn send_verified_reply(focused_lead: &str, sender: &str, body: &str) -> String {
    let status = Command::new("agent-msg")
        .args(["status", "--json"])
        .output();
    let Ok(status) = status else {
        return "reply refused: focused lead could not be reverified".into();
    };
    let status: serde_json::Value = match serde_json::from_slice(&status.stdout) {
        Ok(value) => value,
        Err(_) => return "reply refused: focused lead could not be reverified".into(),
    };
    let current = status
        .get("lead")
        .filter(|lead| lead.get("state").and_then(serde_json::Value::as_str) == Some("verified"))
        .and_then(|lead| lead.get("name"))
        .and_then(serde_json::Value::as_str);
    if current != Some(focused_lead)
        || body.trim().is_empty()
        || body.len() > relay_core::MAX_MESSAGE_BYTES
    {
        return "reply refused: focus changed or message is empty/too long".into();
    }
    let lookup = Command::new("agent-msg")
        .args(["find", sender, "--report-json"])
        .output();
    let Ok(lookup) = lookup else {
        return "reply refused: sender address could not be verified".into();
    };
    if !lookup.status.success() {
        return "reply refused: sender address could not be verified".into();
    }
    let report: serde_json::Value = match serde_json::from_slice(&lookup.stdout) {
        Ok(value) => value,
        Err(_) => return "reply refused: sender address response was invalid".into(),
    };
    let Some(address) = canonical_sender_address(&report, sender) else {
        return "reply refused: sender has no canonical address".into();
    };
    let sent = Command::new("agent-msg")
        .args(["--as", focused_lead, "send", &address, "--", body])
        .output();
    match sent {
        Ok(output) if output.status.success() => "reply queued to verified sender".into(),
        _ => "reply failed; no confirmation was received".into(),
    }
}

fn canonical_sender_address(report: &serde_json::Value, sender: &str) -> Option<String> {
    let rows = report.get("rows")?.as_array()?;
    let mut matches = rows.iter().filter(|row| {
        row.get("bus_name").and_then(serde_json::Value::as_str) == Some(sender)
            && row.get("on_bus").and_then(serde_json::Value::as_bool) == Some(true)
    });
    let row = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let address = row.get("address")?.as_str()?;
    let parsed = address.parse::<relay_core::Address>().ok()?;
    (parsed.tab().is_some() && parsed.pane().is_none() && address.split(':').count() == 3)
        .then(|| address.to_owned())
}

fn row_relay_state(row: &AgentRow, machine: Option<&MachineState>) -> String {
    if machine.is_some_and(|state| state.herdr == "unavailable") {
        return "unknown · Herdr offline".into();
    }
    if !row.on_bus {
        return "live · not enrolled".into();
    }
    match machine.map(|state| state.watcher.as_str()) {
        Some("fresh") => "live · deliverable".into(),
        Some("degraded" | "stale" | "missing" | "unreadable") => "live · relay uncertain".into(),
        _ => "live · status unknown".into(),
    }
}

fn relative_age(last_seen_ms: Option<i64>) -> String {
    let Some(last_seen_ms) = last_seen_ms else {
        return "unknown".into();
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    let age_seconds = now_ms.saturating_sub(last_seen_ms).max(0) / 1000;
    if age_seconds < 60 {
        format!("{age_seconds}s")
    } else if age_seconds < 3_600 {
        format!("{}m", age_seconds / 60)
    } else {
        format!("{}h", age_seconds / 3_600)
    }
}

fn clean(value: &str) -> String {
    relay_core::text::neutralise(value)
}

fn display_body(value: &str) -> String {
    clean(value).chars().take(1200).collect()
}

fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(input.len().saturating_add(2) / 3 * 4);
    for chunk in input.chunks(3) {
        let first = usize::from(chunk[0]);
        let second = chunk.get(1).copied().map(usize::from);
        let third = chunk.get(2).copied().map(usize::from);
        result.push(char::from(ALPHABET[first >> 2]));
        result.push(char::from(
            ALPHABET[((first & 3) << 4) | second.unwrap_or(0) >> 4],
        ));
        result.push(second.map_or('=', |second| {
            char::from(ALPHABET[((second & 15) << 2) | third.unwrap_or(0) >> 6])
        }));
        result.push(third.map_or('=', |third| char::from(ALPHABET[third & 63])));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PluginManifest {
        id: String,
        name: String,
        version: String,
        min_herdr_version: String,
        description: String,
        platforms: Vec<String>,
        panes: Vec<PluginPane>,
        actions: Vec<PluginAction>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PluginPane {
        id: String,
        title: String,
        description: Option<String>,
        placement: String,
        command: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PluginAction {
        id: String,
        title: String,
        contexts: Vec<String>,
        command: Vec<String>,
    }

    /// Trace: FR-050-AC-6
    #[test]
    fn herdr_manifest_registers_cross_agent_popup_action_and_entrypoint() {
        let manifest: PluginManifest =
            toml::from_str(include_str!("../../../plugins/herdr/herdr-plugin.toml")).unwrap();
        assert_eq!(manifest.id, "agent-ix.ix-relay");
        assert_eq!(manifest.name, "ix-relay");
        assert!(!manifest.version.is_empty());
        assert!(!manifest.min_herdr_version.is_empty());
        assert!(manifest.description.contains("any coding agent"));
        assert!(manifest.platforms.contains(&"linux".into()));
        assert!(manifest.panes.iter().any(|pane| {
            pane.id == "directory"
                && pane.title == "ix-relay online leads"
                && pane.description.is_some()
                && pane.placement == "popup"
                && pane.command == ["relay-directory"]
        }));
        assert!(manifest.actions.iter().any(|action| {
            action.id == "open-directory"
                && action.title == "Browse ix-relay leads"
                && action.contexts == ["pane"]
                && action.command.starts_with(&[
                    "herdr".into(),
                    "plugin".into(),
                    "pane".into(),
                    "open".into(),
                ])
        }));
    }

    fn row() -> AgentRow {
        AgentRow {
            address: Some("hub:swm:code".into()),
            bus_name: Some("swm-lead".into()),
            aliases: vec!["swm".into()],
            machine: "hub".into(),
            workspace_label: Some("SWM".into()),
            tab_label: Some("code".into()),
            pane_id: "w1:p1".into(),
            session_name: Some("Claude Code".into()),
            herdr_status: "working".into(),
            on_bus: true,
            last_seen_ms: Some(1),
        }
    }

    /// Trace: FR-050-AC-4
    #[test]
    fn searches_aliases_and_every_human_location_field() {
        for query in ["swm", "Claude", "hub", "code", "w1:p1"] {
            assert!(matches_query(&row(), query), "query {query:?}");
        }
        assert!(!matches_query(&row(), "not-present"));
    }

    /// Trace: FR-050-AC-5
    #[test]
    fn copy_encoding_is_valid_base64_for_terminal_clipboard_sequences() {
        assert_eq!(base64(b"hub:swm:code"), "aHViOnN3bTpjb2Rl");
        let mut unregistered = row();
        unregistered.on_bus = false;
        assert_eq!(verified_address(&unregistered), None);
        assert_eq!(verified_address(&row()), Some("hub:swm:code"));
    }

    /// Trace: FR-050-AC-5
    #[test]
    fn labels_are_neutralised_before_terminal_rendering() {
        assert_eq!(clean("hub\u{1b}[31m"), "hub�[31m");
    }

    /// Trace: FR-051-AC-3
    #[test]
    fn message_bodies_remain_data_and_are_bounded_before_display() {
        let body = display_body("first\nsecond\u{1b}[31m");
        assert_eq!(body, "first\n    second�[31m");
        assert_eq!(display_body(&"x".repeat(1300)).len(), 1200);
    }

    /// Trace: FR-051-AC-4
    #[test]
    fn reply_requires_one_live_sender_with_a_full_canonical_address() {
        let report = serde_json::json!({"rows":[{
            "bus_name":"source-lead","on_bus":true,"address":"hub:SWM:code"
        }]});
        assert_eq!(
            canonical_sender_address(&report, "source-lead").as_deref(),
            Some("hub:SWM:code")
        );
        let unregistered = serde_json::json!({"rows":[{
            "bus_name":"source-lead","on_bus":false,"address":null
        }]});
        assert_eq!(canonical_sender_address(&unregistered, "source-lead"), None);
        let ambiguous = serde_json::json!({"rows":[
            {"bus_name":"source-lead","on_bus":true,"address":"hub:SWM:code"},
            {"bus_name":"source-lead","on_bus":true,"address":"hub:OTHER:code"}
        ]});
        assert_eq!(canonical_sender_address(&ambiguous, "source-lead"), None);
    }

    /// Trace: FR-050-AC-5
    #[test]
    fn only_a_fresh_watcher_and_verified_registration_are_deliverable() {
        let fresh = MachineState {
            machine: "hub".into(),
            herdr: "live".into(),
            watcher: "fresh".into(),
            last_tick_ms: Some(1),
            detail: None,
        };
        assert_eq!(row_relay_state(&row(), Some(&fresh)), "live · deliverable");
        let stale = MachineState {
            watcher: "stale".into(),
            ..fresh
        };
        assert_eq!(
            row_relay_state(&row(), Some(&stale)),
            "live · relay uncertain"
        );
        let mut unregistered = row();
        unregistered.on_bus = false;
        assert_eq!(
            row_relay_state(
                &unregistered,
                Some(&MachineState {
                    watcher: "fresh".into(),
                    ..stale
                })
            ),
            "live · not enrolled"
        );
    }
}
