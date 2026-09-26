//! Fault-injection tests for [`run`]: a fake [`Bus`], a fake clock, a real
//! spool in a temp dir, and writers that can fail. Every stage is failed on
//! purpose and each failure must leave exactly one record and no panic, inside
//! the budget.
//!
//! The fake models Redis's read timeout as `SO_RCVTIMEO` really works: it bounds
//! each *read*, not a whole call. An op is a list of reads; each read is
//! compared with the timeout armed at the time on its own.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    rc::Rc,
    time::Duration,
};

use proptest::prelude::*;
use relay_core::{RefusedDemotion, Target};

use super::*;
use crate::{budget::testing::FakeClock, spool::SpoolRecord};

/// One thing the hook asks of the bus, for injecting faults and latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Op {
    Connect,
    PaneCheck,
    Probe,
    Derive,
    RegisteredPlace,
    Commit,
    Pop,
    Ack,
    Book,
    Requeue,
    Push,
}

/// Shared state behind a [`FakeBus`], so a test can look inside afterwards.
struct World {
    clock: Rc<FakeClock>,
    /// The replies each op waits for, in order, one entry per read. Missing is
    /// one instant reply.
    latency: BTreeMap<Op, Vec<Duration>>,
    /// Ops that fail with a connection error.
    faults: BTreeSet<Op>,
    /// Ops that panic.
    panics: BTreeSet<Op>,
    /// The timeout the hook last armed. A read slower than this times out.
    timeout: Duration,
    /// Every op called, in order.
    calls: Vec<Op>,
    /// The timeout each connect was given.
    connect_timeouts: Vec<Duration>,
    /// Every `(clock, timeout)` the hook armed, for the budget assertions.
    armed: Vec<(Duration, Duration)>,
    inbox: Vec<Message>,
    /// Batches taken by a pop and neither acknowledged nor put back, by id.
    inflight: Vec<(u64, Vec<Message>)>,
    next_batch: u64,
    /// Every batch acknowledged, in order.
    acked: Vec<u64>,
    /// The next pop reports the in-flight cap and takes nothing.
    capped: bool,
    /// Rows the pop reports as spent but not dead-letterable.
    stuck: usize,
    /// The lease runs out between the pop and the acknowledgement: the batch is
    /// gone from flight when the acknowledgement arrives.
    reaped_before_ack: bool,
    /// Entries the pop reports as undecodable.
    rejected: usize,
    /// Demotions the commit refuses.
    refused: usize,
    /// Every entry pushed to the health stream, in order.
    health: Vec<HookHealthEntry>,
    /// How many pushes were made.
    pushes: usize,
    commits: Vec<IdentityCommit>,
    /// What `derive` answers.
    identity: Option<Derived>,
    /// When set, `derive` finds no free variant of this name, every call.
    collision: Option<AgentName>,
    /// When set, herdr could not name the session and `derive` says so; the flag
    /// is whether the budget was spent (rather than herdr itself failing).
    herdr_failed: Option<bool>,
    /// Sessions marked unresolved, as `SET NX` would.
    unresolved: BTreeSet<String>,
    /// Sessions whose herdr failure was recorded, as `SET NX` would.
    herdr_marked: BTreeSet<String>,
    /// The place `registered_place` answers.
    registered: Option<Place>,
}

type Shared = Rc<RefCell<World>>;

fn new_world(clock: &Rc<FakeClock>) -> Shared {
    Rc::new(RefCell::new(World {
        clock: Rc::clone(clock),
        latency: BTreeMap::new(),
        faults: BTreeSet::new(),
        panics: BTreeSet::new(),
        timeout: Duration::from_secs(60),
        calls: Vec::new(),
        connect_timeouts: Vec::new(),
        armed: Vec::new(),
        inbox: Vec::new(),
        capped: false,
        stuck: 0,
        reaped_before_ack: false,
        inflight: Vec::new(),
        next_batch: 1,
        acked: Vec::new(),
        rejected: 0,
        refused: 0,
        health: Vec::new(),
        pushes: 0,
        commits: Vec::new(),
        identity: Some(Derived::cached(name("receiver"))),
        collision: None,
        herdr_failed: None,
        unresolved: BTreeSet::new(),
        herdr_marked: BTreeSet::new(),
        registered: None,
    }))
}

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn io_fault() -> relay_core::Error {
    relay_core::Error::Redis(redis::RedisError::from(io::Error::from(
        io::ErrorKind::ConnectionRefused,
    )))
}

fn timeout_fault() -> relay_core::Error {
    relay_core::Error::Redis(redis::RedisError::from(io::Error::from(
        io::ErrorKind::TimedOut,
    )))
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

impl World {
    /// Waits for each of `op`'s replies in turn. Each read gets the armed
    /// timeout to itself: a reply slower than it fails the op, but a call of
    /// several reads may take longer than the timeout in all.
    fn spend(&mut self, op: Op) -> relay_core::Result<()> {
        self.calls.push(op);
        assert!(!self.panics.contains(&op), "injected panic in {op:?}");
        let reads = self
            .latency
            .get(&op)
            .cloned()
            .unwrap_or_else(|| vec![ms(0)]);
        for read in reads {
            self.clock.advance(read.min(self.timeout));
            if read > self.timeout {
                return Err(timeout_fault());
            }
        }
        if self.faults.contains(&op) {
            return Err(io_fault());
        }
        Ok(())
    }
}

struct FakeBus(Shared);

fn connect(world: &Shared, timeout: Duration) -> relay_core::Result<FakeBus> {
    let mut w = world.borrow_mut();
    w.timeout = timeout;
    w.connect_timeouts.push(timeout);
    w.spend(Op::Connect)?;
    drop(w);
    Ok(FakeBus(Rc::clone(world)))
}

impl Bus for FakeBus {
    fn set_timeout(&mut self, timeout: Duration) -> relay_core::Result<()> {
        let mut w = self.0.borrow_mut();
        let now = w.clock.elapsed();
        w.armed.push((now, timeout));
        w.timeout = timeout;
        Ok(())
    }

    fn pane_place(&mut self) -> relay_core::Result<Option<Place>> {
        self.0.borrow_mut().spend(Op::PaneCheck)?;
        Ok(None)
    }

    fn probe_session(&mut self, _session_id: &SessionId) -> relay_core::Result<SessionProbe> {
        self.0.borrow_mut().spend(Op::Probe)?;
        Ok(SessionProbe::default())
    }

    fn derive(
        &mut self,
        _request: &HookRequest<'_>,
        _probe: &SessionProbe,
        _budget: &Budget<'_>,
    ) -> Result<HookIdentity, CollisionError> {
        let mut w = self.0.borrow_mut();
        w.spend(Op::Derive)?;
        if let Some(candidate) = w.collision.clone() {
            return Err(CollisionError::NoFreeName { candidate });
        }
        if let Some(budget_spent) = w.herdr_failed {
            return Ok(HookIdentity::HerdrFailed { budget_spent });
        }
        Ok(w.identity
            .clone()
            .map_or(HookIdentity::NotALead, |d| HookIdentity::Lead(Box::new(d))))
    }

    fn mark_unresolved(&mut self, session_id: &SessionId) -> relay_core::Result<bool> {
        Ok(self
            .0
            .borrow_mut()
            .unresolved
            .insert(session_id.to_string()))
    }

    fn mark_herdr_failed(&mut self, session_id: &SessionId) -> relay_core::Result<bool> {
        Ok(self
            .0
            .borrow_mut()
            .herdr_marked
            .insert(session_id.as_str().to_owned()))
    }

    fn registered_place(&mut self, _name: &AgentName) -> relay_core::Result<Option<Place>> {
        let mut w = self.0.borrow_mut();
        w.spend(Op::RegisteredPlace)?;
        Ok(w.registered.clone())
    }

    fn commit_identity(&mut self, commit: &IdentityCommit) -> relay_core::Result<CommitOutcome> {
        let mut w = self.0.borrow_mut();
        w.spend(Op::Commit)?;
        w.commits.push(commit.clone());
        Ok(CommitOutcome {
            refused: (0..w.refused)
                .map(|i| RefusedDemotion {
                    alias: name(&format!("old-{i}")),
                    session_id: None,
                    herdr_pane: None,
                })
                .collect(),
        })
    }

    fn pop_inbox(&mut self, _name: &AgentName) -> relay_core::Result<BudgetPopResult> {
        let mut w = self.0.borrow_mut();
        // The server removes first; whether the reply arrives in time is the
        // hazard the pop window guards.
        let take = if w.capped { 0 } else { w.inbox.len().min(3) };
        let messages: Vec<Message> = w.inbox.drain(..take).collect();
        let batch = (take > 0).then(|| {
            let id = w.next_batch;
            w.next_batch += 1;
            w.inflight.push((id, messages.clone()));
            BatchId::new(id)
        });
        w.spend(Op::Pop)?;
        Ok(BudgetPopResult {
            pending: w.inbox.len(),
            redelivered: vec![false; messages.len()],
            at_cap: w.capped,
            stuck: w.stuck,
            evicted: 0,
            batch,
            messages,
            rejected: (0..w.rejected)
                .map(|_| relay_core::Error::Decode {
                    key: "inbox".into(),
                    source: serde_json::from_str::<Message>("{").unwrap_err(),
                })
                .collect(),
        })
    }

    fn ack(&mut self, _name: &AgentName, batch: BatchId) -> relay_core::Result<bool> {
        let mut w = self.0.borrow_mut();
        w.spend(Op::Ack)?;
        if w.reaped_before_ack {
            w.inflight.clear();
        }
        let before = w.inflight.len();
        w.inflight.retain(|(id, _)| *id != batch.get());
        w.acked.push(batch.get());
        Ok(w.inflight.len() < before)
    }

    fn requeue_batch(&mut self, _name: &AgentName, batch: BatchId) -> relay_core::Result<usize> {
        let mut w = self.0.borrow_mut();
        w.spend(Op::Requeue)?;
        let Some(at) = w.inflight.iter().position(|(id, _)| *id == batch.get()) else {
            return Ok(0);
        };
        let (_, messages) = w.inflight.remove(at);
        let moved = messages.len();
        for m in messages.into_iter().rev() {
            w.inbox.insert(0, m);
        }
        Ok(moved)
    }

    fn address_book(&mut self) -> AddressBook {
        // A round trip that cannot fail the call: only the time it takes counts.
        let _ = self.0.borrow_mut().spend(Op::Book);
        AddressBook::default()
    }

    fn push_hook_health(&mut self, entries: &[HookHealthEntry]) -> relay_core::Result<()> {
        let mut w = self.0.borrow_mut();
        w.pushes += 1;
        w.spend(Op::Push)?;
        w.health.extend(entries.iter().cloned());
        Ok(())
    }
}

/// A writer that can be told to fail, or to panic like `eprintln!` on a closed
/// stderr.
#[derive(Default)]
struct Sink {
    fail: bool,
    panic: bool,
    written: Vec<u8>,
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        assert!(!self.panic, "failed printing to stderr");
        if self.fail {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A reader that always fails.
struct BrokenStdin;

impl Read for BrokenStdin {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::InvalidData))
    }
}

