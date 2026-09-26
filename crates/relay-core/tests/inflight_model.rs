//! Model-based evidence for the in-flight list (FR-020, FR-040, AGE-2032): the
//! real Redis-backed [`Relay`] is driven with generated sequences of sends,
//! pops by two consumers, acknowledgements (current and stale), requeues,
//! abandoned batches, lease expiries and reaps, and compared after every step
//! against an in-memory model of the protocol written from the requirement text,
//! not from the Lua.
//!
//! Three checks stand apart from the model, because a model that shares the
//! script's mistake would agree with it:
//!
//! - **Conservation**, read from Redis itself: every message sent is in exactly
//!   one of the inbox, the in-flight record, the dead-letter list, or the set the
//!   test settled by a successful acknowledgement.
//! - **No early duplicate**, a monitor of leases: a message that a pop returned
//!   is not returned again before its lease deadline unless it was
//!   acknowledged or requeued first.
//! - **Every pop is checked against what it returns**, including the redelivery
//!   flag the recipient is shown.
//!
//! The clock is the test's own ([`Now::AtMs`]): no step waits for a lease.
//!
//! Needs `IX_RELAY_TEST_URL`; each test returns early and says so on stderr
//! without it (`make test-redis` sets it).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    num::NonZeroUsize,
    time::Duration,
};

use proptest::prelude::*;
use relay_core::{
    AgentName, BatchId, Claim, DEAD_LETTER_CAP, MAX_INFLIGHT_BATCHES, MAX_REDELIVERIES, Message,
    Now, Popper, Registration, Target,
};

use relay_test_support::Bus;

const START_MS: i64 = 1_000_000;

fn lead(i: usize) -> AgentName {
    format!("lead-{i}").parse().expect("valid name")
}

fn message(n: u32) -> Message {
    Message {
        from: lead(0),
        to: Target::Agent(lead(1)),
        text: format!("m-{n}"),
        sent_at_ms: i64::from(n),
    }
}

