// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The one View selector dropdown and the F1 help popup of the dashboard (FR-065).
//!
//! Popups own their keys exclusively while open, draw only inside the work area and
//! read no source: they only present and change in-memory view state.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    monitor::{App, Popup, View, select_view},
    monitor_ui::{FOREGROUND, MUTED, PRIMARY, SELECTION},
};

/// Outer width of the dropdown, borders included.
const DROPDOWN_WIDTH: u16 = 26;
/// Lines moved by PageUp and PageDown in help.
const PAGE: u16 = 5;
/// Lines before the first help entry.
const HELP_HEADER_LINES: usize = 3;

/// One row of the help table (FR-065): keys, action, owner and where it has a visible effect.
struct HelpEntry {
    keys: &'static str,
    action: &'static str,
    owner: &'static str,
    views: &'static [View],
    note: &'static str,
}

const ALL_VIEWS: &[View] = &View::ALL;
const LEAD_VIEWS: &[View] = &[View::Tickets, View::Agents, View::Scope];
const DEPARTMENT_VIEWS: &[View] = &[View::Status, View::Health, View::Retro];

const HELP: [HelpEntry; 11] = [
    HelpEntry {
        keys: "v",
        action: "View selector",
        owner: "FR-065",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "t, a, p, 1, 2, 3",
        action: "view shortcuts",
        owner: "FR-048, FR-059",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "Up, Down, j, k, PageUp, PageDown",
        action: "row selection and paging",
        owner: "FR-048",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "d, Enter",
        action: "details",
        owner: "FR-048",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "Left, Right",
        action: "scroll details",
        owner: "FR-059",
        views: DEPARTMENT_VIEWS,
        note: "with details open",
    },
    HelpEntry {
        keys: "r",
        action: "refresh",
        owner: "FR-048",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "s",
        action: "source timestamps",
        owner: "FR-048",
        views: LEAD_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "e",
        action: "source errors",
        owner: "FR-059",
        views: DEPARTMENT_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "c",
        action: "completed projects",
        owner: "FR-060",
        views: &[View::Scope],
        note: "",
    },
    HelpEntry {
        keys: "q, Escape",
        action: "quit, when no popup is open",
        owner: "FR-048",
        views: ALL_VIEWS,
        note: "",
    },
    HelpEntry {
        keys: "F1, ?",
        action: "help",
        owner: "FR-065",
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

fn muted(text: impl Into<std::borrow::Cow<'static, str>>) -> Span<'static> {
    Span::styled(text, Style::default().fg(MUTED))
}

/// Every help line for `view`; entries that apply in it carry a check mark.
pub(crate) fn help_lines(view: View) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(muted(format!(
            "✓ marks the keys that apply in {}.",
            view.label()
        ))),
        Line::from(muted(
            "Each key names the requirement that owns its action.",
        )),
        Line::default(),
    ];
    for entry in &HELP {
        let applies = entry.views.contains(&view);
        lines.push(Line::from(vec![
            Span::styled(
                if applies { "✓ " } else { "  " },
                Style::default().fg(FOREGROUND),
            ),
            Span::styled(entry.keys, Style::default().fg(PRIMARY)),
            muted("  "),
            muted(entry.action),
            muted(format!(" ({})", entry.owner)),
        ]));
        let views = if entry.views.len() == View::ALL.len() {
            "all".to_owned()
        } else {
            entry
                .views
                .iter()
                .map(|v| v.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(Line::from(muted(if entry.note.is_empty() {
            format!("    Views: {views}")
        } else {
            format!("    Views: {views}, {}", entry.note)
        })));
    }
    lines
}

fn last_help_line() -> u16 {
    let count = HELP_HEADER_LINES + HELP.len() * 2;
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
    }
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
    use crate::monitor::handle_key;
    use ix_trace_rs::trace;
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyModifiers};

    fn press(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn draw(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::monitor_ui::render(frame, app, std::time::Instant::now()))
            .unwrap();
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

    /// Trace: FR-065-AC-1
    #[trace("FR-065-AC-1")]
    #[test]
    fn dropdown_lists_every_view_and_selects_like_the_compatibility_key() {
        for (width, height) in [(80, 24), (120, 40)] {
            for start in View::ALL {
                let mut app = crate::monitor_ui::tests::app();
                app.view = start;
                press(&mut app, KeyCode::Char('v'));
                let screen = draw(&mut app, width, height);
                for view in View::ALL {
                    assert!(screen.contains(view.label()), "{start:?}\n{screen}");
                }
                assert!(screen.contains("View: "), "{screen}");
                assert!(screen.contains("(current)"), "{screen}");
                for entry in [
                    "t  Tickets",
                    "a  Agents",
                    "p  Scope",
                    "1  Status",
                    "2  Health",
                    "3  Retro",
                ] {
                    assert!(screen.contains(entry), "{entry}\n{screen}");
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
        // Selection and scroll are reset exactly as the compatibility key does.
        let mut keyed = crate::monitor_ui::tests::app();
        let mut picked = crate::monitor_ui::tests::app();
        for app in [&mut keyed, &mut picked] {
            app.table.select(Some(2));
            app.detail_scroll = 3;
            app.row_count = 5;
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
        assert!(picked.popup.is_none());
        // Selecting the current view changes nothing else.
        picked.table.select(Some(1));
        press(&mut picked, KeyCode::Char('v'));
        press(&mut picked, KeyCode::Enter);
        assert_eq!(picked.view, View::Health);
        assert_eq!(picked.table.selected(), Some(1));
    }

    /// Trace: FR-065-AC-2
    #[trace("FR-065-AC-2")]
    #[test]
    fn cancel_preserves_state_clamps_and_never_quits() {
        for closer in [KeyCode::Esc, KeyCode::Char('v')] {
            let mut app = crate::monitor_ui::tests::app();
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
        let mut app = crate::monitor_ui::tests::app();
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit, "Escape with no popup still quits");
    }

    /// Trace: FR-065-AC-2
    #[trace("FR-065-AC-2")]
    #[test]
    fn a_short_work_area_scrolls_the_dropdown_and_resize_keeps_it_open() {
        let mut app = crate::monitor_ui::tests::app();
        press(&mut app, KeyCode::Char('v'));
        // 24 rows leave room for all six entries; a short pane must scroll to the last.
        for _ in 0..5 {
            press(&mut app, KeyCode::Down);
        }
        let tall = draw(&mut app, 120, 40);
        assert!(tall.contains("Retro"), "{tall}");
        let short = draw(&mut app, 80, 12);
        assert!(short.contains("Retro"), "highlight stays visible\n{short}");
        let Some(Popup::Dropdown { highlight, offset }) = app.popup else {
            panic!("dropdown stays open across a resize");
        };
        assert_eq!(highlight, 5);
        assert!(offset <= highlight && highlight < offset + 6);
        let screen = draw(&mut app, 80, 24);
        for view in View::ALL {
            assert!(screen.contains(view.label()), "{screen}");
        }
    }

    /// Trace: FR-065-AC-3
    #[trace("FR-065-AC-3")]
    #[test]
    fn help_marks_exactly_the_keys_that_apply_in_each_view() {
        let table = [
            (View::Tickets, "s", true),
            (View::Agents, "s", true),
            (View::Scope, "s", true),
            (View::Status, "s", false),
            (View::Health, "e", true),
            (View::Tickets, "e", false),
            (View::Scope, "c", true),
            (View::Retro, "c", false),
            (View::Status, "Left, Right", true),
            (View::Scope, "Left, Right", false),
        ];
        for (view, keys, applies) in table {
            assert_eq!(
                applicable_keys(view).contains(&keys),
                applies,
                "{view:?} {keys}"
            );
        }
        for view in View::ALL {
            let applicable = applicable_keys(view);
            for always in [
                "v",
                "t, a, p, 1, 2, 3",
                "r",
                "d, Enter",
                "q, Escape",
                "F1, ?",
            ] {
                assert!(applicable.contains(&always), "{view:?} {always}");
            }
            let marked = help_lines(view)
                .iter()
                .filter(|line| line.spans.first().is_some_and(|s| s.content == "✓ "))
                .count();
            assert_eq!(marked, applicable.len(), "{view:?}");
        }
    }

    /// Trace: FR-065-AC-3
    #[trace("FR-065-AC-3")]
    #[test]
    fn every_help_line_is_reachable_at_80x24_and_f1_question_and_escape_close_it() {
        for opener in [KeyCode::F(1), KeyCode::Char('?')] {
            for closer in [KeyCode::F(1), KeyCode::Char('?'), KeyCode::Esc] {
                let mut app = crate::monitor_ui::tests::app();
                app.table.select(Some(1));
                press(&mut app, opener);
                assert!(matches!(app.popup, Some(Popup::Help { .. })));
                press(&mut app, closer);
                assert!(app.popup.is_none() && !app.should_quit);
                assert_eq!(app.table.selected(), Some(1));
            }
        }
        for view in View::ALL {
            let mut app = crate::monitor_ui::tests::app();
            app.view = view;
            press(&mut app, KeyCode::F(1));
            let mut seen = String::new();
            for _ in 0..40 {
                seen.push_str(&draw(&mut app, 80, 24));
                press(&mut app, KeyCode::PageDown);
            }
            for entry in &HELP {
                assert!(seen.contains(entry.keys), "{view:?} {}", entry.keys);
                assert!(seen.contains(entry.owner), "{view:?} {}", entry.owner);
            }
            assert!(!seen.contains("named focus"), "nothing about f is asserted");
            draw(&mut app, 80, 24);
            let Some(Popup::Help { scroll }) = app.popup else {
                panic!("help stays open");
            };
            // A resize re-lays the popup out with the scroll in range.
            let tall = draw(&mut app, 120, 40);
            assert!(tall.contains("Help"));
            let Some(Popup::Help { scroll: after }) = app.popup else {
                panic!("help stays open across a resize");
            };
            assert!(after <= scroll);
        }
    }

    /// Trace: FR-065-AC-4
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
        for open in [KeyCode::Char('v'), KeyCode::F(1)] {
            for code in codes {
                let mut app = crate::monitor_ui::tests::app();
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
}
