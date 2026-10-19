//! Exhaustive evidence for the watcher's typing decision (FR-014-AC-1) and the
//! pop/prompt/requeue ordering it implies (FR-013's watcher half).
//!
//! `try_deliver` is a pure function of the registered session id, the answers
//! of its `Herdr` and `Inbox` seams, and a notice state, so its whole decision
//! space is enumerated: every registered and live session id (absent, equal,
//! different), every status, every pair of `state_change_seq` reads (equal or
//! not), every `get` outcome (no agent, error, present), every notice, every
//! pop outcome and every prompt outcome. Each combination is run and compared
//! with the requirement's rule; that is a proof over the model, and Kani could
//! lift it to arbitrary sequence numbers (only equality of the two matters).

use std::cell::RefCell;

use relay_core::Message;

use super::*;
use crate::herdr::AgentGet;

const STATUSES: [AgentStatus; 5] = [
    AgentStatus::Idle,
    AgentStatus::Done,
    AgentStatus::Working,
    AgentStatus::Blocked,
    AgentStatus::Unknown,
];

/// A scripted `Herdr` that records every prompt and notification.
struct Scripted {
    gets: RefCell<Vec<Result<Option<AgentGet>, ProcessError>>>,
    prompt: Result<(), ProcessError>,
    prompts: RefCell<Vec<String>>,
    notifies: RefCell<u32>,
}

impl Herdr for Scripted {
    fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
        let mut gets = self.gets.borrow_mut();
        assert!(
            !gets.is_empty(),
            "try_deliver called `get` more often than the script has answers"
        );
        gets.remove(0)
    }

    fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
        *self.notifies.borrow_mut() += 1;
        Ok(())
    }
}

impl Deliverer for Scripted {
    fn deliver(&self, _pane: &PaneId, text: &str) -> Result<(), DeliverError> {
        self.prompts.borrow_mut().push(text.to_owned());
        self.prompt.clone().map_err(DeliverError::from)
    }
}

/// An inbox that counts pops and remembers requeued batches.
struct Counting {
    pop: Option<PopOutcome>,
    pops: u32,
    requeued: Vec<Vec<Message>>,
    acked: Vec<relay_core::BatchId>,
}

impl Inbox for Counting {
    fn pop(&mut self) -> PopOutcome {
        self.pops += 1;
        self.pop.take().expect("try_deliver popped twice")
    }

    fn ack(&mut self, batch: &PoppedBatch) {
        self.acked.push(batch.batch);
    }

    fn requeue(&mut self, batch: &PoppedBatch) {
        self.requeued.push(batch.messages.clone());
    }

    fn dead_letter(&mut self, _batch: &PoppedBatch) {}
}

fn batch() -> PoppedBatch {
    PoppedBatch {
        batch: relay_core::BatchId::new(7),
        messages: vec![Message {
            from: "qsl-lead".parse().expect("valid"),
            to: "tl-lead".parse().expect("valid"),
            text: "hi".into(),
            sent_at_ms: 0,
            forwarded: false,
        }],
        rendered: "[ix-relay] from qsl-lead: hi".into(),
        senders: "qsl-lead".into(),
    }
}

fn agent(
    status: AgentStatus,
    seq: u64,
    session: Option<&str>,
) -> Result<Option<AgentGet>, ProcessError> {
    Ok(Some(AgentGet {
        status,
        state_change_seq: seq,
        session_id: session.map(|s| s.parse().unwrap()),
    }))
}

fn reported(code: &str) -> ProcessError {
    ProcessError::Reported {
        code: code.into(),
        message: "scripted".into(),
    }
}