/// The message number a text carries.
fn number(text: &str) -> u32 {
    text.strip_prefix("m-")
        .and_then(|n| n.parse().ok())
        .expect("a test message")
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Batch {
    id: u64,
    deadline: i64,
    messages: Vec<u32>,
}

#[derive(Debug, Default)]
struct Model {
    now: i64,
    last_id: u64,
    inbox: VecDeque<u32>,
    /// In flight, in pop order.
    inflight: Vec<Batch>,
    /// How many times each message's lease expired and it went back.
    redelivered: BTreeMap<u32, u32>,
    dead: Vec<u32>,
    /// Messages settled by an acknowledgement that deleted their batch.
    settled: BTreeSet<u32>,
}

impl Model {
    /// FR-020: every due batch goes back to the front of the inbox, oldest
    /// batch first, each in its own order, each batch on its own lease; a message already redelivered
    /// [`MAX_REDELIVERIES`] times is dead-lettered instead (FR-040).
    fn reap(&mut self) -> (usize, usize) {
        let (mut requeued, mut dead) = (0, 0);
        let mut back = Vec::new();
        let mut keep = Vec::new();
        for batch in std::mem::take(&mut self.inflight) {
            if batch.deadline > self.now {
                keep.push(batch);
                continue;
            }
            for m in batch.messages {
                let count = self.redelivered.get(&m).copied().unwrap_or(0);
                if count >= MAX_REDELIVERIES {
                    self.redelivered.remove(&m);
                    self.dead.push(m);
                    dead += 1;
                } else {
                    self.redelivered.insert(m, count + 1);
                    back.push(m);
                    requeued += 1;
                }
            }
        }
        self.inflight = keep;
        for m in back.into_iter().rev() {
            self.inbox.push_front(m);
        }
        (requeued, dead)
    }

    /// A pop at `now`: reap, refuse at the cap, else take the oldest `count`.
    fn pop(&mut self, count: usize, lease_ms: i64) -> Option<(u64, Vec<(u32, bool)>)> {
        self.reap();
        if self.inflight.len() >= MAX_INFLIGHT_BATCHES {
            return None;
        }
        let n = count.min(self.inbox.len());
        if n == 0 {
            return None;
        }
        self.last_id += 1;
        let messages: Vec<u32> = self.inbox.drain(..n).collect();
        let shown = messages
            .iter()
            .map(|m| (*m, self.redelivered.get(m).copied().unwrap_or(0) > 0))
            .collect();
        self.inflight.push(Batch {
            id: self.last_id,
            deadline: self.now + lease_ms,
            messages,
        });
        Some((self.last_id, shown))
    }

    /// FR-020: an acknowledgement deletes exactly the batch it names.
    fn ack(&mut self, id: u64) -> bool {
        let Some(at) = self.inflight.iter().position(|b| b.id == id) else {
            return false;
        };
        let batch = self.inflight.remove(at);
        for m in batch.messages {
            self.redelivered.remove(&m);
            self.settled.insert(m);
        }
        true
    }

    /// FR-020: a requeue puts exactly that batch back, in order, uncounted.
    fn requeue(&mut self, id: u64) -> usize {
        let Some(at) = self.inflight.iter().position(|b| b.id == id) else {
            return 0;
        };
        let batch = self.inflight.remove(at);
        let moved = batch.messages.len();
        for m in batch.messages.into_iter().rev() {
            self.inbox.push_front(m);
        }
        moved
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Which batch an acknowledgement or a requeue names.
#[derive(Debug, Clone, Copy)]
enum Named {
    /// What consumer `c` last popped and still holds.
    Held(usize),
    /// The `k`th batch id ever handed out (modulo how many there are): often one
    /// that expired, was requeued or was acknowledged already.
    Ever(usize),
}

#[derive(Debug, Clone)]
enum Op {
    Send,
    Pop {
        consumer: usize,
        count: usize,
        watcher: bool,
        lease_ms: i64,
    },
    Ack(Named),
    Requeue(Named),
    /// The consumer dies with its batch: nothing is acknowledged.
    Abandon {
        consumer: usize,
    },
    Advance(i64),
    Reap,
}

fn named() -> impl Strategy<Value = Named> {
    prop_oneof![
        (0usize..2).prop_map(Named::Held),
        (0usize..64).prop_map(Named::Ever),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => Just(Op::Send),
        7 => (0usize..2, 1usize..5, any::<bool>(), prop::sample::select(vec![1_000i64, 20_000, 60_000]))
            .prop_map(|(consumer, count, watcher, lease_ms)| Op::Pop { consumer, count, watcher, lease_ms }),
        4 => named().prop_map(Op::Ack),
        2 => named().prop_map(Op::Requeue),
        2 => (0usize..2).prop_map(|consumer| Op::Abandon { consumer }),
        4 => prop::sample::select(vec![0i64, 500, 1_000, 25_000, 61_000]).prop_map(Op::Advance),
        1 => Just(Op::Reap),
    ]
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

struct Run {
    bus: Bus,
    model: Model,
    sent: u32,
    /// Consumer -> the batch it holds.
    held: [Option<(BatchId, Vec<u32>)>; 2],
    /// Every batch id handed out, in order.
    ids: Vec<BatchId>,
    /// The popper each batch id was handed to.
    poppers: BTreeMap<u64, Popper>,
    /// Message -> the deadline of the lease it was last returned under, cleared
    /// when it is acknowledged or requeued. The monitor of "no early duplicate".
    leased_until: BTreeMap<u32, i64>,
}

fn register(bus: &mut Bus, i: usize) {
    bus.relay
        .register(&Registration {
            name: lead(i),
            machine: "node-a".parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 1,
        })
        .expect("register");
}

impl Run {
    /// Who popped `id` (a batch id nobody was handed is the hook's, as any stale id).
    fn popper_of(&self, id: BatchId) -> Popper {
        self.poppers.get(&id.get()).copied().unwrap_or(Popper::Hook)
    }

    fn new(test: &str) -> Option<Self> {
        let mut bus = Bus::connect(test)?;
        register(&mut bus, 0);
        register(&mut bus, 1);
        Some(Self {
            bus,
            model: Model {
                now: START_MS,
                ..Model::default()
            },
            sent: 0,
            held: [None, None],
            ids: Vec::new(),
            poppers: BTreeMap::new(),
            leased_until: BTreeMap::new(),
        })
    }

    fn range(&mut self, tail: &str) -> Vec<Vec<u8>> {
        redis::cmd("LRANGE")
            .arg(self.bus.key(tail))
            .arg(0)
            .arg(-1)
            .query(&mut self.bus.raw)
            .expect("LRANGE")
    }

    fn resolve(&self, named: Named) -> Option<BatchId> {
        match named {
            Named::Held(c) => self.held[c].as_ref().map(|(id, _)| *id),
            Named::Ever(k) => (!self.ids.is_empty()).then(|| self.ids[k % self.ids.len()]),
        }
    }

    fn step(&mut self, op: &Op) -> Result<(), String> {
        let now = Now::AtMs(self.model.now);
        let me = lead(1);
        match op {
            Op::Send => {
                self.bus
                    .relay
                    .send(&message(self.sent))
                    .map_err(|e| e.to_string())?;
                self.model.inbox.push_back(self.sent);
                self.sent += 1;
            }
            Op::Advance(ms) => self.model.now += ms,
            Op::Reap => {
                let got = self
                    .bus
                    .relay
                    .reap_expired(&me, now)
                    .map_err(|e| e.to_string())?;
                let (requeued, dead) = self.model.reap();
                if (got.requeued, got.dead) != (requeued, dead) {
                    return Err(format!(
                        "reap returned {got:?}, model requeued {requeued} and dead-lettered {dead}"
                    ));
                }
            }
            Op::Pop {
                consumer,
                count,
                watcher,
                lease_ms,
            } => {
                let popper = if *watcher {
                    Popper::Watcher
                } else {
                    Popper::Hook
                };
                let claim = Claim::new(popper)
                    .with_lease(Duration::from_millis(
                        u64::try_from(*lease_ms).expect("positive"),
                    ))
                    .at(now);
                let got = self
                    .bus
                    .relay
                    .pop_inbox_budget(
                        &me,
                        NonZeroUsize::new(*count).expect("nonzero"),
                        usize::MAX,
                        claim,
                    )
                    .map_err(|e| e.to_string())?;
                let want = self.model.pop(*count, *lease_ms);
                let shown: Vec<(u32, bool)> = got
                    .messages
                    .iter()
                    .map(|m| number(&m.text))
                    .zip(got.redelivered.iter().copied())
                    .collect();
                match (&want, got.batch) {
                    (Some((_, want_shown)), Some(id)) => {
                        if &shown != want_shown {
                            return Err(format!("pop showed {shown:?}, model {want_shown:?}"));
                        }
                        // The monitor: nothing comes back while its lease stands.
                        for (m, _) in &shown {
                            if let Some(until) = self.leased_until.get(m)
                                && *until > self.model.now
                            {
                                return Err(format!(
                                    "m-{m} returned again at {} under a lease to {until}",
                                    self.model.now
                                ));
                            }
                            self.leased_until.insert(*m, self.model.now + lease_ms);
                        }
                        self.ids.push(id);
                        self.poppers.insert(id.get(), popper);
                        self.held[*consumer] = Some((id, shown.iter().map(|(m, _)| *m).collect()));
                    }
                    (None, None) => {
                        if !shown.is_empty() {
                            return Err("a pop with no batch returned messages".into());
                        }
                    }
                    (want, got) => {
                        return Err(format!("pop batch {got:?}, model {want:?}"));
                    }
                }
                let pending = self.model.inbox.len();
                if got.pending != pending {
                    return Err(format!("pending {}, model {pending}", got.pending));
                }
            }
            Op::Ack(target) => {
                let Some(id) = self.resolve(*target) else {
                    return Ok(());
                };
                let got = self
                    .bus
                    .relay
                    .ack(&me, id, self.popper_of(id))
                    .map_err(|e| e.to_string())?;
                let want = self.model.ack(id.get());
                if got != want {
                    return Err(format!("ack of {id} deleted {got}, model {want}"));
                }
                if got {
                    self.leased_until
                        .retain(|m, _| !self.model.settled.contains(m));
                }
            }
            Op::Requeue(target) => {
                let Some(id) = self.resolve(*target) else {
                    return Ok(());
                };
                let got = self
                    .bus
                    .relay
                    .requeue_batch(&me, id, self.popper_of(id))
                    .map_err(|e| e.to_string())?;
                let want = self.model.requeue(id.get());
                if got != want {
                    return Err(format!("requeue of {id} moved {got}, model {want}"));
                }
                if got > 0 {
                    let inbox = self.model.inbox.clone();
                    self.leased_until.retain(|m, _| !inbox.contains(m));
                }
            }
            Op::Abandon { consumer } => self.held[*consumer] = None,
        }
        self.check()
    }

    /// The store, read back, against the model and against conservation.
    fn check(&mut self) -> Result<(), String> {
        let inbox: Vec<u32> = self
            .range("inbox:lead-1")
            .iter()
            .map(|raw| number(&decode(raw).text))
            .collect();
        if self.model.inbox != inbox {
            return Err(format!("inbox {inbox:?}, model {:?}", self.model.inbox));
        }
        // In-flight rows: `<id> <popper> <deadline> <entry>`.
        let rows = self.range("inflight:lead-1");
        let mut inflight: Vec<(u64, i64, u32)> = Vec::new();
        for row in &rows {
            let text = std::str::from_utf8(row).map_err(|e| e.to_string())?;
            let mut parts = text.splitn(4, ' ');
            let id: u64 = parts.next().and_then(|p| p.parse().ok()).ok_or("row id")?;
            let _popper = parts.next();
            let deadline: i64 = parts
                .next()
                .and_then(|p| p.parse().ok())
                .ok_or("row deadline")?;
            let m = number(&decode(parts.next().ok_or("row entry")?.as_bytes()).text);
            inflight.push((id, deadline, m));
        }
        let want: Vec<(u64, i64, u32)> = self
            .model
            .inflight
            .iter()
            .flat_map(|b| b.messages.iter().map(|m| (b.id, b.deadline, *m)))
            .collect();
        if inflight != want {
            return Err(format!("in flight {inflight:?}, model {want:?}"));
        }
        let dead = self
            .bus
            .relay
            .dead_letters(
                &lead(1),
                NonZeroUsize::new(DEAD_LETTER_CAP).expect("nonzero"),
            )
            .map_err(|e| e.to_string())?;
        let dead: Vec<u32> = dead.messages.iter().map(|m| number(&m.text)).collect();
        if dead != self.model.dead {
            return Err(format!(
                "dead letters {dead:?}, model {:?}",
                self.model.dead
            ));
        }
        // Nothing but the lists and the counter: no side table of counts.
        let keys = self.bus.keys();
        if let Some(extra) = keys.iter().find(|k| k.contains("redelivered")) {
            return Err(format!("a side table of counts exists: {extra}"));
        }
        // Conservation, from the store's own lists: each message sent is in
        // exactly one place.
        let mut places: BTreeMap<u32, usize> = BTreeMap::new();
        for m in inbox
            .iter()
            .chain(inflight.iter().map(|(_, _, m)| m))
            .chain(dead.iter())
            .chain(self.model.settled.iter())
        {
            *places.entry(*m).or_default() += 1;
        }
        for m in 0..self.sent {
            match places.get(&m) {
                Some(1) => {}
                other => return Err(format!("m-{m} is in {other:?} places, not exactly one")),
            }
        }
        Ok(())
    }
}

fn decode(raw: &[u8]) -> Message {
    serde_json::from_slice(raw).expect("a stored message")
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

    /// Tracing: TC-020-03, TC-020-05, TC-020-07, TC-040-01, TC-040-02, TC-040-03
    /// ACs: FR-020-AC-3, FR-020-AC-5, FR-020-AC-7, FR-040-AC-1, FR-040-AC-2, FR-040-AC-3
    ///
    /// Two consumers popping, acknowledging (also by a stale id), requeueing,
    /// dying with a batch, and leases expiring under a clock the test drives:
    /// the store matches the model after every step, every message is in
    /// exactly one place, and none is returned again while its lease stands.
    /// Long enough sequences reach the dead-letter list.
    #[test]
    fn tc_020_03_the_store_matches_the_in_flight_model_over_generated_interleavings(
        ops in prop::collection::vec(op(), 1..80)
    ) {
        let Some(mut run) = Run::new("inflight-model") else { return Ok(()); };
        for (i, op) in ops.iter().enumerate() {
            run.step(op).map_err(|e| TestCaseError::fail(format!("after step {i} {op:?}: {e}")))?;
        }
    }
}

// ---------------------------------------------------------------------------
// Named cases the generator would only reach by chance
// ---------------------------------------------------------------------------

fn pop(run: &mut Run, count: usize, lease_ms: i64) -> Vec<(u32, bool)> {
    let now = run.model.now;
    let got = run
        .bus
        .relay
        .pop_inbox_budget(
            &lead(1),
            NonZeroUsize::new(count).expect("nonzero"),
            usize::MAX,
            Claim::new(Popper::Hook)
                .with_lease(Duration::from_millis(
                    u64::try_from(lease_ms).expect("positive"),
                ))
                .at(Now::AtMs(now)),
        )
        .expect("pop");
    let shown: Vec<(u32, bool)> = got
        .messages
        .iter()
        .map(|m| number(&m.text))
        .zip(got.redelivered)
        .collect();
    if let Some(id) = got.batch {
        run.ids.push(id);
    }
    shown
}

fn seeded(test: &str, n: u32) -> Option<Run> {
    let mut run = Run::new(test)?;
    for _ in 0..n {
        run.step(&Op::Send).expect("send");
    }
    Some(run)
}

/// Tracing: TC-020-04
/// ACs: FR-020-AC-4
///
/// A hook that pops two messages and is killed before it writes: nothing is
/// acknowledged, and once the lease is out the next pop returns the same two,
/// in the same order, marked as a redelivery.
#[test]
fn tc_020_04_a_batch_never_acknowledged_returns_in_order_after_its_lease() {
    let Some(mut run) = seeded("inflight-killed", 3) else {
        return;
    };
    assert_eq!(pop(&mut run, 2, 20_000), [(0, false), (1, false)]);
    assert_eq!(
        run.bus.relay.inbox_len(&lead(1)).unwrap(),
        1,
        "the two are out of the inbox"
    );
    // Before the lease is out they are not offered again: the next pop takes the third.
    assert_eq!(pop(&mut run, 5, 20_000), [(2, false)]);
    run.model.now += 20_001;
    assert_eq!(pop(&mut run, 5, 20_000), [(0, true), (1, true), (2, true)]);
}

/// Tracing: TC-020-05
/// ACs: FR-020-AC-5
///
/// An acknowledgement by the id of a batch whose lease expired and which was
/// popped again deletes nothing: the newer batch is still in flight.
#[test]
fn tc_020_05_a_stale_acknowledgement_does_not_delete_the_newer_batch() {
    let Some(mut run) = seeded("inflight-stale-ack", 2) else {
        return;
    };
    assert_eq!(pop(&mut run, 5, 1_000), [(0, false), (1, false)]);
    let stale = run.ids[0];
    run.model.now += 1_001;
    assert_eq!(pop(&mut run, 5, 60_000), [(0, true), (1, true)]);
    let fresh = run.ids[1];
    assert_ne!(stale, fresh, "ids are not reused");

    assert!(
        !run.bus
            .relay
            .ack(&lead(1), stale, relay_core::Popper::Hook)
            .unwrap()
    );
    assert_eq!(run.bus.relay.inflight_len(&lead(1)).unwrap(), 2);
    assert!(
        run.bus
            .relay
            .ack(&lead(1), fresh, relay_core::Popper::Hook)
            .unwrap()
    );
    assert_eq!(run.bus.relay.inflight_len(&lead(1)).unwrap(), 0);
    assert!(
        !run.bus
            .relay
            .ack(&lead(1), fresh, relay_core::Popper::Hook)
            .unwrap(),
        "a second acknowledgement of the same batch deletes nothing"
    );
}

/// Tracing: TC-020-06
/// ACs: FR-020-AC-6
///
/// A batch the watcher never acknowledged (an ambiguous prompt) is returned
/// by the reap once its lease is out, ahead of mail that arrived meanwhile.
#[test]
fn tc_020_06_an_unacknowledged_watcher_batch_returns_ahead_of_newer_mail() {
    let Some(mut run) = seeded("inflight-ambiguous", 2) else {
        return;
    };
    let claim = Claim::new(Popper::Watcher).at(Now::AtMs(run.model.now));
    let got = run
        .bus
        .relay
        .pop_inbox_budget(&lead(1), NonZeroUsize::new(5).unwrap(), usize::MAX, claim)
        .unwrap();
    assert_eq!(got.messages.len(), 2);
    run.model.pop(5, 60_000);
    run.step(&Op::Send).unwrap();
    // Just inside the watcher's lease: nothing returns. Just past it: the batch does.
    let early = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(run.model.now + 59_999))
        .unwrap();
    assert_eq!((early.requeued, early.dead), (0, 0));
    let late = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(run.model.now + 60_000))
        .unwrap();
    assert_eq!((late.requeued, late.dead), (2, 0));
    let inbox: Vec<u32> = run
        .range("inbox:lead-1")
        .iter()
        .map(|raw| number(&decode(raw).text))
        .collect();
    assert_eq!(inbox, [0, 1, 2]);
}

/// Tracing: TC-020-08
/// ACs: FR-020-AC-8
///
/// With 16 unacknowledged batches a pop takes nothing and leaves the inbox
/// alone; acknowledging one lets the next pop through.
#[test]
fn tc_020_08_a_pop_at_the_in_flight_cap_takes_nothing() {
    let Some(mut run) = seeded(
        "inflight-cap",
        u32::try_from(MAX_INFLIGHT_BATCHES).unwrap() + 2,
    ) else {
        return;
    };
    for i in 0..MAX_INFLIGHT_BATCHES {
        let popped = pop(&mut run, 1, 60_000);
        assert_eq!(popped.len(), 1, "batch {i}");
    }
    let before = run.bus.relay.inbox_len(&lead(1)).unwrap();
    let capped = pop_result(&mut run).unwrap();
    assert!(capped.messages.is_empty() && capped.batch.is_none());
    assert!(
        capped.at_cap,
        "the refusal is reported, not passed off as empty"
    );
    assert_eq!(capped.pending, before);
    assert!(pop(&mut run, 1, 60_000).is_empty());
    assert_eq!(run.bus.relay.inbox_len(&lead(1)).unwrap(), before);
    assert!(
        run.bus
            .relay
            .ack(&lead(1), run.ids[0], relay_core::Popper::Hook)
            .unwrap()
    );
    assert_eq!(pop(&mut run, 1, 60_000).len(), 1);
}

/// Tracing: TC-040-02
/// ACs: FR-040-AC-2
///
/// A message popped and never acknowledged is delivered four times in all, is
/// then dead-lettered byte for byte, and is in neither the inbox nor flight.
#[test]
fn tc_040_02_a_message_never_acknowledged_is_dead_lettered_after_four_deliveries() {
    let Some(mut run) = seeded("inflight-dead", 1) else {
        return;
    };
    let mut deliveries = Vec::new();
    for _ in 0..8 {
        let shown = pop(&mut run, 1, 1_000);
        if shown.is_empty() {
            break;
        }
        deliveries.push(shown[0]);
        run.model.now += 1_001;
    }
    assert_eq!(
        deliveries,
        [(0, false), (0, true), (0, true), (0, true)],
        "four deliveries: the first, then three marked redeliveries"
    );
    assert_eq!(run.bus.relay.inbox_len(&lead(1)).unwrap(), 0);
    assert_eq!(run.bus.relay.inflight_len(&lead(1)).unwrap(), 0);
    let dead = run
        .bus
        .relay
        .dead_letters(&lead(1), NonZeroUsize::new(10).unwrap())
        .unwrap();
    assert_eq!(dead.messages, [message(0)]);
    // Byte for byte: no redelivery spaces are left on a dead letter.
    let raw: Vec<Vec<u8>> = redis::cmd("LRANGE")
        .arg(run.bus.key("deadletter:lead-1"))
        .arg(0)
        .arg(-1)
        .query(&mut run.bus.raw)
        .unwrap();
    assert_eq!(raw, [serde_json::to_vec(&message(0)).unwrap()]);
}

/// Tracing: TC-040-03
/// ACs: FR-040-AC-3, FR-040-AC-6
///
/// A message acknowledged after a redelivery leaves nothing behind, and a fresh
/// entry byte-identical to a redelivered one starts at zero: it is not shown as a
/// redelivery, and it is delivered four times before it is dead-lettered, whatever
/// became of its twin.
#[test]
fn tc_040_03_an_identical_fresh_entry_does_not_inherit_a_redelivery_count() {
    let Some(mut run) = seeded("inflight-count", 1) else {
        return;
    };
    // The twin is redelivered twice and left in the inbox, count two.
    pop(&mut run, 1, 1_000);
    run.model.now += 1_001;
    assert_eq!(pop(&mut run, 1, 1_000), [(0, true)]);
    run.model.now += 1_001;
    run.bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(run.model.now))
        .unwrap();
    // An identical entry (same sender, target, text and millisecond) arrives.
    run.bus.relay.send(&message(0)).unwrap();
    // The twin is first (it is at the front); the fresh copy behind it is not marked.
    assert_eq!(
        pop(&mut run, 2, 1_000),
        [(0, true), (0, false)],
        "the fresh copy starts at zero"
    );
    // Acknowledging the batch leaves no trace: no key holds a count.
    let id = *run.ids.last().unwrap();
    assert!(
        run.bus
            .relay
            .ack(&lead(1), id, relay_core::Popper::Hook)
            .unwrap()
    );
    assert!(
        run.bus
            .keys()
            .iter()
            .all(|k| !k.contains("redelivered") && !k.ends_with("inflight:lead-1")),
        "{:?}",
        run.bus.keys()
    );
    // And a later identical entry is fresh again.
    run.bus.relay.send(&message(0)).unwrap();
    assert_eq!(pop(&mut run, 1, 1_000), [(0, false)]);
}

/// Tracing: TC-040-04
/// ACs: FR-040-AC-4
///
/// The dead-letter list keeps the newest [`DEAD_LETTER_CAP`] entries.
#[test]
fn tc_040_04_the_dead_letter_list_is_capped_at_the_newest_entries() {
    let Some(mut run) = seeded("inflight-dead-cap", 0) else {
        return;
    };
    let key = run.bus.key("inflight:lead-1");
    // A row nobody can parse is dead-lettered whole; plant more than the cap.
    for i in 0..u32::try_from(DEAD_LETTER_CAP).unwrap() + 5 {
        redis::cmd("RPUSH")
            .arg(&key)
            .arg(format!("not-a-row-{i}"))
            .exec(&mut run.bus.raw)
            .unwrap();
    }
    let reaped = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(run.model.now))
        .unwrap();
    assert_eq!(reaped.dead, DEAD_LETTER_CAP + 5);
    assert_eq!(reaped.evicted, 5, "the eviction is reported, not silent");
    let len: usize = redis::cmd("LLEN")
        .arg(run.bus.key("deadletter:lead-1"))
        .query(&mut run.bus.raw)
        .unwrap();
    assert_eq!(len, DEAD_LETTER_CAP);
    let newest: Vec<u8> = redis::cmd("LINDEX")
        .arg(run.bus.key("deadletter:lead-1"))
        .arg(-1)
        .query(&mut run.bus.raw)
        .unwrap();
    assert_eq!(
        newest,
        format!("not-a-row-{}", DEAD_LETTER_CAP + 4).into_bytes()
    );
}