fn message(text: &str) -> Message {
    Message {
        from: name("sender"),
        to: Target::Agent(name("receiver")),
        text: text.to_owned(),
        sent_at_ms: 1,
    }
}

fn input(event: &str) -> String {
    format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"}}"#)
}

/// What one hook call printed and did.
struct Outcome {
    stdout: String,
    stderr: String,
}

struct Harness {
    clock: Rc<FakeClock>,
    world: Shared,
    dir: tempfile::TempDir,
    spool: Spool,
}

impl Harness {
    fn new() -> Self {
        let clock = Rc::new(FakeClock::default());
        let world = new_world(&clock);
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path().join("spool").join("hook.jsonl"), 256 * 1024);
        Self {
            clock,
            world,
            dir,
            spool,
        }
    }

    fn with_latency(self, op: Op, millis: u64) -> Self {
        self.with_reads(op, &[millis])
    }

    fn with_reads(self, op: Op, millis: &[u64]) -> Self {
        self.world
            .borrow_mut()
            .latency
            .insert(op, millis.iter().map(|m| ms(*m)).collect());
        self
    }

    /// Every request the hook makes takes `millis`: a link of that round trip.
    fn with_rtt(self, millis: u64) -> Self {
        // The lead's registration already has its place: one point read, not a scan.
        self.world.borrow_mut().registered = Some(Place {
            workspace: "EA".into(),
            tab: "code".into(),
            tabs_in_workspace: 2,
        });
        for op in [
            Op::Probe,
            Op::RegisteredPlace,
            Op::PaneCheck,
            Op::Commit,
            Op::Pop,
            Op::Ack,
            Op::Book,
        ] {
            self.world.borrow_mut().latency.insert(op, vec![ms(millis)]);
        }
        self
    }

    fn with_fault(self, op: Op) -> Self {
        self.world.borrow_mut().faults.insert(op);
        self
    }

    fn with_inbox(self, texts: &[&str]) -> Self {
        self.world
            .borrow_mut()
            .inbox
            .extend(texts.iter().map(|t| message(t)));
        self
    }

    /// One hook call, as a lead named by `--as` (`explicit`) or derived.
    fn run_with(
        &self,
        explicit: Option<&str>,
        stdin: &mut dyn InputSource,
        stdout: &mut Sink,
        stderr: &mut Sink,
    ) {
        let cx = HookCtx {
            cli_name: explicit,
            kind: HookKind::Claude,
            machine: &"Hub.local".parse().unwrap(),
        };
        run(
            &cx,
            &*self.clock,
            Some(&self.spool),
            (stdin, stdout, stderr),
            &mut |timeout| connect(&self.world, timeout),
        );
    }

    fn run(&self, explicit: bool, stdin: &mut dyn InputSource, fail_output: bool) -> Outcome {
        let mut stdout = Sink {
            fail: fail_output,
            ..Sink::default()
        };
        let mut stderr = Sink::default();
        self.run_with(
            explicit.then_some("receiver"),
            stdin,
            &mut stdout,
            &mut stderr,
        );
        Outcome {
            stdout: String::from_utf8(stdout.written).unwrap(),
            stderr: String::from_utf8(stderr.written).unwrap(),
        }
    }

    fn run_event(&self, event: &str) -> String {
        self.run(true, &mut input(event).as_bytes(), false).stdout
    }

    fn spool_path(&self) -> std::path::PathBuf {
        self.dir.path().join("spool").join("hook.jsonl")
    }

    /// Everything ever recorded: shipped to the health stream, then still in
    /// the spool.
    fn all_records(&self) -> Vec<SpoolRecord> {
        let mut all: Vec<SpoolRecord> = self
            .world
            .borrow()
            .health
            .iter()
            .map(|e| serde_json::from_str(&e.record).unwrap())
            .collect();
        all.extend(self.records());
        all
    }

    fn records(&self) -> Vec<SpoolRecord> {
        std::fs::read_to_string(self.spool_path())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn budget_of(event: &str) -> Duration {
    hook_deadline_budget(Event::parse(event).unwrap())
}

const EVENTS: [&str; 4] = ["SessionStart", "PostToolUse", "UserPromptSubmit", "Stop"];

// ---- budgets ---------------------------------------------------------------

/// The budgets are what the docs on `hook_deadline_budget` say: 300 ms where no
/// herdr call is made, 900 ms for the events whose job is delivering mail, and
/// herdr's whole allowance plus 300 ms at `SessionStart`.
#[test]
fn the_budgets_are_the_documented_ones() {
    assert_eq!(budget_of("UserPromptSubmit"), ms(300));
    assert_eq!(budget_of("PostToolUse"), ms(900));
    assert_eq!(budget_of("Stop"), ms(900));
    assert_eq!(budget_of("SessionStart"), ms(3300));
}

// ---- the pop window --------------------------------------------------------

/// Runs one hook over a link with a fixed round trip of `rtt_ms`: every request
/// (probe, place reads, commit) and the pop's reply take exactly that, whatever
/// the hook computed, and `derive_ms` is the time herdr takes. Returns whether
/// the pop lost its mail (a pop reply that outlived the timeout the hook armed).
///
/// The oracle is the link, not the hook's window: a pop is lost exactly when its
/// reply (one round trip) is later than the time the hook left it, so this fails
/// for a hook that pops with less than a round trip left however it got there.
fn pop_lost_over_a_link(
    event: &str,
    explicit: bool,
    session: bool,
    rtt_ms: u64,
    derive_ms: u64,
) -> bool {
    let h = Harness::new().with_inbox(&["a", "b"]);
    {
        let mut w = h.world.borrow_mut();
        for op in [
            Op::Probe,
            Op::PaneCheck,
            Op::RegisteredPlace,
            Op::Commit,
            Op::Pop,
        ] {
            w.latency.insert(op, vec![ms(rtt_ms)]);
        }
        w.latency.insert(Op::Derive, vec![ms(derive_ms)]);
    }
    let stdin = if session {
        input(event)
    } else {
        format!(r#"{{"hook_event_name":"{event}","cwd":"/work"}}"#)
    };
    h.run(explicit, &mut stdin.as_bytes(), false);
    h.records().iter().any(|r| r.stage == Stage::Pop)
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// Over a 250 ms link, on every path that takes a round trip of its own before
/// the pop (a derived name with a session, and `SessionStart`/`UserPromptSubmit`
/// by any name), no herdr latency makes the hook pop with less than that round
/// trip left. A hook that stopped timing its own requests would fall back to a
/// short window and lose mail for the herdr latencies that leave 200-249 ms.
#[test]
fn a_pop_over_a_slow_link_is_never_lost_where_the_hook_timed_a_request() {
    for event in ["SessionStart", "UserPromptSubmit"] {
        for explicit in [true, false] {
            for derive_ms in (0..=900).step_by(5) {
                assert!(
                    !pop_lost_over_a_link(event, explicit, true, 250, derive_ms),
                    "{event} explicit={explicit} derive={derive_ms}ms"
                );
            }
        }
    }
    for event in ["PostToolUse", "Stop"] {
        for derive_ms in (0..=900).step_by(5) {
            assert!(
                !pop_lost_over_a_link(event, false, true, 250, derive_ms),
                "{event} derive={derive_ms}ms"
            );
        }
    }
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// Over a 42 ms tunnel, no path loses mail, including the ones that make no
/// request of their own before the pop (an explicit `PostToolUse`/`Stop`, a
/// hook input with no session id): they use the conservative window, never the
/// connect time or the floor.
#[test]
fn a_pop_over_a_42ms_link_is_never_lost_on_any_path() {
    for event in EVENTS {
        for explicit in [true, false] {
            for session in [true, false] {
                for derive_ms in (0..=900).step_by(5) {
                    assert!(
                        !pop_lost_over_a_link(event, explicit, session, 42, derive_ms),
                        "{event} explicit={explicit} session={session} derive={derive_ms}ms"
                    );
                }
            }
        }
    }
}

/// The window is `max(floor, 2 * rtt + margin)`.
#[test]
fn the_pop_window_follows_the_measured_round_trip() {
    let table = [
        (0, 30),
        (5, 30),
        (10, 40),
        (11, 42),
        (42, 104),
        (84, 188),
        (200, 420),
    ];
    for (rtt_ms, window_ms) in table {
        assert_eq!(pop_window(ms(rtt_ms)), ms(window_ms), "rtt {rtt_ms}ms");
    }
    // The constants the table encodes.
    assert_eq!(POP_WINDOW_FLOOR, ms(30));
    assert_eq!(POP_WINDOW_MARGIN, ms(20));
}

/// Tracing: TC-030-07
/// ACs: FR-030-AC-7
///
/// A `UserPromptSubmit` with too little budget left defers its pop without
/// recording anything: the mail waits for the next `PostToolUse` or `Stop`.
#[test]
fn tc_030_07_a_user_prompt_submit_with_too_little_budget_defers_the_pop_and_records_nothing() {
    // 300ms budget; registering eats 280 of it, under the 30ms floor.
    let h = Harness::new()
        .with_latency(Op::Commit, 280)
        .with_inbox(&["a", "b"]);
    let out = h.run_event("UserPromptSubmit");
    assert_eq!(out, "", "nothing delivered");
    let w = h.world.borrow();
    assert!(!w.calls.contains(&Op::Pop), "popped with 20ms left");
    assert_eq!(w.inbox.len(), 2, "both still queued");
    assert!(
        h.records().is_empty(),
        "an expected deferral is not a failure"
    );
}

/// Tracing: TC-030-06
/// ACs: FR-030-AC-6
///
/// The same shortfall on `PostToolUse` or `Stop` really delays delivery, so it
/// is recorded.
#[test]
fn tc_030_06_a_post_tool_use_or_stop_with_too_little_budget_records_a_budget_failure() {
    for event in ["PostToolUse", "Stop"] {
        let h = Harness::new()
            .with_latency(Op::Connect, 880)
            .with_inbox(&["a", "b"]);
        // Connecting took 880 of 900ms: 20ms left, under the floor's 30 (and
        // under the 1780ms window that connect time itself asks for).
        let out = h.run_event(event);
        assert_eq!(out, "", "{event}");
        assert_eq!(h.world.borrow().inbox.len(), 2, "{event}");
        let records = h.records();
        assert_eq!(records.len(), 1, "{event}: {records:?}");
        assert_eq!(
            (records[0].stage, records[0].error_kind),
            (Stage::Budget, ErrorKind::BudgetExhausted),
            "{event}"
        );
    }
}

/// Exactly the window left is enough; one millisecond less is not.
#[test]
fn the_pop_window_boundary_is_inclusive() {
    // Connect takes 40ms, so the window is 100ms. UserPromptSubmit has 300ms:
    // 160ms of registering leaves exactly 100.
    let budget = 300;
    let at_window = Harness::new()
        .with_latency(Op::Connect, 40)
        .with_latency(Op::Commit, budget - 40 - 100)
        .with_inbox(&["a"]);
    at_window.run_event("UserPromptSubmit");
    assert!(at_window.world.borrow().calls.contains(&Op::Pop));

    let below = Harness::new()
        .with_latency(Op::Connect, 40)
        .with_latency(Op::Commit, budget - 40 - 100 + 1)
        .with_inbox(&["a"]);
    below.run_event("UserPromptSubmit");
    assert!(!below.world.borrow().calls.contains(&Op::Pop));
}

/// A `UserPromptSubmit` that has time to pop after a 42 ms round trip does pop:
/// two round trips before the pop leave 216 ms, over the 104 ms window. (An
/// earlier fixed 150 ms window skipped this case, which `main` delivered.)
#[test]
fn a_user_prompt_submit_over_a_42ms_link_pops_after_three_round_trips() {
    // Through an SSH tunnel's local listener the connect itself is instant; the
    // link shows up in the first request.
    let h = Harness::new()
        .with_latency(Op::RegisteredPlace, 42)
        .with_latency(Op::Commit, 42)
        .with_latency(Op::Pop, 42)
        .with_inbox(&["a"]);
    let out = h.run_event("UserPromptSubmit");
    assert!(out.contains("from sender: a"), "{out}");
    assert!(h.records().is_empty());
}

/// The round trip is measured from the first Redis request, not the connect:
/// a tunnel connects locally in about nothing, so a 100 ms first reply must
/// still ask for a 220 ms window, which the 200 ms left does not cover.
#[test]
fn the_window_is_sized_from_the_first_request_not_from_the_connect() {
    let slow_first_read = Harness::new()
        .with_latency(Op::Connect, 0)
        .with_latency(Op::RegisteredPlace, 100)
        .with_inbox(&["a"]);
    slow_first_read.run_event("UserPromptSubmit");
    assert!(
        !slow_first_read.world.borrow().calls.contains(&Op::Pop),
        "200ms left is under the 220ms window a 100ms round trip asks for"
    );
    let quick_first_read = Harness::new()
        .with_latency(Op::Connect, 0)
        .with_latency(Op::RegisteredPlace, 90)
        .with_inbox(&["a"]);
    quick_first_read.run_event("UserPromptSubmit");
    assert!(
        quick_first_read.world.borrow().calls.contains(&Op::Pop),
        "210ms left covers the 200ms window a 90ms round trip asks for"
    );
}

/// The pane's place is read from the whole registry only when the lead has no
/// registration of its own to keep the place from: a lead that is registered
/// costs no registry scan on a `UserPromptSubmit`.
#[test]
fn the_pane_place_scan_runs_only_when_the_lead_has_no_registered_place() {
    let registered = Harness::new();
    registered.world.borrow_mut().registered = Some(Place {
        workspace: "EA".into(),
        tab: "code".into(),
        tabs_in_workspace: 2,
    });
    registered.run_event("UserPromptSubmit");
    assert!(!registered.world.borrow().calls.contains(&Op::PaneCheck));

    let unregistered = Harness::new();
    unregistered.run_event("UserPromptSubmit");
    assert!(unregistered.world.borrow().calls.contains(&Op::PaneCheck));
}

// ---- one record per failure, at every stage --------------------------------

/// Failing each op in turn yields one record at the stage that owns it.
#[test]
fn a_fault_at_each_stage_writes_exactly_one_record_at_that_stage() {
    let cases = [
        (Op::Connect, Stage::Connect, ErrorKind::Connection),
        (Op::PaneCheck, Stage::Derive, ErrorKind::Connection),
        (Op::Derive, Stage::Derive, ErrorKind::Connection),
        (Op::RegisteredPlace, Stage::Register, ErrorKind::Connection),
        (Op::Commit, Stage::Register, ErrorKind::Connection),
        (Op::Pop, Stage::Pop, ErrorKind::Connection),
    ];
    for (op, stage, kind) in cases {
        // SessionStart reaches every op, deriving because it is not explicit.
        let h = Harness::new().with_fault(op).with_inbox(&["hi"]);
        h.run(false, &mut input("SessionStart").as_bytes(), false);
        let records = h.records();
        assert_eq!(records.len(), 1, "{op:?}: {records:?}");
        assert_eq!(records[0].stage, stage, "{op:?}");
        assert_eq!(records[0].error_kind, kind, "{op:?}");
        assert_eq!(records[0].event, Some(Event::SessionStart), "{op:?}");
        assert_eq!(records[0].session_id.as_deref(), Some("s1"), "{op:?}");
        assert_eq!(records[0].machine.as_str(), "hub", "normalised: {op:?}");
        assert!(records[0].messages.is_empty(), "{op:?}");
    }
}

/// A record names the lead once it is known, and only then.
#[test]
fn a_record_carries_the_lead_when_it_was_known() {
    // Known from `--as` before connecting.
    let h = Harness::new().with_fault(Op::Connect);
    h.run(true, &mut input("Stop").as_bytes(), false);
    assert_eq!(h.records()[0].lead.as_deref(), Some("receiver"));
    // Derived: unknown while deriving, known after.
    let h = Harness::new().with_fault(Op::Derive);
    h.run(false, &mut input("Stop").as_bytes(), false);
    assert_eq!(h.records()[0].lead, None);
    let h = Harness::new().with_fault(Op::Pop);
    h.run(false, &mut input("Stop").as_bytes(), false);
    assert_eq!(h.records()[0].lead.as_deref(), Some("receiver"));
}

#[test]
fn a_redis_timeout_is_recorded_as_a_timeout_not_a_connection_failure() {
    let h = Harness::new().with_latency(Op::Commit, 10_000);
    h.run(false, &mut input("SessionStart").as_bytes(), false);
    let records = h.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].stage, Stage::Register);
    assert_eq!(records[0].error_kind, ErrorKind::Timeout);
}

#[test]
fn unreadable_and_malformed_input_are_decode_failures() {
    let h = Harness::new();
    h.run(true, &mut BrokenStdin, false);
    h.run(true, &mut "not json".as_bytes(), false);
    let records = h.all_records();
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Decode, ErrorKind::Io)
    );
    assert_eq!(
        (records[1].stage, records[1].error_kind),
        (Stage::Decode, ErrorKind::Malformed)
    );
}