/// Tracing: TC-014-01
/// ACs: FR-014-AC-1, FR-013-AC-2, FR-013-AC-3
///
/// For every combination of registered session id, live session id, status,
/// both sequence numbers, the outcome of each of the two `get` calls, notice, pop outcome and prompt
/// outcome (14,580 in all), the watcher prompts herdr only when both session
/// ids are present and equal, the status is idle or done, and the two
/// sequence numbers are equal; it pops only under the same conditions; it
/// pops exactly once and prompts exactly once when it does pop a batch; and it
/// notifies only for a blocked lead or an ambiguous prompt failure.
#[test]
fn tc_014_01_the_watcher_prompts_only_when_sessions_status_and_sequence_all_agree() {
    let sessions: [Option<&str>; 3] = [None, Some("a"), Some("b")];
    let registered_ids: [Option<&str>; 2] = [None, Some("a")];
    let mut combinations = 0u32;
    for registered in registered_ids {
        for live in sessions {
            for status in STATUSES {
                for recheck_seq_same in [true, false] {
                    for first in [Recheck::Present, Recheck::NoAgent, Recheck::Failed] {
                        for recheck in [Recheck::Present, Recheck::NoAgent, Recheck::Failed] {
                            for notice in [Notice::Needed, Notice::Done, Notice::RetryLater] {
                                for pop in [Pop::Popped, Pop::Empty, Pop::Failed] {
                                    for prompt in [
                                        Ok(()),
                                        Err(reported("agent_blocked")),
                                        Err(reported("other")),
                                    ] {
                                        combinations += 1;
                                        run(Case {
                                            registered,
                                            live,
                                            status,
                                            first,
                                            recheck_seq_same,
                                            recheck,
                                            notice,
                                            pop,
                                            prompt,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // 2 registered x 3 live x 5 statuses x 2 seq x 3 firsts x 3 rechecks x 3 notices x 3 pops x 3 prompts.
    assert_eq!(combinations, 2 * 3 * 5 * 2 * 3 * 3 * 3 * 3 * 3);
}

#[derive(Clone, Copy, Debug)]
enum Recheck {
    Present,
    NoAgent,
    Failed,
}

#[derive(Clone, Copy, Debug)]
enum Pop {
    Popped,
    Empty,
    Failed,
}

/// One point of the enumerated space.
struct Case<'a> {
    registered: Option<&'a str>,
    live: Option<&'a str>,
    status: AgentStatus,
    first: Recheck,
    recheck_seq_same: bool,
    recheck: Recheck,
    notice: Notice,
    pop: Pop,
    prompt: Result<(), ProcessError>,
}

fn run(case: Case<'_>) {
    let Case {
        registered,
        live,
        status,
        first,
        recheck_seq_same,
        recheck,
        notice,
        pop,
        prompt,
    } = case;
    let seq = 10;
    let first_get = match first {
        Recheck::Present => agent(status, seq, live),
        Recheck::NoAgent => Ok(None),
        Recheck::Failed => Err(reported("get_failed")),
    };
    let second = match recheck {
        Recheck::Present => agent(status, if recheck_seq_same { seq } else { seq + 1 }, live),
        Recheck::NoAgent => Ok(None),
        Recheck::Failed => Err(reported("get_failed")),
    };
    let herdr = Scripted {
        gets: RefCell::new(vec![first_get, second]),
        prompt: prompt.clone(),
        prompts: RefCell::default(),
        notifies: RefCell::default(),
    };
    let popped = batch();
    let mut inbox = Counting {
        pop: Some(match pop {
            Pop::Popped => PopOutcome::Popped(popped.clone()),
            Pop::Empty => PopOutcome::Empty,
            Pop::Failed => PopOutcome::Failed("scripted".into()),
        }),
        pops: 0,
        requeued: vec![],
        acked: vec![],
    };
    let outcome = try_deliver(
        &herdr,
        "tl-lead",
        "claude",
        &"w1:p1".parse().unwrap(),
        registered.map(|s| s.parse().unwrap()).as_ref(),
        notice,
        &mut inbox,
    );
    let context = format!(
        "registered {registered:?}, live {live:?}, {status:?}, recheck {recheck:?} same seq {recheck_seq_same}, \
         {notice:?}, pop {pop:?}, prompt {prompt:?} -> {outcome:?}"
    );

    let sessions_agree = matches!((registered, live), (Some(r), Some(l)) if r == l);
    let ready = matches!(status, AgentStatus::Idle | AgentStatus::Done);
    let recheck_agrees = matches!(recheck, Recheck::Present) && recheck_seq_same;
    let may_pop = matches!(first, Recheck::Present) && sessions_agree && ready && recheck_agrees;

    assert_eq!(inbox.pops, u32::from(may_pop), "pops: {context}");
    let prompts = herdr.prompts.borrow();
    let should_prompt = may_pop && matches!(pop, Pop::Popped);
    assert_eq!(
        prompts.len(),
        usize::from(should_prompt),
        "prompts: {context}"
    );
    if should_prompt {
        // Everything popped is handed to herdr, and nothing else is.
        assert_eq!(prompts[0], popped.rendered, "{context}");
        match &prompt {
            Ok(()) => {
                assert!(
                    inbox.requeued.is_empty(),
                    "delivered yet requeued: {context}"
                );
                assert_eq!(inbox.acked, vec![popped.batch], "acked: {context}");
                assert_eq!(outcome, Outcome::Delivered { messages: 1 }, "{context}");
            }
            Err(e) if e.definitely_not_sent() => {
                assert_eq!(
                    inbox.requeued,
                    vec![popped.messages.clone()],
                    "requeue: {context}"
                );
                assert!(
                    inbox.acked.is_empty(),
                    "a rejected prompt was acked: {context}"
                );
                assert!(
                    matches!(outcome, Outcome::PromptRejected { requeued: 1, .. }),
                    "{context}"
                );
            }
            Err(_) => {
                assert!(
                    inbox.requeued.is_empty(),
                    "an ambiguous failure was requeued: {context}"
                );
                assert!(
                    inbox.acked.is_empty(),
                    "an ambiguous failure was acknowledged: {context}"
                );
                assert!(
                    matches!(outcome, Outcome::PromptAmbiguous { notified: true, .. }),
                    "{context}"
                );
            }
        }
    } else {
        assert!(
            inbox.requeued.is_empty() && inbox.acked.is_empty(),
            "requeued or acked without a pop: {context}"
        );
    }
    // Notifications: a blocked lead when one is needed, or an ambiguous prompt.
    let blocked_notice = matches!(first, Recheck::Present)
        && sessions_agree
        && status == AgentStatus::Blocked
        && matches!(notice, Notice::Needed);
    let ambiguous = should_prompt && matches!(&prompt, Err(e) if !e.definitely_not_sent());
    assert_eq!(
        *herdr.notifies.borrow(),
        u32::from(blocked_notice) + u32::from(ambiguous),
        "notifies: {context}"
    );
}