/// Tracing: TC-020-07
/// ACs: FR-020-AC-7
///
/// The store's own view of a lost reply: a pop whose reply the caller never
/// read (the batch id dropped) is still fully recoverable by lease expiry.
/// Two consumers race for one inbox of 40 messages; whatever either drops is
/// popped again after the lease, and in the end each message is settled once.
#[test]
fn tc_020_07_consumers_that_drop_batches_still_settle_every_message_once() {
    let Some(mut run) = seeded("inflight-race", 40) else {
        return;
    };
    let mut settled: BTreeSet<u32> = BTreeSet::new();
    for round in 0..400usize {
        let shown = pop(&mut run, 3, 1_000);
        if shown.is_empty() {
            run.model.now += 1_001;
            if run.bus.relay.inbox_len(&lead(1)).unwrap() == 0
                && run.bus.relay.inflight_len(&lead(1)).unwrap() == 0
            {
                break;
            }
            continue;
        }
        // Three of the batches are dropped on the floor (a killed consumer): a
        // message can then be dropped at most three times, and it is delivered
        // four times before a fifth expiry would dead-letter it.
        if !matches!(round, 0 | 2 | 5) {
            let id = *run.ids.last().unwrap();
            assert!(
                run.bus
                    .relay
                    .ack(&lead(1), id, relay_core::Popper::Hook)
                    .unwrap()
            );
            for (m, _) in shown {
                assert!(settled.insert(m), "m-{m} was settled twice");
            }
        }
    }
    assert_eq!(settled.len(), 40, "{settled:?}");
    let dead = run
        .bus
        .relay
        .dead_letters(&lead(1), NonZeroUsize::new(10).unwrap())
        .unwrap();
    assert!(dead.messages.is_empty());
}