/// A stdin that never yields and never closes.
struct SilentPipe(std::sync::mpsc::Receiver<()>);

impl Read for SilentPipe {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        // Blocks until the sender is dropped, which the test never does in time.
        let _ = self.0.recv();
        Ok(0)
    }
}

/// Tracing: TC-N001-M-05
/// ACs: NFR-001-M-5
#[test]
fn tc_n001_m_05_a_stdin_that_never_closes_records_one_timeout_and_pops_nothing() {
    let h = Harness::new().with_inbox(&["waiting"]);
    let (_hold_open, rx) = std::sync::mpsc::channel();
    let clock = crate::budget::testing::TickingClock::new(ms(100));
    let mut stdout = Sink::default();
    let cx = HookCtx {
        cli_name: Some("receiver"),
        kind: HookKind::Claude,
        machine: &"Hub.local".parse().unwrap(),
    };
    run(
        &cx,
        &clock,
        Some(&h.spool),
        (
            &mut crate::stdin::ThreadedStdin::new(SilentPipe(rx)),
            &mut stdout,
            &mut Sink::default(),
        ),
        &mut |timeout| connect(&h.world, timeout),
    );
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Decode, ErrorKind::StdinTimeout)
    );
    assert!(stdout.written.is_empty());
    let w = h.world.borrow();
    assert!(w.calls.is_empty(), "no bus call at all: {:?}", w.calls);
    assert!(w.connect_timeouts.is_empty(), "not even a connect");
    assert_eq!(w.inbox.len(), 1, "the inbox was not popped");
}

/// Tracing: TC-030-01
/// ACs: FR-030-AC-1
///
/// A payload past the cap is one `too_large` decode record, nothing is popped,
/// and (a persistent input fault) exactly one connect ships the spool.
#[test]
fn an_oversized_input_is_one_too_large_decode_record_and_one_flush() {
    let h = Harness::new().with_inbox(&["waiting"]);
    let big = vec![b' '; crate::stdin::MAX_INPUT_BYTES + 1];
    h.run(true, &mut big.as_slice(), false);
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Decode, ErrorKind::TooLarge)
    );
    let w = h.world.borrow();
    assert_eq!(w.inbox.len(), 1);
    assert_eq!(
        w.connect_timeouts.len(),
        1,
        "one connect, to flush the spool"
    );
}

/// A source that takes `delay` on the harness clock and then yields `bytes`:
/// a payload that arrives late but arrives.
struct SlowSource {
    clock: Rc<FakeClock>,
    delay: Duration,
    bytes: Vec<u8>,
}

impl InputSource for SlowSource {
    fn read_input(&mut self, budget: &Budget<'_>) -> Result<Vec<u8>, InputFault> {
        // Honours the read budget like the real reader: a payload due after it
        // is spent never arrives.
        match budget.remaining() {
            Some(left) if self.delay < left => {
                self.clock.advance(self.delay);
                Ok(self.bytes.clone())
            }
            left => {
                self.clock.advance(left.unwrap_or_default());
                Err(InputFault::Timeout)
            }
        }
    }
}

/// A source fed by [`crate::stdin::collect`] from arrivals at absolute times on
/// the harness clock: the real idle-window and cap logic, without threads.
struct ArrivalSource {
    clock: Rc<FakeClock>,
    arrivals: Vec<(u64, &'static [u8])>,
}

impl InputSource for ArrivalSource {
    fn read_input(&mut self, budget: &Budget<'_>) -> Result<Vec<u8>, InputFault> {
        crate::stdin::collect(
            budget,
            crate::stdin::testing::arrivals(&self.clock, &self.arrivals),
        )
    }
}

/// A payload split in two, the second half arriving at `second` ms.
fn late_payload(h: &Harness, event: &str, second: u64) -> ArrivalSource {
    let whole: &'static str = Box::leak(input(event).into_boxed_str());
    let (a, b) = whole.as_bytes().split_at(10);
    ArrivalSource {
        clock: Rc::clone(&h.clock),
        arrivals: vec![(200, a), (second, b)],
    }
}

/// Tracing: TC-N001-M-05
/// ACs: NFR-001-M-5
///
/// A payload that keeps arriving is read past the 300 ms window. A
/// `UserPromptSubmit` one that completes at 500 ms finds its own 300 ms spent:
/// one `budget`/`budget_exhausted` record, no connection, nothing popped. A
/// `PostToolUse` one at the same time is served with what is left of 900 ms.
#[test]
fn tc_n001_m_05_a_late_payload_is_read_and_meets_its_events_budget() {
    let h = Harness::new().with_inbox(&["waiting"]);
    h.run(true, &mut late_payload(&h, "UserPromptSubmit", 500), false);
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Budget, ErrorKind::BudgetExhausted)
    );
    {
        let w = h.world.borrow();
        assert!(w.connect_timeouts.is_empty(), "no connection");
        assert_eq!(w.inbox.len(), 1, "nothing popped");
    }

    let h = Harness::new().with_inbox(&["waiting"]);
    h.run(true, &mut late_payload(&h, "PostToolUse", 500), false);
    assert_eq!(h.world.borrow().connect_timeouts, [ms(400)]);
    assert!(h.world.borrow().inbox.is_empty(), "popped and delivered");
}

