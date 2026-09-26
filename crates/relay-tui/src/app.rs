//! Pure state and update logic for the escalation inbox: the status filter,
//! the selection, new-arrival detection, and key handling. None of this
//! touches a terminal, Redis, or herdr — [`handle_key`] and
//! [`App::apply_refresh`] are plain functions over [`App`], unit-testable
//! without any of that (see this module's `tests`). The terminal (`ui.rs`)
//! and the background workers (`ops.rs`) are the only parts of this crate
//! that do I/O.

use std::collections::HashSet;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use relay_core::{Escalation, EscalationId, EscalationStatus};

/// Which escalations the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// Every status.
    All,
    /// Only [`EscalationStatus::Waiting`].
    Waiting,
    /// Only [`EscalationStatus::Investigating`].
    Investigating,
    /// Only [`EscalationStatus::Resolved`].
    Resolved,
}

impl Filter {
    /// Whether `status` belongs in this filter's view.
    #[must_use]
    pub fn matches(self, status: EscalationStatus) -> bool {
        match self {
            Self::All => true,
            Self::Waiting => status == EscalationStatus::Waiting,
            Self::Investigating => status == EscalationStatus::Investigating,
            Self::Resolved => status == EscalationStatus::Resolved,
        }
    }

    /// What `tab` cycles to next: All -> Waiting -> Investigating -> Resolved -> All.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Waiting,
            Self::Waiting => Self::Investigating,
            Self::Investigating => Self::Resolved,
            Self::Resolved => Self::All,
        }
    }

    /// A short label for the header.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Waiting => "waiting",
            Self::Investigating => "investigating",
            Self::Resolved => "resolved",
        }
    }
}

/// What the user is doing right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Browsing the list.
    Normal,
    /// Typing a decision for `id`, to resolve it on Enter.
    ResolveInput {
        /// The escalation being resolved.
        id: EscalationId,
        /// What's been typed so far.
        buffer: String,
    },
}

/// A background operation the UI asked for. Dispatched to `ops.rs`'s worker
/// thread; never run on the UI thread (see this crate's module doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `i` (fork) or `I` (fresh).
    Investigate {
        /// The escalation id.
        id: EscalationId,
        /// `true` for a fresh session (`I`); `false` to fork (`i`).
        fresh: bool,
    },
    /// `r`, after Enter on the decision input.
    Resolve {
        /// The escalation id.
        id: EscalationId,
        /// The typed decision.
        decision: String,
    },
    /// `p`.
    PostLinear {
        /// The escalation id.
        id: EscalationId,
    },
    /// Fired once per newly arrived waiting escalation, when notifications
    /// are enabled (`--notify`).
    Notify {
        /// The escalation id.
        id: EscalationId,
        /// Its ticket, for the notification text.
        ticket: String,
        /// Its lead, for the notification text.
        lead: String,
    },
}

/// A poll result from the background poller: the escalations that decoded
/// cleanly, and how many entries were dropped as corrupt (see
/// [`relay_core::EscalationList::rejected`]).
#[derive(Debug, Clone, Default)]
pub struct RefreshData {
    /// Every escalation, oldest first (as [`relay_core::Relay::list`] returns them).
    pub escalations: Vec<Escalation>,
    /// How many entries failed to decode and were dropped.
    pub rejected_count: usize,
}

