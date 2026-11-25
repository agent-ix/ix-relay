// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The View selector dropdown, the F1 help popup and the tracker inspector of the dashboard.
//!
//! Popups own their keys exclusively while open, draw only inside the work area and
//! read no source: they only present and change in-memory view state. The tracker
//! inspector presents the notice the dashboard already holds and redraws as it changes.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    monitor::{App, Popup, View, display_text, select_view},
    monitor_ui::{FOREGROUND, MUTED, PRIMARY, SELECTION, muted},
};

/// Outer width of the dropdown, borders included.
const DROPDOWN_WIDTH: u16 = 26;
/// Lines moved by PageUp and PageDown in help.
const PAGE: u16 = 5;
/// Lines before the first help entry.
const HELP_HEADER_LINES: usize = 1;

/// One row of the help table: keys, action and where it has a visible effect.
struct HelpEntry {
    keys: &'static str,
    action: &'static str,
    views: &'static [View],
    note: &'static str,
}

const ALL_VIEWS: &[View] = &View::ALL;
const LEAD_VIEWS: &[View] = &[View::Tickets, View::Agents, View::Scope];
const DEPARTMENT_VIEWS: &[View] = &[View::Status, View::Health, View::Retro];

const HELP: [HelpEntry; 11] = [
    HelpEntry {
        keys: "v",
        action: "open the View selector",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "t, a, p, 1, 2, 3",
        action: "switch view",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "Up, Down, j, k, PageUp, PageDown",
        action: "select rows and page",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "d, Enter",
        action: "show or hide details",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "Left, Right",
        action: "scroll details",
        views: DEPARTMENT_VIEWS,
        note: "with details open",
    },
    HelpEntry {
        keys: "r",
        action: "refresh sources",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "s",
        action: "show source timestamps",
        views: LEAD_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "e",
        action: "show source errors and tracker quota detail",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "c",
        action: "show or hide completed projects",
        views: &[View::Scope],
        note: "",
    },
    HelpEntry {
        keys: "q, Escape",
        action: "quit when no popup is open",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "F1, ?",
        action: "open or close this help",
        views: ALL_VIEWS,
        note: "",
    },
];

/// Keys the help popup marks as applicable in `view`, in table order.
#[cfg(test)]
pub(crate) fn applicable_keys(view: View) -> Vec<&'static str> {
    HELP.iter()
        .filter(|entry| entry.views.contains(&view))
        .map(|entry| entry.keys)
        .collect()
}