/// Tracing: TC-N001-M-05
/// ACs: NFR-001-M-5
///
/// Bytes every 200 ms never idle the window, so only the overall cap ends the
/// read: at `STDIN_TOTAL` exactly (900 ms, on the clock), as one `stdin_timeout`
/// record with nothing left of the call for a connection.
#[test]
fn tc_n001_m_05_a_drip_of_bytes_is_cut_at_the_overall_stdin_cap() {
    let h = Harness::new().with_inbox(&["waiting"]);
    let mut source = ArrivalSource {
        clock: Rc::clone(&h.clock),
        arrivals: (1..=30).map(|i| (i * 200, &b" "[..])).collect(),
    };
    h.run(true, &mut source, false);
    assert_eq!(h.clock.elapsed(), STDIN_TOTAL);
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Decode, ErrorKind::StdinTimeout)
    );
    let w = h.world.borrow();
    assert!(
        w.connect_timeouts.is_empty(),
        "the call is spent: no connect"
    );
    assert_eq!(w.inbox.len(), 1);
}

/// Tracing: TC-N001-M-05
/// ACs: NFR-001-M-5
///
/// The event's budget counts from the start of the call, stdin included: a
/// `UserPromptSubmit` payload that arrives at 250 ms leaves 50 ms of its 300.
/// (Starting the budget after the read would leave the full 300 ms.)
#[test]
fn tc_n001_m_05_the_event_budget_counts_from_before_the_stdin_read() {
    let h = Harness::new();
    let mut source = SlowSource {
        clock: Rc::clone(&h.clock),
        delay: ms(250),
        bytes: input("UserPromptSubmit").into_bytes(),
    };
    h.run(true, &mut source, false);
    let w = h.world.borrow();
    assert_eq!(w.connect_timeouts, [ms(50)]);
}

/// The spool flush after a persistent input fault counts from the start of the
/// call too, so a slow read never lets the call run past 300 ms in all.
#[test]
fn the_flush_after_a_decode_failure_gets_what_is_left_of_the_call() {
    let h = Harness::new();
    let slow = |delay| SlowSource {
        clock: Rc::clone(&h.clock),
        delay,
        bytes: b"{}".to_vec(),
    };
    h.run(true, &mut slow(ms(250)), false);
    assert_eq!(h.world.borrow().connect_timeouts, [ms(50)]);
    h.run(true, &mut slow(ms(300)), false);
    assert_eq!(
        h.world.borrow().connect_timeouts.len(),
        1,
        "nothing left, so no second connect"
    );
    assert_eq!(h.all_records().len(), 2);
}

/// What the first JSON value of the input decodes to, old (`read_to_string` +
/// `from_str`) against new. Each row is a deliberate, specified behaviour.
#[test]
fn input_decoding_takes_the_first_object_and_ignores_what_follows() {
    let whole = input("Stop");
    let ok = |raw: &[u8]| decode_input(raw).is_ok();
    // Trailing bytes after the first object: accepted, ignored (was malformed).
    assert!(ok(format!("{whole}garbage").as_bytes()));
    assert!(ok(format!("{whole}{whole}").as_bytes()));
    assert!(ok(format!("{whole}\0").as_bytes()));
    assert!(ok([whole.as_bytes(), &[0xff, 0xfe]].concat().as_slice()));
    // Invalid UTF-8 inside a string: malformed (was an `io` failure).
    let bad = br#"{"hook_event_name":"Stop","session_id":"\xff"}"#
        .map(|b| if b == b'\\' { 0xff } else { b });
    assert!(!ok(&bad));
    // Not an object, or an object that is not a hook payload: malformed.
    for raw in [
        &b"{}"[..],
        b"[]",
        b"",
        b"not json",
        b"7",
        b"{\"hook_event_name\":5}",
    ] {
        assert!(!ok(raw), "{raw:?}");
    }
}

/// A complete object that is not a hook payload on a held-open pipe is
/// `malformed` at once, not a wait for the deadline (`stdin_timeout`).
#[test]
fn tc_030_01_a_complete_non_hook_object_on_an_open_pipe_is_malformed_not_a_timeout() {
    let h = Harness::new();
    let (hold, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    hold.send(b"{}".to_vec()).unwrap();
    let clock = crate::budget::testing::TickingClock::new(ms(10));
    let cx = HookCtx {
        cli_name: Some("receiver"),
        kind: HookKind::Claude,
        machine: &"Hub.local".parse().unwrap(),
    };
    run(
        &cx,
        &clock,
        Some(&h.spool),
        (
            &mut crate::stdin::ThreadedStdin::new(ChannelPipe(rx)),
            &mut Sink::default(),
            &mut Sink::default(),
        ),
        &mut |timeout| connect(&h.world, timeout),
    );
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Decode, ErrorKind::Malformed)
    );
}

/// A pipe that yields what the test sends and then blocks (open, quiet).
struct ChannelPipe(std::sync::mpsc::Receiver<Vec<u8>>);

impl Read for ChannelPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Ok(bytes) = self.0.recv() else {
            return Ok(0);
        };
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}

#[test]
fn an_invalid_agent_msg_name_is_a_derive_failure() {
    let h = Harness::new();
    let mut out = Sink::default();
    let mut err = Sink::default();
    h.run_with(
        Some("Bad Name"),
        &mut input("Stop").as_bytes(),
        &mut out,
        &mut err,
    );
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Derive, ErrorKind::InvalidName)
    );
    assert!(
        String::from_utf8(err.written)
            .unwrap()
            .contains("invalid agent name")
    );
}

/// Tracing: TC-022-06
/// ACs: FR-022-AC-6
///
/// A derivation that found no free name registers nothing, pops nothing, prints
/// nothing and leaves one `derive` record with its own code and the collided
/// name: the holder's registration is never written.
#[test]
fn tc_022_06_no_free_name_writes_nothing_and_records_one_derive_failure() {
    for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] {
        let h = Harness::new().with_inbox(&["kept"]);
        h.world.borrow_mut().collision = Some(name("qsl"));
        let out = h.run(false, &mut input(event).as_bytes(), false);
        assert_eq!(out.stdout, "", "{event}");
        let w = h.world.borrow();
        assert!(w.commits.is_empty(), "{event}: nothing is registered");
        assert!(!w.calls.contains(&Op::Pop), "{event}: no name, no pop");
        assert_eq!(w.inbox.len(), 1, "{event}: the mail stays queued");
        drop(w);
        let records = h.all_records();
        assert_eq!(records.len(), 1, "{event}: {records:?}");
        assert_eq!(
            (records[0].stage, records[0].error_kind),
            (Stage::Derive, ErrorKind::NoFreeName),
            "{event}"
        );
        assert_eq!(records[0].candidate.as_deref(), Some("qsl"), "{event}");
    }
}

/// Tracing: TC-030-10
/// ACs: FR-030-AC-10
///
/// A hook in a herdr pane whose herdr could not name the session (AGE-2059) is a
/// failed hook, not a silent non-lead: it registers and pops nothing and leaves
/// exactly one record, `budget` when the deadline was spent before herdr could
/// be asked and `derive`/`herdr_failed` when herdr itself failed.
#[test]
fn tc_030_10_a_herdr_failure_that_names_no_session_leaves_one_record() {
    for (budget_spent, expected) in [
        (true, (Stage::Budget, ErrorKind::BudgetExhausted)),
        (false, (Stage::Derive, ErrorKind::HerdrFailed)),
    ] {
        for event in ["SessionStart", "PostToolUse", "Stop"] {
            let h = Harness::new().with_inbox(&["kept"]);
            h.world.borrow_mut().herdr_failed = Some(budget_spent);
            let out = h.run(false, &mut input(event).as_bytes(), false);
            assert_eq!(out.stdout, "", "{event}");
            let w = h.world.borrow();
            assert!(w.commits.is_empty(), "{event}: nothing is registered");
            assert!(!w.calls.contains(&Op::Pop), "{event}: no name, no pop");
            drop(w);
            let records = h.all_records();
            assert_eq!(
                records.len(),
                1,
                "{event}, spent={budget_spent}: {records:?}"
            );
            assert_eq!(
                (records[0].stage, records[0].error_kind),
                expected,
                "{event}"
            );
        }
    }
}

/// Tracing: TC-030-11
/// ACs: FR-030-AC-11
///
/// A herdr outage records once per session per window, not once per tool call
/// (it would turn over the whole spool and the shared health stream), and never
/// stops the derivation: every call asks again, and the first one herdr answers
/// registers the lead.
#[test]
fn tc_030_11_a_herdr_outage_records_once_per_session_and_the_derivation_retries() {
    let h = Harness::new();
    h.world.borrow_mut().herdr_failed = Some(false);
    for _ in 0..5 {
        let out = h.run(false, &mut input("PostToolUse").as_bytes(), false);
        assert_eq!(out.stdout, "");
    }
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Derive, ErrorKind::HerdrFailed)
    );
    let derives = |h: &Harness| {
        h.world
            .borrow()
            .calls
            .iter()
            .filter(|c| **c == Op::Derive)
            .count()
    };
    assert_eq!(derives(&h), 5, "every call derived again");
    assert!(
        h.world.borrow().commits.is_empty(),
        "nothing registered yet"
    );

    // herdr is back: the next call derives the lead, no mark stands in the way.
    {
        let mut w = h.world.borrow_mut();
        w.herdr_failed = None;
        w.identity = Some(Derived::cached(name("qsl")));
    }
    h.run(false, &mut input("PostToolUse").as_bytes(), false);
    assert_eq!(derives(&h), 6);
    assert_eq!(h.all_records().len(), 1, "recovery adds no record");
    assert!(
        h.world.borrow().calls.contains(&Op::Pop),
        "the lead was derived and served"
    );
}