/// Tracing: TC-044-05
/// ACs: FR-044-AC-5
///
/// A batch the receiver denied goes to the dead-letter list, in flight no more,
/// entries unchanged, and only that batch; a second call and a batch not in
/// flight move nothing.
#[test]
fn tc_044_05_a_denied_batch_goes_to_the_dead_letters_and_only_that_batch() {
    let Some(mut run) = seeded("inflight-deadletter", 3) else {
        return;
    };
    assert_eq!(pop(&mut run, 1, 20_000), [(0, false)]);
    assert_eq!(pop(&mut run, 1, 20_000), [(1, false)]);
    let (denied, other) = (run.ids[0], run.ids[1]);
    assert_eq!(
        run.bus
            .relay
            .dead_letter_batch(&lead(1), denied, Popper::Hook)
            .unwrap(),
        1
    );
    assert_eq!(
        run.bus
            .relay
            .dead_letter_batch(&lead(1), denied, Popper::Hook)
            .unwrap(),
        0
    );
    let dead = run
        .bus
        .relay
        .dead_letters(&lead(1), NonZeroUsize::new(10).unwrap())
        .unwrap();
    let texts: Vec<u32> = dead.messages.iter().map(|m| number(&m.text)).collect();
    assert_eq!(texts, [0]);
    assert_eq!(
        run.bus.relay.inflight_len(&lead(1)).unwrap(),
        1,
        "the other stays"
    );
    assert!(
        run.bus
            .relay
            .ack(&lead(1), other, relay_core::Popper::Hook)
            .unwrap()
    );
    assert_eq!(
        run.bus
            .relay
            .dead_letter_batch(&lead(1), other, Popper::Hook)
            .unwrap(),
        0
    );
}

