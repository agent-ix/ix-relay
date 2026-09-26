//! Rendering: turns [`App`] into a [`ratatui::Frame`]. Pure with respect to
//! I/O — takes the frame and the state, writes widgets, returns nothing —
//! so it's exercised in tests with [`ratatui::backend::TestBackend`] and no
//! real terminal (see this module's `tests`).

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use relay_core::{Escalation, EscalationStatus};

use crate::app::{App, Mode};

const HELP: &str = "j/k move  Tab filter  i fork  I fresh  r resolve  p post-linear  q quit";

/// Renders the whole inbox: a header, a list/detail body, and a status line
/// (or, in [`Mode::ResolveInput`], a decision input in place of it).
pub fn render(frame: &mut Frame, app: &App) {
    let footer_height = match &app.mode {
        Mode::ResolveInput { .. } => 2,
        Mode::Normal => 1,
    };
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(footer_height),
    ])
    .areas(frame.area());

    render_header(frame, app, header_area);
    render_body(frame, app, body_area);
    render_footer(frame, app, footer_area);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let line = format!("ix-relay — filter: {}  |  {HELP}", app.filter.label());
    frame.render_widget(Paragraph::new(line), area);
}

fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    let [list_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(area);
    render_list(frame, app, list_area);
    render_detail(frame, app, detail_area);
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let escalations = app.filtered();
    let title = format!("Escalations ({})", escalations.len());
    let selected_index = app
        .selected()
        .and_then(|sel| escalations.iter().position(|e| e.id == sel.id));

    let items: Vec<ListItem> = escalations.iter().map(|e| list_item(app, e)).collect();

    let mut state = ListState::default();
    state.select(selected_index);

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, area, &mut state);
}

fn list_item<'a>(app: &App, e: &'a Escalation) -> ListItem<'a> {
    let marker = if app.is_new(&e.id) { "* " } else { "  " };
    let line = format!(
        "{marker}{:<10} {:<8} {:<12} {}",
        e.id,
        e.status.as_str(),
        e.filing.ticket,
        e.filing.lead
    );
    let style = if app.is_new(&e.id) {
        Style::default()
            .fg(status_color(e.status))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(status_color(e.status))
    };
    ListItem::new(Line::styled(line, style))
}

fn status_color(status: EscalationStatus) -> Color {
    match status {
        EscalationStatus::Waiting => Color::Yellow,
        EscalationStatus::Investigating => Color::Cyan,
        EscalationStatus::Resolved => Color::Green,
    }
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Detail");
    let text = match app.selected() {
        Some(e) => detail_text(e),
        None => Text::from("no escalations"),
    };
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
        area,
    );
}

