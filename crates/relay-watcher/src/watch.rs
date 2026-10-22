//! One lead, one tick: decide whether to type its queued messages in.
//!
//! Kept separate from `main`'s Redis-polling loop so [`try_deliver`] can be
//! unit-tested against a fake [`Herdr`] with no live herdr session or Redis.

use relay_core::{BatchId, Message, PaneId, SessionId};

use crate::{
    deliver::{BatchTarget, DeliverError, Deliverer},
    episodes::{EpisodeKind, Notice},
    herdr::{AgentStatus, Herdr, ProcessError},
};

/// A batch popped off a lead's inbox and rendered for `herdr agent prompt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoppedBatch {
    /// The in-flight batch these messages are recorded under: what the
    /// acknowledgement or the requeue names.
    pub batch: BatchId,
    /// The messages that make up this batch, oldest first.
    pub messages: Vec<Message>,
    /// `messages`, rendered and joined, ready to hand to `herdr agent prompt`.
    pub rendered: String,
    /// The distinct sender addresses of `messages`, joined with `, `, as they
    /// read in `rendered`.
    pub senders: String,
}

/// What popping a lead's inbox produced, for [`try_deliver`].
#[derive(Debug)]
pub enum PopOutcome {
    /// Something to deliver.
    Popped(PoppedBatch),
    /// The inbox was already empty — another consumer (the lead's own hook)
    /// got there first.
    Empty,
    /// Nothing was popped because the lead has as much in flight as one more pop
    /// may add to: the mail is waiting for a lease to free room.
    AtCap,
    /// The pop itself failed (a Redis error), distinct from `Empty` so it is
    /// never logged as "emptied by its hook" (AGE-2013's PR review, finding 8).
    Failed(String),
}

/// The inbox operations `try_deliver` needs around one lead's popped batch:
/// pop it, acknowledge it once delivered, and put it back if delivery turns out
/// to have failed. A trait
/// (rather than two separate closures) so both can share one mutable borrow
/// of the caller's `Relay` — two `FnMut`/`FnOnce` closures both capturing
/// `&mut Relay` can't coexist, since the borrow checker can't see that
/// `try_deliver` only ever calls one of them.
pub trait Inbox {
    /// Pops the batch to deliver. Called at most once, only after both
    /// readiness checks pass.
    fn pop(&mut self) -> PopOutcome;
    /// Acknowledges `batch`: the transport reported success, so its in-flight
    /// record is dropped. Never called for a failed or ambiguous delivery, whose
    /// batch stays in flight and is redelivered when its lease expires.
    fn ack(&mut self, batch: &PoppedBatch);
    /// Puts `batch` back at the front of the inbox. Called only when the
    /// transport is certain no input was sent for it.
    fn requeue(&mut self, batch: &PoppedBatch);
    /// Moves `batch` to the lead's dead letters. Called only when the transport
    /// knows it will never be delivered (the receiver's user denied it).
    fn dead_letter(&mut self, batch: &PoppedBatch);
}