/// Tracing: TC-044-05
/// ACs: FR-044-AC-5
///
/// A held batch's lease is renewed: it survives a reap past its old deadline, is
/// not popped by another consumer, keeps its redelivery count, is never shortened,
/// and a batch that is gone (acknowledged, reaped) matches nothing.
#[test]
fn tc_044_05_a_renewed_lease_keeps_a_batch_in_flight_past_its_old_deadline() {
    let Some(mut run) = seeded("inflight-renew", 2) else {
        return;
    };
    assert_eq!(pop(&mut run, 1, 60_000), [(0, false)]);
    let id = run.ids[0];
    let now = run.model.now;
    let lease = Duration::from_secs(120);
    assert_eq!(
        run.bus
            .relay
            .renew_batch(&lead(1), id, lease, Now::AtMs(now), Popper::Hook)
            .unwrap(),
        1
    );
    // Past the old 60 s deadline, inside the renewed one: not reaped, not popped.
    let reaped = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(now + 90_000))
        .unwrap();
    assert_eq!((reaped.requeued, reaped.dead), (0, 0));
    assert_eq!(run.bus.relay.inflight_len(&lead(1)).unwrap(), 1);
    // A shorter renew never shortens it.
    assert_eq!(
        run.bus
            .relay
            .renew_batch(
                &lead(1),
                id,
                Duration::from_secs(1),
                Now::AtMs(now),
                Popper::Hook
            )
            .unwrap(),
        0
    );
    // Past the renewed deadline it is reaped once, as any lapsed lease.
    let reaped = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(now + 130_000))
        .unwrap();
    assert_eq!(reaped.requeued, 1);
    // Gone: a renew matches nothing.
    assert_eq!(
        run.bus
            .relay
            .renew_batch(&lead(1), id, lease, Now::AtMs(now), Popper::Hook)
            .unwrap(),
        0
    );
    assert!(
        run.bus
            .relay
            .ack(&lead(1), id, relay_core::Popper::Hook)
            .is_ok()
    );
}

/// Tracing: TC-044-05
/// ACs: FR-044-AC-5
///
/// A key of the wrong type makes `renew_batch` and `dead_letter_batch` refuse
/// before any write: the same typed error as every other script and nothing changed.
#[test]
fn tc_044_05_renew_and_dead_letter_refuse_a_key_of_the_wrong_type_and_change_nothing() {
    let Some(mut run) = seeded("inflight-wrongtype-batch", 2) else {
        return;
    };
    assert_eq!(pop(&mut run, 1, 20_000), [(0, false)]);
    let id = run.ids[0];
    let unreadable =
        |e: &relay_core::Error| matches!(e, relay_core::Error::InflightUnreadable { .. });
    // The dead-letter key is the wrong type: the batch stays in flight.
    break_key(&mut run, "deadletter:lead-1");
    let before = everything(&mut run);
    let err = run
        .bus
        .relay
        .dead_letter_batch(&lead(1), id, Popper::Hook)
        .unwrap_err();
    assert!(unreadable(&err), "{err}");
    assert_eq!(
        everything(&mut run),
        before,
        "the refused dead-letter changed something"
    );
    // The in-flight key is the wrong type: both scripts refuse.
    break_key(&mut run, "inflight:lead-1");
    let before = everything(&mut run);
    let err = run
        .bus
        .relay
        .dead_letter_batch(&lead(1), id, Popper::Hook)
        .unwrap_err();
    assert!(unreadable(&err), "{err}");
    let err = run
        .bus
        .relay
        .renew_batch(
            &lead(1),
            id,
            Duration::from_secs(60),
            Now::AtMs(0),
            Popper::Hook,
        )
        .unwrap_err();
    assert!(unreadable(&err), "{err}");
    assert_eq!(everything(&mut run), before);
}