/// Tracing: TC-022-08
/// ACs: FR-022-AC-8
///
/// Repeated hooks of one session that all find no free name leave one record,
/// not one per call: the first sets the mark, later ones find it and stay silent.
#[test]
fn tc_022_08_repeated_no_free_name_hooks_leave_one_record() {
    let h = Harness::new();
    h.world.borrow_mut().collision = Some(name("qsl"));
    for _ in 0..5 {
        let out = h.run(false, &mut input("PostToolUse").as_bytes(), false);
        assert_eq!(out.stdout, "");
    }
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(h.world.borrow().commits.is_empty());
}

/// Tracing: TC-022-07
/// ACs: FR-022-AC-7
///
/// A holder whose liveness could not be checked was treated as live: the hook
/// still registers and delivers, and records why as a note, under its own code
/// for a herdr failure and for a spent budget.
#[test]
fn tc_022_07_a_liveness_note_is_recorded_and_the_hook_carries_on() {
    for (note, kind) in [
        (
            LivenessNote::HerdrDown {
                candidate: name("qsl"),
                source: relay_ops::naming::ProcessError::Spawn {
                    program: "herdr".into(),
                    reason: "gone".into(),
                },
            },
            ErrorKind::LivenessUnknown,
        ),
        (
            LivenessNote::BudgetSpent {
                candidate: name("qsl"),
            },
            ErrorKind::BudgetExhausted,
        ),
    ] {
        let h = Harness::new().with_inbox(&["hello"]);
        h.world.borrow_mut().identity = Some(Derived {
            note: Some(note),
            ..Derived::cached(name("qsl-2"))
        });
        let out = h.run(false, &mut input("SessionStart").as_bytes(), false);
        assert!(
            out.stdout.contains("hello"),
            "the hook delivers: {}",
            out.stdout
        );
        assert_eq!(h.world.borrow().commits.len(), 1, "and registers");
        let records = h.all_records();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(
            (records[0].stage, records[0].error_kind),
            (Stage::Derive, kind)
        );
        assert_eq!(records[0].candidate.as_deref(), Some("qsl"));
    }
}

#[test]
fn things_that_are_not_failures_write_nothing() {
    let h = Harness::new().with_inbox(&["hi"]);
    // An event we ignore, a subagent, and a session that is not a lead.
    h.run(true, &mut input("PreCompact").as_bytes(), false);
    let subagent = r#"{"hook_event_name":"Stop","session_id":"s1","agent_id":"a"}"#;
    h.run(true, &mut subagent.as_bytes(), false);
    h.world.borrow_mut().identity = None;
    h.run(false, &mut input("Stop").as_bytes(), false);
    assert!(h.records().is_empty());
    assert_eq!(h.world.borrow().inbox.len(), 1, "nothing popped");
}

/// Undecodable inbox entries and refused demotions are recorded too, without
/// failing the hook: the messages still arrive.
#[test]
fn a_rejected_inbox_entry_and_a_refused_demotion_are_recorded_and_the_hook_delivers() {
    let h = Harness::new().with_inbox(&["hi"]);
    {
        let mut w = h.world.borrow_mut();
        w.rejected = 2;
        w.refused = 1;
    }
    let out = h.run(true, &mut input("UserPromptSubmit").as_bytes(), false);
    assert!(out.stdout.contains("from sender: hi"), "{}", out.stdout);
    let kinds: Vec<(Stage, ErrorKind)> = h
        .all_records()
        .iter()
        .map(|r| (r.stage, r.error_kind))
        .collect();
    // The demotion is refused at the commit; the rejected entries at the pop.
    assert_eq!(
        kinds,
        [
            (Stage::Register, ErrorKind::Rejected),
            (Stage::Decode, ErrorKind::Corrupt),
            (Stage::Decode, ErrorKind::Corrupt),
        ]
    );
}

// ---- stderr that cannot be written, and panics -----------------------------

/// Tracing: TC-030-08
/// ACs: FR-030-AC-8
///
/// A closed stderr changes nothing: every stage still leaves exactly one
/// record and the hook does not panic.
#[test]
fn tc_030_08_every_stage_records_exactly_once_with_a_closed_stderr() {
    for op in [
        Op::Connect,
        Op::PaneCheck,
        Op::Derive,
        Op::RegisteredPlace,
        Op::Commit,
        Op::Pop,
    ] {
        let h = Harness::new().with_fault(op).with_inbox(&["hi"]);
        let mut out = Sink::default();
        let mut err = Sink {
            fail: true,
            ..Sink::default()
        };
        h.run_with(
            None,
            &mut input("SessionStart").as_bytes(),
            &mut out,
            &mut err,
        );
        assert_eq!(h.all_records().len(), 1, "{op:?}");
    }
    // The stages before a bus exists.
    let h = Harness::new();
    let mut err = Sink {
        fail: true,
        ..Sink::default()
    };
    h.run_with(
        Some("Bad Name"),
        &mut input("Stop").as_bytes(),
        &mut Sink::default(),
        &mut err,
    );
    h.run_with(
        Some("receiver"),
        &mut "nope".as_bytes(),
        &mut Sink::default(),
        &mut err,
    );
    assert_eq!(h.all_records().len(), 2);
    // The stdin faults: too large, and a pipe that never yields.
    let h = Harness::new();
    let big = vec![b' '; crate::stdin::MAX_INPUT_BYTES + 1];
    h.run_with(
        Some("receiver"),
        &mut big.as_slice(),
        &mut Sink::default(),
        &mut err,
    );
    assert_eq!(h.all_records().len(), 1);
    let (_hold, rx) = std::sync::mpsc::channel::<()>();
    let ticking = crate::budget::testing::TickingClock::new(ms(100));
    let cx = HookCtx {
        cli_name: Some("receiver"),
        kind: HookKind::Claude,
        machine: &"Hub.local".parse().unwrap(),
    };
    run(
        &cx,
        &ticking,
        Some(&h.spool),
        (
            &mut crate::stdin::ThreadedStdin::new(SilentPipe(rx)),
            &mut Sink::default(),
            &mut err,
        ),
        &mut |timeout| connect(&h.world, timeout),
    );
    assert_eq!(h.all_records().len(), 2);
    // Output and requeue failures.
    let h = Harness::new().with_inbox(&["m"]);
    let mut out = Sink {
        fail: true,
        ..Sink::default()
    };
    h.run_with(
        Some("receiver"),
        &mut input("Stop").as_bytes(),
        &mut out,
        &mut err,
    );
    assert_eq!(h.all_records().len(), 1);
    let h = Harness::new().with_fault(Op::Requeue).with_inbox(&["m"]);
    let mut out = Sink {
        fail: true,
        ..Sink::default()
    };
    h.run_with(
        Some("receiver"),
        &mut input("Stop").as_bytes(),
        &mut out,
        &mut err,
    );
    assert_eq!(h.all_records().len(), 1);
    // A budget failure.
    let h = Harness::new()
        .with_latency(Op::Connect, 890)
        .with_inbox(&["m"]);
    h.run_with(
        Some("receiver"),
        &mut input("Stop").as_bytes(),
        &mut Sink::default(),
        &mut err,
    );
    assert_eq!(h.all_records().len(), 1);
}

/// The spool record of a failed requeue, which holds a copy of the messages, is
/// written before anything that could panic on stderr.
#[test]
fn the_requeue_failed_record_is_written_before_stderr_is_touched() {
    let h = Harness::new()
        .with_fault(Op::Requeue)
        .with_inbox(&["m1", "m2"]);
    let mut out = Sink {
        fail: true,
        ..Sink::default()
    };
    // A stderr that panics like `eprintln!` on a closed pipe.
    let mut err = Sink {
        panic: true,
        ..Sink::default()
    };
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.run_with(
            Some("receiver"),
            &mut input("Stop").as_bytes(),
            &mut out,
            &mut err,
        );
    }));
    assert!(
        caught.is_err(),
        "the stderr panic is not swallowed by `run` itself"
    );
    let records = h.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].stage, Stage::RequeueFailed);
    assert_eq!(records[0].messages.len(), 2);
}

/// Tracing: TC-030-08
/// ACs: FR-030-AC-8
///
/// A panic anywhere in a bus call is caught, recorded once as `panic`, and does
/// not escape `run`.
#[test]
fn tc_030_08_a_panic_at_any_stage_is_caught_and_recorded_once() {
    for op in [
        Op::Connect,
        Op::PaneCheck,
        Op::Derive,
        Op::RegisteredPlace,
        Op::Commit,
        Op::Pop,
    ] {
        let h = Harness::new().with_inbox(&["hi"]);
        h.world.borrow_mut().panics.insert(op);
        let out = h.run(false, &mut input("SessionStart").as_bytes(), false);
        let records = h.records();
        assert_eq!(records.len(), 1, "{op:?}: {records:?}");
        assert_eq!(
            (records[0].stage, records[0].error_kind),
            (Stage::Panic, ErrorKind::Panic),
            "{op:?}"
        );
        assert_eq!(records[0].event, Some(Event::SessionStart), "{op:?}");
        assert!(out.stderr.contains("panicked"), "{op:?}: {}", out.stderr);
    }
}

// ---- messages after a late failure -----------------------------------------

/// A failed output write puts the messages back, in order, ahead of newer
/// ones, and records one `Output` failure.
#[test]
fn a_failed_output_requeues_the_messages_in_order() {
    let h = Harness::new().with_inbox(&["m1", "m2", "m3", "m4"]);
    h.run(true, &mut input("Stop").as_bytes(), true);
    let texts: Vec<String> = h
        .world
        .borrow()
        .inbox
        .iter()
        .map(|m| m.text.clone())
        .collect();
    assert_eq!(texts, ["m1", "m2", "m3", "m4"]);
    let records = h.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Output, ErrorKind::Io)
    );
    assert!(records[0].messages.is_empty(), "they are safe in Redis");
}

/// When even the requeue fails, the batch is left in flight (the lease returns
/// it) and the spool record keeps a copy of the messages; it is the only record
/// of that failure.
#[test]
fn a_failed_requeue_keeps_the_messages_in_the_spool() {
    let h = Harness::new()
        .with_fault(Op::Requeue)
        .with_inbox(&["m1", "m2"]);
    let out = h.run(true, &mut input("Stop").as_bytes(), true);
    let records = h.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::RequeueFailed, ErrorKind::Connection)
    );
    let kept: Vec<&str> = records[0]
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(kept, ["m1", "m2"]);
    assert!(h.world.borrow().inbox.is_empty());
    assert!(out.stderr.contains("2 popped message(s)"), "{}", out.stderr);
}

