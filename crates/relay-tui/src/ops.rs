//! Background work: polling Redis for the escalation list, and running the
//! actions the UI dispatches (investigate/resolve/post-linear/notify).
//! Nothing here touches the terminal; everything here reuses
//! `relay_ops::escalate`'s command functions rather than reimplementing
//! them (AGE-2015's whole point) and reads their *typed* outcomes — never
//! text-parses a captured stdout buffer, since `relay_ops` itself prints
//! nothing at all (AGE-2015 PR review).
//!
//! Both the poller and the action worker run on their own thread ([`spawn`])
//! and talk to the UI thread only through an [`mpsc::Sender<UiEvent>`], so a
//! slow or hung Redis/herdr/linear call never blocks rendering or input. A
//! panic while handling one poll tick or one action is caught with
//! [`std::panic::catch_unwind`] and reported like any other failure — the
//! worker thread itself keeps running, so one bad tick doesn't require
//! restarting `relay-tui`. A [`JoinHandle`] for each thread is returned so
//! the UI can still notice the rare case where a thread exits anyway (a
//! panic `catch_unwind` couldn't catch, such as a second panic while
//! unwinding, or an abort).

use std::{
    any::Any,
    panic::{self, AssertUnwindSafe},
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use relay_core::{EscalationId, Relay, config::CoreSettings};
use relay_ops::{
    escalate::{self, LinearOutcome},
    herdr::{Herdr, RealHerdr},
    linear::{Linear, RealLinear},
};

use crate::{
    app::{Action, RefreshData},
    notify::{Notifier, RealNotifier},
};

/// How often the poller refreshes the escalation list.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Connect/read/write timeout for both the poller and the action worker's
/// own Redis connection — same budget `agent-msg`'s own CLI commands use,
/// since each is one short-lived Redis round trip. Herdr and Linear calls
/// have their own, separately-tuned deadlines inside `relay_ops` itself.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// What a background thread reports back to the UI thread.
pub enum UiEvent {
    /// A poll succeeded or failed.
    Refresh(std::result::Result<RefreshData, String>),
    /// One action finished (or panicked): its escalation id, when it has
    /// one, so the UI can clear that id's in-flight guard, and the message
    /// for the status line.
    ActionDone {
        /// The escalation the action was for, when it was one of
        /// investigate/resolve/post-linear/notify — always `Some` today,
        /// kept `Option` so a future action with no single escalation
        /// doesn't need a fake id.
        id: Option<EscalationId>,
        /// The status-line message.
        message: String,
    },
    // A worker thread exiting on its own — not caught by this module's own
    // `catch_unwind` around each unit of work — has no event here: with
    // every panic already caught per-tick/per-action, the only way a
    // worker's loop actually ends is the channel closing on shutdown (see
    // `action_loop`), which needs no report. `main.rs`'s `check_workers_alive`
    // instead polls each `JoinHandle::is_finished` directly, so that
    // vanishingly rare case (a second panic while unwinding, an abort, ...)
    // is still caught without this enum carrying a variant nothing here
    // would ever construct.
}

/// The channels and thread handles [`spawn`] starts.
pub struct Workers {
    /// Events from both background threads.
    pub events: Receiver<UiEvent>,
    /// Where the UI sends [`Action`]s.
    pub actions: Sender<Action>,
    /// The poller thread. Never expected to finish; the UI polls
    /// [`JoinHandle::is_finished`] to notice if it ever does.
    pub poll_handle: JoinHandle<()>,
    /// The action-worker thread. Finishes on its own once [`Workers::actions`]
    /// is dropped and any in-flight action completes — see `main.rs`'s
    /// shutdown path.
    pub action_handle: JoinHandle<()>,
}

/// Starts the poller and action-worker threads.
#[must_use]
pub fn spawn(settings: CoreSettings) -> Workers {
    let (ui_tx, ui_rx) = mpsc::channel();
    let (action_tx, action_rx) = mpsc::channel();

    let poll_settings = settings.clone();
    let poll_tx = ui_tx.clone();
    let poll_handle = thread::spawn(move || poll_loop(&poll_settings, &poll_tx));

    let action_handle = thread::spawn(move || action_loop(&settings, &action_rx, &ui_tx));

    Workers {
        events: ui_rx,
        actions: action_tx,
        poll_handle,
        action_handle,
    }
}

/// Connects fresh and lists every escalation, every [`POLL_INTERVAL`],
/// forever (barring the rare unrecoverable panic — see this module's own
/// doc comment). A short-lived connection per tick (rather than one held
/// for the process lifetime) means a Redis restart between polls is just
/// the next poll failing and the one after that succeeding — no reconnect
/// logic needed.
fn poll_loop(settings: &CoreSettings, tx: &Sender<UiEvent>) {
    loop {
        let result = panic::catch_unwind(AssertUnwindSafe(|| poll_once(settings)))
            .unwrap_or_else(|payload| Err(format!("poller panicked: {}", panic_message(&payload))));
        if tx.send(UiEvent::Refresh(result)).is_err() {
            return; // UI thread is gone; nothing left to poll for.
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// How many times one poll tries before it reports an interrupted read.
const POLL_ATTEMPTS: usize = 3;

fn poll_once(settings: &CoreSettings) -> std::result::Result<RefreshData, String> {
    retry_interrupted(POLL_ATTEMPTS, || {
        let mut relay = Relay::connect(&settings.url.value, CONNECT_TIMEOUT)?
            .with_prefix(settings.prefix.value.clone());
        let listed = relay.list(None)?;
        Ok(RefreshData {
            escalations: listed.escalations,
            rejected_count: listed.rejected.len(),
        })
    })
    .map_err(|e| e.to_string())
}

/// Runs the read-only `op` up to `attempts` times, again only after a read a
/// signal interrupted ([`relay_core::Error::is_interrupted`]).
///
/// This thread waits on a Redis socket while the action thread spawns `herdr`
/// and `linear`; a child's exit (`SIGCHLD`) can wake the waiting thread, whose
/// `recv` then fails with `EINTR`, and redis-rs does not retry it (measured in
/// `agent-msg doctor`, about 1 run in 60). The poll only reads and opens a fresh
/// connection per attempt, so repeating it is safe and needs no lock: the
/// alternative, holding a lock across every child, would freeze the list for the
/// up to two minutes `agent start` may take. Any other error is returned at once.
///
/// `op` must be a read that opens its own connection: a write is not safe to
/// repeat, and a connection an interrupt hit is not reused.
fn retry_interrupted<T>(
    attempts: usize,
    mut op: impl FnMut() -> relay_core::Result<T>,
) -> relay_core::Result<T> {
    let mut tried = 1;
    loop {
        match op() {
            Err(e) if e.is_interrupted() && tried < attempts => tried += 1,
            other => return other,
        }
    }
}

/// Runs every [`Action`] the UI sends, in order, one at a time — actions
/// never race each other, only the poller (which is fine: `relay_core`'s own
/// transition script is what actually guards concurrent resolves). Ends
/// once [`Workers::actions`] is dropped and the channel is drained, which is
/// how `main.rs` lets an in-flight action finish before a confirmed quit.
fn action_loop(settings: &CoreSettings, rx: &Receiver<Action>, tx: &Sender<UiEvent>) {
    let herdr = RealHerdr::new();
    let linear = RealLinear::new();
    let notifier = RealNotifier::new();
    for action in rx {
        let id = action_id(&action);
        let message = panic::catch_unwind(AssertUnwindSafe(|| {
            run_action(settings, &herdr, &linear, &notifier, action)
        }))
        .unwrap_or_else(|payload| format!("action worker panicked: {}", panic_message(&payload)));
        if tx.send(UiEvent::ActionDone { id, message }).is_err() {
            return;
        }
    }
}

/// The escalation id every [`Action`] variant carries.
fn action_id(action: &Action) -> Option<EscalationId> {
    let id = match action {
        Action::Investigate { id, .. }
        | Action::Resolve { id, .. }
        | Action::PostLinear { id }
        | Action::Notify { id, .. } => id,
    };
    Some(id.clone())
}

/// The three actions that touch the bus, herdr or Linear through a fresh
/// [`Relay`] connection — everything [`Action`] carries except `Notify`,
/// which needs none of that. Splitting these out means the match in
/// [`run_relay_action`] is exhaustive on its own: no `Notify` arm to ever
/// reach and no `unreachable!()` standing in for a case the type system
/// should rule out instead.
enum RelayAction {
    Investigate { id: EscalationId, fresh: bool },
    Resolve { id: EscalationId, decision: String },
    PostLinear { id: EscalationId },
}

/// Renders `action`'s result (or error) as one status-line string. Never
/// blocks the UI thread: this runs entirely inside [`action_loop`], on its
/// own thread, and never panics past its caller's `catch_unwind`.
fn run_action(
    settings: &CoreSettings,
    herdr: &impl Herdr,
    linear: &impl Linear,
    notifier: &impl Notifier,
    action: Action,
) -> String {
    match action {
        Action::Notify { id, ticket, lead } => {
            let title = format!("ix-relay: {ticket}");
            let body = format!("escalation {id} from {lead}");
            match notifier.notify(&title, &body) {
                Ok(()) => format!("notified for {id}"),
                Err(e) => format!("notify {id} failed: {e}"),
            }
        }
        Action::Investigate { id, fresh } => run_relay_action(
            settings,
            herdr,
            linear,
            RelayAction::Investigate { id, fresh },
        ),
        Action::Resolve { id, decision } => run_relay_action(
            settings,
            herdr,
            linear,
            RelayAction::Resolve { id, decision },
        ),
        Action::PostLinear { id } => {
            run_relay_action(settings, herdr, linear, RelayAction::PostLinear { id })
        }
    }
}

fn run_relay_action(
    settings: &CoreSettings,
    herdr: &impl Herdr,
    linear: &impl Linear,
    action: RelayAction,
) -> String {
    let mut relay = match Relay::connect(&settings.url.value, CONNECT_TIMEOUT) {
        Ok(r) => r.with_prefix(settings.prefix.value.clone()),
        Err(e) => return format!("could not connect to redis: {e}"),
    };
    match action {
        RelayAction::Investigate { id, fresh } => {
            match escalate::run_investigate(&mut relay, herdr, &settings.machine.value, &id, fresh)
            {
                Ok(outcome) if outcome.fresh => format!(
                    "started a fresh {} session for {} on {} in pane {}",
                    outcome.kind, outcome.escalation_id, outcome.machine, outcome.pane_id
                ),
                Ok(outcome) => format!(
                    "forked {} into escalation {} on {} in pane {}",
                    outcome.session_id, outcome.escalation_id, outcome.machine, outcome.pane_id
                ),
                Err(e) => format!("investigate {id} failed: {e}"),
            }
        }
        RelayAction::Resolve { id, decision } => {
            match escalate::run_resolve(
                &mut relay,
                linear,
                &id,
                &decision,
                &settings.human.value,
                now_ms(),
            ) {
                Ok(outcome) => {
                    let mut message = if outcome.lead_notified {
                        format!(
                            "resolved {} and notified {}",
                            outcome.escalation.id, outcome.escalation.filing.lead
                        )
                    } else {
                        format!(
                            "resolved {} but could not notify {}",
                            outcome.escalation.id, outcome.escalation.filing.lead
                        )
                    };
                    if let LinearOutcome::Failed(_) = &outcome.linear {
                        message.push_str(" (Linear post failed; press p to retry)");
                    }
                    message
                }
                Err(e) => format!("resolve {id} failed: {e}"),
            }
        }
        RelayAction::PostLinear { id } => {
            match escalate::run_escalation_post_linear(&mut relay, linear, &id) {
                Ok(esc) => format!("posted a comment on {}", esc.filing.ticket),
                Err(e) => format!("post-linear {id} failed: {e}"),
            }
        }
    }
}

/// The panic payload's message, when it's a `&str` or `String` (what
/// `panic!`/`.unwrap()`/`.expect()` all produce) — `"unknown panic"`
/// otherwise, so a status line always has *something* to show rather than
/// failing to format at all.
///
/// Takes the `Box` itself, not `&(dyn Any + Send)`: coercing `&Box<dyn Any
/// + Send>` to `&(dyn Any + Send)` at a plain function-call site silently
/// produces a reference whose `downcast_ref` never matches anything (a real
/// rustc footgun, confirmed empirically — `payload.downcast_ref` called
/// directly on the `Box` works; passing `&*payload` through a fn boundary
/// typed `&(dyn Any + Send)` does not). Taking `&Box<_>` and calling
/// `downcast_ref` through its `Deref` sidesteps the coercion entirely.
fn panic_message(payload: &Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use relay_core::{EscalationFiling, EscalationStatus, PaneId, Registration, TabId};
    use relay_ops::{herdr::CreatedTab, linear::LinearError};

    use super::*;

    fn settings(url: &str, prefix: &str) -> CoreSettings {
        use relay_core::config::{Resolved, Source};
        fn resolved<T>(value: T) -> Resolved<T> {
            Resolved {
                value,
                source: Source::Default,
                origin: None,
            }
        }
        CoreSettings {
            url: resolved(url.to_owned()),
            prefix: resolved(prefix.to_owned()),
            machine: resolved("node-a".parse().unwrap()),
            human: resolved("operator".to_owned()),
        }
    }

    fn test_settings(test: &str) -> Option<CoreSettings> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        Some(settings(
            &url,
            &format!("test:relay-tui-ops:{}:{test}:", std::process::id()),
        ))
    }

    #[derive(Default)]
    struct FakeHerdr {
        fail: bool,
        panic: bool,
    }

    impl Herdr for FakeHerdr {
        fn tab_create(
            &self,
            _machine: Option<&str>,
            _cwd: &str,
            _label: &str,
        ) -> std::result::Result<CreatedTab, herdr_cli::ProcessError> {
            if self.panic {
                panic!("FakeHerdr::tab_create panicked on purpose");
            }
            if self.fail {
                return Err(herdr_cli::ProcessError::Reported {
                    code: "repository_not_trusted".into(),
                    message: "untrusted".into(),
                });
            }
            Ok(CreatedTab {
                pane_id: "w1:p1".parse().unwrap(),
                tab_id: "w1:t1".parse().unwrap(),
            })
        }
        fn agent_start(
            &self,
            _machine: Option<&str>,
            _name: &str,
            _kind: &str,
            _pane: &PaneId,
            _args: &[&str],
        ) -> std::result::Result<(), herdr_cli::ProcessError> {
            Ok(())
        }
        fn tab_close(
            &self,
            _machine: Option<&str>,
            _tab_id: &TabId,
        ) -> std::result::Result<(), herdr_cli::ProcessError> {
            Ok(())
        }
    }

    struct FakeLinear {
        fail: bool,
        calls: RefCell<Vec<(String, String)>>,
    }

    impl Linear for FakeLinear {
        fn comment(&self, ticket: &str, body: &str) -> std::result::Result<(), LinearError> {
            self.calls
                .borrow_mut()
                .push((ticket.to_owned(), body.to_owned()));
            if self.fail {
                Err(LinearError::Failed {
                    status: 1,
                    stderr: "not logged in".into(),
                })
            } else {
                Ok(())
            }
        }
    }

    struct FakeNotifier {
        calls: RefCell<Vec<(String, String)>>,
    }

    impl Notifier for FakeNotifier {
        fn notify(
            &self,
            title: &str,
            body: &str,
        ) -> std::result::Result<(), herdr_cli::ProcessError> {
            self.calls
                .borrow_mut()
                .push((title.to_owned(), body.to_owned()));
            Ok(())
        }
    }

    fn eid(id: &str) -> EscalationId {
        id.parse().unwrap()
    }

    fn file_escalation(settings: &CoreSettings) -> EscalationId {
        let mut relay = Relay::connect(&settings.url.value, CONNECT_TIMEOUT)
            .unwrap()
            .with_prefix(settings.prefix.value.clone());
        relay
            .register(&Registration {
                name: "qsl-lead".parse().unwrap(),
                machine: "node-a".parse().unwrap(),
                kind: "claude".into(),
                session_id: Some("sid-1".parse().unwrap()),
                cwd: Some("/work".into()),
                herdr_pane: Some("w1:p1".parse().unwrap()),
                place: None,
                last_seen_ms: 1,
            })
            .unwrap();
        relay
            .escalate(EscalationFiling {
                ticket: "AGE-2015".parse().unwrap(),
                lead: "qsl-lead".parse().unwrap(),
                machine: "node-a".parse().unwrap(),
                session_id: "sid-1".parse().unwrap(),
                cwd: Some("/work".into()),
                kind: "claude".into(),
                context: "ctx".into(),
                options: vec!["a".into()],
                recommendation: "a".into(),
                created_at_ms: 1,
            })
            .unwrap()
            .id
    }

    /// A poll that a signal interrupted reads again, up to its bound, on a new
    /// connection; any other failure is reported at once.
    #[test]
    fn a_poll_interrupted_by_a_signal_is_tried_again_up_to_its_bound() {
        let interrupted = || {
            relay_core::Error::Redis(redis::RedisError::from(std::io::Error::from(
                std::io::ErrorKind::Interrupted,
            )))
        };
        let mut calls = 0;
        let got = retry_interrupted(3, || {
            calls += 1;
            if calls < 3 {
                Err(interrupted())
            } else {
                Ok(calls)
            }
        });
        assert_eq!(got.unwrap(), 3, "the third attempt succeeds");

        let mut calls = 0;
        let got: relay_core::Result<()> = retry_interrupted(3, || {
            calls += 1;
            Err(interrupted())
        });
        assert!(got.unwrap_err().is_interrupted());
        assert_eq!(calls, 3, "and stops at the bound");

        let mut calls = 0;
        let got: relay_core::Result<()> = retry_interrupted(3, || {
            calls += 1;
            Err(relay_core::Error::Redis(redis::RedisError::from(
                std::io::Error::from(std::io::ErrorKind::ConnectionReset),
            )))
        });
        assert!(got.is_err());
        assert_eq!(calls, 1, "a reset is not retried");
    }

    #[test]
    fn poll_once_lists_escalations_and_counts_rejected_as_zero_on_a_clean_store() {
        let Some(s) = test_settings("poll") else {
            return;
        };
        let id = file_escalation(&s);
        let data = poll_once(&s).unwrap();
        assert!(data.escalations.iter().any(|e| e.id == id));
        assert_eq!(data.rejected_count, 0);
    }

    #[test]
    fn poll_once_reports_a_connect_failure_as_an_error_string() {
        let s = settings("redis://127.0.0.1:1", "test:unused:");
        let err = poll_once(&s).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn a_panic_inside_poll_once_is_caught_and_reported_not_propagated() {
        // poll_loop wraps poll_once in catch_unwind; exercise that wrapper
        // directly with a closure that panics, standing in for poll_once.
        let result = panic::catch_unwind(AssertUnwindSafe(
            || -> std::result::Result<RefreshData, String> { panic!("simulated poll panic") },
        ))
        .unwrap_or_else(|payload| Err(format!("poller panicked: {}", panic_message(&payload))));
        assert_eq!(result.unwrap_err(), "poller panicked: simulated poll panic");
    }

    #[test]
    fn run_action_investigate_reuses_relay_ops_and_reports_the_first_line() {
        let Some(s) = test_settings("investigate") else {
            return;
        };
        let id = file_escalation(&s);
        let herdr = FakeHerdr::default();
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let notifier = FakeNotifier {
            calls: RefCell::default(),
        };
        let msg = run_action(
            &s,
            &herdr,
            &linear,
            &notifier,
            Action::Investigate {
                id: id.clone(),
                fresh: false,
            },
        );
        assert!(msg.contains(id.as_str()), "{msg}");

        let mut relay = Relay::connect(&s.url.value, CONNECT_TIMEOUT)
            .unwrap()
            .with_prefix(s.prefix.value.clone());
        assert_eq!(
            relay.get(&id).unwrap().status,
            EscalationStatus::Investigating
        );
    }

    #[test]
    fn run_action_investigate_failure_is_reported_not_panicked() {
        let Some(s) = test_settings("investigate-fail") else {
            return;
        };
        let id = file_escalation(&s);
        let herdr = FakeHerdr {
            fail: true,
            panic: false,
        };
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let notifier = FakeNotifier {
            calls: RefCell::default(),
        };
        let msg = run_action(
            &s,
            &herdr,
            &linear,
            &notifier,
            Action::Investigate { id, fresh: false },
        );
        assert!(msg.contains("failed"), "{msg}");
    }

    /// AGE-2015 PR review: a panic while handling one action must not kill
    /// the worker thread — `action_loop`'s own `catch_unwind` (exercised
    /// here directly, the same way `poll_once`'s is above) reports it as an
    /// ordinary status message instead.
    #[test]
    fn a_panic_inside_run_action_is_caught_and_reported_not_propagated() {
        let Some(s) = test_settings("investigate-panics") else {
            return;
        };
        let id = file_escalation(&s);
        let herdr = FakeHerdr {
            fail: false,
            panic: true,
        };
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let notifier = FakeNotifier {
            calls: RefCell::default(),
        };
        // The same catch_unwind wrapping action_loop does around run_action,
        // exercised directly rather than by spawning a real thread.
        let message = panic::catch_unwind(AssertUnwindSafe(|| {
            run_action(
                &s,
                &herdr,
                &linear,
                &notifier,
                Action::Investigate { id, fresh: false },
            )
        }))
        .unwrap_or_else(|payload| format!("action worker panicked: {}", panic_message(&payload)));
        assert!(message.contains("panicked"), "{message}");
        assert!(
            message.contains("tab_create panicked on purpose"),
            "{message}"
        );
    }

    #[test]
    fn run_action_resolve_notes_a_failed_linear_post_in_the_status_line_from_the_typed_outcome() {
        let Some(s) = test_settings("resolve-linear-fails") else {
            return;
        };
        let id = file_escalation(&s);
        let herdr = FakeHerdr::default();
        let linear = FakeLinear {
            fail: true,
            calls: RefCell::default(),
        };
        let notifier = FakeNotifier {
            calls: RefCell::default(),
        };
        let msg = run_action(
            &s,
            &herdr,
            &linear,
            &notifier,
            Action::Resolve {
                id: id.clone(),
                decision: "go with a".into(),
            },
        );
        assert!(msg.contains("Linear post failed"), "{msg}");
        assert!(msg.contains("press p to retry"), "{msg}");
    }

    #[test]
    fn run_action_resolve_reports_success_when_linear_posts_cleanly() {
        let Some(s) = test_settings("resolve-ok") else {
            return;
        };
        let id = file_escalation(&s);
        let herdr = FakeHerdr::default();
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let notifier = FakeNotifier {
            calls: RefCell::default(),
        };
        let msg = run_action(
            &s,
            &herdr,
            &linear,
            &notifier,
            Action::Resolve {
                id: id.clone(),
                decision: "go with a".into(),
            },
        );
        assert!(!msg.contains("failed"), "{msg}");
        assert!(msg.contains(id.as_str()), "{msg}");
    }

    #[test]
    fn run_action_post_linear_retries_regardless_of_any_prior_failure_state() {
        let Some(s) = test_settings("post-linear") else {
            return;
        };
        let id = file_escalation(&s);
        let herdr = FakeHerdr::default();
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let notifier = FakeNotifier {
            calls: RefCell::default(),
        };
        run_action(
            &s,
            &herdr,
            &linear,
            &notifier,
            Action::Resolve {
                id: id.clone(),
                decision: "go with a".into(),
            },
        );
        let msg = run_action(
            &s,
            &herdr,
            &linear,
            &notifier,
            Action::PostLinear { id: id.clone() },
        );
        assert!(msg.contains(id.as_str()) || msg.contains("posted"), "{msg}");
        assert_eq!(linear.calls.borrow().len(), 2, "resolve + the manual retry");
    }

    #[test]
    fn run_action_notify_calls_the_notifier_with_ticket_and_lead() {
        let s = settings("redis://127.0.0.1:1", "test:unused:");
        let herdr = FakeHerdr::default();
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let notifier = FakeNotifier {
            calls: RefCell::default(),
        };
        let msg = run_action(
            &s,
            &herdr,
            &linear,
            &notifier,
            Action::Notify {
                id: eid("e7"),
                ticket: "AGE-2015".into(),
                lead: "qsl-lead".into(),
            },
        );
        assert!(msg.contains("e7"), "{msg}");
        assert_eq!(notifier.calls.borrow().len(), 1);
        assert!(notifier.calls.borrow()[0].0.contains("AGE-2015"));
        assert!(notifier.calls.borrow()[0].1.contains("e7"));
        assert!(notifier.calls.borrow()[0].1.contains("qsl-lead"));
    }

    #[test]
    fn action_id_reads_the_id_out_of_every_variant() {
        assert_eq!(
            action_id(&Action::Investigate {
                id: eid("e1"),
                fresh: false
            }),
            Some(eid("e1"))
        );
        assert_eq!(
            action_id(&Action::Resolve {
                id: eid("e2"),
                decision: "d".into()
            }),
            Some(eid("e2"))
        );
        assert_eq!(
            action_id(&Action::PostLinear { id: eid("e3") }),
            Some(eid("e3"))
        );
        assert_eq!(
            action_id(&Action::Notify {
                id: eid("e4"),
                ticket: "AGE-1".into(),
                lead: "l".into()
            }),
            Some(eid("e4"))
        );
    }
}
