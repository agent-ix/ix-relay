//! Model-based evidence for the store: the real Redis-backed [`Relay`] is
//! driven with generated operation sequences and compared, after every step,
//! against a small in-memory model of the same protocol. The generated
//! sequences are sampled, not exhaustive, and the model's "two consumers" act
//! one after the other: this is not concurrency evidence (see
//! `escalation_model.rs` and `tests/store.rs` for the concurrent tests).
//!
//! Covers registry and alias exclusivity (FR-006-AC-4), tolerant registry
//! decoding (FR-008), send and broadcast (FR-010, FR-011, FR-015-AC-1/2),
//! budgeted pops and rejected entries (FR-012), requeue, two consumers and the
//! no-loss, no-duplicate, in-order protocol (FR-012/FR-013, FR-020-AC-3's
//! store half), and pruning (FR-009-AC-4's store half).
//!
//! Needs `IX_RELAY_TEST_URL`; each test returns early and says so on stderr
//! without it (`make test-redis` sets it). Every case runs in its own key
//! prefix and deletes its keys afterwards.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    num::NonZeroUsize,
};

use proptest::prelude::*;
use relay_core::{AgentName, Claim, Error, Message, Popper, Registration, Target};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

use relay_test_support::Bus;

fn lead(i: usize) -> AgentName {
    format!("lead-{i}").parse().expect("valid name")
}

const LEADS: usize = 5;

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// One inbox entry: a message, or a raw record somebody wrote by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    Msg(Message),
    /// Bytes that do not decode as a `Message` (not JSON, an unknown field,
    /// or not even UTF-8).
    Raw(Vec<u8>),
}

/// The fixed bytes a rendered message can take beyond its text, at most: the
/// signature, the longest sender label a listing shows, the broadcast mark, the
/// redelivery mark (FR-040), the `": "` and the separator that joins it to the
/// next message. Written from the requirement text (FR-012, FR-018-AC-4,
/// FR-040), not from the store.
const RENDER_FIXED_MAX: usize = "[ix-relay] from ".len()
    + relay_core::MAX_LABEL_BYTES
    + " (to all)".len()
    + " (redelivered)".len()
    + 2
    + 1;

/// The fewest bytes any stored message occupies apart from its text:
/// `{"from":"a","to":"b","text":"","sent_at_ms":0}`.
const MIN_STORED_FIXED: usize = 46;

/// A rendered message never takes more than this many bytes: the contract the
/// budget bounds (a line feed is 5 bytes rendered, an unsafe character 3).
fn rendered_max(m: &Message) -> usize {
    let mark = if matches!(m.to, Target::All) {
        0
    } else {
        " (to all)".len()
    };
    RENDER_FIXED_MAX - mark + relay_core::text::neutralise(&m.text).len()
}