fn detail_text(e: &Escalation) -> Text<'static> {
    let mut lines = vec![
        Line::from(format!("id:             {}", e.id)),
        Line::from(format!("ticket:         {}", e.filing.ticket)),
        Line::from(format!("lead:           {}", e.filing.lead)),
        Line::from(format!(
            "machine:        {}",
            one_line(e.filing.machine.as_str())
        )),
        Line::from(format!(
            "cwd:            {}",
            one_line(e.filing.cwd.as_deref().unwrap_or("-"))
        )),
        Line::from(format!("kind:           {}", one_line(&e.filing.kind))),
        Line::from(format!("status:         {}", e.status)),
        Line::from(format!(
            "filed:          {}",
            relay_ops::clock(e.filing.created_at_ms)
        )),
        Line::from(""),
        Line::from(Span::styled(
            "context:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    push_multiline(&mut lines, &e.filing.context);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "options:",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for option in &e.filing.options {
        let sanitized = sanitize(option);
        let mut option_lines = sanitized.lines();
        if let Some(first) = option_lines.next() {
            lines.push(Line::from(format!("- {first}")));
        }
        for continuation in option_lines {
            lines.push(Line::from(format!("  {continuation}")));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "recommendation:",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    push_multiline(&mut lines, &e.filing.recommendation);
    if let Some(decision) = &e.decision {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "decision:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        push_multiline(&mut lines, decision);
    }
    Text::from(lines)
}

/// Sanitizes `text` (a lead-supplied free-text field: context,
/// recommendation, an option, or a decision) and pushes one [`Line`] per
/// source line — `str::lines()` splits on `\n` (and trims a trailing `\r`),
/// so a multi-line field actually renders as multiple lines instead of one
/// line ratatui would otherwise show with its embedded newlines as raw,
/// unrendered bytes.
fn push_multiline(lines: &mut Vec<Line<'static>>, text: &str) {
    let sanitized = sanitize(text);
    if sanitized.is_empty() {
        lines.push(Line::from(""));
        return;
    }
    for line in sanitized.lines() {
        lines.push(Line::from(line.to_owned()));
    }
}

/// A header field (machine, cwd, kind) that must stay on one line and carry no
/// escape codes: see [`relay_core::text::sanitise_line`].
fn one_line(text: &str) -> String {
    relay_core::text::sanitise_line(text)
}

/// Replaces every character that has no business appearing as plain terminal
/// text (ESC, other control characters, bidi controls, line separators — see
/// [`relay_core::text::is_unsafe_char`], the rule shared with message
/// rendering) in lead-supplied free text before it is rendered.
/// `relay_core` already refuses these at store time; this is defense in depth
/// for whatever is in Redis right now — a hand-edited record, or data written
/// before that check existed.
fn sanitize(text: &str) -> String {
    relay_core::text::sanitise(text).into_owned()
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(died) = &app.worker_died {
        // Persistent and unconditional: once a worker is gone, nothing it
        // was doing will ever complete, so there's nothing more useful the
        // footer could show instead — not even the resolve-input prompt.
        frame.render_widget(
            Paragraph::new(Span::styled(
                died.as_str(),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            area,
        );
        return;
    }
    match &app.mode {
        Mode::Normal => {
            frame.render_widget(Paragraph::new(app.status.as_str()), area);
        }
        Mode::ResolveInput { buffer, .. } => {
            let lines = vec![
                Line::from(format!("Decision: {buffer}\u{2588}")),
                Line::from(Span::styled(
                    "Enter submit   Esc cancel",
                    Style::default().add_modifier(Modifier::DIM),
                )),
            ];
            frame.render_widget(Paragraph::new(lines), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};
    use relay_core::{EscalationFiling, TicketId};

    use super::*;
    use crate::app::RefreshData;

    fn esc(id: &str, status: EscalationStatus) -> Escalation {
        Escalation {
            id: id.parse().unwrap(),
            filing: EscalationFiling {
                ticket: "AGE-2015".parse::<TicketId>().unwrap(),
                lead: "qsl-lead".parse().unwrap(),
                machine: "node-a".parse().unwrap(),
                session_id: "sid-1".parse().unwrap(),
                cwd: Some("/work".into()),
                kind: "claude".into(),
                context: "which eviction policy?".into(),
                options: vec!["noeviction".into(), "allkeys-lru".into()],
                recommendation: "noeviction".into(),
                created_at_ms: 1,
            },
            status,
            decision: None,
        }
    }

    fn rendered(app: &App) -> String {
        rendered_rows(app).join("")
    }

    /// Same render, but as one string per terminal row (rather than one
    /// giant concatenation) — needed to tell "these two substrings appear
    /// somewhere in the frame" apart from "they appear on different lines",
    /// which plain substring checks on the concatenated form can't do.
    fn rendered_rows(app: &App) -> Vec<String> {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let width = usize::from(buffer.area.width);
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn empty_list_renders_the_no_escalations_placeholder_and_zero_count() {
        let app = App::new(false);
        let out = rendered(&app);
        assert!(out.contains("Escalations (0)"), "{out}");
        assert!(out.contains("no escalations"), "{out}");
    }

    #[test]
    fn list_plus_detail_shows_the_selected_escalations_full_detail() {
        let mut app = App::new(false);
        app.apply_refresh(RefreshData {
            escalations: vec![esc("e1", EscalationStatus::Waiting)],
            rejected_count: 0,
        });
        let out = rendered(&app);
        assert!(out.contains("Escalations (1)"), "{out}");
        assert!(out.contains("e1"), "{out}");
        assert!(out.contains("AGE-2015"), "{out}");
        assert!(out.contains("qsl-lead"), "{out}");
        assert!(out.contains("which eviction policy?"), "{out}");
        assert!(out.contains("noeviction"), "{out}");
        assert!(out.contains("allkeys-lru"), "{out}");
    }

    #[test]
    fn resolved_escalation_detail_shows_its_decision() {
        let mut app = App::new(false);
        let mut e = esc("e1", EscalationStatus::Resolved);
        e.decision = Some("go with noeviction".into());
        app.apply_refresh(RefreshData {
            escalations: vec![e],
            rejected_count: 0,
        });
        let out = rendered(&app);
        assert!(out.contains("go with noeviction"), "{out}");
    }

    #[test]
    fn resolve_input_mode_shows_the_typed_buffer_and_the_key_hints() {
        let mut app = App::new(false);
        app.apply_refresh(RefreshData {
            escalations: vec![esc("e1", EscalationStatus::Waiting)],
            rejected_count: 0,
        });
        crate::app::handle_key(
            &mut app,
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('r'),
                ratatui::crossterm::event::KeyModifiers::empty(),
            ),
        );
        if let crate::app::Mode::ResolveInput { buffer, .. } = &mut app.mode {
            buffer.push_str("go with a");
        }
        let out = rendered(&app);
        assert!(out.contains("Decision: go with a"), "{out}");
        assert!(out.contains("Enter submit"), "{out}");
        assert!(out.contains("Esc cancel"), "{out}");
    }

    /// AGE-2015 PR review, finding 7: multi-line context must actually
    /// render as multiple lines, and ESC / a bidi-override character must
    /// never reach the terminal, even though `relay_core` already refuses
    /// them at store time — this is defense in depth against whatever's
    /// actually sitting in Redis right now.
    #[test]
    fn multiline_context_renders_as_separate_lines_with_esc_and_bidi_override_replaced() {
        let mut app = App::new(false);
        let mut e = esc("e1", EscalationStatus::Waiting);
        e.filing.context = "line one\x1b[201~\nline\u{202e}two".into();
        app.apply_refresh(RefreshData {
            escalations: vec![e],
            rejected_count: 0,
        });

        let rows = rendered_rows(&app);
        let joined = rows.join("\n");
        assert!(joined.contains("line one"), "{joined}");
        assert!(joined.contains("line\u{fffd}two"), "{joined}");
        assert!(
            !joined.contains('\u{1b}'),
            "ESC must never reach the terminal: {joined}"
        );
        assert!(
            !joined.contains('\u{202e}'),
            "the bidi override must never reach the terminal: {joined}"
        );

        let first_row = rows.iter().position(|r| r.contains("line one")).unwrap();
        let second_row = rows.iter().position(|r| r.contains("two")).unwrap();
        assert_ne!(
            first_row, second_row,
            "the two context lines must render on separate rows, not run together: {rows:?}"
        );
    }

    /// AGE-2026 review: cwd and kind are lead-supplied too. (A machine name cannot
    /// carry an escape: `MachineName` refuses one at decode.)
    #[test]
    fn the_header_fields_never_carry_escape_or_bidi_characters() {
        let mut app = App::new(false);
        let mut e = esc("e1", EscalationStatus::Waiting);
        e.filing.cwd = Some("/w\u{1b}[2J\u{202e}ork".into());
        e.filing.kind = "cl\u{1b}aude".into();
        app.apply_refresh(RefreshData {
            escalations: vec![e],
            rejected_count: 0,
        });
        let joined = rendered_rows(&app).join("\n");
        assert!(
            !joined.contains('\u{1b}') && !joined.contains('\u{202e}'),
            "{joined}"
        );
        assert!(joined.contains("/w\u{fffd}[2J\u{fffd}ork"), "{joined}");
    }

    #[test]
    fn a_new_arrival_is_marked_in_the_list() {
        let mut app = App::new(false);
        app.apply_refresh(RefreshData {
            escalations: vec![esc("e1", EscalationStatus::Waiting)],
            rejected_count: 0,
        });
        app.apply_refresh(RefreshData {
            escalations: vec![
                esc("e1", EscalationStatus::Waiting),
                esc("e2", EscalationStatus::Waiting),
            ],
            rejected_count: 0,
        });
        let out = rendered(&app);
        assert!(out.contains('*'), "expected a new-arrival marker: {out}");
    }
}