/// Tracing: TC-044-05
/// ACs: FR-044-AC-5
///
/// A batch is renewed or dead-lettered only by the popper it names: an id that another
/// consumer's batch holds (a stale or forged one, a counter that was reset) matches
/// nothing.
#[test]
fn tc_044_05_a_renew_or_dead_letter_for_another_poppers_batch_touches_nothing() {
    let Some(mut run) = seeded("inflight-popper-guard", 1) else {
        return;
    };
    assert_eq!(pop(&mut run, 1, 20_000), [(0, false)]); // popped by the hook
    let id = run.ids[0];
    let now = run.model.now;
    let before = everything(&mut run);
    assert_eq!(
        run.bus
            .relay
            .renew_batch(
                &lead(1),
                id,
                Duration::from_secs(120),
                Now::AtMs(now),
                Popper::Watcher
            )
            .unwrap(),
        0
    );
    assert_eq!(
        run.bus
            .relay
            .dead_letter_batch(&lead(1), id, Popper::Watcher)
            .unwrap(),
        0
    );
    // The same for an acknowledgement and a requeue: the watcher's are refused for the hook's
    // batch (the reviewer's probe: an owed ack deleted it, an owed requeue delivered it twice).
    assert!(!run.bus.relay.ack(&lead(1), id, Popper::Watcher).unwrap());
    assert_eq!(
        run.bus
            .relay
            .requeue_batch(&lead(1), id, Popper::Watcher)
            .unwrap(),
        0
    );
    assert_eq!(
        everything(&mut run),
        before,
        "the hook's batch is untouched"
    );
    assert_eq!(
        run.bus
            .relay
            .dead_letter_batch(&lead(1), id, Popper::Hook)
            .unwrap(),
        1
    );
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-1, FR-020-AC-3
///
/// A consumer that knows its delivery failed puts the batch back: at the front,
/// in order, ahead of mail that arrived since, in one step, and not as a
/// redelivery (the recipient never saw it, so it is not marked).
#[test]
fn tc_020_03_a_known_failed_delivery_goes_back_to_the_front_in_order_unmarked() {
    let Some(mut run) = seeded("inflight-requeue", 3) else {
        return;
    };
    assert_eq!(pop(&mut run, 2, 20_000), [(0, false), (1, false)]);
    run.bus.relay.send(&message(3)).unwrap();
    let id = run.ids[0];
    assert_eq!(
        run.bus
            .relay
            .requeue_batch(&lead(1), id, relay_core::Popper::Hook)
            .unwrap(),
        2
    );
    assert_eq!(
        run.bus
            .relay
            .requeue_batch(&lead(1), id, relay_core::Popper::Hook)
            .unwrap(),
        0,
        "a second requeue of the same batch moves nothing"
    );
    let inbox: Vec<u32> = run
        .range("inbox:lead-1")
        .iter()
        .map(|raw| number(&decode(raw).text))
        .collect();
    assert_eq!(inbox, [0, 1, 2, 3]);
    assert_eq!(run.bus.relay.inflight_len(&lead(1)).unwrap(), 0);
    assert_eq!(
        pop(&mut run, 4, 20_000),
        [(0, false), (1, false), (2, false), (3, false)]
    );
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// The watcher's sweep reaps every lead in one request; a lead whose keys cannot
/// be read is that lead's error and the others are still reaped.
#[test]
fn tc_020_03_a_sweep_reaps_the_readable_leads_when_one_lead_is_broken() {
    let Some(mut run) = seeded("inflight-sweep", 2) else {
        return;
    };
    assert_eq!(pop(&mut run, 2, 1_000), [(0, false), (1, false)]);
    // lead-0's in-flight key is the wrong type.
    redis::cmd("SET")
        .arg(run.bus.key("inflight:lead-0"))
        .arg("not a list")
        .exec(&mut run.bus.raw)
        .unwrap();
    let later = Now::AtMs(run.model.now + 1_001);
    let (broken, fine) = (lead(0), lead(1));
    let sweep = run
        .bus
        .relay
        .reap_and_inbox_lens(&[&broken, &fine], &[&fine], later)
        .unwrap();
    assert!(
        matches!(&sweep.reaped[0], Err(relay_core::Error::InflightUnreadable { lead }) if *lead == broken),
        "{:?}",
        sweep.reaped[0]
    );
    assert_eq!(sweep.reaped[1].as_ref().unwrap().requeued, 2);
    assert_eq!(
        *sweep.lens[0].as_ref().unwrap(),
        2,
        "counted after the reap"
    );
}

// ---------------------------------------------------------------------------
// Failure discipline (FR-020-AC-10): a key of the wrong type loses nothing
// ---------------------------------------------------------------------------

/// Every list of the lead, in full, for before/after comparison.
fn snapshot(run: &mut Run) -> Vec<(String, Vec<Vec<u8>>)> {
    ["inbox:lead-1", "inflight:lead-1", "deadletter:lead-1"]
        .into_iter()
        .map(|tail| {
            let key = run.bus.key(tail);
            let kind: String = redis::cmd("TYPE")
                .arg(&key)
                .query(&mut run.bus.raw)
                .unwrap();
            let rows = if kind == "list" {
                run.range(tail)
            } else {
                Vec::new()
            };
            (tail.to_owned(), rows)
        })
        .collect()
}

/// A lead with work for every script: a due row that has spent its redeliveries
/// (three trailing spaces) at the front of the in-flight list, a due batch A, a
/// leased batch B, and two messages still in the inbox.
fn busy(test: &str) -> Option<Run> {
    let mut run = seeded(test, 6)?;
    pop(&mut run, 2, 1_000);
    pop(&mut run, 2, 3_600_000);
    let spent = format!("99 h 1 {}   ", serde_json::to_string(&message(90)).unwrap());
    redis::cmd("LPUSH")
        .arg(run.bus.key("inflight:lead-1"))
        .arg(spent)
        .exec(&mut run.bus.raw)
        .unwrap();
    run.model.now += 2_000;
    Some(run)
}

/// Puts a value of the wrong type at `tail`, keeping what was there under a stash
/// key so the comparison still sees it.
fn break_key(run: &mut Run, tail: &str) {
    let key = run.bus.key(tail);
    let stash = run.bus.key(&format!("stash:{tail}"));
    let kind: String = redis::cmd("TYPE")
        .arg(&key)
        .query(&mut run.bus.raw)
        .unwrap();
    if kind != "none" {
        redis::cmd("RENAME")
            .arg(&key)
            .arg(&stash)
            .exec(&mut run.bus.raw)
            .unwrap();
    }
    // A stream where a list is expected, a list where a string or stream is.
    let want_list = !tail.ends_with("seq") && tail != "log";
    if want_list {
        redis::cmd("SET")
            .arg(&key)
            .arg("not a list")
            .exec(&mut run.bus.raw)
            .unwrap();
    } else {
        redis::cmd("RPUSH")
            .arg(&key)
            .arg("not a string or stream")
            .exec(&mut run.bus.raw)
            .unwrap();
    }
}

fn everything(run: &mut Run) -> Vec<(String, Vec<Vec<u8>>)> {
    let mut all = snapshot(run);
    for tail in [
        "stash:inbox:lead-1",
        "stash:inflight:lead-1",
        "stash:deadletter:lead-1",
    ] {
        let key = run.bus.key(tail);
        let rows: Vec<Vec<u8>> = redis::cmd("LRANGE")
            .arg(&key)
            .arg(0)
            .arg(-1)
            .query(&mut run.bus.raw)
            .unwrap_or_default();
        all.push((tail.to_owned(), rows));
    }
    all
}

/// Tracing: TC-020-10
/// ACs: FR-020-AC-10
///
/// For every script and every key it may write: with that key holding the wrong
/// type the call refuses as [`Error::InflightUnreadable`] and every message is
/// exactly where it was (Redis does not undo a script's earlier writes, so the
/// scripts check every key before their first write).
#[test]
fn tc_020_10_a_wrong_type_key_changes_nothing_in_any_script() {
    use relay_core::Error;
    let unreadable = |e: &Error| matches!(e, Error::InflightUnreadable { .. });
    type Op = fn(&mut Run) -> Result<(), Error>;
    let cases: [(&str, &str, Op); 10] = [
        ("pop", "inbox:lead-1", |r| pop_result(r).map(drop)),
        ("pop", "inflight:lead-1", |r| pop_result(r).map(drop)),
        ("pop", "inflight-seq", |r| pop_result(r).map(drop)),
        ("reap", "inbox:lead-1", |r| reap_result(r).map(drop)),
        ("reap", "inflight:lead-1", |r| reap_result(r).map(drop)),
        ("sweep", "inbox:lead-1", |r| sweep_result(r)),
        ("sweep", "inflight:lead-1", |r| sweep_result(r)),
        ("ack", "inflight:lead-1", |r| ack_result(r).map(drop)),
        ("requeue", "inflight:lead-1", |r| {
            requeue_result(r).map(drop)
        }),
        ("requeue", "inbox:lead-1", |r| requeue_result(r).map(drop)),
    ];
    for (script, tail, op) in cases {
        let Some(mut run) = busy("inflight-wrongtype") else {
            return;
        };
        break_key(&mut run, tail);
        let before = everything(&mut run);
        let err = op(&mut run).expect_err(&format!("{script} with {tail} broken must refuse"));
        assert!(unreadable(&err), "{script}/{tail}: {err}");
        assert_eq!(
            everything(&mut run),
            before,
            "{script}/{tail} changed something"
        );
    }
    // The log is the fourth key of a requeue.
    let Some(mut run) = busy("inflight-wrongtype-log") else {
        return;
    };
    let log = run.bus.key("log");
    redis::cmd("SET")
        .arg(&log)
        .arg("x")
        .exec(&mut run.bus.raw)
        .unwrap();
    let before = everything(&mut run);
    let err = requeue_result(&mut run).expect_err("requeue with the log broken must refuse");
    assert!(unreadable(&err), "{err}");
    assert_eq!(everything(&mut run), before);
}

fn pop_result(run: &mut Run) -> Result<relay_core::BudgetPopResult, relay_core::Error> {
    run.bus.relay.pop_inbox_budget(
        &lead(1),
        NonZeroUsize::new(5).unwrap(),
        usize::MAX,
        Claim::new(Popper::Hook).at(Now::AtMs(run.model.now)),
    )
}

fn reap_result(run: &mut Run) -> Result<relay_core::Reaped, relay_core::Error> {
    run.bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(run.model.now))
}

fn sweep_result(run: &mut Run) -> Result<(), relay_core::Error> {
    let me = lead(1);
    let sweep = run
        .bus
        .relay
        .reap_and_inbox_lens(&[&me], &[], Now::AtMs(run.model.now))?;
    sweep.reaped.into_iter().next().unwrap().map(drop)
}

/// An acknowledgement of batch 1 (the due batch A).
fn ack_result(run: &mut Run) -> Result<bool, relay_core::Error> {
    run.bus
        .relay
        .ack(&lead(1), BatchId::new(1), relay_core::Popper::Hook)
}

fn requeue_result(run: &mut Run) -> Result<usize, relay_core::Error> {
    run.bus
        .relay
        .requeue_batch(&lead(1), BatchId::new(1), relay_core::Popper::Hook)
}

/// Tracing: TC-020-10
/// ACs: FR-020-AC-10
///
/// A dead-letter key of the wrong type must not make every pop fail forever: the
/// rows that would be dead-lettered stay in flight (reported as stuck, not
/// blocking the rows behind them), and everything else works.
#[test]
fn tc_020_10_a_wrong_type_dead_letter_key_degrades_instead_of_failing_every_pop() {
    let Some(mut run) = busy("inflight-dead-broken") else {
        return;
    };
    break_key(&mut run, "deadletter:lead-1");
    let spent_row_before = run.range("inflight:lead-1")[0].clone();
    let popped = pop_result(&mut run).expect("the pop works");
    assert_eq!(popped.stuck, 1, "the spent row is counted, once");
    // A (due) came back and was popped again ahead of the inbox, marked; the
    // spent row is still in flight, untouched, at the front.
    let texts: Vec<u32> = popped.messages.iter().map(|m| number(&m.text)).collect();
    assert_eq!(&texts[..2], [0, 1]);
    assert!(popped.redelivered[0] && popped.redelivered[1]);
    assert_eq!(run.range("inflight:lead-1")[0], spent_row_before);
    // Every later pop and reap keeps working and keeps reporting it.
    let again = reap_result(&mut run).expect("the reap works");
    assert_eq!(again.stuck, 1);
    assert_eq!(again.dead, 0);
}

/// Tracing: TC-020-10
/// ACs: FR-020-AC-10
///
/// A batch counter that is not an integer is refused before the pop's first write
/// (not by a failing `INCR` after its reap), as the lead being unreadable, and
/// changes nothing: no lead's pops fail with a generic Redis error.
#[test]
fn tc_020_10_a_pop_whose_counter_is_not_an_integer_is_refused_before_any_write() {
    let Some(mut run) = busy("inflight-midway-pop") else {
        return;
    };
    redis::cmd("SET")
        .arg(run.bus.key("inflight-seq"))
        .arg("not-an-integer")
        .exec(&mut run.bus.raw)
        .unwrap();
    let before = everything(&mut run);
    let err = pop_result(&mut run).expect_err("the counter is unusable");
    assert!(
        matches!(err, relay_core::Error::InflightUnreadable { .. }),
        "{err}"
    );
    assert_eq!(
        everything(&mut run),
        before,
        "the refused pop changed something"
    );
}

// ---------------------------------------------------------------------------
// Caps, order, leases, budget and visibility
// ---------------------------------------------------------------------------

/// A parseable in-flight row of exactly `len` bytes, leased far into the future.
fn filler_row(len: usize) -> String {
    let head = "5 h 99999999999999 ";
    format!("{head}{}", "x".repeat(len - head.len()))
}

/// Tracing: TC-020-08
/// ACs: FR-020-AC-8
///
/// The byte cap is about what a pop would ADD: with in-flight rows one byte short
/// of leaving room for the batch the pop would take, it takes nothing (and says
/// so); with exactly enough room it takes the batch. Deleting the check, or
/// testing only whether the cap is already reached, fails the first half.
#[test]
fn tc_020_08_a_pop_that_would_exceed_the_byte_cap_takes_nothing_and_one_that_fits_does() {
    // In-flight bytes that leave exactly enough room for the batch (`0`) or one
    // byte too little (`1`).
    for (over, takes) in [(1_usize, false), (0, true)] {
        let Some(mut run) = seeded("inflight-bytes", 2) else {
            return;
        };
        let entry = serde_json::to_vec(&message(0)).unwrap().len();
        let projected = 2 * (entry + relay_core::INFLIGHT_ROW_OVERHEAD);
        let in_flight = relay_core::MAX_INFLIGHT_BYTES - projected + over;
        redis::cmd("LPUSH")
            .arg(run.bus.key("inflight:lead-1"))
            .arg(filler_row(in_flight))
            .exec(&mut run.bus.raw)
            .unwrap();
        let popped = pop_result(&mut run).unwrap();
        if takes {
            assert_eq!(popped.messages.len(), 2, "exactly enough room");
            assert!(!popped.at_cap);
        } else {
            assert!(popped.messages.is_empty(), "one byte too many");
            assert!(popped.at_cap);
            assert_eq!(run.bus.relay.inbox_len(&lead(1)).unwrap(), 2);
        }
    }
}

/// Tracing: TC-020-11
/// ACs: FR-020-AC-11
///
/// Each batch returns on its own lease: a hook batch (20 s) popped after a watcher
/// batch (60 s) comes back first, and the watcher's batch then returns ahead of it
/// (pop order). A redelivered batch may follow newer mail; it is marked.
#[test]
fn tc_020_11_a_redelivered_batch_returns_on_its_own_lease_and_may_follow_newer_mail() {
    let Some(mut run) = seeded("inflight-order", 3) else {
        return;
    };
    let t0 = run.model.now;
    let claim = |popper, at: i64| Claim::new(popper).at(Now::AtMs(at));
    let watcher = run
        .bus
        .relay
        .pop_inbox_budget(
            &lead(1),
            NonZeroUsize::new(2).unwrap(),
            usize::MAX,
            claim(Popper::Watcher, t0),
        )
        .unwrap();
    assert_eq!(watcher.messages.len(), 2);
    let hook = run
        .bus
        .relay
        .pop_inbox_budget(
            &lead(1),
            NonZeroUsize::new(1).unwrap(),
            usize::MAX,
            claim(Popper::Hook, t0 + 1),
        )
        .unwrap();
    assert_eq!(hook.messages.len(), 1);
    // The hook's lease is out (t0 + 21 s), the watcher's (t0 + 60 s) is not.
    let mid = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(t0 + 22_000))
        .unwrap();
    assert_eq!(
        mid.requeued, 1,
        "the hook's batch is not held for the watcher's"
    );
    let late = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(t0 + 61_000))
        .unwrap();
    assert_eq!(late.requeued, 2);
    let inbox: Vec<u32> = run
        .range("inbox:lead-1")
        .iter()
        .map(|raw| number(&decode(raw).text))
        .collect();
    assert_eq!(
        inbox,
        [0, 1, 2],
        "the earlier-popped batch returns ahead of it"
    );
}