/// Every help line for `view`: one row per key, with a check mark where it applies. The
/// views are named only for keys that apply in some views and not all.
pub(crate) fn help_lines(view: View) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(muted(format!(
        "✓ marks the keys that apply in {}.",
        view.label()
    )))];
    for entry in &HELP {
        let applies = entry.views.contains(&view);
        let mut spans = vec![
            Span::styled(
                if applies { "✓ " } else { "  " },
                Style::default().fg(FOREGROUND),
            ),
            Span::styled(entry.keys, Style::default().fg(PRIMARY)),
            muted("  "),
            muted(entry.action),
        ];
        if entry.views.len() < View::ALL.len() {
            spans.push(muted(format!(
                " · {}",
                entry
                    .views
                    .iter()
                    .map(|v| v.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
            if !entry.note.is_empty() {
                spans.push(muted(format!(" ({})", entry.note)));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn last_help_line() -> u16 {
    let count = HELP_HEADER_LINES + HELP.len();
    u16::try_from(count.saturating_sub(1)).unwrap_or(u16::MAX)
}

/// Gives an open popup the key; it acts only on keys it owns and ignores every other key.
pub(crate) fn handle_popup_key(app: &mut App, popup: Popup, key: KeyEvent) {
    let plain = key.modifiers.is_empty();
    match popup {
        Popup::Dropdown { highlight, offset } => {
            let last = View::ALL.len() - 1;
            let highlight = highlight.min(last);
            let moved = match key.code {
                KeyCode::Up => Some(highlight.saturating_sub(1)),
                KeyCode::Char('k') if plain => Some(highlight.saturating_sub(1)),
                KeyCode::Down => Some(highlight.saturating_add(1).min(last)),
                KeyCode::Char('j') if plain => Some(highlight.saturating_add(1).min(last)),
                _ => None,
            };
            if let Some(highlight) = moved {
                app.popup = Some(Popup::Dropdown { highlight, offset });
                return;
            }
            match key.code {
                KeyCode::Enter => {
                    app.popup = None;
                    select_view(app, View::ALL[highlight]);
                }
                KeyCode::Esc => app.popup = None,
                KeyCode::Char('v') if plain => app.popup = None,
                _ => {}
            }
        }
        Popup::Help { scroll } => {
            let last = last_help_line();
            let scrolled = match key.code {
                KeyCode::Up => Some(scroll.saturating_sub(1)),
                KeyCode::Down => Some(scroll.saturating_add(1).min(last)),
                KeyCode::PageUp => Some(scroll.saturating_sub(PAGE)),
                KeyCode::PageDown => Some(scroll.saturating_add(PAGE).min(last)),
                _ => None,
            };
            if let Some(scroll) = scrolled {
                app.popup = Some(Popup::Help { scroll });
                return;
            }
            if matches!(key.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?')) {
                app.popup = None;
            }
        }
        Popup::Tracker { scroll } => {
            // The inspector clamps the scroll to its wrapped length when it is drawn.
            let scrolled = match key.code {
                KeyCode::Up => Some(scroll.saturating_sub(1)),
                KeyCode::Down => Some(scroll.saturating_add(1)),
                KeyCode::PageUp => Some(scroll.saturating_sub(PAGE)),
                KeyCode::PageDown => Some(scroll.saturating_add(PAGE)),
                _ => None,
            };
            if let Some(scroll) = scrolled {
                app.popup = Some(Popup::Tracker { scroll });
                return;
            }
            if key.code == KeyCode::Esc || (key.code == KeyCode::Char('e') && plain) {
                app.popup = None;
            }
        }
    }
}

/// Splits `text` into rows no wider than `width` columns, breaking at spaces and splitting a
/// word that is wider than a row.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split_whitespace() {
        for piece in split_word(word, width) {
            let gap = usize::from(!row.is_empty());
            if !row.is_empty() && text_width(&row) + gap + text_width(&piece) > width {
                rows.push(std::mem::take(&mut row));
            }
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(&piece);
        }
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

/// Cuts one word into pieces no wider than `width` columns, each holding at least one character.
fn split_word(word: &str, width: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    for character in word.chars() {
        let mut widened = piece.clone();
        widened.push(character);
        if !piece.is_empty() && text_width(&widened) > width {
            pieces.push(std::mem::replace(&mut piece, character.to_string()));
        } else {
            piece = widened;
        }
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    pieces
}

fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

/// Appends `text` wrapped to `width`, with `marker` on its first row and blank indent on the
/// rest.
fn push_wrapped(
    lines: &mut Vec<Line<'static>>,
    marker: &str,
    text: &str,
    width: usize,
    style: Style,
) {
    let indent = text_width(marker);
    let text = display_text(text, 4096);
    for (index, row) in wrap_text(&text, width.saturating_sub(indent))
        .into_iter()
        .enumerate()
    {
        let lead = if index == 0 {
            marker.to_owned()
        } else {
            " ".repeat(indent)
        };
        lines.push(Line::from(vec![muted(lead), Span::styled(row, style)]));
    }
}

/// Every line of the tracker inspector at `width` columns: the primary statement, then each
/// action, then each reported fact. A failure that is not a quota matter shows its one
/// diagnostic. The route to every blocker is the scroll, never a truncation.
fn tracker_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let primary = Style::default().fg(FOREGROUND);
    if let Some(notice) = &app.tracker_notice {
        push_wrapped(&mut lines, "", &notice.summary, width, primary);
        for action in &notice.actions {
            push_wrapped(&mut lines, "Action: ", action.text(), width, primary);
        }
        for fact in &notice.facts {
            push_wrapped(&mut lines, "• ", fact, width, primary);
        }
    } else if let Some(error) = &app.tracker.error {
        push_wrapped(&mut lines, "", error, width, primary);
    } else {
        lines.push(Line::from(muted("No tracker failure or hold to show.")));
    }
    lines
}

/// Draws the tracker inspector over `area` and returns its scroll clamped to the wrapped text.
fn render_tracker(frame: &mut Frame, app: &App, scroll: u16, area: Rect) -> u16 {
    let visible = usize::from(area.height.saturating_sub(2));
    if visible == 0 || area.width < 3 {
        return scroll;
    }
    let lines = tracker_lines(app, usize::from(area.width.saturating_sub(2)));
    let max = u16::try_from(lines.len().saturating_sub(visible)).unwrap_or(u16::MAX);
    let scroll = scroll.min(max);
    let title = Line::from(vec![
        Span::styled("Tracker", Style::default().fg(FOREGROUND)),
        muted(" · "),
        Span::styled("e", Style::default().fg(PRIMARY)),
        muted("/"),
        Span::styled("Esc", Style::default().fg(PRIMARY)),
        muted(" close · "),
        Span::styled("Up/Down/PgUp/PgDn", Style::default().fg(PRIMARY)),
        muted(" scroll"),
    ]);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(MUTED))
                .title(title),
        ),
        area,
    );
    scroll
}

/// Draws the open popup inside `area`, clamping its highlight or scroll to the current size.
pub(crate) fn render_popup(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.popup {
        Some(Popup::Dropdown { highlight, offset }) => {
            let (highlight, offset) = render_dropdown(frame, app.view, highlight, offset, area);
            app.popup = Some(Popup::Dropdown { highlight, offset });
        }
        Some(Popup::Help { scroll }) => {
            let scroll = render_help(frame, app.view, scroll, area);
            app.popup = Some(Popup::Help { scroll });
        }
        Some(Popup::Tracker { scroll }) => {
            let scroll = render_tracker(frame, app, scroll, area);
            app.popup = Some(Popup::Tracker { scroll });
        }
        None => {}
    }
}

fn render_dropdown(
    frame: &mut Frame,
    current: View,
    highlight: usize,
    offset: usize,
    area: Rect,
) -> (usize, usize) {
    let total = View::ALL.len();
    let highlight = highlight.min(total - 1);
    let height = u16::try_from(total + 2).unwrap_or(8).min(area.height);
    let width = DROPDOWN_WIDTH.min(area.width);
    let visible = usize::from(height.saturating_sub(2));
    if visible == 0 || width < 3 {
        return (highlight, 0);
    }
    let mut offset = offset.min(total - visible);
    if highlight < offset {
        offset = highlight;
    } else if highlight >= offset + visible {
        offset = highlight + 1 - visible;
    }
    let rect = Rect::new(area.x, area.y, width, height);
    let lines = View::ALL
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible)
        .map(|(index, view)| {
            let selected = index == highlight;
            let mut spans = vec![
                muted(if selected { "› " } else { "  " }),
                Span::styled(view.key().to_string(), Style::default().fg(PRIMARY)),
                muted("  "),
                muted(view.label()),
            ];
            if *view == current {
                spans.push(muted(" (current)"));
            }
            let line = Line::from(spans);
            if selected {
                line.style(Style::default().bg(SELECTION))
            } else {
                line
            }
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled("View", Style::default().fg(FOREGROUND))),
        ),
        rect,
    );
    (highlight, offset)
}

fn render_help(frame: &mut Frame, view: View, scroll: u16, area: Rect) -> u16 {
    let lines = help_lines(view);
    let visible = usize::from(area.height.saturating_sub(2));
    if visible == 0 || area.width < 3 {
        return scroll.min(last_help_line());
    }
    let max = u16::try_from(lines.len().saturating_sub(visible)).unwrap_or(u16::MAX);
    let scroll = scroll.min(max);
    let title = Line::from(vec![
        Span::styled("Help", Style::default().fg(FOREGROUND)),
        muted(" · "),
        Span::styled("F1", Style::default().fg(PRIMARY)),
        muted("/"),
        Span::styled("?", Style::default().fg(PRIMARY)),
        muted("/"),
        Span::styled("Esc", Style::default().fg(PRIMARY)),
        muted(" close · "),
        Span::styled("Up/Down/PgUp/PgDn", Style::default().fg(PRIMARY)),
        muted(" scroll"),
    ]);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(MUTED))
                .title(title),
        ),
        area,
    );
    scroll
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        monitor::{DepartmentInput, handle_key},
        monitor_ops::quota::fixture,
        monitor_ui::tests::{app, flat, screen},
    };
    use ix_trace_rs::trace;
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyModifiers};

    fn press(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn text_of(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[trace("FR-065-AC-1")]
    #[test]
    fn dropdown_lists_every_view_and_selects_like_the_compatibility_key() {
        for (width, height) in [(80, 24), (120, 40)] {
            for start in View::ALL {
                let mut app = app();
                app.view = start;
                press(&mut app, KeyCode::Char('v'));
                let drawn = screen(&mut app, width, height);
                assert!(drawn.contains("View: "), "{drawn}");
                assert!(drawn.contains("(current)"), "{drawn}");
                for entry in [
                    "t  Tickets",
                    "a  Agents",
                    "p  Scope",
                    "1  Status",
                    "2  Health",
                    "3  Retro",
                ] {
                    assert!(drawn.contains(entry), "{entry}\n{drawn}");
                }
                assert_eq!(
                    app.popup,
                    Some(Popup::Dropdown {
                        highlight: View::ALL.iter().position(|v| *v == start).unwrap(),
                        offset: 0
                    })
                );
            }
        }
        // Selection and scroll reset exactly as the compatibility key does; scope is kept.
        let mut keyed = app();
        let mut picked = app();
        for app in [&mut keyed, &mut picked] {
            app.table.select(Some(2));
            app.detail_scroll = 3;
            app.row_count = 5;
            app.team = Some("SWM".into());
            app.projects = vec!["native-project".into()];
            app.show_completed = true;
            app.department_input = DepartmentInput::Configured;
        }
        press(&mut keyed, KeyCode::Char('2'));
        press(&mut picked, KeyCode::Char('v'));
        for _ in 0..4 {
            press(&mut picked, KeyCode::Down);
        }
        press(&mut picked, KeyCode::Up);
        press(&mut picked, KeyCode::Char('j'));
        press(&mut picked, KeyCode::Char('k'));
        // Highlight path: 0 -> 4 (Health) -> 3 -> 4 -> 3 -> 4 again with `j`.
        press(&mut picked, KeyCode::Char('j'));
        assert_eq!(
            picked.popup,
            Some(Popup::Dropdown {
                highlight: 4,
                offset: 0
            })
        );
        press(&mut picked, KeyCode::Enter);
        assert_eq!(picked.view, View::Health);
        assert_eq!(picked.view, keyed.view);
        assert_eq!(picked.table.selected(), keyed.table.selected());
        assert_eq!(picked.detail_scroll, keyed.detail_scroll);
        assert_eq!(picked.row_count, keyed.row_count);
        assert_eq!(picked.table.selected(), None);
        assert_eq!(picked.detail_scroll, 0);
        assert_eq!(picked.team.as_deref(), Some("SWM"));
        assert_eq!(picked.projects, ["native-project"]);
        assert!(picked.show_completed);
        assert_eq!(picked.department_input, DepartmentInput::Configured);
        assert!(picked.popup.is_none());
        // Selecting the current view changes nothing else.
        picked.table.select(Some(1));
        press(&mut picked, KeyCode::Char('v'));
        press(&mut picked, KeyCode::Enter);
        assert_eq!(picked.view, View::Health);
        assert_eq!(picked.table.selected(), Some(1));
        // Health and Retro are selectable with no data at all.
        for (downs, view) in [(4, View::Health), (5, View::Retro)] {
            let mut empty = App::default();
            press(&mut empty, KeyCode::Char('v'));
            for _ in 0..downs {
                press(&mut empty, KeyCode::Down);
            }
            press(&mut empty, KeyCode::Enter);
            assert_eq!(empty.view, view);
            let drawn = screen(&mut empty, 80, 24);
            assert!(
                drawn.contains(&format!("View: {}", view.label())),
                "{drawn}"
            );
            assert!(flat(&drawn).contains("Not configured"), "{drawn}");
        }
    }

    #[trace("FR-065-AC-2")]
    #[test]
    fn cancel_preserves_state_clamps_and_never_quits() {
        for closer in [KeyCode::Esc, KeyCode::Char('v')] {
            let mut app = app();
            app.view = View::Scope;
            app.details = true;
            app.show_completed = true;
            app.table.select(Some(1));
            app.detail_scroll = 2;
            press(&mut app, KeyCode::Char('v'));
            for _ in 0..9 {
                press(&mut app, KeyCode::Down);
            }
            assert_eq!(
                app.popup,
                Some(Popup::Dropdown {
                    highlight: 5,
                    offset: 0
                })
            );
            for _ in 0..9 {
                press(&mut app, KeyCode::Up);
            }
            assert_eq!(
                app.popup,
                Some(Popup::Dropdown {
                    highlight: 0,
                    offset: 0
                })
            );
            press(&mut app, closer);
            assert!(app.popup.is_none() && !app.should_quit);
            assert_eq!(app.view, View::Scope);
            assert_eq!(app.table.selected(), Some(1));
            assert_eq!(app.detail_scroll, 2);
            assert!(app.details && app.show_completed);
        }
        let mut app = app();
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit, "Escape with no popup still quits");
    }

    #[trace("FR-065-AC-2")]
    #[test]
    fn a_short_work_area_scrolls_the_dropdown_and_resize_keeps_it_open() {
        let mut app = app();
        press(&mut app, KeyCode::Char('v'));
        for _ in 0..5 {
            press(&mut app, KeyCode::Down);
        }
        let tall = screen(&mut app, 120, 40);
        assert!(tall.contains("Retro"), "{tall}");
        let short = screen(&mut app, 80, 12);
        assert!(short.contains("Retro"), "highlight stays visible\n{short}");
        let Some(Popup::Dropdown { highlight, offset }) = app.popup else {
            panic!("dropdown stays open across a resize");
        };
        assert_eq!(highlight, 5);
        assert!(offset > 0 && highlight < offset + 6);
        let drawn = screen(&mut app, 80, 24);
        for view in View::ALL {
            assert!(drawn.contains(view.label()), "{drawn}");
        }
        assert_eq!(
            app.popup,
            Some(Popup::Dropdown {
                highlight: 5,
                offset: 0
            })
        );
    }

    /// The Help table's views, written out: one row per key, one column per view in the
    /// order Tickets, Agents, Scope, Status, Health, Retro.
    const APPLIES: [(&str, [bool; 6]); 11] = [
        ("v", [true; 6]),
        ("t, a, p, 1, 2, 3", [true; 6]),
        ("Up, Down, j, k, PageUp, PageDown", [true; 6]),
        ("d, Enter", [true; 6]),
        ("Left, Right", [false, false, false, true, true, true]),
        ("r", [true; 6]),
        ("s", [true, true, true, false, false, false]),
        ("e", [true; 6]),
        ("c", [false, false, true, false, false, false]),
        ("q, Escape", [true; 6]),
        ("F1, ?", [true; 6]),
    ];

    #[trace("FR-065-AC-3")]
    #[test]
    fn help_marks_exactly_the_keys_that_apply_in_each_view() {
        for (column, view) in View::ALL.into_iter().enumerate() {
            let expected: Vec<&str> = APPLIES
                .iter()
                .filter(|(_, applies)| applies[column])
                .map(|(keys, _)| *keys)
                .collect();
            assert_eq!(applicable_keys(view), expected, "{view:?}");
            // The drawn marks agree with the literal table, row by row.
            let lines = help_lines(view);
            assert_eq!(lines.len(), 1 + APPLIES.len(), "{view:?}: one row per key");
            for (line, (keys, applies)) in lines[1..].iter().zip(APPLIES) {
                let text = text_of(line);
                let mark = if applies[column] { "✓ " } else { "  " };
                assert!(
                    text.starts_with(&format!("{mark}{keys}  ")),
                    "{view:?}: {text:?}"
                );
            }
        }
    }

    #[trace("FR-065-AC-3")]
    #[test]
    fn help_rows_state_views_only_where_a_key_does_not_apply_everywhere() {
        let rows: Vec<String> = help_lines(View::Tickets)[1..].iter().map(text_of).collect();
        let expected = [
            "✓ v  open the View selector",
            "✓ t, a, p, 1, 2, 3  switch view",
            "✓ Up, Down, j, k, PageUp, PageDown  select rows and page",
            "✓ d, Enter  show or hide details",
            "  Left, Right  scroll details · Status, Health, Retro (with details open)",
            "✓ r  refresh sources",
            "✓ s  show source timestamps · Tickets, Agents, Scope",
            "✓ e  show source errors and tracker quota detail",
            "  c  show or hide completed projects · Scope",
            "✓ q, Escape  quit when no popup is open",
            "✓ F1, ?  open or close this help",
        ];
        assert_eq!(rows, expected);
    }

    #[trace("FR-065-AC-3")]
    #[test]
    fn every_help_line_is_reachable_and_the_frame_shows_no_requirement_text() {
        for opener in [KeyCode::F(1), KeyCode::Char('?')] {
            for closer in [KeyCode::F(1), KeyCode::Char('?'), KeyCode::Esc] {
                let mut app = app();
                app.table.select(Some(1));
                press(&mut app, opener);
                assert!(matches!(app.popup, Some(Popup::Help { .. })));
                press(&mut app, closer);
                assert!(app.popup.is_none() && !app.should_quit);
                assert_eq!(app.table.selected(), Some(1));
            }
        }
        for view in View::ALL {
            let lines: Vec<String> = help_lines(view).iter().map(text_of).collect();
            let last = lines.last().unwrap().clone();
            assert_eq!(last, "✓ F1, ?  open or close this help");
            let mut app = app();
            app.view = view;
            press(&mut app, KeyCode::F(1));
            // At 80x24 the whole help fits, so every line is on the first frame.
            let wide = flat(&screen(&mut app, 80, 24));
            for line in &lines {
                assert!(wide.contains(&flat(line)), "{view:?}: {line}\n{wide}");
            }
            // In a short pane the first frame cuts the end off; paging reaches the last line.
            let mut app = crate::monitor_ui::tests::app();
            app.view = view;
            press(&mut app, KeyCode::F(1));
            let first = flat(&screen(&mut app, 80, 12));
            assert!(first.contains(&flat(&lines[0])), "{view:?}\n{first}");
            assert!(!first.contains(&flat(&last)), "{view:?}\n{first}");
            for _ in 0..4 {
                press(&mut app, KeyCode::PageDown);
            }
            let end = flat(&screen(&mut app, 80, 12));
            assert!(end.contains(&flat(&last)), "{view:?}\n{end}");
            let Some(Popup::Help { scroll }) = app.popup else {
                panic!("help stays open");
            };
            assert!(scroll > 0);
            // A resize re-lays help out, keeps it open and clamps the scroll to the new range.
            let tall = flat(&screen(&mut app, 120, 40));
            assert!(tall.contains(&flat(&lines[0])) && tall.contains(&flat(&last)));
            assert_eq!(app.popup, Some(Popup::Help { scroll: 0 }));
            // The operator-facing text carries no requirement ids, owners or repeated "all".
            let everything = format!("{} {tall} {wide} {end}", lines.join(" "));
            for forbidden in ["FR-", "requirement", "owner", "Views:", "named focus"] {
                assert!(!everything.contains(forbidden), "{view:?}: {forbidden}");
            }
            assert!(!lines.iter().any(|line| line.ends_with(" · all")));
        }
    }

    #[trace("FR-065-AC-4")]
    #[test]
    fn a_popup_ignores_every_key_it_does_not_own_and_q_does_not_quit() {
        let codes = [
            KeyCode::Char('q'),
            KeyCode::Char('r'),
            KeyCode::Char('d'),
            KeyCode::Enter,
            KeyCode::Char('s'),
            KeyCode::Char('e'),
            KeyCode::Char('c'),
            KeyCode::Char('t'),
            KeyCode::Char('a'),
            KeyCode::Char('p'),
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Char('3'),
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::PageDown,
            KeyCode::PageUp,
            KeyCode::F(1),
            KeyCode::Char('?'),
            KeyCode::Char('v'),
        ];
        for open in [KeyCode::Char('v'), KeyCode::F(1), KeyCode::Char('e')] {
            for code in codes {
                let mut app = app();
                app.view = View::Scope;
                app.table.select(Some(1));
                app.detail_scroll = 4;
                press(&mut app, open);
                let popup = app.popup.unwrap();
                let owned = match popup {
                    Popup::Dropdown { .. } => matches!(
                        code,
                        KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::Enter
                            | KeyCode::Char('j' | 'k' | 'v')
                    ),
                    Popup::Help { .. } => matches!(
                        code,
                        KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                            | KeyCode::F(1)
                            | KeyCode::Char('?')
                    ),
                    Popup::Tracker { .. } => matches!(
                        code,
                        KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                            | KeyCode::Char('e')
                    ),
                };
                if owned {
                    continue;
                }
                press(&mut app, code);
                assert_eq!(app.popup, Some(popup), "{popup:?} {code:?}");
                assert!(!app.should_quit && !app.refresh_now && !app.show_status);
                assert!(!app.show_diagnostics && !app.details && !app.show_completed);
                assert_eq!(app.view, View::Scope);
                assert_eq!(app.table.selected(), Some(1));
                assert_eq!(app.detail_scroll, 4);
            }
        }
    }

    /// The dashboard after one refresh the service typed as refused with `decision`, keeping
    /// its snapshot.
    fn hold(app: &mut App, decision: ix_board::quota::QuotaDecision, now: std::time::Instant) {
        let snapshot = app.tracker.value.as_ref().unwrap().clone();
        let read = crate::monitor_ops::TrackerRead {
            snapshot,
            freshness: Some(ix_board::CacheFreshness {
                last_successful_sync: None,
                last_full_reconciliation: None,
                stale: true,
                age_seconds: Some(120),
                rate_limit_until: None,
                failure_reason: Some("transport detail".into()),
            }),
            outcome: ix_board::RefreshOutcome::QuotaRefused { decision },
        };
        app.apply_tracker_refresh(Ok(read), now);
    }

    /// One frame at a fixed instant, so a countdown row is the same in every frame.
    fn draw(app: &mut App, now: std::time::Instant, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::monitor_ui::render(frame, app, now))
            .unwrap();
        crate::monitor_ui::tests::frame_text(&terminal)
    }

    #[trace("FR-048-AC-5", "FR-065-AC-3")]
    #[test]
    fn the_tracker_inspector_opens_from_each_lead_view_and_help_names_its_key() {
        for view in [View::Tickets, View::Agents, View::Scope] {
            let mut app = app();
            app.view = view;
            app.table.select(Some(1));
            hold(
                &mut app,
                fixture::mixed_backoff(),
                std::time::Instant::now(),
            );
            for closer in [KeyCode::Char('e'), KeyCode::Esc] {
                press(&mut app, KeyCode::Char('e'));
                assert_eq!(app.popup, Some(Popup::Tracker { scroll: 0 }), "{view:?}");
                press(&mut app, closer);
                assert!(app.popup.is_none() && !app.should_quit);
                assert_eq!(app.view, view);
                assert_eq!(app.table.selected(), Some(1));
                assert!(!app.show_diagnostics);
            }
            let help: Vec<String> = help_lines(view).iter().map(text_of).collect();
            assert!(
                help.iter()
                    .any(|row| row.starts_with("✓ e  ") && row.contains("tracker quota detail")),
                "{view:?}: {help:?}"
            );
        }
    }

    /// Every blocker, value, the attempt, the earliest reevaluation and each action of a held
    /// read is on screen at 100 columns and a short pane by scrolling, and closing restores
    /// the Scope view as it was.
    #[trace("FR-048-AC-5", "FR-065-AC-11")]
    #[test]
    fn the_inspector_reaches_every_blocker_and_action_by_scrolling_and_closing_restores_scope() {
        let mut app = app();
        // A second scoped project gives Scope two visible rows, so selection 1 is valid and
        // the draw before the popup does not clamp it.
        let snapshot = app.tracker.value.as_mut().unwrap();
        let mut other = snapshot.projects[0].clone();
        other.id = ix_board::ProjectId("other".into());
        other.name = "Other".into();
        snapshot.projects.push(other);
        let mut extra = snapshot.issues[2].clone();
        extra.id = ix_board::IssueId("extra".into());
        extra.identifier = "SWM-99".into();
        extra.project_id = Some(ix_board::ProjectId("other".into()));
        snapshot.issues.push(extra);
        app.report
            .value
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .scope
            .as_mut()
            .unwrap()
            .projects
            .push("other".into());
        app.view = View::Scope;
        app.table.select(Some(1));
        app.show_completed = true;
        let now = std::time::Instant::now();
        let rows = draw(&mut app, now, 100, 24);
        assert!(rows.contains("Product") && rows.contains("Other"), "{rows}");
        assert_eq!(app.table.selected(), Some(1), "valid before the popup");
        hold(&mut app, fixture::mixed_backoff(), now);
        let before = draw(&mut app, now, 100, 14);
        assert!(before.contains("Tracker rate-limited (next attempt in 30m 00s)"));
        press(&mut app, KeyCode::Char('e'));
        let first = flat(&draw(&mut app, now, 100, 14));
        assert!(first.contains("Linear rate limited; showing the last snapshot"));
        assert!(first.contains("Action: Ask the quota owner to reconcile"));

        let mut reached = String::new();
        for _ in 0..24 {
            reached.push_str(&flat(&draw(&mut app, now, 100, 14)));
            reached.push(' ');
            press(&mut app, KeyCode::Down);
        }
        for shown in [
            "• attempt graphql with estimated complexity 10000",
            "• 3 pre-fence reservations are pending reconciliation by the quota owner",
            "• earliest reevaluation 1m 30s after the attempt; admission is not guaranteed",
            "• hold: server backoff (reevaluate after 1m 30s)",
            "• hold: evidence unavailable (no reevaluation time)",
            "• hold: local allocation held requests (capacity 8, charge 2, local usage 7,",
            // The last hold wraps: its end is on the next row, with the reevaluation time.
            "reevaluate after 33m",
            "20s)",
        ] {
            assert!(reached.contains(shown), "{shown}\n{reached}");
        }
        for _ in 0..24 {
            press(&mut app, KeyCode::Up);
        }
        assert_eq!(flat(&draw(&mut app, now, 100, 14)), first);

        press(&mut app, KeyCode::Esc);
        assert_eq!(draw(&mut app, now, 100, 14), before);
        assert_eq!(app.view, View::Scope);
        assert_eq!(app.table.selected(), Some(1));
        assert!(app.show_completed && !app.should_quit);
    }

    /// The inspector acts only on its keys, leaks none below it, and redraws from the
    /// notice as sources update while it is open.
    #[trace("FR-048-AC-5", "FR-065-AC-4", "FR-065-AC-11")]
    #[test]
    fn the_inspector_owns_its_keys_and_follows_source_updates() {
        let mut app = app();
        app.view = View::Scope;
        app.table.select(Some(1));
        hold(&mut app, fixture::local_held(), std::time::Instant::now());
        press(&mut app, KeyCode::Char('e'));
        for code in "rqvtapcsdj1".chars().map(KeyCode::Char) {
            press(&mut app, code);
            assert_eq!(app.popup, Some(Popup::Tracker { scroll: 0 }), "{code:?}");
            assert!(!app.should_quit && !app.refresh_now && !app.show_status);
            assert!(!app.details && !app.show_completed && !app.show_diagnostics);
            assert_eq!((app.view, app.table.selected()), (View::Scope, Some(1)));
        }
        assert!(flat(&screen(&mut app, 100, 24)).contains("Linear local allocation held"));

        hold(&mut app, fixture::reserve_held(), std::time::Instant::now());
        let updated = flat(&screen(&mut app, 100, 24));
        assert!(updated.contains("Linear server reserve held"), "{updated}");
        assert!(
            !updated.contains("Linear local allocation held"),
            "{updated}"
        );

        let snapshot = app.tracker.value.as_ref().unwrap().clone();
        app.apply_tracker_refresh(
            Ok(crate::monitor_ops::TrackerRead::unreported(snapshot)),
            std::time::Instant::now(),
        );
        assert!(matches!(app.popup, Some(Popup::Tracker { .. })));
        let recovered = flat(&screen(&mut app, 100, 24));
        assert!(recovered.contains("No tracker failure or hold to show."));
    }

    /// A failure that is not a quota matter shows its one redacted diagnostic.
    #[trace("FR-059-AC-6")]
    #[test]
    fn the_inspector_shows_the_one_diagnostic_of_an_authentication_failure() {
        let mut app = app();
        app.apply_tracker_refresh(
            Err(ix_board::Error::MissingApiKey.into()),
            std::time::Instant::now(),
        );
        press(&mut app, KeyCode::Char('e'));
        let frame = screen(&mut app, 100, 24);
        // The popup wraps the command between rows, and each row sits between the popup's
        // side borders, so the body is the text inside the borders, joined and normalized.
        let body = flat(
            &frame
                .lines()
                .filter_map(|row| {
                    let (first, last) = (row.find('│')?, row.rfind('│')?);
                    (first < last).then(|| &row[first + '│'.len_utf8()..last])
                })
                .collect::<Vec<_>>()
                .join(" "),
        );
        assert!(body.contains("ix-board auth login"), "{body}\n{frame}");
        assert!(!frame.contains("LINEAR_API_KEY"), "{frame}");
    }

    /// In the department views `e` keeps toggling the source error list and opens no popup.
    #[trace("FR-059-AC-5", "FR-065-AC-3")]
    #[test]
    fn in_the_department_views_e_still_lists_source_errors() {
        for view in [View::Status, View::Health, View::Retro] {
            let mut app = app();
            app.view = view;
            press(&mut app, KeyCode::Char('e'));
            assert!(app.popup.is_none() && app.show_diagnostics, "{view:?}");
            press(&mut app, KeyCode::Char('e'));
            assert!(!app.show_diagnostics, "{view:?}");
        }
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_splits_a_word_wider_than_a_row() {
        assert_eq!(wrap_text("alpha beta gamma", 10), ["alpha beta", "gamma"]);
        assert_eq!(wrap_text("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
        assert_eq!(wrap_text("", 5), [""]);
        assert_eq!(wrap_text("a  b", 4), ["a b"]);
    }
}