/// No time left to requeue is not attempted, and the messages are still kept.
#[test]
fn a_requeue_with_no_budget_left_is_spooled_instead() {
    // Connect 40ms makes the window 100. Registering leaves exactly 100; the
    // pop then takes all of it, so the failed output has no budget to requeue
    // with.
    let h = Harness::new()
        .with_latency(Op::Connect, 40)
        .with_latency(Op::Commit, 160)
        .with_latency(Op::Pop, 100)
        .with_inbox(&["m1"]);
    h.run(true, &mut input("UserPromptSubmit").as_bytes(), true);
    let records = h.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::RequeueFailed, ErrorKind::BudgetExhausted)
    );
    assert_eq!(records[0].messages.len(), 1);
    assert!(!h.world.borrow().calls.contains(&Op::Requeue));
}

// ---- the in-flight list: acknowledgement and put-back ------------------------

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// A delivered batch is acknowledged once, after the write, and only then:
/// nothing stays in flight and the inbox holds only what was not popped.
#[test]
fn a_delivered_batch_is_acknowledged_once_after_the_pop() {
    let h = Harness::new().with_inbox(&["m1", "m2", "m3", "m4"]);
    let out = h.run(true, &mut input("Stop").as_bytes(), false);
    assert!(out.stdout.contains("m3"), "{}", out.stdout);
    let w = h.world.borrow();
    assert_eq!(w.acked, [1], "the one batch, once");
    assert!(w.inflight.is_empty());
    assert_eq!(w.inbox.len(), 1, "the fourth was not popped");
    let pop = w.calls.iter().position(|c| *c == Op::Pop).unwrap();
    let ack = w.calls.iter().position(|c| *c == Op::Ack).unwrap();
    assert!(pop < ack);
    assert!(!w.calls.contains(&Op::Requeue));
    drop(w);
    assert!(h.all_records().is_empty(), "{:?}", h.all_records());
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// A batch is acknowledged only when the output was written: a failed write
/// puts the batch back (one requeue, no acknowledgement) and nothing is in flight.
#[test]
fn a_failed_write_requeues_the_batch_and_never_acknowledges_it() {
    let h = Harness::new().with_inbox(&["m1", "m2"]);
    h.run(true, &mut input("Stop").as_bytes(), true);
    let w = h.world.borrow();
    assert!(w.acked.is_empty());
    assert!(w.calls.contains(&Op::Requeue));
    assert!(
        w.inflight.is_empty(),
        "back in the inbox, not also in flight"
    );
    assert_eq!(w.inbox.len(), 2);
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// When the put-back itself fails the batch stays in flight, unacknowledged:
/// the lease will return it.
#[test]
fn a_failed_requeue_leaves_the_batch_in_flight_and_unacknowledged() {
    let h = Harness::new()
        .with_fault(Op::Requeue)
        .with_inbox(&["m1", "m2"]);
    h.run(true, &mut input("Stop").as_bytes(), true);
    let w = h.world.borrow();
    assert!(w.acked.is_empty());
    assert_eq!(w.inflight.len(), 1, "still recorded, to be redelivered");
    assert_eq!(w.inflight[0].1.len(), 2);
    assert!(w.inbox.is_empty());
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// An acknowledgement that fails after the output was written is recorded as an
/// `ack` failure, and the messages were delivered: the batch stays in flight and
/// will be redelivered, marked. It is not requeued (that would duplicate now).
#[test]
fn a_failed_acknowledgement_is_recorded_and_the_batch_stays_in_flight() {
    let h = Harness::new().with_fault(Op::Ack).with_inbox(&["m1"]);
    let out = h.run(true, &mut input("Stop").as_bytes(), false);
    assert!(out.stdout.contains("m1"), "{}", out.stdout);
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Ack, ErrorKind::Connection)
    );
    assert!(out.stderr.contains("redelivered"), "{}", out.stderr);
    let w = h.world.borrow();
    assert_eq!(w.inflight.len(), 1);
    assert!(!w.calls.contains(&Op::Requeue));
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// With no time left at all for the acknowledgement it is not attempted, and the
/// skipped acknowledgement is recorded like a failed one.
#[test]
fn an_acknowledgement_with_the_budget_spent_is_skipped_and_recorded() {
    // Connect 40 ms makes the window 100. The pop takes all that is left.
    let h = Harness::new()
        .with_latency(Op::Connect, 40)
        .with_latency(Op::Commit, 160)
        .with_latency(Op::Pop, 100)
        .with_inbox(&["m1"]);
    let out = h.run(true, &mut input("UserPromptSubmit").as_bytes(), false);
    assert!(out.stdout.contains("m1"), "{}", out.stdout);
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Ack, ErrorKind::BudgetExhausted)
    );
    assert!(!h.world.borrow().calls.contains(&Op::Ack));
}

/// Tracing: TC-020-15
/// ACs: FR-020-AC-15
///
/// On a 60 ms link an explicit-name `UserPromptSubmit` prints, and acknowledges,
/// and leaves no record: the address read is the step that yields (it needs a
/// window of its own that is not left), so bare bus names are printed, and the
/// acknowledgement follows the write at once.
#[test]
fn tc_020_15_an_explicit_name_prompt_at_60_ms_prints_and_acknowledges_with_no_record() {
    let h = Harness::new().with_rtt(60).with_inbox(&["m1"]);
    let out = h.run(true, &mut input("UserPromptSubmit").as_bytes(), false);
    assert!(
        out.stdout.contains("m1"),
        "stdout: {} stderr: {}",
        out.stdout,
        out.stderr
    );
    let w = h.world.borrow();
    assert_eq!(
        w.acked,
        [1],
        "acknowledged, so not delivered twice: {:?}",
        w.calls
    );
    assert!(w.inflight.is_empty());
    assert!(!w.calls.contains(&Op::Book), "the address read gave way");
    drop(w);
    assert!(h.all_records().is_empty(), "{:?}", h.all_records());
}

/// Tracing: TC-020-15
/// ACs: FR-020-AC-15
///
/// A derived name makes one more request before the pop. At 60 ms the pop does not
/// fit its window, so the prompt defers silently, the mail stays queued for the
/// next `PostToolUse` or `Stop`, and there is no record and no duplicate. At 50
/// ms it pops, prints and acknowledges, again with no record.
#[test]
fn tc_020_15_a_derived_name_prompt_defers_at_60_ms_and_delivers_and_acknowledges_at_50() {
    let slow = Harness::new().with_rtt(60).with_inbox(&["m1"]);
    let out = slow.run(false, &mut input("UserPromptSubmit").as_bytes(), false);
    assert_eq!(out.stdout, "", "nothing printed");
    let w = slow.world.borrow();
    assert!(!w.calls.contains(&Op::Pop), "the pop was not issued");
    assert_eq!(w.inbox.len(), 1, "the mail is still queued");
    drop(w);
    assert!(slow.all_records().is_empty(), "{:?}", slow.all_records());

    let ok = Harness::new().with_rtt(50).with_inbox(&["m1"]);
    let out = ok.run(false, &mut input("UserPromptSubmit").as_bytes(), false);
    assert!(
        out.stdout.contains("m1"),
        "stdout: {} stderr: {}",
        out.stdout,
        out.stderr
    );
    assert_eq!(ok.world.borrow().acked, [1]);
    assert!(ok.all_records().is_empty(), "{:?}", ok.all_records());
}

/// Tracing: TC-020-15
/// ACs: FR-020-AC-15
///
/// The acknowledgement is the next request after the write and does not need the
/// address book: on a fast link the book is read before the write and nothing
/// stands between the write and the acknowledgement.
#[test]
fn tc_020_15_the_acknowledgement_follows_the_write_directly() {
    let h = Harness::new().with_rtt(5).with_inbox(&["m1"]);
    h.run(true, &mut input("Stop").as_bytes(), false);
    let calls = h.world.borrow().calls.clone();
    let pop = calls.iter().position(|c| *c == Op::Pop).unwrap();
    assert_eq!(&calls[pop..], [Op::Pop, Op::Book, Op::Ack]);
}

/// Tracing: TC-020-14
/// ACs: FR-020-AC-14
///
/// A hook whose pop took nothing because the lead is at its in-flight cap records
/// it (mail is delayed), except at `UserPromptSubmit` where the deferral is
/// expected and silent; a `Stop` hook prints nothing and does not acknowledge.
#[test]
fn tc_020_14_a_hook_at_the_in_flight_cap_records_it_except_at_user_prompt_submit() {
    for (event, recorded) in [("Stop", true), ("UserPromptSubmit", false)] {
        let h = Harness::new().with_inbox(&["m1"]);
        h.world.borrow_mut().capped = true;
        let out = h.run(true, &mut input(event).as_bytes(), false);
        assert_eq!(out.stdout, "", "{event}");
        let records = h.all_records();
        assert_eq!(records.len(), usize::from(recorded), "{event}: {records:?}");
        if recorded {
            assert_eq!(
                (records[0].stage, records[0].error_kind),
                (Stage::Pop, ErrorKind::InflightFull)
            );
        }
        assert!(h.world.borrow().acked.is_empty());
    }
}

/// Tracing: TC-020-10
/// ACs: FR-020-AC-10
///
/// A pop that reports rows it could not dead-letter (the lead's dead-letter key
/// is unusable) still delivers, and the hook records it.
#[test]
fn a_pop_that_reports_stuck_rows_still_delivers_and_records_it() {
    let h = Harness::new().with_inbox(&["m1"]);
    h.world.borrow_mut().stuck = 2;
    let out = h.run(true, &mut input("Stop").as_bytes(), false);
    assert!(out.stdout.contains("m1"));
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Pop, ErrorKind::Corrupt)
    );
    assert!(
        out.stderr.contains("could not be dead-lettered"),
        "{}",
        out.stderr
    );
}

/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
///
/// The record of an acknowledgement says what happened: a batch already gone from
/// flight (its lease expired first) is `lease_expired`, not a failed request.
#[test]
fn an_acknowledgement_that_finds_its_batch_gone_is_recorded_as_such() {
    let h = Harness::new().with_inbox(&["m1"]);
    h.world.borrow_mut().reaped_before_ack = true;
    let out = h.run(true, &mut input("Stop").as_bytes(), false);
    assert!(out.stdout.contains("m1"));
    let records = h.all_records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        (records[0].stage, records[0].error_kind),
        (Stage::Ack, ErrorKind::LeaseExpired)
    );
    assert!(out.stderr.contains("no longer in flight"), "{}", out.stderr);
}

// ---- SessionStart: identity writes are one step, at the end ----------------

/// Every identity write is one `commit_identity` call, after every read and
/// before the pop (which the acknowledgement of what it delivered follows).
#[test]
fn identity_is_written_once_after_the_reads_and_before_the_pop() {
    let h = Harness::new().with_inbox(&["hi"]);
    h.run(false, &mut input("SessionStart").as_bytes(), false);
    let w = h.world.borrow();
    assert_eq!(
        w.calls,
        [
            Op::Connect,
            Op::Probe,
            Op::Derive,
            Op::RegisteredPlace,
            Op::PaneCheck,
            Op::Commit,
            Op::Pop,
            Op::Book,
            Op::Ack
        ]
    );
    assert_eq!(w.commits.len(), 1);
    assert!(w.commits[0].registration.is_some());
}