impl Entry {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Entry::Msg(m) => serde_json::to_vec(m).expect("serialises"),
            Entry::Raw(raw) => raw.clone(),
        }
    }

    /// What one entry counts for in a budgeted pop, re-derived from the
    /// documented rule (`Relay::pop_inbox_budget`): its stored length, plus the
    /// fixed overhead a rendering can add over the smallest stored form, plus
    /// what its text grows by when rendered: an escaped line feed is 2 bytes
    /// stored and 5 printed (3 more); an escaped `\r`, `\b`, `\f`, a raw
    /// control byte or DEL, a C1 character and U+061C each print as the 3-byte
    /// replacement, so they add the difference. Written independently of the
    /// script that implements it.
    fn weight(&self) -> usize {
        let b = self.bytes();
        let mut extra = 0;
        let mut i = 0;
        while i < b.len() {
            match (b[i], b.get(i + 1).copied()) {
                (b'\\', Some(b'n')) => {
                    extra += 3;
                    i += 2;
                }
                (b'\\', Some(b'r' | b'b' | b'f')) => {
                    extra += 1;
                    i += 2;
                }
                (0..=31 | 127, _) => {
                    extra += 2;
                    i += 1;
                }
                (0xC2, Some(0x80..=0x9F)) | (0xD8, Some(0x9C)) => {
                    extra += 1;
                    i += 2;
                }
                _ => i += 1,
            }
        }
        b.len() + (RENDER_FIXED_MAX - MIN_STORED_FIXED) + extra
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Slot {
    Registered {
        last_seen_ms: i64,
    },
    /// A record in the registry hash that does not decode.
    Corrupt,
}

/// A consumer's popped-but-not-yet-settled batch and the inbox it came from.
#[derive(Debug, Default, Clone)]
struct Held {
    inbox: usize,
    batch: Vec<Message>,
    /// Everything the pop took, undecodable entries too: what a requeue of the
    /// in-flight batch puts back, byte for byte.
    taken: Vec<Entry>,
    /// The in-flight batch of a budgeted pop, which the consumer acknowledges
    /// or requeues; an unbudgeted pop is the interactive read and has none.
    id: Option<relay_core::BatchId>,
}

#[derive(Debug, Default)]
struct Model {
    agents: BTreeMap<usize, Slot>,
    inbox: BTreeMap<usize, VecDeque<Entry>>,
    log: Vec<Message>,
    /// Consumer -> the batch it popped last and has neither delivered nor requeued.
    held: BTreeMap<usize, Held>,
    /// Every message a store pop returned and its consumer settled for good.
    delivered: Vec<Message>,
    /// How many copies of each message text the sends queued (one per recipient).
    copies: BTreeMap<String, usize>,
    next_seen: i64,
}

/// What a send should do, per FR-010, FR-011 and FR-015.
#[derive(Debug, PartialEq, Eq)]
enum SendOutcome {
    Queued(Vec<usize>),
    UnknownAgent,
    TooLarge,
    Unsafe,
}

impl Model {
    fn send(&mut self, from: usize, to: &To, text: &str) -> SendOutcome {
        if text.len() > 4096 {
            return SendOutcome::TooLarge;
        }
        if text.chars().any(|c| {
            matches!(
                c,
                '\x1b' | '\r' | '\x0b' | '\u{85}' | '\u{202e}' | '\u{200b}'
            )
        }) {
            return SendOutcome::Unsafe;
        }
        let recipients: Vec<usize> = match to {
            To::One(name) => {
                let canonical = *name;
                if !self.agents.contains_key(&canonical) {
                    return SendOutcome::UnknownAgent;
                }
                vec![canonical]
            }
            To::All => self.agents.keys().copied().filter(|n| *n != from).collect(),
        };
        let message = Message {
            from: lead(from),
            to: match to {
                To::One(n) => Target::Agent(lead(*n)),
                To::All => Target::All,
            },
            text: text.to_owned(),
            sent_at_ms: 1,
            forwarded: false,
        };
        for r in &recipients {
            self.inbox
                .entry(*r)
                .or_default()
                .push_back(Entry::Msg(message.clone()));
        }
        *self.copies.entry(text.to_owned()).or_default() += recipients.len();
        self.log.push(message);
        SendOutcome::Queued(recipients)
    }

    /// The oldest entries a budgeted pop takes: at most `count`, the run whose
    /// summed weight fits `bytes`, and always at least the first.
    fn take(&mut self, who: usize, count: usize, bytes: Option<usize>) -> Vec<Entry> {
        let queue = self.inbox.entry(who).or_default();
        let mut taken = 0;
        let mut total = 0;
        for (i, e) in queue.iter().enumerate().take(count) {
            let len = e.weight();
            if i > 0 && bytes.is_some_and(|budget| total + len > budget) {
                break;
            }
            total += len;
            taken = i + 1;
        }
        queue.drain(..taken).collect()
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum To {
    One(usize),
    All,
}

#[derive(Debug, Clone)]
enum Body {
    Plain,
    Multiline,
    Exactly4096,
    Oversize,
    Unsafe(char),
}

#[derive(Debug, Clone, Copy)]
enum RawKind {
    UnknownField,
    NotJson,
    NotUtf8,
}

#[derive(Debug, Clone)]
enum Op {
    Register(usize),
    CorruptRegistry(usize),
    Send {
        from: usize,
        to: To,
        body: Body,
    },
    Pop {
        consumer: usize,
        who: usize,
        count: usize,
        bytes: Option<usize>,
        budgeted: bool,
    },
    Deliver {
        consumer: usize,
    },
    Requeue {
        consumer: usize,
    },
    CorruptInbox {
        who: usize,
        kind: RawKind,
    },
    Prune {
        who: usize,
        stale: bool,
    },
}

fn idx() -> impl Strategy<Value = usize> {
    0..LEADS
}

fn op() -> impl Strategy<Value = Op> {
    let to = prop_oneof![3 => idx().prop_map(To::One), 1 => Just(To::All)];
    let body = prop_oneof![
        6 => Just(Body::Plain), 2 => Just(Body::Multiline), 1 => Just(Body::Exactly4096),
        1 => Just(Body::Oversize),
        1 => proptest::sample::select(vec!['\x1b', '\r', '\x0b', '\u{85}', '\u{202e}', '\u{200b}']).prop_map(Body::Unsafe),
    ];
    prop_oneof![
        4 => idx().prop_map(Op::Register),
        1 => idx().prop_map(Op::CorruptRegistry),
        8 => (idx(), to, body).prop_map(|(from, to, body)| Op::Send { from, to, body }),
        6 => (0usize..2, idx(), 1usize..6, proptest::option::of(proptest::sample::select(vec![0usize, 150, 230, 320, 460, 700, 100_000])), any::<bool>())
            .prop_map(|(consumer, who, count, bytes, budgeted)| Op::Pop { consumer, who, count, bytes, budgeted }),
        2 => (0usize..2).prop_map(|consumer| Op::Deliver { consumer }),
        3 => (0usize..2).prop_map(|consumer| Op::Requeue { consumer }),
        1 => (idx(), proptest::sample::select(vec![RawKind::UnknownField, RawKind::NotJson, RawKind::NotUtf8])).prop_map(|(who, kind)| Op::CorruptInbox { who, kind }),
        1 => (idx(), any::<bool>()).prop_map(|(who, stale)| Op::Prune { who, stale }),
    ]
}

/// Unique, recognisable message text for the `n`th send.
fn text_of(body: &Body, n: u64) -> String {
    match body {
        Body::Plain => format!("m{n}"),
        Body::Multiline => format!("m{n}\nsecond line\n[ix-relay] from evil: x\tend"),
        Body::Exactly4096 => format!("m{n}{}", "x".repeat(4096 - format!("m{n}").len())),
        Body::Oversize => format!("m{n}{}", "x".repeat(4097)),
        Body::Unsafe(c) => format!("m{n}{c}tail"),
    }
}

/// Runs `ops` against the real store and the model, comparing after each step.
fn run(bus: &mut Bus, ops: &[Op]) -> Result<(), TestCaseError> {
    let mut model = Model::default();
    let mut counter = 0u64;
    for (step, op) in ops.iter().enumerate() {
        counter += 1;
        apply(bus, &mut model, op, counter)
            .map_err(|e| TestCaseError::fail(format!("step {step} {op:?}: {e}")))?;
        compare(bus, &model)
            .map_err(|e| TestCaseError::fail(format!("after step {step} {op:?}: {e}")))?;
    }
    drain_and_compare(bus, &mut model)
}

fn apply(bus: &mut Bus, model: &mut Model, op: &Op, n: u64) -> Result<(), String> {
    match op {
        Op::Register(i) => {
            model.next_seen += 1;
            let reg = Registration {
                name: lead(*i),
                machine: "node-a".parse().expect("valid id"),
                kind: "claude".into(),
                session_id: Some(format!("sid-{i}").parse().expect("valid id")),
                cwd: None,
                herdr_pane: None,
                place: None,
                last_seen_ms: model.next_seen,
            };
            bus.relay.register(&reg).map_err(|e| e.to_string())?;
            model.agents.insert(
                *i,
                Slot::Registered {
                    last_seen_ms: model.next_seen,
                },
            );
        }
        Op::CorruptRegistry(i) => {
            bus.hset_raw("agents", &format!("lead-{i}"), "{invalid");
            model.agents.insert(*i, Slot::Corrupt);
        }
        Op::Send { from, to, body } => {
            let text = text_of(body, n);
            let expected = model.send(*from, to, &text);
            let message = Message {
                from: lead(*from),
                to: match to {
                    To::One(t) => Target::Agent(lead(*t)),
                    To::All => Target::All,
                },
                text,
                sent_at_ms: 1,
                forwarded: false,
            };
            let got = bus.relay.send(&message);
            match (expected, got) {
                (SendOutcome::Queued(rs), Ok(got)) => {
                    let want: Vec<AgentName> = rs.iter().map(|r| lead(*r)).collect();
                    if got != want {
                        return Err(format!("recipients {got:?}, model {want:?}"));
                    }
                }
                (SendOutcome::UnknownAgent, Err(Error::UnknownAgent(_)))
                | (SendOutcome::TooLarge, Err(Error::MessageTooLarge { .. }))
                | (SendOutcome::Unsafe, Err(Error::UnsafeMessageText { .. })) => {}
                (expected, got) => return Err(format!("model {expected:?}, store {got:?}")),
            }
        }
        Op::Pop {
            consumer,
            who,
            count,
            bytes,
            budgeted,
        } => {
            let before = model.inbox.entry(*who).or_default().len();
            if let Some(previous) = model.held.remove(consumer) {
                // A consumer that pops again has handed its previous batch over.
                if let Some(id) = previous.id {
                    bus.relay
                        .ack(&lead(previous.inbox), id, relay_core::Popper::Hook)
                        .map_err(|e| e.to_string())?;
                }
                model.delivered.extend(previous.batch);
            }
            let taken = model.take(*who, *count, if *budgeted { *bytes } else { None });
            let want_messages: Vec<Message> = taken
                .iter()
                .filter_map(|e| {
                    if let Entry::Msg(m) = e {
                        Some(m.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let want_rejected = taken.len() - want_messages.len();
            let max = NonZeroUsize::new(*count).expect("count is at least 1");
            let (messages, rejected, pending, id) = if *budgeted {
                let r = bus
                    .relay
                    .pop_inbox_budget(
                        &lead(*who),
                        max,
                        bytes.unwrap_or(usize::MAX),
                        Claim::new(Popper::Hook),
                    )
                    .map_err(|e| e.to_string())?;
                (r.messages, r.rejected.len(), Some(r.pending), r.batch)
            } else {
                let r = bus
                    .relay
                    .pop_inbox(&lead(*who), max)
                    .map_err(|e| e.to_string())?;
                (r.messages, r.rejected.len(), None, None)
            };
            // The contract the budget exists for: a batch of more than one
            // message renders within it, whatever the exact weight rule.
            if let (true, Some(budget)) = (*budgeted && messages.len() > 1, bytes) {
                let rendered: usize = messages.iter().map(rendered_max).sum();
                if rendered > *budget {
                    return Err(format!(
                        "a batch of {} renders up to {rendered} bytes, over the budget {budget}",
                        messages.len()
                    ));
                }
            }
            if messages != want_messages || rejected != want_rejected {
                return Err(format!(
                    "pop returned {} messages / {rejected} rejected, model {} / {want_rejected}",
                    messages.len(),
                    want_messages.len()
                ));
            }
            let left = model.inbox[who].len();
            // FR-012-AC-1/3: what was taken plus what remains is what was there.
            if messages.len() + rejected + left != before {
                return Err("popped plus remaining is not what was queued".into());
            }
            if let Some(pending) = pending
                && pending != left
            {
                return Err(format!("pending {pending}, inbox holds {left}"));
            }
            if before > 0 && messages.is_empty() && rejected == 0 {
                return Err("a non-empty inbox popped nothing".into());
            }
            model.held.insert(
                *consumer,
                Held {
                    inbox: *who,
                    batch: messages,
                    taken,
                    id,
                },
            );
        }
        Op::Deliver { consumer } => {
            if let Some(held) = model.held.remove(consumer) {
                if let Some(id) = held.id {
                    bus.relay
                        .ack(&lead(held.inbox), id, relay_core::Popper::Hook)
                        .map_err(|e| e.to_string())?;
                }
                model.delivered.extend(held.batch);
            }
        }
        Op::Requeue { consumer } => {
            if let Some(held) = model.held.remove(consumer) {
                match held.id {
                    Some(id) => {
                        bus.relay
                            .requeue_batch(&lead(held.inbox), id, relay_core::Popper::Hook)
                            .map_err(|e| e.to_string())?;
                    }
                    None => bus
                        .relay
                        .requeue_front(&lead(held.inbox), &held.batch)
                        .map_err(|e| e.to_string())?,
                }
                let queue = model.inbox.entry(held.inbox).or_default();
                let restored: Vec<Entry> = if held.id.is_some() {
                    held.taken
                } else {
                    held.batch.into_iter().map(Entry::Msg).collect()
                };
                for entry in restored.into_iter().rev() {
                    queue.push_front(entry);
                }
            }
        }
        Op::CorruptInbox { who, kind } => {
            let raw: &[u8] = match kind {
                RawKind::UnknownField => {
                    br#"{"from":"lead-0","to":"lead-1","text":"x","sent_at_ms":1,"extra":true}"#
                }
                RawKind::NotJson => b"definitely not json",
                RawKind::NotUtf8 => &[0xff, 0xfe, 0x80],
            };
            bus.rpush_raw(&format!("lead-{who}"), raw);
            model
                .inbox
                .entry(*who)
                .or_default()
                .push_back(Entry::Raw(raw.to_vec()));
        }
        Op::Prune { who, stale } => {
            let seen = match model.agents.get(who) {
                Some(Slot::Registered { last_seen_ms }) => *last_seen_ms + i64::from(*stale),
                _ => 0,
            };
            let got = bus
                .relay
                .prune(&lead(*who), seen)
                .map_err(|e| e.to_string())?;
            let matches = matches!(model.agents.get(who), Some(Slot::Registered { last_seen_ms }) if *last_seen_ms == seen);
            match (matches, got) {
                (true, Some(removed)) => {
                    if !removed.is_empty() {
                        return Err("unexpected prune payload".into());
                    }
                    model.agents.remove(who);
                }
                (false, None) => {}
                (m, g) => return Err(format!("prune matched={m}, store returned {g:?}")),
            }
        }
    }
    Ok(())
}

/// The observable state of the store equals the model's.
fn compare(bus: &mut Bus, model: &Model) -> Result<(), String> {
    for i in 0..LEADS {
        let want = model.inbox.get(&i).map_or(0, VecDeque::len);
        let got = bus.relay.inbox_len(&lead(i)).map_err(|e| e.to_string())?;
        if got != want {
            return Err(format!("inbox lead-{i} holds {got}, model {want}"));
        }
    }
    let list = bus.relay.agents().map_err(|e| e.to_string())?;
    let registered: BTreeSet<String> = list
        .registrations
        .iter()
        .map(|r| r.name.to_string())
        .collect();
    let skipped: BTreeSet<String> = list.skipped.iter().map(|s| s.field.clone()).collect();
    let want_registered: BTreeSet<String> = model
        .agents
        .iter()
        .filter(|(_, s)| matches!(s, Slot::Registered { .. }))
        .map(|(i, _)| format!("lead-{i}"))
        .collect();
    let want_skipped: BTreeSet<String> = model
        .agents
        .iter()
        .filter(|(_, s)| matches!(s, Slot::Corrupt))
        .map(|(i, _)| format!("lead-{i}"))
        .collect();
    if registered != want_registered || skipped != want_skipped {
        return Err(format!(
            "registry {registered:?} + skipped {skipped:?}, model {want_registered:?} + {want_skipped:?}"
        ));
    }
    for r in &list.registrations {
        let i: usize = r.name.as_str()["lead-".len()..].parse().expect("index");
        if model.agents.get(&i)
            != Some(&Slot::Registered {
                last_seen_ms: r.last_seen_ms,
            })
        {
            return Err(format!("{} last seen {}", r.name, r.last_seen_ms));
        }
    }
    let log = bus.relay.log(0, 100_000).map_err(|e| e.to_string())?;
    if log != model.log {
        return Err(format!(
            "log holds {} messages, model {}",
            log.len(),
            model.log.len()
        ));
    }
    Ok(())
}

/// Drains every inbox and checks nothing was lost, duplicated or reordered:
/// what remains is exactly the model's queue, and every message ever sent to
/// a lead is in exactly one of its inbox, a consumer's held batch, or the
/// delivered set.
fn drain_and_compare(bus: &mut Bus, model: &mut Model) -> Result<(), TestCaseError> {
    let big = NonZeroUsize::new(1000).expect("nonzero");
    for i in 0..LEADS {
        let want = model.inbox.remove(&i).unwrap_or_default();
        let got = bus
            .relay
            .pop_inbox(&lead(i), big)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let want_msgs: Vec<Message> = want
            .iter()
            .filter_map(|e| {
                if let Entry::Msg(m) = e {
                    Some(m.clone())
                } else {
                    None
                }
            })
            .collect();
        prop_assert_eq!(&got.messages, &want_msgs, "drain of lead-{}", i);
        prop_assert_eq!(got.rejected.len(), want.len() - want_msgs.len());
        model.delivered.extend(got.messages);
    }
    for held in model.held.values() {
        model.delivered.extend(held.batch.clone());
    }
    // No loss and no duplicate: each text was queued once per recipient, and
    // the messages the store returned for it, settled or still held, number
    // exactly that many. (Texts are unique per send.)
    let mut returned: BTreeMap<String, usize> = BTreeMap::new();
    for m in &model.delivered {
        *returned.entry(m.text.clone()).or_default() += 1;
    }
    returned.retain(|_, n| *n > 0);
    let queued: BTreeMap<String, usize> = model
        .copies
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(t, n)| (t.clone(), *n))
        .collect();
    prop_assert_eq!(returned, queued, "messages lost or duplicated");
    Ok(())
}

fn cases() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(cases())]

    /// Tracing: TC-006-04, TC-008-02, TC-010-01, TC-010-03, TC-010-04, TC-011-01, TC-012-01, TC-012-02, TC-012-03, TC-012-05, TC-013-01, TC-015-01, TC-015-02
    /// ACs: FR-006-AC-4, FR-008-AC-2, FR-010-AC-1, FR-010-AC-3, FR-010-AC-4, FR-011-AC-1, FR-012-AC-1, FR-012-AC-2, FR-012-AC-3, FR-012-AC-5, FR-013-AC-1, FR-015-AC-1, FR-015-AC-2
    ///
    /// Model-based: any sequence of register, alias, raw-corruption, send,
    /// broadcast, budgeted and unbudgeted pop by either of two consumers,
    /// deliver, requeue and prune leaves the real store observably equal to
    /// the model after every step, and the final drain equals the model's
    /// queues. This checks, against the real Lua scripts: exactly-one queueing
    /// per recipient, refusal without side effects, pop order, count and byte
    /// budgets, `pending`, rejected entries, requeue order, and alias/registry
    /// exclusivity.
    #[test]
    fn tc_012_01_the_store_matches_the_queue_model_over_generated_operation_sequences(
        ops in proptest::collection::vec(op(), 1..40),
    ) {
        let Some(mut bus) = Bus::connect("model") else { return Ok(()) };
        run(&mut bus, &ops)?;
    }
}

/// Tracing: TC-012-05
/// ACs: FR-012-AC-5
///
/// A queued entry that is not even UTF-8 (bytes some other writer pushed) is
/// popped and reported as rejected like any other undecodable entry, and the
/// good messages around it are returned, by both pops. The failure this guards
/// against is the worst kind: the server has already removed the batch when
/// the client fails to convert the reply, so every message in it is lost.
#[test]
fn tc_012_05_a_non_utf8_entry_is_rejected_and_the_neighbouring_messages_survive() {
    for budgeted in [false, true] {
        let Some(mut bus) = Bus::connect("non-utf8") else {
            return;
        };
        let good = |n: u64| Message {
            from: lead(0),
            to: Target::Agent(lead(1)),
            text: format!("m{n}"),
            sent_at_ms: 1,
            forwarded: false,
        };
        bus.relay
            .register(&Registration {
                name: lead(1),
                machine: "node-a".parse().expect("valid id"),
                kind: "claude".into(),
                session_id: None,
                cwd: None,
                herdr_pane: None,
                place: None,
                last_seen_ms: 1,
            })
            .expect("register");
        bus.relay.send(&good(1)).expect("send first");
        redis::cmd("RPUSH")
            .arg(bus.key("inbox:lead-1"))
            .arg(&[0xffu8, 0xfe, 0x80][..])
            .exec(&mut bus.raw)
            .expect("plant bytes");
        bus.relay.send(&good(2)).expect("send third");
        let max = NonZeroUsize::new(10).expect("nonzero");
        let (messages, rejected) = if budgeted {
            let r = bus
                .relay
                .pop_inbox_budget(&lead(1), max, 100_000, Claim::new(Popper::Hook))
                .expect("pop must not fail as a whole");
            (r.messages, r.rejected.len())
        } else {
            let r = bus
                .relay
                .pop_inbox(&lead(1), max)
                .expect("pop must not fail as a whole");
            (r.messages, r.rejected.len())
        };
        assert_eq!(messages, vec![good(1), good(2)], "budgeted={budgeted}");
        assert_eq!(rejected, 1, "budgeted={budgeted}");
    }
}

/// Tracing: TC-012-02
/// ACs: FR-012-AC-2
///
/// The byte budget, deterministically (the model only reaches it by chance):
/// three identical messages, each with an escaped line feed so the rendering
/// grows, popped with a budget of exactly two weights take two; one byte less
/// takes one; a zero budget still takes the first; a count limit of one wins
/// over a large budget; and whatever is taken renders within the budget.
#[test]
fn tc_012_02_the_byte_budget_takes_the_longest_run_that_fits_and_at_least_one() {
    for (budget_in_weights, extra, count, want) in [
        (2usize, 0isize, 5usize, 2usize),
        (2, -1, 5, 1),
        (3, 0, 5, 3),
        (0, 0, 5, 1),
        (100, 0, 1, 1),
    ] {
        let Some(mut bus) = Bus::connect("budget") else {
            return;
        };
        bus.relay
            .register(&Registration {
                name: lead(1),
                machine: "node-a".parse().expect("valid id"),
                kind: "claude".into(),
                session_id: None,
                cwd: None,
                herdr_pane: None,
                place: None,
                last_seen_ms: 1,
            })
            .expect("register");
        let message = |n: u32| Message {
            from: lead(0),
            to: Target::Agent(lead(1)),
            text: format!("line {n}\nsecond line"),
            sent_at_ms: 1,
            forwarded: false,
        };
        for n in 0..3 {
            bus.relay.send(&message(n)).expect("send");
        }
        let weight = Entry::Msg(message(0)).weight();
        let budget =
            usize::try_from(isize::try_from(budget_in_weights * weight).expect("fits") + extra)
                .expect("non-negative");
        let got = bus
            .relay
            .pop_inbox_budget(
                &lead(1),
                NonZeroUsize::new(count).expect("nonzero"),
                budget,
                Claim::new(Popper::Hook),
            )
            .expect("pop");
        assert_eq!(
            got.messages.len(),
            want,
            "budget {budget} (weight {weight}), count {count}"
        );
        assert_eq!(got.pending, 3 - want);
        if got.messages.len() > 1 {
            let rendered: usize = got.messages.iter().map(rendered_max).sum();
            assert!(rendered <= budget, "{rendered} > {budget}");
        }
    }
}