/// Tracing: TC-020-12
/// ACs: FR-020-AC-12
///
/// A lease longer than [`relay_core::MAX_LEASE`] is clamped to it: a huge value
/// neither overflows the deadline nor makes a row that cannot be read (which
/// the reap would dead-letter whole).
#[test]
fn tc_020_12_a_huge_lease_is_clamped_and_the_row_stays_readable() {
    let Some(mut run) = seeded("inflight-lease-clamp", 1) else {
        return;
    };
    let claim = Claim::new(Popper::Hook).with_lease(Duration::MAX);
    assert_eq!(claim.lease(), relay_core::MAX_LEASE);
    let t0 = run.model.now;
    let got = run
        .bus
        .relay
        .pop_inbox_budget(
            &lead(1),
            NonZeroUsize::new(1).unwrap(),
            usize::MAX,
            claim.at(Now::AtMs(t0)),
        )
        .unwrap();
    assert_eq!(got.messages.len(), 1);
    let row = String::from_utf8(run.range("inflight:lead-1").remove(0)).unwrap();
    let deadline: i64 = row.split(' ').nth(2).unwrap().parse().unwrap();
    assert_eq!(
        deadline,
        t0 + i64::try_from(relay_core::MAX_LEASE.as_millis()).unwrap()
    );
    // Just past the clamped lease the row is redelivered, not dead-lettered.
    let reaped = run
        .bus
        .relay
        .reap_expired(&lead(1), Now::AtMs(deadline + 1))
        .unwrap();
    assert_eq!((reaped.requeued, reaped.dead), (1, 0));
}