/// Running out of budget before the write leaves nothing written: no
/// registration, no cache, no aliases.
#[test]
fn a_budget_overrun_before_the_commit_writes_nothing() {
    let millis = u64::try_from(budget_of("SessionStart").as_millis()).unwrap();
    let h = Harness::new().with_latency(Op::Derive, millis);
    h.run(false, &mut input("SessionStart").as_bytes(), false);
    let w = h.world.borrow();
    assert!(w.commits.is_empty());
    assert!(!w.calls.contains(&Op::Commit));
    let records = h.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].stage, Stage::Budget);
}

/// A session that must never join the bus is remembered, once, as the only write.
#[test]
fn a_suppressed_session_is_committed_alone() {
    let h = Harness::new();
    // `derive` answers `Suppress` for this session via a dedicated bus.
    struct SuppressBus(FakeBus);
    impl Bus for SuppressBus {
        fn set_timeout(&mut self, t: Duration) -> relay_core::Result<()> {
            self.0.set_timeout(t)
        }
        fn pane_place(&mut self) -> relay_core::Result<Option<Place>> {
            self.0.pane_place()
        }
        fn probe_session(&mut self, s: &SessionId) -> relay_core::Result<SessionProbe> {
            self.0.probe_session(s)
        }
        fn derive(
            &mut self,
            _r: &HookRequest<'_>,
            _p: &SessionProbe,
            _b: &Budget<'_>,
        ) -> Result<HookIdentity, CollisionError> {
            Ok(HookIdentity::Suppress {
                session_id: Some("s1".parse().unwrap()),
            })
        }
        fn mark_unresolved(&mut self, s: &SessionId) -> relay_core::Result<bool> {
            self.0.mark_unresolved(s)
        }
        fn mark_herdr_failed(&mut self, s: &SessionId) -> relay_core::Result<bool> {
            self.0.mark_herdr_failed(s)
        }
        fn registered_place(&mut self, n: &AgentName) -> relay_core::Result<Option<Place>> {
            self.0.registered_place(n)
        }
        fn commit_identity(&mut self, c: &IdentityCommit) -> relay_core::Result<CommitOutcome> {
            self.0.commit_identity(c)
        }
        fn pop_inbox(&mut self, n: &AgentName) -> relay_core::Result<BudgetPopResult> {
            self.0.pop_inbox(n)
        }
        fn ack(&mut self, n: &AgentName, b: BatchId) -> relay_core::Result<bool> {
            self.0.ack(n, b)
        }
        fn requeue_batch(&mut self, n: &AgentName, b: BatchId) -> relay_core::Result<usize> {
            self.0.requeue_batch(n, b)
        }
        fn address_book(&mut self) -> AddressBook {
            self.0.address_book()
        }
        fn push_hook_health(&mut self, e: &[HookHealthEntry]) -> relay_core::Result<()> {
            self.0.push_hook_health(e)
        }
    }
    let cx = HookCtx {
        cli_name: None,
        kind: HookKind::Claude,
        machine: &"hub".parse().unwrap(),
    };
    run(
        &cx,
        &*h.clock,
        Some(&h.spool),
        (
            &mut input("SessionStart").as_bytes(),
            &mut Sink::default(),
            &mut Sink::default(),
        ),
        &mut |t| connect(&h.world, t).map(SuppressBus),
    );
    let w = h.world.borrow();
    assert_eq!(w.commits.len(), 1);
    assert_eq!(
        w.commits[0].suppress_session.as_ref().map(|s| s.as_str()),
        Some("s1")
    );
    assert!(w.commits[0].registration.is_none());
    assert!(!w.calls.contains(&Op::Pop));
    assert!(h.records().is_empty());
}

// ---- the spool is shipped by the next healthy hook -------------------------

#[test]
fn the_next_healthy_hook_ships_the_spool_with_attribution_and_removes_it() {
    let h = Harness::new().with_fault(Op::Pop);
    h.run_event("Stop");
    assert_eq!(h.records().len(), 1);
    // Recovered: the next hook ships the record.
    h.world.borrow_mut().faults.clear();
    h.run_event("Stop");
    let w = h.world.borrow();
    assert_eq!(w.health.len(), 1);
    assert_eq!(w.health[0].machine.as_str(), "hub");
    assert_eq!(w.health[0].lead.as_deref(), Some("receiver"));
    let shipped: SpoolRecord = serde_json::from_str(&w.health[0].record).unwrap();
    assert_eq!(shipped.stage, Stage::Pop);
    drop(w);
    assert!(h.records().is_empty(), "removed");
    // Shipping again finds nothing: idempotent.
    h.run_event("Stop");
    assert_eq!(h.world.borrow().health.len(), 1);
}

#[test]
fn a_failed_ship_leaves_the_spool_untouched_and_is_not_itself_recorded() {
    let h = Harness::new().with_fault(Op::Pop);
    h.run_event("Stop");
    let before = std::fs::read(h.spool_path()).unwrap();
    {
        let mut w = h.world.borrow_mut();
        w.faults.clear();
        w.faults.insert(Op::Push);
    }
    h.run_event("Stop");
    assert_eq!(before, std::fs::read(h.spool_path()).unwrap());
    assert!(h.world.borrow().health.is_empty());
}

#[test]
fn a_healthy_hook_with_an_empty_spool_makes_no_push() {
    let h = Harness::new();
    h.run_event("Stop");
    assert!(!h.world.borrow().calls.contains(&Op::Push));
}

/// A spool bigger than one round trip is shipped in chunks, all of it, in
/// order.
#[test]
fn a_large_spool_is_shipped_whole_in_chunks() {
    let h = Harness::new();
    let subject = crate::spool::Subject::on("hub".parse().unwrap());
    for n in 0..450 {
        h.spool
            .record(&SpoolRecord::new(
                n,
                &subject,
                Stage::Pop,
                ErrorKind::Timeout,
            ))
            .unwrap();
    }
    h.run_event("Stop");
    let w = h.world.borrow();
    assert_eq!(w.health.len(), 450);
    assert_eq!(w.pushes, 3, "200 + 200 + 50");
    let times: Vec<i64> = w
        .health
        .iter()
        .map(|e| {
            serde_json::from_str::<SpoolRecord>(&e.record)
                .unwrap()
                .at_ms
        })
        .collect();
    assert_eq!(times, (0..450).collect::<Vec<i64>>());
    drop(w);
    assert!(h.records().is_empty());
}

/// Lines of the spool that are not records (a machine outside its type's rule,
/// bytes no binary wrote) are removed with the shipped ones, and the removal is
/// counted in a record of its own, never silent.
#[test]
fn spool_lines_that_are_not_records_are_counted_when_the_flush_removes_them() {
    let h = Harness::new();
    let subject = crate::spool::Subject::on("hub".parse().unwrap());
    h.spool
        .record(&SpoolRecord::new(
            1,
            &subject,
            Stage::Pop,
            ErrorKind::Timeout,
        ))
        .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(h.spool_path())
        .unwrap();
    std::io::Write::write_all(
        &mut file,
        b"{\"at_ms\":2,\"machine\":\"\",\"lead\":null,\"event\":null,\"session_id\":null,\"stage\":\"pop\",\"error_kind\":\"timeout\"}\nnot json at all\n",
    )
    .unwrap();
    drop(file);
    h.run_event("Stop");
    assert_eq!(h.world.borrow().health.len(), 1, "only the record ships");
    let left = h.records();
    assert_eq!(
        left.len(),
        1,
        "one record is left: the count of the lines removed"
    );
    assert_eq!(left[0].dropped, 2);
    assert_eq!(left[0].stage, Stage::Decode);
}

/// Tracing: TC-030-04
/// ACs: FR-030-AC-4
///
/// Running out of budget between chunks removes only what was shipped; the
/// rest waits for the next hook, and nothing is lost or repeated.
#[test]
fn tc_030_04_a_flush_that_runs_out_of_budget_keeps_the_unshipped_rest() {
    let h = Harness::new();
    let subject = crate::spool::Subject::on("hub".parse().unwrap());
    for n in 0..450 {
        h.spool
            .record(&SpoolRecord::new(
                n,
                &subject,
                Stage::Pop,
                ErrorKind::Timeout,
            ))
            .unwrap();
    }
    // Stop has 900ms: each push takes 400ms, so only two fit above the
    // 100ms flush window.
    h.world.borrow_mut().latency.insert(Op::Push, vec![ms(400)]);
    h.run_event("Stop");
    assert_eq!(h.world.borrow().health.len(), 400);
    assert_eq!(h.records().len(), 50, "the rest is still spooled");
    // The next hook ships the remainder, once.
    h.world.borrow_mut().latency.clear();
    h.h_next();
    let w = h.world.borrow();
    assert_eq!(w.health.len(), 450);
    let mut times: Vec<i64> = w
        .health
        .iter()
        .map(|e| {
            serde_json::from_str::<SpoolRecord>(&e.record)
                .unwrap()
                .at_ms
        })
        .collect();
    times.sort_unstable();
    assert_eq!(times, (0..450).collect::<Vec<i64>>());
}

impl Harness {
    /// Another hook call on a fresh clock.
    fn h_next(&self) {
        let fresh = Rc::new(FakeClock::default());
        let world = Rc::clone(&self.world);
        world.borrow_mut().clock = Rc::clone(&fresh);
        let cx = HookCtx {
            cli_name: Some("receiver"),
            kind: HookKind::Claude,
            machine: &"hub".parse().unwrap(),
        };
        run(
            &cx,
            &*fresh,
            Some(&self.spool),
            (
                &mut input("Stop").as_bytes(),
                &mut Sink::default(),
                &mut Sink::default(),
            ),
            &mut |t| connect(&world, t),
        );
    }
}

#[test]
fn a_hook_short_of_budget_does_not_ship() {
    let h = Harness::new().with_fault(Op::Pop);
    h.run_event("Stop");
    h.world.borrow_mut().faults.clear();
    // Stop has 900ms. Connecting takes 100 (a 220ms window), the pop takes 700,
    // leaving 100 - under nothing, but exactly the flush window less 1.
    {
        let mut w = h.world.borrow_mut();
        w.latency.insert(Op::Connect, vec![ms(100)]);
        w.latency.insert(Op::Pop, vec![ms(701)]);
    }
    let before = h.world.borrow().calls.len();
    h.run_event("Stop");
    let w = h.world.borrow();
    assert!(w.calls[before..].contains(&Op::Pop), "the pop still ran");
    assert!(
        !w.calls[before..].contains(&Op::Push),
        "but nothing shipped"
    );
    drop(w);
    assert_eq!(h.records().len(), 1, "still waiting to ship");
}