/// What happened trying to deliver to one lead this tick.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Typed `messages` messages into the pane.
    Delivered {
        /// How many messages were in the batch.
        messages: usize,
    },
    /// The lead is mid-turn. Its own `PostToolUse`/`Stop` hook will deliver.
    SkippedWorking,
    /// herdr could not classify the lead's status confidently. Distinct from
    /// `SkippedWorking` only for logging (AGE-2013's PR review, finding 9):
    /// both wait for the lead's own hook.
    SkippedUnknown,
    /// The pane's occupant doesn't match who we registered (different herdr
    /// session id, or either side is missing one): typing in would go to the
    /// wrong agent, so this is skipped (AGE-2013's PR review, finding 4).
    /// Logged once per episode; see [`Outcome::episode`].
    StaleRegistration,
    /// The lead is waiting on an approval or question dialog and the operator has
    /// been told (this tick or an earlier one of the same stretch).
    SkippedBlocked,
    /// The lead is blocked, an earlier notification failed, and the retry is
    /// not due yet: herdr was not called.
    NotifyPending,
    /// herdr's own notification call failed while the lead was blocked.
    NotifyFailed(ProcessError),
    /// The lead's Claude Code holds the batch behind an approval prompt
    /// (FR-044). It stays in flight; `notified` says whether the operator was told this
    /// tick (they are told once per episode).
    SocketHeld {
        /// Why it is held.
        why: String,
        /// Whether a notification was shown this tick.
        notified: bool,
    },
    /// The transport holds mail for this lead (Claude Code parks it behind an
    /// approval prompt) and asked for none to be popped: the mail waits in the
    /// inbox. Part of the held episode, so it is logged once.
    SocketHeldQuiet,
    /// The socket accepted the batch and its lease is renewed pending consumption.
    SocketPending(String),
    /// The transport chose not to try the batch now: it was put back at the front
    /// of the inbox unmarked.
    SocketDeferred {
        /// How many messages were put back.
        requeued: usize,
    },
    /// The transport did not post the batch because its time on sockets this tick is spent:
    /// it was put back unmarked. Says nothing about the lead: no episode, no notification.
    SocketPostponed {
        /// How many messages were put back.
        requeued: usize,
    },
    /// The receiver's user denied the message within the wait: the batch was
    /// dead-lettered.
    SocketDenied {
        /// How many messages were dead-lettered.
        dead: usize,
    },
    /// The lead started a new turn between the readiness check and the
    /// prompt (its `state_change_seq` moved). Its own hook will deliver.
    SkippedSeqChanged,
    /// Another consumer (the lead's own hook) emptied the inbox between our
    /// `inbox_len` check and the pop.
    RaceEmptied,
    /// The lead already has too much in flight for another pop (unacknowledged
    /// batches of its own hook or of this watcher): its mail waits for a lease to
    /// expire. An episode: logged once when it starts and once when it ends.
    InflightFull,
    /// Popping the inbox itself failed (a Redis error). The messages, if
    /// any, are still in the inbox — nothing was lost.
    PopFailed(String),
    /// The registered pane has no agent (exited, replaced, or never one there).
    NoAgent,
    /// `herdr agent get` failed.
    GetFailed(ProcessError),
    /// The transport failed in a way it is certain sent no input (for herdr:
    /// `agent_blocked`, `agent_not_found`, or the binary never ran at all).
    /// The popped batch was requeued at the front of the inbox, so it will be
    /// retried.
    PromptRejected {
        /// How many messages were requeued.
        requeued: usize,
        /// Why the delivery was rejected.
        error: DeliverError,
    },
    /// The transport failed in a way that does *not* guarantee no input was
    /// sent (a timeout, an unrecognized error, a bare nonzero exit). The batch
    /// is neither acknowledged nor requeued — either could lose or double-type
    /// it — so it stays in flight and returns to the inbox, marked as a
    /// redelivery, when its lease expires. `notified` says whether the "check on
    /// this lead" notification to the operator went through.
    PromptAmbiguous {
        /// Why the delivery might have failed.
        error: DeliverError,
        /// Whether `herdr notification show` succeeded.
        notified: bool,
    },
}

impl Outcome {
    /// The episode this outcome belongs to, if it is a condition that persists
    /// across ticks and so is logged once per stretch rather than every tick.
    /// Outcomes that are events (a delivery, a lead mid-turn, a race) are not.
    #[must_use]
    pub fn episode(&self) -> Option<EpisodeKind> {
        match self {
            Self::SkippedBlocked => Some(EpisodeKind::Blocked),
            Self::StaleRegistration => Some(EpisodeKind::Stale),
            Self::NoAgent => Some(EpisodeKind::NoAgent),
            Self::GetFailed(_) => Some(EpisodeKind::GetFailed),
            Self::NotifyFailed(_) | Self::NotifyPending => Some(EpisodeKind::NotifyFailed),
            Self::SocketHeld { .. }
            | Self::SocketHeldQuiet
            | Self::SocketDeferred { .. }
            | Self::SocketDenied { .. } => Some(EpisodeKind::SocketHeld),
            Self::InflightFull => Some(EpisodeKind::InflightFull),
            Self::SocketPostponed { .. }
            | Self::SocketPending(_)
            | Self::Delivered { .. }
            | Self::SkippedWorking
            | Self::SkippedUnknown
            | Self::SkippedSeqChanged
            | Self::RaceEmptied
            | Self::PopFailed(_)
            | Self::PromptRejected { .. }
            | Self::PromptAmbiguous { .. } => None,
        }
    }