/// The escalation inbox's whole state.
pub struct App {
    /// Every escalation from the last successful refresh, newest first.
    escalations: Vec<Escalation>,
    /// The current status filter.
    pub filter: Filter,
    /// The selected escalation's id, stable across refreshes and filter
    /// changes as long as it's still in view.
    selected_id: Option<EscalationId>,
    /// Ids to highlight as newly arrived, until the user selects them.
    new_ids: HashSet<EscalationId>,
    /// Every id ever seen, so a refresh can tell a genuinely new arrival
    /// from one already known.
    seen_ids: HashSet<EscalationId>,
    /// Whether [`App::apply_refresh`] has run at least once. The very first
    /// refresh seeds `seen_ids` from the existing backlog without flagging
    /// any of it as "new" — only escalations that arrive after the TUI is
    /// already running light up.
    primed: bool,
    /// What the user is doing right now.
    pub mode: Mode,
    /// The one-line status/progress/error message shown at the bottom.
    pub status: String,
    /// Whether to fire a herdr notification for each new waiting escalation.
    pub notify_on_new: bool,
    /// Escalation ids with an `i`/`I`/`r`/`p` action currently running on
    /// the background worker. Guards against dispatching a second action
    /// for the same id before the first one's result comes back.
    in_flight: HashSet<EscalationId>,
    /// Set by a first `q` while [`App::in_flight`] is non-empty: the next
    /// key either confirms the quit (another `q`) or, on any other key,
    /// cancels the pending confirmation.
    quit_confirm_pending: bool,
    /// Set once a background worker thread has died outright (not an
    /// ordinary action/poll failure, which shows in [`App::status`] instead
    /// — this is the ops.rs `UiEvent::WorkerDied` case). Once set, the
    /// footer shows this instead of `status`, permanently: nothing
    /// dispatched through that worker will ever complete again, so there is
    /// nothing more informative to show later.
    pub worker_died: Option<String>,
    /// Set by `q`; the event loop exits once this is true.
    pub should_quit: bool,
}

impl App {
    /// A fresh, empty inbox. `notify_on_new` mirrors the `--notify` flag.
    #[must_use]
    pub fn new(notify_on_new: bool) -> Self {
        Self {
            escalations: Vec::new(),
            filter: Filter::All,
            selected_id: None,
            new_ids: HashSet::new(),
            seen_ids: HashSet::new(),
            primed: false,
            mode: Mode::Normal,
            status: "connecting...".to_owned(),
            notify_on_new,
            in_flight: HashSet::new(),
            quit_confirm_pending: false,
            worker_died: None,
            should_quit: false,
        }
    }

    /// The escalations currently in view, newest first, per [`App::filter`].
    #[must_use]
    pub fn filtered(&self) -> Vec<&Escalation> {
        self.escalations
            .iter()
            .filter(|e| self.filter.matches(e.status))
            .collect()
    }

    /// The selected escalation, if any (there's always one selected escalation
    /// as long as the current filter's view is non-empty).
    #[must_use]
    pub fn selected(&self) -> Option<&Escalation> {
        let id = self.selected_id.as_ref()?;
        self.escalations.iter().find(|e| e.id == *id)
    }

    /// Whether `id` is a still-highlighted new arrival.
    #[must_use]
    pub fn is_new(&self, id: &EscalationId) -> bool {
        self.new_ids.contains(id)
    }

    /// Whether `id` has an `i`/`I`/`r`/`p` action currently running.
    #[must_use]
    pub fn is_in_flight(&self, id: &EscalationId) -> bool {
        self.in_flight.contains(id)
    }

    /// Marks `id` as having an action running on the background worker.
    fn mark_in_flight(&mut self, id: &EscalationId) {
        self.in_flight.insert(id.clone());
    }

    /// Marks `id`'s action as finished, freeing it for another `i`/`I`/`r`/`p`.
    /// A no-op if `id` was never in flight (e.g. the completion of an action
    /// this app instance never itself started, or a `--notify` action, which
    /// isn't guarded).
    pub fn clear_in_flight(&mut self, id: &EscalationId) {
        self.in_flight.remove(id);
    }

    /// Records that `worker` has died. Idempotent: if more than one worker
    /// dies, the footer keeps showing whichever was reported first (both
    /// are equally fatal to what that worker did, so there's nothing to
    /// gain from overwriting one report with another).
    pub fn mark_worker_died(&mut self, worker: &str, message: &str) {
        if self.worker_died.is_none() {
            self.worker_died = Some(format!("{worker} died: {message}, restart relay-tui"));
        }
    }

    /// `id`s currently in flight, joined for the status line (`"e1"`, or
    /// `"e1, e2"` when more than one action is running at once).
    fn in_flight_label(&self) -> String {
        let mut ids: Vec<&str> = self.in_flight.iter().map(EscalationId::as_str).collect();
        ids.sort_unstable();
        ids.join(", ")
    }