/// A fault that repeats on every call and happens before the hook connects
/// cannot wait for a later healthy hook: it ships the spool itself, once, and
/// still exits.
#[test]
fn an_invalid_name_ships_the_spool_itself() {
    let h = Harness::new();
    let mut out = Sink::default();
    let mut err = Sink::default();
    h.run_with(
        Some("Bad Name"),
        &mut input("Stop").as_bytes(),
        &mut out,
        &mut err,
    );
    let w = h.world.borrow();
    assert_eq!(w.health.len(), 1, "its own record was shipped");
    assert_eq!(
        serde_json::from_str::<SpoolRecord>(&w.health[0].record)
            .unwrap()
            .stage,
        Stage::Derive
    );
    drop(w);
    assert!(h.records().is_empty());
}

/// When the connection it then tries is refused, the record stays in the spool.
#[test]
fn an_invalid_name_with_a_dead_redis_keeps_its_record() {
    let h = Harness::new().with_fault(Op::Connect);
    h.run_with(
        Some("Bad Name"),
        &mut input("Stop").as_bytes(),
        &mut Sink::default(),
        &mut Sink::default(),
    );
    assert_eq!(h.records().len(), 1);
}

/// A connect failure is not a persistent input fault: it does not connect
/// again to ship.
#[test]
fn a_connect_failure_does_not_retry_to_ship() {
    let h = Harness::new().with_fault(Op::Connect);
    h.run_event("Stop");
    assert_eq!(
        h.world
            .borrow()
            .calls
            .iter()
            .filter(|op| **op == Op::Connect)
            .count(),
        1
    );
}

// ---- properties ------------------------------------------------------------

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Connect),
        Just(Op::PaneCheck),
        Just(Op::Derive),
        Just(Op::RegisteredPlace),
        Just(Op::Commit),
        Just(Op::Pop),
        Just(Op::Requeue),
        Just(Op::Push),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// P9: whatever fails and however slow the bus is, every event's hook
    /// finishes within its budget, never records more than one failure, and
    /// records one whenever an op it reached was faulted. Ops are single
    /// replies here; a `derive` of several commands is not bounded by the
    /// budget in total, only per read (AGE-2028).
    #[test]
    fn every_hook_path_stays_in_budget_and_records_one_failure_per_failure(
        event in prop::sample::select(EVENTS.to_vec()),
        explicit in any::<bool>(),
        fail_output in any::<bool>(),
        faults in prop::collection::btree_set(op_strategy(), 0..3),
        latencies in prop::collection::btree_map(op_strategy(), 0u64..900, 0..5),
        inbox in 0usize..5,
    ) {
        let h = Harness::new();
        {
            let mut w = h.world.borrow_mut();
            w.faults = faults.clone();
            w.latency = latencies.iter().map(|(op, m)| (*op, vec![ms(*m)])).collect();
            w.inbox = (0..inbox).map(|i| message(&format!("<m-{i}>"))).collect();
        }
        h.run(explicit, &mut input(event).as_bytes(), fail_output);

        prop_assert!(h.clock.elapsed() <= budget_of(event),
            "ran {:?} of {:?}", h.clock.elapsed(), budget_of(event));
        let records = h.records();
        // A shipped-on-the-spot record leaves the file empty again, so count
        // what was recorded in all: still at most one failure.
        let shipped = h.world.borrow().health.len();
        prop_assert!(records.len() + shipped <= 1, "{records:?}");
        let w = h.world.borrow();
        let hit_fault = w.calls.iter().any(|op| *op != Op::Push && faults.contains(op));
        if hit_fault {
            prop_assert_eq!(records.len() + shipped, 1, "{:?} {:?}", w.calls, records);
        }
        // Every wait was bounded by what was left of the budget when it was armed.
        for (at, timeout) in &w.armed {
            prop_assert!(*at + *timeout <= budget_of(event), "armed {timeout:?} at {at:?}");
        }
    }
}

/// One step of the conservation model.
#[derive(Debug, Clone)]
enum Step {
    Send,
    /// A hook whose connect takes `connect_ms`, whose bus calls before the pop
    /// each take `pre_ms`, and whose output write fails per `output`. The pop
    /// reply takes exactly the window the hook measured.
    Hook {
        connect_ms: u64,
        pre_ms: u64,
        output: OutputFault,
    },
}

#[derive(Debug, Clone, Copy)]
enum OutputFault {
    Fine,
    WriteFails,
    WriteAndRequeueFail,
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::Send),
        (
            prop::sample::select(vec![0u64, 10, 40]),
            prop::sample::select(vec![0u64, 60, 80, 95, 100, 250]),
            prop_oneof![
                Just(OutputFault::Fine),
                Just(OutputFault::WriteFails),
                Just(OutputFault::WriteAndRequeueFail)
            ],
        )
            .prop_map(|(connect_ms, pre_ms, output)| Step::Hook {
                connect_ms,
                pre_ms,
                output
            }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Tracing: TC-020-03
    /// ACs: FR-020-AC-3
    ///
    /// Partial: covers the failure points after the pop that this crate can
    /// inject (output write, requeue). A pop reply that outlives the timeout is
    /// prevented by the pop window, not recovered from; see the window property
    /// below and `pop_window.rs`. A message kept by a failed requeue is
    /// reported in the spool record and on stderr rather than stdout.
    ///
    /// Conservation: over any sequence of sends and hooks — popping, skipping
    /// the pop for budget, failing the output, failing the requeue too — every
    /// message sent is in exactly one of: the inbox, delivered, or spooled as
    /// `RequeueFailed`. None is lost and none is duplicated.
    #[test]
    fn no_message_is_ever_lost_or_duplicated(steps in prop::collection::vec(step_strategy(), 1..24)) {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path().join("hook.jsonl"), 256 * 1024);
        let health: Rc<RefCell<Vec<HookHealthEntry>>> = Rc::default();
        let inbox: Rc<RefCell<Vec<Message>>> = Rc::default();
        let mut sent = Vec::new();
        let mut delivered: Vec<String> = Vec::new();

        for step in steps {
            match step {
                Step::Send => {
                    let text = format!("<m-{}>", sent.len());
                    inbox.borrow_mut().push(message(&text));
                    sent.push(text);
                }
                Step::Hook { connect_ms, pre_ms, output } => {
                    let clock = Rc::new(FakeClock::default());
                    let world = new_world(&clock);
                    {
                        let mut w = world.borrow_mut();
                        std::mem::swap(&mut w.inbox, &mut *inbox.borrow_mut());
                        std::mem::swap(&mut w.health, &mut *health.borrow_mut());
                        w.latency.insert(Op::Connect, vec![ms(connect_ms)]);
                        for op in [Op::PaneCheck, Op::RegisteredPlace, Op::Commit] {
                            w.latency.insert(op, vec![ms(pre_ms)]);
                        }
                        // A pop answers in exactly the window the hook computed
                        // from its connect time: the assumption the window rests on.
                        w.latency.insert(Op::Pop, vec![pop_window(ms(connect_ms.max(pre_ms)))]);
                        if matches!(output, OutputFault::WriteAndRequeueFail) {
                            w.faults.insert(Op::Requeue);
                        }
                    }
                    let cx = HookCtx { cli_name: Some("receiver"), kind: HookKind::Claude, machine: &"hub".parse().unwrap() };
                    let mut out = Sink { fail: !matches!(output, OutputFault::Fine), ..Sink::default() };
                    run(&cx, &*clock, Some(&spool),
                        (&mut input("UserPromptSubmit").as_bytes(), &mut out, &mut Sink::default()),
                        &mut |t| connect(&world, t));
                    let printed = String::from_utf8(out.written).unwrap();
                    for text in &sent {
                        if printed.contains(text.as_str()) {
                            delivered.push(text.clone());
                        }
                    }
                    let mut w = world.borrow_mut();
                    std::mem::swap(&mut w.inbox, &mut *inbox.borrow_mut());
                    std::mem::swap(&mut w.health, &mut *health.borrow_mut());
                }
            }
        }

        let mut spooled: Vec<String> = Vec::new();
        let lines = std::fs::read_to_string(dir.path().join("hook.jsonl")).unwrap_or_default();
        for line in lines.lines().map(str::to_owned).chain(health.borrow().iter().map(|e| e.record.clone())) {
            let record: SpoolRecord = serde_json::from_str(&line).unwrap();
            if record.stage == Stage::RequeueFailed {
                spooled.extend(record.messages.into_iter().map(|m| m.text));
            }
        }
        let mut seen: Vec<String> = inbox.borrow().iter().map(|m| m.text.clone()).collect();
        seen.extend(delivered);
        seen.extend(spooled);
        seen.sort();
        let mut expected = sent;
        expected.sort();
        prop_assert_eq!(seen, expected);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]

    /// Tracing: TC-020-03
    /// ACs: FR-020-AC-3
    ///
    /// Partial (see the conservation property above). For any measured connect
    /// time and any budget left, if the hook pops,
    /// then a single reply of at most the window it computed never loses mail:
    /// the messages are delivered, and no pop failure is recorded.
    #[test]
    fn a_pop_reply_within_the_window_never_loses_mail(
        event in prop::sample::select(EVENTS.to_vec()),
        connect_ms in 0u64..250,
        pre_ms in 0u64..140,
        reply_fraction in 0u64..=100,
        inbox in 1usize..4,
    ) {
        let sampled = matches!(event, "SessionStart" | "UserPromptSubmit");
        let window = if sampled {
            pop_window(ms(connect_ms.max(pre_ms)))
        } else {
            pop_window(ms(connect_ms)).max(UNSAMPLED_POP_WINDOW)
        };
        let reply = window * u32::try_from(reply_fraction).unwrap() / 100;
        let h = Harness::new();
        {
            let mut w = h.world.borrow_mut();
            w.latency.insert(Op::Connect, vec![ms(connect_ms)]);
            for op in [Op::PaneCheck, Op::RegisteredPlace, Op::Commit] {
                w.latency.insert(op, vec![ms(pre_ms)]);
            }
            w.latency.insert(Op::Pop, vec![reply]);
            w.inbox = (0..inbox).map(|i| message(&format!("<m-{i}>"))).collect();
        }
        let out = h.run(true, &mut input(event).as_bytes(), false);
        let popped = h.world.borrow().calls.contains(&Op::Pop);
        let left = h.world.borrow().inbox.len();
        if popped {
            // Every message is delivered or still queued (pending); none vanished.
            let delivered = (0..inbox).filter(|i| out.stdout.contains(&format!("<m-{i}>"))).count();
            prop_assert_eq!(delivered + left, inbox);
            prop_assert!(h.records().iter().all(|r| r.stage != Stage::Pop), "{:?}", h.records());
        } else {
            prop_assert_eq!(left, inbox, "a skipped pop touches nothing");
        }
    }
}