    /// One line describing the outcome, for the per-delivery stderr log.
    pub fn log_line(&self, lead: &str, pane: &PaneId) -> String {
        match self {
            Self::Delivered { messages } => {
                format!("delivered {messages} message(s) to {lead} ({pane})")
            }
            Self::SkippedWorking => format!("{lead} ({pane}) is working; left for its hook"),
            Self::SkippedUnknown => {
                format!("{lead} ({pane}) has unknown status; left for its hook")
            }
            Self::StaleRegistration => {
                format!("{lead}'s registration for {pane} is stale (session id mismatch); skipped")
            }
            Self::SkippedBlocked => format!("{lead} ({pane}) is blocked; the operator notified"),
            Self::NotifyPending => {
                format!("{lead} ({pane}) is blocked; notification retry not due")
            }
            Self::NotifyFailed(e) => {
                format!("{lead} ({pane}) is blocked; notification failed: {e}")
            }
            Self::SocketPostponed { requeued } => format!(
                "{lead} ({pane}): the socket's time this tick is spent; put {requeued} message(s) \
                 back unmarked for the next tick"
            ),
            Self::SocketDenied { dead } => format!(
                "{lead} ({pane}): the receiver's user denied the message; dead-lettered {dead} message(s)"
            ),
            Self::SocketHeldQuiet => format!(
                "{lead} ({pane}): Claude Code holds relay mail for approval; new mail waits in the inbox"
            ),
            Self::SocketPending(why) => format!("{lead} ({pane}): {why}"),
            Self::SocketDeferred { requeued } => format!(
                "{lead} ({pane}): delivery deferred; put {requeued} message(s) back unmarked"
            ),
            Self::SocketHeld { why, notified } => format!(
                "{lead} ({pane}): Claude Code holds relay mail for approval ({why}); left in \
                 flight; the operator {}",
                if *notified {
                    "notified"
                } else {
                    "already told (or the notification failed)"
                }
            ),
            Self::SkippedSeqChanged => {
                format!("{lead} ({pane}) started a turn just before prompting; left for its hook")
            }
            Self::RaceEmptied => format!("{lead} ({pane}) inbox was emptied by its hook first"),
            Self::InflightFull => {
                format!(
                    "{lead}: in-flight cap reached; its mail waits until a lease expires or a batch is acknowledged"
                )
            }
            Self::PopFailed(e) => format!("{lead}: failed to pop inbox: {e}"),
            Self::NoAgent => format!("{lead}'s pane {pane} has no agent; skipped"),
            Self::GetFailed(e) => format!("{lead} ({pane}): herdr agent get failed: {e}"),
            Self::PromptRejected { requeued, error } => format!(
                "{lead} ({pane}): herdr agent prompt rejected ({error}); requeued {requeued} message(s)"
            ),
            Self::PromptAmbiguous { error, notified } => format!(
                "{lead} ({pane}): delivery may have failed ({error}); left in flight, redelivered when its lease expires; the operator {}",
                if *notified {
                    "notified"
                } else {
                    "notification also failed"
                }
            ),
        }
    }
}

/// Whether a pane's occupant matches who we think is registered there.
/// `false` (stale) whenever either side is missing an id — an unknown
/// occupant is never safe to type into.
pub(crate) fn session_matches(registered: Option<&SessionId>, live: Option<&SessionId>) -> bool {
    matches!((registered, live), (Some(r), Some(l)) if r == l)
}