/// Tracing: TC-020-09
/// ACs: FR-020-AC-9
///
/// A redelivered batch of two whose marked renders, joined by a newline, total one
/// byte more than the hook's 6 KiB budget takes only one message: the marker is
/// inside the weight, end to end.
#[test]
fn tc_020_09_a_redelivered_batch_over_the_budget_by_one_byte_takes_only_one() {
    const HOOK_BYTE_BUDGET: usize = 6 * 1024;
    let book = relay_core::AddressBook::default();
    // Two messages whose marked renders are 3,072 bytes each: 3,072 + 1 + 3,072.
    let sized = |n: u32| {
        let mut m = message(n);
        m.text = "y".repeat(3_072 - book.render_marked(&message(n), true).len() + 3);
        m.text.replace_range(..3, &format!("{n:03}"));
        m
    };
    let (first, second) = (sized(0), sized(1));
    let printed =
        book.render_marked(&first, true).len() + 1 + book.render_marked(&second, true).len();
    assert_eq!(printed, HOOK_BYTE_BUDGET + 1);

    let Some(mut bus) = Bus::connect("inflight-budget") else {
        return;
    };
    let me = lead(1);
    bus.relay
        .requeue_front(&me, &[first.clone(), second])
        .unwrap();
    let claim = Claim::new(Popper::Hook).with_lease(Duration::from_millis(1));
    let cap = NonZeroUsize::new(50).unwrap();
    // Pop both under a roomy budget so they are in flight, let the lease lapse:
    // the next pop returns them as redeliveries.
    let first_pop = bus
        .relay
        .pop_inbox_budget(&me, cap, usize::MAX, claim.at(Now::AtMs(START_MS)))
        .unwrap();
    assert_eq!(first_pop.messages.len(), 2);
    let again = bus
        .relay
        .pop_inbox_budget(
            &me,
            cap,
            HOOK_BYTE_BUDGET,
            Claim::new(Popper::Hook).at(Now::AtMs(START_MS + 10)),
        )
        .unwrap();
    assert_eq!(
        again.messages.len(),
        1,
        "the second would print 1 byte over"
    );
    assert!(again.redelivered[0]);
    let shown = book.render_marked(&again.messages[0], true);
    assert!(shown.len() <= HOOK_BYTE_BUDGET);
    assert_eq!(again.pending, 1);
}

/// Tracing: TC-020-13
/// ACs: FR-020-AC-13
///
/// The backlog scan lists in-flight and dead-letter counts for every lead that has
/// some, including one that is not registered (pruned or renamed), and says when
/// it stopped at its bound.
#[test]
fn tc_020_13_the_backlog_lists_stranded_leads_and_reports_its_bound() {
    let Some(mut run) = seeded("inflight-backlog", 2) else {
        return;
    };
    pop(&mut run, 2, 60_000);
    for (tail, rows) in [("inflight:ghost", 3), ("deadletter:ghost", 2)] {
        for i in 0..rows {
            redis::cmd("RPUSH")
                .arg(run.bus.key(tail))
                .arg(format!("row {i}"))
                .exec(&mut run.bus.raw)
                .unwrap();
        }
    }
    let backlog = run.bus.relay.backlog().unwrap();
    assert!(!backlog.truncated);
    let by_name: BTreeMap<&str, (usize, usize)> = backlog
        .leads
        .iter()
        .map(|l| (l.lead.as_str(), (l.in_flight, l.dead)))
        .collect();
    assert_eq!(by_name["lead-1"], (2, 0));
    assert_eq!(by_name["ghost"], (3, 2), "an unregistered lead is listed");

    // Past the bound the scan stops and says so.
    for i in 0..=relay_core::BACKLOG_SCAN_BOUND {
        redis::cmd("RPUSH")
            .arg(run.bus.key(&format!("inflight:bulk-{i}")))
            .arg("row")
            .exec(&mut run.bus.raw)
            .unwrap();
    }
    let truncated = run.bus.relay.backlog().unwrap();
    assert!(truncated.truncated);
}

/// A row that has spent its redeliveries (three trailing spaces) and is long due.
fn spent_row(id: u32) -> String {
    format!(
        "{id} h 1 {}   ",
        serde_json::to_string(&message(100 + id)).unwrap()
    )
}

/// Tracing: TC-020-10
/// ACs: FR-020-AC-10
///
/// Rows that cannot be dead-lettered (a wrong-type dead-letter key) count against
/// neither cap, however many pile up: sixteen or more stuck batches do not stop
/// the lead's pops. Counting them (deleting the exemption) fails this.
#[test]
fn tc_020_10_stuck_rows_do_not_count_against_the_in_flight_caps() {
    let Some(mut run) = seeded("inflight-stuck-caps", 1) else {
        return;
    };
    break_key(&mut run, "deadletter:lead-1");
    for id in 1..=u32::try_from(MAX_INFLIGHT_BATCHES).unwrap() + 4 {
        redis::cmd("RPUSH")
            .arg(run.bus.key("inflight:lead-1"))
            .arg(spent_row(id))
            .exec(&mut run.bus.raw)
            .unwrap();
    }
    let popped = pop_result(&mut run).unwrap();
    assert!(!popped.at_cap, "stuck rows are not in flight for the caps");
    assert_eq!(popped.messages.len(), 1);
    assert_eq!(popped.stuck, MAX_INFLIGHT_BATCHES + 4);
}

/// Tracing: TC-020-08
/// ACs: FR-020-AC-8
///
/// A single entry is always taken when nothing is in flight, however large (a
/// hand-written one over the whole cap): the cap only holds a pop back behind other
/// rows, so one oversized entry cannot stop the lead's pops forever.
#[test]
fn tc_020_08_a_single_oversized_entry_is_taken_when_nothing_is_in_flight() {
    let Some(mut run) = seeded("inflight-oversized", 0) else {
        return;
    };
    let mut big = message(0);
    big.text = "x".repeat(1_100_000);
    redis::cmd("RPUSH")
        .arg(run.bus.key("inbox:lead-1"))
        .arg(serde_json::to_vec(&big).unwrap())
        .exec(&mut run.bus.raw)
        .unwrap();
    run.bus.relay.send(&message(1)).unwrap();
    let popped = pop_result(&mut run).unwrap();
    assert!(!popped.at_cap);
    assert_eq!(popped.messages.len(), 2, "taken, and its neighbour with it");
}

/// Tracing: TC-020-14
/// ACs: FR-020-AC-14
///
/// A lead at its batch cap with nothing waiting is not "capped": there is nothing
/// to take and nothing delayed, so nothing is reported.
#[test]
fn tc_020_14_the_cap_is_not_reported_when_the_inbox_is_empty() {
    let Some(mut run) = seeded(
        "inflight-cap-empty",
        u32::try_from(MAX_INFLIGHT_BATCHES).unwrap(),
    ) else {
        return;
    };
    for _ in 0..MAX_INFLIGHT_BATCHES {
        assert_eq!(pop(&mut run, 1, 60_000).len(), 1);
    }
    let popped = pop_result(&mut run).unwrap();
    assert!(!popped.at_cap);
    assert!(popped.messages.is_empty() && popped.batch.is_none());
}

/// Tracing: TC-020-13
/// ACs: FR-020-AC-13
///
/// A lead whose in-flight key holds the wrong type is listed as unreadable, not as
/// zero, and a key prefix with glob characters is matched literally.
#[test]
fn tc_020_13_the_backlog_reports_an_unreadable_key_and_matches_the_prefix_literally() {
    let Some(mut run) = seeded("inflight-backlog-unreadable", 0) else {
        return;
    };
    redis::cmd("SET")
        .arg(run.bus.key("inflight:ghost"))
        .arg("oops")
        .exec(&mut run.bus.raw)
        .unwrap();
    let backlog = run.bus.relay.backlog().unwrap();
    let ghost = backlog.leads.iter().find(|l| l.lead == "ghost").unwrap();
    assert!(ghost.unreadable);

    let odd = format!("{}g[l]*?:", run.bus.prefix);
    let mut other = relay_core::Relay::connect(&run.bus.url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(odd.clone());
    redis::cmd("RPUSH")
        .arg(format!("{odd}inflight:zed"))
        .arg("row")
        .exec(&mut run.bus.raw)
        .unwrap();
    let found = other.backlog().unwrap();
    assert!(
        found
            .leads
            .iter()
            .any(|l| l.lead == "zed" && l.in_flight == 1)
    );
}