    /// Applies one poll result: stores the escalations (newest first),
    /// updates the selection and new-arrival set, and returns the
    /// escalations that just arrived as newly waiting (empty on the very
    /// first call — see [`App::primed`]), for the caller to fire
    /// `--notify` actions for.
    pub fn apply_refresh(&mut self, data: RefreshData) -> Vec<Escalation> {
        let mut escalations = data.escalations;
        // `Relay::list` returns oldest first; the inbox wants newest first.
        escalations.reverse();

        if !self.primed {
            for e in &escalations {
                self.seen_ids.insert(e.id.clone());
            }
            self.primed = true;
            self.escalations = escalations;
            self.ensure_selection();
            self.note_rejected(data.rejected_count);
            return Vec::new();
        }

        let mut new_arrivals = Vec::new();
        for e in &escalations {
            if e.status == EscalationStatus::Waiting && !self.seen_ids.contains(&e.id) {
                new_arrivals.push(e.clone());
                self.new_ids.insert(e.id.clone());
            }
            self.seen_ids.insert(e.id.clone());
        }
        self.escalations = escalations;
        self.ensure_selection();
        self.note_rejected(data.rejected_count);
        new_arrivals
    }

    fn note_rejected(&mut self, rejected_count: usize) {
        if rejected_count > 0 {
            self.status = format!("dropped {rejected_count} corrupt escalation entry/entries");
        }
    }

    /// Selects `id`, clearing its new-arrival highlight (the user has now
    /// looked at it).
    fn select(&mut self, id: Option<EscalationId>) {
        if let Some(id) = &id {
            self.new_ids.remove(id);
        }
        self.selected_id = id;
    }

    /// Keeps the current selection if it's still in [`App::filtered`]'s
    /// view; otherwise falls back to the first visible escalation, or none.
    ///
    /// Deliberately does *not* go through [`App::select`]: this runs after
    /// every refresh (including the very first one, and one where the
    /// fallback lands on an escalation that just arrived), and an automatic
    /// fallback is not the user looking at anything — only [`App::move_selection`]
    /// (a real `j`/`k`/arrow key) clears a new-arrival highlight.
    fn ensure_selection(&mut self) {
        let still_visible = self
            .selected_id
            .as_ref()
            .is_some_and(|sel| self.filtered().iter().any(|e| e.id == *sel));
        if still_visible {
            return;
        }
        self.selected_id = self.filtered().first().map(|e| e.id.clone());
    }

    /// Moves the selection by `delta` within the current filtered view
    /// (`1` for `j`/Down, `-1` for `k`/Up), clamped to the ends.
    fn move_selection(&mut self, delta: i32) {
        let ids: Vec<EscalationId> = self.filtered().iter().map(|e| e.id.clone()).collect();
        if ids.is_empty() {
            self.select(None);
            return;
        }
        let current = self
            .selected_id
            .as_ref()
            .and_then(|sel| ids.iter().position(|id| id == sel));
        // `ids` comes from `filtered()`, which is bounded by the number of
        // escalations the TUI holds — nowhere near i32::MAX — so these
        // conversions never truncate; `try_from` still keeps that an
        // explicit, checked fact rather than a silent `as` cast.
        let last = i32::try_from(ids.len()).unwrap_or(i32::MAX) - 1;
        let next_index = match current {
            Some(i) => {
                let i = i32::try_from(i).unwrap_or(i32::MAX);
                (i + delta).clamp(0, last)
            }
            None => 0,
        };
        let next_id = ids[usize::try_from(next_index).unwrap_or(0)].clone();
        self.select(Some(next_id));
    }

    /// Cycles the status filter and keeps the selection stable if the
    /// selected escalation is still visible under the new filter.
    fn cycle_filter(&mut self) {
        self.filter = self.filter.next();
        self.ensure_selection();
    }
}