/// Tries to deliver to one lead whose inbox is known non-empty (the caller
/// checks `Relay::inbox_len` before calling this, so an idle lead with
/// nothing queued never costs a `herdr agent get`).
///
/// Order matters: get, check the session id and readiness, recheck
/// `state_change_seq`, only then `pop`, only then `prompt` — so a lead that
/// starts a turn (or gets replaced) between the check and the prompt never
/// gets typed into.
///
/// `inbox` performs the actual pop (a Redis call in production), the
/// acknowledgement when the [`Deliverer`] reports success, and, if it is certain
/// no input was sent ([`DeliverError::NotSent`]), the requeue. An ambiguous
/// failure does neither: the batch stays in flight (FR-020).
pub fn try_deliver<H: Herdr + Deliverer>(
    herdr: &H,
    lead: &str,
    kind: &str,
    pane: &PaneId,
    registered_session_id: Option<&SessionId>,
    notice: Notice,
    inbox: &mut impl Inbox,
) -> Outcome {
    let first = match herdr.get(pane) {
        Ok(Some(info)) => info,
        Ok(None) => return Outcome::NoAgent,
        Err(e) => return Outcome::GetFailed(e),
    };

    if !session_matches(registered_session_id, first.session_id.as_ref()) {
        return Outcome::StaleRegistration;
    }

    let socket = herdr.prefers_socket(kind);
    match first.status {
        AgentStatus::Working if !socket => Outcome::SkippedWorking,
        AgentStatus::Unknown if !socket => Outcome::SkippedUnknown,
        AgentStatus::Blocked if !socket => {
            match notice {
                Notice::Done => return Outcome::SkippedBlocked,
                Notice::RetryLater => return Outcome::NotifyPending,
                Notice::Needed => {}
            }
            let title = format!("ix-relay: {lead} is blocked");
            let body = "A queued message is waiting; approve or dismiss the dialog to receive it.";
            match herdr.notify(&title, body) {
                Ok(()) => Outcome::SkippedBlocked,
                Err(e) => Outcome::NotifyFailed(e),
            }
        }
        _ => {
            let recheck = match herdr.get(pane) {
                Ok(Some(info)) => info,
                Ok(None) => return Outcome::NoAgent,
                Err(e) => return Outcome::GetFailed(e),
            };
            if !session_matches(registered_session_id, recheck.session_id.as_ref()) {
                return Outcome::StaleRegistration;
            }
            if !socket && recheck.state_change_seq != first.state_change_seq {
                return Outcome::SkippedSeqChanged;
            }
            if relay_core::tries_socket(kind) && herdr.holds(lead, registered_session_id) {
                return Outcome::SocketHeldQuiet;
            }
            match inbox.pop() {
                PopOutcome::Empty => Outcome::RaceEmptied,
                PopOutcome::AtCap => Outcome::InflightFull,
                PopOutcome::Failed(e) => Outcome::PopFailed(e),
                PopOutcome::Popped(batch) => match herdr.deliver_to(
                    &BatchTarget {
                        lead,
                        kind,
                        session: registered_session_id,
                        batch: batch.batch,
                        senders: &batch.senders,
                    },
                    pane,
                    &batch.rendered,
                ) {
                    Ok(()) => {
                        inbox.ack(&batch);
                        Outcome::Delivered {
                            messages: batch.messages.len(),
                        }
                    }
                    Err(error @ DeliverError::NotSent(_)) => {
                        let requeued = batch.messages.len();
                        inbox.requeue(&batch);
                        Outcome::PromptRejected { requeued, error }
                    }
                    Err(DeliverError::Denied(_)) => {
                        let dead = batch.messages.len();
                        inbox.dead_letter(&batch);
                        Outcome::SocketDenied { dead }
                    }
                    Err(DeliverError::Postponed(_)) => {
                        let requeued = batch.messages.len();
                        inbox.requeue(&batch);
                        Outcome::SocketPostponed { requeued }
                    }
                    Err(DeliverError::Deferred(_)) => {
                        let requeued = batch.messages.len();
                        inbox.requeue(&batch);
                        Outcome::SocketDeferred { requeued }
                    }
                    Err(DeliverError::Held(why)) => {
                        let notified = notice == Notice::Needed
                            && {
                                let title = format!("ix-relay: {lead} is in bypass mode");
                                let body = "Claude Code holds relay mail behind an approval prompt: set \
                                crossSessionInbound to accept in its settings, or set delivery = \
                                herdr in ~/.ix/relay/config.toml.";
                                herdr.notify(&title, body).is_ok()
                            };
                        Outcome::SocketHeld { why, notified }
                    }
                    Err(DeliverError::Pending(why)) => Outcome::SocketPending(why),
                    Err(e @ DeliverError::Ambiguous(_)) => {
                        let title = format!("ix-relay: prompt to {lead} may have failed");
                        let body = format!(
                            "{e}. The batch stays in flight and is redelivered, marked, \
                             if its lease expires; see `agent-msg inbox --dead` if it never is."
                        );
                        let notified = herdr.notify(&title, &body).is_ok();
                        Outcome::PromptAmbiguous { error: e, notified }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod evidence;

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use crate::herdr::AgentGet;

    use super::*;
    use relay_core::PaneId;

    /// A scripted [`Herdr`] and [`Deliverer`]: each call to `get` returns the
    /// next queued response; `deliver`/`notify` record their calls and return
    /// a canned result (a herdr failure is classified as the real transport
    /// classifies it).
    struct FakeHerdr {
        gets: RefCell<VecDeque<Result<Option<AgentGet>, ProcessError>>>,
        prompt_result: Result<(), ProcessError>,
        notify_result: Result<(), ProcessError>,
        prompts: RefCell<Vec<(String, String)>>,
        notifies: RefCell<Vec<(String, String)>>,
        /// When set, the transport reports the batch held for approval.
        held: Option<String>,
        /// When set, the transport is holding mail for the lead.
        holds: bool,
        /// When set, the transport defers the batch.
        deferred: bool,
        /// When set, the transport postpones the batch (its time this tick is spent).
        postponed: bool,
        /// The (lead, session, senders) each `deliver_to` was asked for.
        targets: RefCell<Vec<(String, Option<String>, String)>>,
    }

    impl FakeHerdr {
        fn with_gets(gets: Vec<Result<Option<AgentGet>, ProcessError>>) -> Self {
            Self {
                gets: RefCell::new(gets.into()),
                prompt_result: Ok(()),
                notify_result: Ok(()),
                prompts: RefCell::default(),
                notifies: RefCell::default(),
                held: None,
                holds: false,
                deferred: false,
                postponed: false,
                targets: RefCell::default(),
            }
        }
    }

    impl Herdr for FakeHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            self.gets
                .borrow_mut()
                .pop_front()
                .expect("test provided enough scripted get() calls")
        }

        fn notify(&self, title: &str, body: &str) -> Result<(), ProcessError> {
            self.notifies
                .borrow_mut()
                .push((title.to_owned(), body.to_owned()));
            self.notify_result.clone()
        }
    }

    impl Deliverer for FakeHerdr {
        fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), DeliverError> {
            self.prompts
                .borrow_mut()
                .push((pane.to_string(), text.to_owned()));
            self.prompt_result.clone().map_err(DeliverError::from)
        }

        fn deliver_to(
            &self,
            target: &BatchTarget<'_>,
            pane: &PaneId,
            text: &str,
        ) -> Result<(), DeliverError> {
            self.targets.borrow_mut().push((
                target.lead.to_owned(),
                target.session.map(ToString::to_string),
                target.senders.to_owned(),
            ));
            if self.postponed {
                return Err(DeliverError::Postponed("spent".into()));
            }
            if self.deferred {
                return Err(DeliverError::Deferred("held".into()));
            }
            match &self.held {
                Some(why) => Err(DeliverError::Held(why.clone())),
                None => self.deliver(pane, text),
            }
        }

        fn holds(&self, _lead: &str, _session: Option<&SessionId>) -> bool {
            self.holds
        }
    }

    const SID: &str = "sess-1";

    fn ready(status: AgentStatus, seq: u64) -> Result<Option<AgentGet>, ProcessError> {
        ready_with_session(status, seq, Some(SID))
    }

    fn ready_with_session(
        status: AgentStatus,
        seq: u64,
        session_id: Option<&str>,
    ) -> Result<Option<AgentGet>, ProcessError> {
        Ok(Some(AgentGet {
            status,
            state_change_seq: seq,
            session_id: session_id.map(|s| s.parse().unwrap()),
        }))
    }

    fn batch(text: &str) -> PoppedBatch {
        PoppedBatch {
            batch: relay_core::BatchId::new(7),
            messages: vec![relay_core::Message {
                from: "qsl-lead".parse().unwrap(),
                to: "tl-lead".parse().unwrap(),
                text: text.into(),
                sent_at_ms: 0,
                forwarded: false,
            }],
            rendered: format!("[ix-relay] from qsl-lead: {text}"),
            senders: "qsl-lead".into(),
        }
    }

    /// A scripted [`Inbox`]: `pop` returns each queued outcome in turn
    /// (panicking if `try_deliver` asks for more than were scripted, so an
    /// unexpected pop call fails loudly); `ack` and `requeue` record what they
    /// were given.
    #[derive(Default)]
    struct FakeInbox {
        pops: VecDeque<PopOutcome>,
        requeued: Vec<Vec<Message>>,
        acked: Vec<relay_core::BatchId>,
        dead: Vec<relay_core::BatchId>,
    }

    impl FakeInbox {
        fn once(outcome: PopOutcome) -> Self {
            Self {
                pops: VecDeque::from([outcome]),
                requeued: Vec::new(),
                acked: Vec::new(),
                dead: Vec::new(),
            }
        }

        fn none() -> Self {
            Self::default()
        }
    }

    impl Inbox for FakeInbox {
        fn pop(&mut self) -> PopOutcome {
            self.pops
                .pop_front()
                .expect("try_deliver popped more times than the test scripted")
        }

        fn ack(&mut self, batch: &PoppedBatch) {
            self.acked.push(batch.batch);
        }

        fn requeue(&mut self, batch: &PoppedBatch) {
            self.requeued.push(batch.messages.clone());
        }

        fn dead_letter(&mut self, batch: &PoppedBatch) {
            self.dead.push(batch.batch);
        }
    }

    fn deliver<H: Herdr + Deliverer>(herdr: &H, notice: Notice, inbox: &mut FakeInbox) -> Outcome {
        try_deliver(
            herdr,
            "tl-lead",
            "claude",
            &"w1:p1".parse().unwrap(),
            Some(&SID.parse().unwrap()),
            notice,
            inbox,
        )
    }

    #[test]
    fn idle_lead_is_delivered_to() {
        let herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        let outcome = deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(outcome, Outcome::Delivered { messages: 1 });
        assert!(inbox.requeued.is_empty());
        assert_eq!(inbox.acked, vec![relay_core::BatchId::new(7)]);
        assert_eq!(
            herdr.prompts.into_inner(),
            vec![(
                "w1:p1".to_owned(),
                "[ix-relay] from qsl-lead: hi".to_owned()
            )]
        );
    }

    /// Tracing: TC-044-05
    /// ACs: FR-044-AC-5
    ///
    /// A batch a transport reports held stays in flight: not acknowledged, not
    /// requeued, and the operator is told once, on the first tick of the episode.
    #[test]
    fn tc_044_05_a_held_batch_stays_in_flight_and_notifies_once_per_episode() {
        let held = |notice| {
            let mut herdr = FakeHerdr::with_gets(vec![
                ready(AgentStatus::Idle, 10),
                ready(AgentStatus::Idle, 10),
            ]);
            herdr.held = Some("bypass".into());
            let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
            let outcome = deliver(&herdr, notice, &mut inbox);
            assert!(inbox.acked.is_empty() && inbox.requeued.is_empty());
            assert!(
                herdr.prompts.borrow().is_empty(),
                "no other transport is tried"
            );
            (outcome, herdr)
        };
        let (first, herdr) = held(Notice::Needed);
        assert_eq!(
            first,
            Outcome::SocketHeld {
                why: "bypass".into(),
                notified: true
            }
        );
        let notes = herdr.notifies.borrow();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].0.contains("bypass mode"), "{notes:?}");
        assert!(notes[0].1.contains("crossSessionInbound"), "{notes:?}");
        assert_eq!(first.episode(), Some(EpisodeKind::SocketHeld));
        let (again, herdr) = held(Notice::Done);
        assert_eq!(
            again,
            Outcome::SocketHeld {
                why: "bypass".into(),
                notified: false
            }
        );
        assert!(herdr.notifies.borrow().is_empty());
    }

    /// Tracing: TC-044-05
    /// ACs: FR-044-AC-5
    ///
    /// While the transport holds mail for a lead nothing is popped for it: the
    /// mail waits in the inbox, and no other transport is tried.
    #[test]
    fn tc_044_05_a_lead_the_transport_holds_has_nothing_popped() {
        let mut herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        herdr.holds = true;
        // `none()` panics on a pop: any pop fails the test.
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::SocketHeldQuiet);
        assert_eq!(outcome.episode(), Some(EpisodeKind::SocketHeld));
        assert!(herdr.prompts.borrow().is_empty() && herdr.notifies.borrow().is_empty());
    }

    /// Tracing: TC-044-05
    /// ACs: FR-044-AC-5
    ///
    /// A batch the transport defers goes back to the front unmarked: not
    /// acknowledged, not typed by another transport, no notification.
    #[test]
    fn tc_044_05_a_deferred_batch_is_requeued_without_herdr_and_without_a_redelivery() {
        let mut herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        herdr.deferred = true;
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        let outcome = deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(outcome, Outcome::SocketDeferred { requeued: 1 });
        assert_eq!(inbox.requeued.len(), 1);
        assert!(inbox.acked.is_empty());
        assert!(herdr.prompts.borrow().is_empty() && herdr.notifies.borrow().is_empty());
    }

    /// Tracing: TC-044-14
    /// ACs: FR-044-AC-14
    ///
    /// A batch the transport postpones (its time this tick is spent) is put back with no
    /// episode and no notification, so it never marks the lead notified: when the next
    /// tick finds the mail held (bypass mode) the notification is still sent, once.
    #[test]
    fn tc_044_14_a_postponed_batch_does_not_swallow_the_bypass_notification() {
        let name: relay_core::AgentName = "tl-lead".parse().unwrap();
        let mut episodes = crate::episodes::Episodes::default();
        let now = std::time::Instant::now();
        let mut herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        herdr.postponed = true;
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        let first = deliver(&herdr, episodes.notice(&name, now), &mut inbox);
        assert_eq!(first, Outcome::SocketPostponed { requeued: 1 });
        assert_eq!(first.episode(), None, "no episode");
        assert!(herdr.notifies.borrow().is_empty(), "no notification");
        assert!(
            !first
                .log_line("tl-lead", &"w1:p1".parse().unwrap())
                .contains("held")
        );
        assert!(inbox.acked.is_empty());
        assert_eq!(inbox.requeued.len(), 1);
        episodes.observe(&name, first.episode());
        // The next tick: held. Nobody was told yet, so the notice is still needed.
        let mut herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        herdr.held = Some("bypass".into());
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        let second = deliver(&herdr, episodes.notice(&name, now), &mut inbox);
        assert_eq!(
            second,
            Outcome::SocketHeld {
                why: "bypass".into(),
                notified: true
            }
        );
        assert_eq!(herdr.notifies.borrow().len(), 1, "exactly one notification");
    }

    /// Tracing: TC-044-03
    /// ACs: FR-044-AC-3
    ///
    /// The transport is told whom the batch is for: the lead, the session it
    /// registered under and the sender addresses.
    #[test]
    fn tc_044_03_the_transport_is_told_the_lead_the_session_and_the_senders() {
        let herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(
            *herdr.targets.borrow(),
            [(
                "tl-lead".to_owned(),
                Some(SID.to_owned()),
                "qsl-lead".to_owned()
            )]
        );
    }

    #[test]
    fn done_lead_is_also_delivered_to() {
        let herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Done, 3),
            ready(AgentStatus::Done, 3),
        ]);
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        let outcome = deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(outcome, Outcome::Delivered { messages: 1 });
    }

    #[test]
    fn working_lead_is_skipped_without_a_second_get() {
        let herdr = FakeHerdr::with_gets(vec![ready(AgentStatus::Working, 5)]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::SkippedWorking);
    }

    #[test]
    fn replacing_the_session_between_checks_never_pops_its_mail() {
        let herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready_with_session(AgentStatus::Idle, 10, Some("another-session")),
        ]);
        assert_eq!(
            deliver(&herdr, Notice::Needed, &mut FakeInbox::none()),
            Outcome::StaleRegistration
        );
        assert!(herdr.prompts.borrow().is_empty());
    }

    #[test]
    fn unknown_status_is_logged_distinctly_from_working() {
        let herdr = FakeHerdr::with_gets(vec![ready(AgentStatus::Unknown, 5)]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::SkippedUnknown);
        assert_ne!(
            outcome.log_line("tl-lead", &"w1:p1".parse().unwrap()),
            Outcome::SkippedWorking.log_line("tl-lead", &"w1:p1".parse().unwrap())
        );
    }

    #[test]
    fn blocked_lead_notifies_once_per_episode() {
        let herdr = FakeHerdr::with_gets(vec![ready(AgentStatus::Blocked, 1)]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::SkippedBlocked);
        assert_eq!(herdr.notifies.borrow().len(), 1);

        // Second tick of the same episode: already notified, so no second call.
        let herdr = FakeHerdr::with_gets(vec![ready(AgentStatus::Blocked, 1)]);
        let outcome = deliver(&herdr, Notice::Done, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::SkippedBlocked);
        assert!(herdr.notifies.borrow().is_empty());
    }

    /// Tracing: TC-N003-M-06 (NFR-003-M-6). Evidence: unit test.
    #[test]
    fn a_blocked_lead_whose_notification_retry_is_not_due_does_not_call_herdr() {
        let herdr = FakeHerdr::with_gets(vec![ready(AgentStatus::Blocked, 1)]);
        let outcome = deliver(&herdr, Notice::RetryLater, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::NotifyPending);
        assert!(herdr.notifies.borrow().is_empty());
    }

    #[test]
    fn seq_change_between_checks_skips_without_popping() {
        let herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Working, 11),
        ]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::SkippedSeqChanged);
    }

    #[test]
    fn prompt_rejected_before_input_is_requeued_in_order() {
        let mut herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        herdr.prompt_result = Err(ProcessError::Reported {
            code: "agent_blocked".into(),
            message: "blocked".into(),
        });
        let b = batch("hi");
        let expected_messages = b.messages.clone();
        let mut inbox = FakeInbox::once(PopOutcome::Popped(b));
        let outcome = deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(
            outcome,
            Outcome::PromptRejected {
                requeued: 1,
                error: DeliverError::NotSent(
                    ProcessError::Reported {
                        code: "agent_blocked".into(),
                        message: "blocked".into(),
                    }
                    .to_string()
                ),
            }
        );
        assert_eq!(inbox.requeued, vec![expected_messages]);
        // agent_blocked is a submission rejection, not a "the lead is
        // blocked" episode; no notification for it here.
        assert!(herdr.notifies.borrow().is_empty());
    }

    #[test]
    fn prompt_ambiguous_failure_is_not_requeued_and_notifies() {
        let mut herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        herdr.prompt_result = Err(ProcessError::Timeout {
            program: "herdr".into(),
            secs: 10,
        });
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        let outcome = deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(
            outcome,
            Outcome::PromptAmbiguous {
                error: DeliverError::Ambiguous(
                    ProcessError::Timeout {
                        program: "herdr".into(),
                        secs: 10,
                    }
                    .to_string()
                ),
                notified: true,
            }
        );
        assert!(
            inbox.requeued.is_empty(),
            "ambiguous failures are never requeued"
        );
        assert_eq!(herdr.notifies.borrow().len(), 1);
    }

    #[test]
    fn spawn_failure_on_prompt_is_also_requeued() {
        let mut herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        herdr.prompt_result = Err(ProcessError::Spawn {
            program: "herdr".into(),
            reason: "enoent".into(),
        });
        let mut inbox = FakeInbox::once(PopOutcome::Popped(batch("hi")));
        let outcome = deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(
            outcome,
            Outcome::PromptRejected {
                requeued: 1,
                error: DeliverError::NotSent(
                    ProcessError::Spawn {
                        program: "herdr".into(),
                        reason: "enoent".into(),
                    }
                    .to_string()
                ),
            }
        );
        assert_eq!(inbox.requeued.len(), 1);
    }

    #[test]
    fn no_agent_in_the_pane_is_reported_and_never_pops() {
        let herdr = FakeHerdr::with_gets(vec![Ok(None)]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::NoAgent);
    }

    #[test]
    fn get_error_is_reported() {
        let timeout = ProcessError::Timeout {
            program: "herdr".into(),
            secs: 10,
        };
        let herdr = FakeHerdr::with_gets(vec![Err(timeout.clone())]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::GetFailed(timeout));
    }

    #[test]
    fn race_with_the_hook_emptying_the_inbox_is_reported() {
        let herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        let outcome = deliver(
            &herdr,
            Notice::Needed,
            &mut FakeInbox::once(PopOutcome::Empty),
        );
        assert_eq!(outcome, Outcome::RaceEmptied);
        assert!(herdr.prompts.borrow().is_empty());
    }

    #[test]
    fn pop_failure_is_reported_distinctly_from_a_race() {
        let herdr = FakeHerdr::with_gets(vec![
            ready(AgentStatus::Idle, 10),
            ready(AgentStatus::Idle, 10),
        ]);
        let mut inbox = FakeInbox::once(PopOutcome::Failed("redis: connection refused".into()));
        let outcome = deliver(&herdr, Notice::Needed, &mut inbox);
        assert_eq!(
            outcome,
            Outcome::PopFailed("redis: connection refused".into())
        );
        assert_ne!(outcome, Outcome::RaceEmptied);
        assert!(herdr.prompts.borrow().is_empty());
    }

    #[test]
    fn mismatched_session_id_is_stale_and_never_gets() {
        let herdr = FakeHerdr::with_gets(vec![ready_with_session(
            AgentStatus::Idle,
            10,
            Some("other-sess"),
        )]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::StaleRegistration);

        // The same stale registration on the next tick is the same outcome; the
        // caller's episode tracking, not this function, keeps it out of the log.
        let herdr = FakeHerdr::with_gets(vec![ready_with_session(
            AgentStatus::Idle,
            10,
            Some("other-sess"),
        )]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::StaleRegistration);
    }

    #[test]
    fn missing_session_id_on_either_side_is_also_stale() {
        let herdr = FakeHerdr::with_gets(vec![ready_with_session(AgentStatus::Idle, 10, None)]);
        let outcome = deliver(&herdr, Notice::Needed, &mut FakeInbox::none());
        assert_eq!(outcome, Outcome::StaleRegistration);

        let herdr = FakeHerdr::with_gets(vec![ready(AgentStatus::Idle, 10)]);
        let outcome = try_deliver(
            &herdr,
            "tl-lead",
            "claude",
            &"w1:p1".parse().unwrap(),
            None,
            Notice::Needed,
            &mut FakeInbox::none(),
        );
        assert_eq!(outcome, Outcome::StaleRegistration);
    }
}