/// Handles one key press against `app`, mutating its state in place and
/// returning the background [`Action`] it triggers, if any.
pub fn handle_key(app: &mut App, key: KeyEvent) -> Option<Action> {
    match &mut app.mode {
        Mode::ResolveInput { id, buffer } => match key.code {
            KeyCode::Enter => {
                let id = id.clone();
                let decision = buffer.clone();
                app.mode = Mode::Normal;
                if decision.trim().is_empty() {
                    app.status = "decision was empty; not resolved".to_owned();
                    None
                } else {
                    app.mark_in_flight(&id);
                    app.status = format!("resolving {id}… (can take up to 30s)");
                    Some(Action::Resolve { id, decision })
                }
            }
            KeyCode::Esc => {
                app.mode = Mode::Normal;
                app.status = "resolve cancelled".to_owned();
                None
            }
            KeyCode::Backspace => {
                buffer.pop();
                None
            }
            KeyCode::Char(c) => {
                // No control characters: this text is typed straight into a
                // Redis-stored decision and then into the lead's bus
                // message. Plain and Shift-modified characters only.
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                    buffer.push(c);
                }
                None
            }
            _ => None,
        },
        Mode::Normal => {
            // Only a second, immediate `q` confirms a pending quit; any
            // other key cancels the confirmation instead of leaving it
            // armed for some unrelated later `q`.
            if key.code != KeyCode::Char('q') {
                app.quit_confirm_pending = false;
            }
            match key.code {
                KeyCode::Char('q') => {
                    if !app.in_flight.is_empty() && !app.quit_confirm_pending {
                        app.quit_confirm_pending = true;
                        app.status = format!(
                            "action running for {}; press q again to quit anyway",
                            app.in_flight_label()
                        );
                    } else {
                        app.should_quit = true;
                    }
                    None
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    app.move_selection(1);
                    None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    app.move_selection(-1);
                    None
                }
                KeyCode::Tab => {
                    app.cycle_filter();
                    None
                }
                KeyCode::Char('i') => start_guarded(app, "forking", "2 min", |id| {
                    Action::Investigate { id, fresh: false }
                }),
                KeyCode::Char('I') => {
                    start_guarded(app, "starting a fresh session for", "2 min", |id| {
                        Action::Investigate { id, fresh: true }
                    })
                }
                KeyCode::Char('r') => {
                    if let Some(id) = selected_id(app) {
                        if app.is_in_flight(&id) {
                            app.status = format!("already running for {id}");
                        } else {
                            app.mode = Mode::ResolveInput {
                                id,
                                buffer: String::new(),
                            };
                        }
                    }
                    None
                }
                KeyCode::Char('p') => {
                    let resolved = app
                        .selected()
                        .is_some_and(|e| e.status == EscalationStatus::Resolved);
                    if resolved {
                        start_guarded(app, "posting to linear for", "30s", |id| {
                            Action::PostLinear { id }
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
    }
}

fn selected_id(app: &App) -> Option<EscalationId> {
    app.selected_id.clone()
}

/// Shared guard for `i`/`I`/`p`: refuses to dispatch a second action for an
/// id that already has one in flight (shows "already running for eN"
/// instead), and otherwise marks it in flight, sets a status line naming
/// what's starting and its rough worst-case time, and builds the action.
fn start_guarded(
    app: &mut App,
    verb: &str,
    budget: &str,
    build: impl FnOnce(EscalationId) -> Action,
) -> Option<Action> {
    let id = selected_id(app)?;
    if app.is_in_flight(&id) {
        app.status = format!("already running for {id}");
        return None;
    }
    app.mark_in_flight(&id);
    app.status = format!("{verb} {id}… (can take up to {budget})");
    Some(build(id))
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyEventKind;
    use relay_core::{EscalationFiling, TicketId};

    use super::*;

    fn eid(id: &str) -> EscalationId {
        id.parse().unwrap()
    }

    fn esc(id: &str, status: EscalationStatus, created_at_ms: i64) -> Escalation {
        Escalation {
            id: eid(id),
            filing: EscalationFiling {
                ticket: "AGE-2015".parse::<TicketId>().unwrap(),
                lead: "qsl-lead".parse().unwrap(),
                machine: "node-a".parse().unwrap(),
                session_id: "sid-1".parse().unwrap(),
                cwd: Some("/work".into()),
                kind: "claude".into(),
                context: "ctx".into(),
                options: vec!["a".into(), "b".into()],
                recommendation: "a".into(),
                created_at_ms,
            },
            status,
            decision: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn refresh(app: &mut App, escalations: Vec<Escalation>) -> Vec<Escalation> {
        app.apply_refresh(RefreshData {
            escalations,
            rejected_count: 0,
        })
    }

    // ---- filter -----------------------------------------------------

    #[test]
    fn filter_cycles_through_every_status_and_back_to_all() {
        let mut f = Filter::All;
        for expect in [
            Filter::Waiting,
            Filter::Investigating,
            Filter::Resolved,
            Filter::All,
        ] {
            f = f.next();
            assert_eq!(f, expect);
        }
    }

    #[test]
    fn tab_cycles_the_filter_and_keeps_a_still_visible_selection() {
        let mut app = App::new(false);
        refresh(
            &mut app,
            vec![
                esc("e1", EscalationStatus::Waiting, 1),
                esc("e2", EscalationStatus::Resolved, 2),
            ],
        );
        // Newest first: e2 selected initially.
        assert_eq!(app.selected().unwrap().id, "e2");

        assert!(handle_key(&mut app, key(KeyCode::Tab)).is_none());
        assert_eq!(app.filter, Filter::Waiting);
        // e2 isn't waiting, so the selection falls back to the only visible one.
        assert_eq!(app.selected().unwrap().id, "e1");
    }

    // ---- selection / movement ----------------------------------------

    #[test]
    fn j_and_k_move_selection_within_the_filtered_view_and_clamp_at_the_ends() {
        let mut app = App::new(false);
        refresh(
            &mut app,
            vec![
                esc("e1", EscalationStatus::Waiting, 1),
                esc("e2", EscalationStatus::Waiting, 2),
                esc("e3", EscalationStatus::Waiting, 3),
            ],
        );
        // Newest first: e3, e2, e1.
        assert_eq!(app.selected().unwrap().id, "e3");
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.selected().unwrap().id, "e2");
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.selected().unwrap().id, "e1");
        // Clamped: one more 'j' does nothing past the end.
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.selected().unwrap().id, "e1");
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.selected().unwrap().id, "e2");
        // Arrow keys do the same as j/k.
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.selected().unwrap().id, "e3");
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.selected().unwrap().id, "e2");
    }

    #[test]
    fn selection_stays_on_the_same_escalation_across_a_refresh_even_as_new_items_arrive() {
        let mut app = App::new(false);
        refresh(
            &mut app,
            vec![
                esc("e1", EscalationStatus::Waiting, 1),
                esc("e2", EscalationStatus::Waiting, 2),
            ],
        );
        handle_key(&mut app, key(KeyCode::Char('j'))); // select e1
        assert_eq!(app.selected().unwrap().id, "e1");

        // A new escalation arrives ahead of both, newest-first.
        refresh(
            &mut app,
            vec![
                esc("e1", EscalationStatus::Waiting, 1),
                esc("e2", EscalationStatus::Waiting, 2),
                esc("e3", EscalationStatus::Waiting, 3),
            ],
        );
        assert_eq!(
            app.selected().unwrap().id,
            "e1",
            "selection must not silently jump to whatever is now first"
        );
    }

    #[test]
    fn empty_filtered_view_has_no_selection_and_movement_is_a_no_op() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        app.filter = Filter::Resolved;
        app.ensure_selection();
        assert!(app.selected().is_none());
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert!(app.selected().is_none());
    }

    // ---- new-arrival detection -----------------------------------------

    #[test]
    fn the_first_refresh_seeds_the_backlog_without_flagging_any_arrivals() {
        let mut app = App::new(false);
        let new = refresh(
            &mut app,
            vec![
                esc("e1", EscalationStatus::Waiting, 1),
                esc("e2", EscalationStatus::Investigating, 2),
            ],
        );
        assert!(new.is_empty(), "startup backlog must not be treated as new");
        assert!(!app.is_new(&eid("e1")));
    }

    #[test]
    fn a_later_waiting_escalation_is_reported_as_a_new_arrival_and_highlighted() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        let new = refresh(
            &mut app,
            vec![
                esc("e1", EscalationStatus::Waiting, 1),
                esc("e2", EscalationStatus::Waiting, 2),
            ],
        );
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, "e2");
        assert!(app.is_new(&eid("e2")));
        assert!(!app.is_new(&eid("e1")));
    }

    /// Caught live (AGE-2015): when a new arrival is the only escalation to
    /// select (the prior selection just went out of view, or there was none
    /// yet), `ensure_selection`'s automatic fallback must not itself count
    /// as the user looking at it — only an explicit `j`/`k`/arrow move does.
    /// Before the fix, a solo new arrival's highlight vanished before it was
    /// ever rendered, because auto-selecting it ran through the same
    /// highlight-clearing path a deliberate move does.
    #[test]
    fn a_solo_new_arrival_that_gets_auto_selected_keeps_its_highlight() {
        let mut app = App::new(false);
        refresh(&mut app, vec![]); // primes with an empty backlog
        let new = refresh(&mut app, vec![esc("e9", EscalationStatus::Waiting, 9)]);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, "e9");
        assert_eq!(
            app.selected().unwrap().id,
            "e9",
            "auto-selected as the only escalation"
        );
        assert!(
            app.is_new(&eid("e9")),
            "auto-selection must not clear the highlight before it's ever been rendered"
        );
    }

    #[test]
    fn an_escalation_moving_to_investigating_is_never_reported_as_a_new_arrival() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        let new = refresh(
            &mut app,
            vec![esc("e1", EscalationStatus::Investigating, 1)],
        );
        assert!(new.is_empty());
    }

    #[test]
    fn selecting_a_new_arrival_clears_its_highlight() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        refresh(
            &mut app,
            vec![
                esc("e1", EscalationStatus::Waiting, 1),
                esc("e2", EscalationStatus::Waiting, 2),
            ],
        );
        assert!(app.is_new(&eid("e2")));
        // Selection stays put on e1 across the refresh (it's still visible);
        // e2 is newest-first, one 'k'/Up away.
        assert_eq!(app.selected().unwrap().id, "e1");
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.selected().unwrap().id, "e2");
        assert!(
            !app.is_new(&eid("e2")),
            "looking at it clears the highlight"
        );
    }

    // ---- resolve input --------------------------------------------------

    #[test]
    fn r_opens_the_decision_input_for_the_selected_escalation() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        assert!(handle_key(&mut app, key(KeyCode::Char('r'))).is_none());
        assert_eq!(
            app.mode,
            Mode::ResolveInput {
                id: eid("e1"),
                buffer: String::new()
            }
        );
    }

    #[test]
    fn typing_then_enter_submits_a_resolve_action_and_returns_to_normal_mode() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('r')));
        for c in "go with a".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        let action = handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Resolve {
                id: eid("e1"),
                decision: "go with a".into()
            })
        );
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn backspace_edits_the_buffer() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('r')));
        handle_key(&mut app, key(KeyCode::Char('a')));
        handle_key(&mut app, key(KeyCode::Char('b')));
        handle_key(&mut app, key(KeyCode::Backspace));
        assert_eq!(
            app.mode,
            Mode::ResolveInput {
                id: eid("e1"),
                buffer: "a".into()
            }
        );
    }

    #[test]
    fn esc_cancels_without_an_action() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('r')));
        handle_key(&mut app, key(KeyCode::Char('x')));
        let action = handle_key(&mut app, key(KeyCode::Esc));
        assert!(action.is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn enter_on_an_empty_decision_does_not_resolve() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('r')));
        let action = handle_key(&mut app, key(KeyCode::Enter));
        assert!(action.is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    // ---- investigate / post-linear --------------------------------------

    #[test]
    fn i_and_shift_i_dispatch_fork_and_fresh_investigate() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        assert_eq!(
            handle_key(&mut app, key(KeyCode::Char('i'))),
            Some(Action::Investigate {
                id: eid("e1"),
                fresh: false
            })
        );
        // The first press marked e1 in flight; simulate its completion
        // before checking that 'I' dispatches its own action independently.
        app.clear_in_flight(&eid("e1"));
        assert_eq!(
            handle_key(&mut app, key(KeyCode::Char('I'))),
            Some(Action::Investigate {
                id: eid("e1"),
                fresh: true
            })
        );
    }

    // ---- per-id in-flight guard ------------------------------------------

    #[test]
    fn a_second_i_for_the_same_in_flight_id_is_ignored_and_reports_already_running() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        assert!(handle_key(&mut app, key(KeyCode::Char('i'))).is_some());
        assert!(app.is_in_flight(&eid("e1")));

        assert_eq!(handle_key(&mut app, key(KeyCode::Char('i'))), None);
        assert_eq!(app.status, "already running for e1");
        // Every guarded key is blocked, not just the one that started it.
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('I'))), None);
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('p'))), None);
        assert!(handle_key(&mut app, key(KeyCode::Char('r'))).is_none());
        assert_eq!(app.mode, Mode::Normal, "r must not open the input either");
    }

    #[test]
    fn clearing_in_flight_lets_the_next_action_through() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('i')));
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('i'))), None);

        app.clear_in_flight(&eid("e1"));
        assert!(handle_key(&mut app, key(KeyCode::Char('i'))).is_some());
    }

    #[test]
    fn resolve_marks_in_flight_only_once_the_decision_is_submitted() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('r')));
        assert!(
            !app.is_in_flight(&eid("e1")),
            "opening the input isn't itself an action in flight"
        );
        handle_key(&mut app, key(KeyCode::Char('x')));
        let action = handle_key(&mut app, key(KeyCode::Enter));
        assert!(action.is_some());
        assert!(app.is_in_flight(&eid("e1")));
    }

    // ---- quit confirmation while an action is running --------------------

    #[test]
    fn q_while_an_action_is_in_flight_asks_for_confirmation_first() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('i')));

        handle_key(&mut app, key(KeyCode::Char('q')));
        assert!(!app.should_quit, "first q only asks for confirmation");
        assert_eq!(
            app.status,
            "action running for e1; press q again to quit anyway"
        );

        handle_key(&mut app, key(KeyCode::Char('q')));
        assert!(app.should_quit, "second, immediate q confirms");
    }

    #[test]
    fn any_other_key_cancels_a_pending_quit_confirmation() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        handle_key(&mut app, key(KeyCode::Char('i')));
        handle_key(&mut app, key(KeyCode::Char('q')));

        handle_key(&mut app, key(KeyCode::Char('j')));
        handle_key(&mut app, key(KeyCode::Char('q')));
        assert!(
            !app.should_quit,
            "q after an unrelated key re-arms confirmation instead of quitting"
        );
    }

    #[test]
    fn p_posts_linear_only_for_a_resolved_selection() {
        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Waiting, 1)]);
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('p'))), None);

        let mut app = App::new(false);
        refresh(&mut app, vec![esc("e1", EscalationStatus::Resolved, 1)]);
        assert_eq!(
            handle_key(&mut app, key(KeyCode::Char('p'))),
            Some(Action::PostLinear { id: eid("e1") })
        );
    }

    #[test]
    fn no_selection_means_i_r_and_p_do_nothing() {
        let mut app = App::new(false);
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('i'))), None);
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('p'))), None);
        assert!(handle_key(&mut app, key(KeyCode::Char('r'))).is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn q_sets_should_quit() {
        let mut app = App::new(false);
        assert!(!app.should_quit);
        handle_key(&mut app, key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    /// Sanity check on [`KeyEventKind`] usage at the call site (the real
    /// event loop only dispatches `Press`, so this import is exercised).
    #[test]
    fn key_helper_builds_a_press_event() {
        assert_eq!(key(KeyCode::Char('q')).kind, KeyEventKind::Press);
    }
}
