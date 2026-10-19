//! `agent-msg hook <kind>` end to end: read the harness's input, work out who
//! this session is, register, pop the inbox, print what the agent should see.
//!
//! # Failure contract
//!
//! A hook never fails the agent's turn: [`run`] returns nothing, catches a
//! panic, and the process exits 0 whatever happens. What it must not do is fail
//! *silently*. Every failure is one [`HookError`], recorded once — first to the
//! local spool ([`crate::spool`]), then to stderr, which may itself be closed
//! and is written without panicking — and the next healthy hook ships the
//! spool to `health:hooks`.
//!
//! A hook that fails before it could connect for a reason that repeats on every
//! call (an invalid `AGENT_MSG_NAME`, unreadable hook input) has no "next
//! healthy hook" of its own, so it makes one attempt to ship the spool itself
//! ([`flush_after_failure`]). A configuration file that will not load, or a
//! command line that will not parse, happens before a Redis URL is known: those
//! records wait for any other hook on the machine that does reach Redis.
//!
//! # Session verification
//!
//! Hooks enroll only the genuine harness session at its unique live Herdr tab.
//! An unavailable or ambiguous placement never creates a substitute identity.
//!
//! Nor is a herdr failure that leaves the session without any name (AGE-2059):
//! in a herdr pane, a hook that cannot learn who the session is registers and
//! pops nothing and records that (`budget`/`budget_exhausted` when its time ran
//! out before herdr could be asked, else `derive`/`herdr_failed`), where it used to
//! pass for a session that is not a lead and leave no record.
//!
//! That record is limited to one per session per [`relay_core::HERDR_FAILED_TTL_SECS`]
//! window by a mark of its own (`SET NX EX`), so an outage does not turn over the
//! spool and the shared health stream; the mark never stops the derivation, which
//! every call retries, so the lead registers on the first call herdr answers.
//!
//! # Pop safety
//!
//! The pop is one `EVAL` (one request, one reply) that takes messages out of the
//! inbox server-side and records them in flight. The Redis read timeout is
//! `SO_RCVTIMEO`: it bounds each read, not the whole call, so a reply that takes
//! longer than the timeout the hook armed is one the hook never sees; the batch
//! is then in flight and returns when its lease expires (before AGE-2032 it was
//! lost). The hook still pops only when at least [`pop_window`] of budget
//! remains: the reply at about twice the round trip the hook measured, so that
//! the acknowledgement after the write also fits, plus a margin. A skipped pop
//! delivers nothing and loses nothing, the next hook delivers; a skipped
//! `PostToolUse`/`Stop` pop is recorded (delivery is really delayed), a skipped
//! `UserPromptSubmit` pop is not (the mail arrives at the next `PostToolUse` or
//! `Stop`, an expected deferral). A failure *after* a successful pop (the output
//! write) puts the batch back at the front of the inbox, and if even that fails
//! it stays in flight and the lease returns it.
//!
//! # In flight
//!
//! The pop moves its batch into the lead's in-flight record in the same script
//! (and first returns every due batch of that lead to the front of the inbox),
//! so a hook killed after the pop loses nothing: the batch returns when its lease
//! ([`relay_core::HOOK_LEASE`]) expires, and is shown marked `(redelivered)`. The
//! hook acknowledges right after its stdout write and flush succeeded, one more
//! request; "delivered" means written to the pipe, the best the hook can know, so
//! a hook killed after the write but before the acknowledgement, or a harness
//! that never reads what was written, delivers twice or not at all. Delivery is
//! at-least-once up to that point (FR-020, FR-040).
//!
//! # Seams
//!
//! [`Bus`] is everything the hook asks of Redis and herdr, [`Clock`] is time,
//! and stdin/stdout/stderr/the spool are plain parameters, so tests drive every
//! failure deterministically without any `cfg(test)` branch in this file.

use std::{
    fmt,
    io::{self, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    time::Duration,
};

use serde::Deserialize;

use relay_core::{
    AddressBook, AgentName, BatchId, BudgetPopResult, Claim, CommitOutcome, HookHealthEntry,
    IdentityCommit, MachineName, Message, Popper, Relay, SessionId, SessionProbe,
};

use crate::{
    budget::{Budget, Clock},
    hook::{self, Event, HOOK_BATCH, HookInput},
    identity::{self, Derived, HookIdentity, HookRequest, IdentityError},
    spool::{ErrorKind, Spool, SpoolRecord, Stage, Subject},
    stdin::{InputFault, InputSource},
};

/// The budget of a hook that never calls herdr (`UserPromptSubmit`), and the part
/// of the others' budgets that is Redis work. A dead tunnel must cost the agent a
/// fraction of a second, not three.
const HOOK_TIMEOUT: Duration = Duration::from_millis(300);

/// The most a hook spends reading stdin before it knows its event: the largest
/// budget of an event the harness kills at 2 s (`PostToolUse`, `Stop`).
const STDIN_TOTAL: Duration = Duration::from_millis(900);

/// The smallest pop window, for a Redis so close that the measured round trip
/// is about zero (loopback, a local container). Below this a scheduler hiccup
/// alone could outlast the timeout.
pub(crate) const POP_WINDOW_FLOOR: Duration = Duration::from_millis(30);

/// What [`pop_window`] adds on top of twice the measured round trip: the Lua
/// itself (at most [`HOOK_BATCH`] entries, well under a millisecond), the
/// server's own queueing, and the client's scheduling.
pub(crate) const POP_WINDOW_MARGIN: Duration = Duration::from_millis(20);

/// The least budget a hook must have left to pop the inbox, given the round trip
/// it measured: `max(`[`POP_WINDOW_FLOOR`]`, 2 * rtt + `[`POP_WINDOW_MARGIN`]`)`.
///
/// The pop is a single `EVAL`, so its reply takes about one round trip; since
/// AGE-2032 a reply that arrives late no longer loses mail (the batch is in
/// flight and its lease returns it). The window's second round trip is for the
/// acknowledgement the hook sends after it has written the output: a pop that
/// leaves no time to acknowledge would deliver, then deliver again 20 s later.
/// The margin covers everything that is not the network. It grows with
/// the measured latency (a tunnel at 42 ms asks for 104 ms, loopback for the
/// floor) instead of assuming one figure for every machine.
///
/// `rtt` is a request/response time, not a connect time. Connecting sends no
/// command (see [`relay_core::Relay::connect`]) and, through an SSH tunnel's
/// local listener, completes locally in about zero whatever the link costs, so
/// it says nothing about the link. The hook times a request of its own instead
/// (the session probe of a derived name, or the read of the lead's registration)
/// and takes the larger of that and the connect time (a connect that itself took
/// long, a slow accept, still counts). When it made neither, the pop gets at
/// least [`UNSAMPLED_POP_WINDOW`].
pub(crate) fn pop_window(rtt: Duration) -> Duration {
    rtt.saturating_mul(2)
        .saturating_add(POP_WINDOW_MARGIN)
        .max(POP_WINDOW_FLOOR)
}

/// The window a pop gets when the hook made no request of its own before it
/// (an explicit name at `PostToolUse`/`Stop`, or a derivation that read nothing),
/// so nothing measured the link. Connecting says nothing about a tunnel, so this
/// is a conservative guess instead of the floor: it covers a link up to about
/// 90 ms RTT, and a pop that does not fit is skipped and recorded, not issued.
pub(crate) const UNSAMPLED_POP_WINDOW: Duration = Duration::from_millis(200);

/// The least budget a hook must have left to ship (a chunk of) the spool: one
/// pipelined round trip, so a little over a normal one.
const FLUSH_MIN_WINDOW: Duration = Duration::from_millis(100);

/// Most records in one pipelined push, so one round trip stays small and each
/// chunk is checked against the remaining budget.
const FLUSH_CHUNK: usize = 200;

/// One deadline for an entire hook call, sized per event, and the link each
/// one is sized for. `rtt` is the round trip to Redis; a call makes several
/// sequential requests, then a pop that needs a window of `2 * rtt + 20 ms` (see
/// [`pop_window`]: the pop's own reply and the acknowledgement that follows it
/// after the write); a pop that does not fit is skipped, never issued short. What
/// follows the pop is the sender-address read, which needs a window of its own
/// left after the pop, and the acknowledgement.
///
/// - `UserPromptSubmit`: [`HOOK_TIMEOUT`] (300 ms). It never calls herdr (see
///   `identity::derive_at_user_prompt_submit`), and its mail also arrives at the
///   next `PostToolUse`/`Stop`, so a skipped pop is silent. A derived name makes 3
///   requests before the pop, so it pops and acknowledges up to `5 * rtt + 20 <=
///   300`, about 56 ms; the address read then also fits (`300 - 4 * rtt >= 2 *
///   rtt + 20`) up to about 46 ms, and between 46 and 56 ms the hook prints bare
///   bus names and still acknowledges. An explicit name makes 2 requests before
///   the pop: about 70 ms to pop, 56 ms for the address read. Over a slower link
///   it defers without recording.
/// - `PostToolUse`/`Stop`: [`identity::LAZY_HERDR_BUDGET`] + 300 ms (900 ms),
///   for hit and miss alike. These events exist to deliver mail, so the budget
///   must not be the thing that stops delivery: a cache hit makes 1 request
///   before the pop and fits it up to about 290 ms RTT (`3 * rtt + 20 <= 900`),
///   the address read up to about 220 ms (`900 - 2 * rtt >= 2 * rtt + 20`), and a
///   session-cache miss still has the herdr allowance before its pop. The price
///   is that a dead tunnel costs a tool call 900 ms.
/// - `SessionStart`: herdr's whole location budget plus [`HOOK_TIMEOUT`]
///   (3300 ms). It always asks herdr, once per session.
pub(crate) fn hook_deadline_budget(event: Event) -> Duration {
    match event {
        Event::SessionStart => identity::SESSION_START_HERDR_BUDGET + HOOK_TIMEOUT,
        Event::UserPromptSubmit => HOOK_TIMEOUT,
        Event::PostToolUse | Event::Stop => identity::LAZY_HERDR_BUDGET + HOOK_TIMEOUT,
    }
}

/// Everything the hook asks of Redis and herdr. [`RelayBus`] is the real one.
pub(crate) trait Bus {
    /// Bounds every later read and write to `timeout`.
    fn set_timeout(&mut self, timeout: Duration) -> relay_core::Result<()>;
    /// Where this pane last registered, under any name.
    /// What the session cache holds for `session_id`, in one request.
    fn probe_session(&mut self, session_id: &SessionId) -> relay_core::Result<SessionProbe>;
    /// Who this session is, given what [`Bus::probe_session`] found. Reads only;
    /// the writes it implies come back in the answer. Every herdr call it makes
    /// is bounded by `budget`.
    fn derive(
        &mut self,
        request: &HookRequest<'_>,
        probe: &SessionProbe,
        budget: &Budget<'_>,
    ) -> Result<HookIdentity, IdentityError>;
    /// Marks `session_id` as having no free name for a while. `true` when this
    /// call set the mark (the first failure), `false` when it was already set.
    /// Marks `session_id` as having had a herdr failure recorded for a while.
    /// `true` when this call set the mark (record it), `false` when it was
    /// already set. The mark limits diagnostics without stopping session verification.
    fn mark_herdr_failed(&mut self, session_id: &SessionId) -> relay_core::Result<bool>;
    /// The place `name` last registered.
    /// Every identity write of this hook call, as one atomic step.
    fn commit_identity(&mut self, commit: &IdentityCommit) -> relay_core::Result<CommitOutcome>;
    /// Takes `name`'s next batch of messages into flight, in one round trip.
    fn pop_inbox(&mut self, name: &AgentName) -> relay_core::Result<BudgetPopResult>;
    /// Acknowledges `batch`: its messages were delivered.
    fn ack(&mut self, name: &AgentName, batch: BatchId) -> relay_core::Result<bool>;
    /// Puts the in-flight `batch` back at the front of `name`'s inbox.
    fn requeue_batch(&mut self, name: &AgentName, batch: BatchId) -> relay_core::Result<usize>;
    /// Sender addresses for rendering. Never fails: a registry that cannot be
    /// read only costs the labels.
    fn address_book(&mut self) -> AddressBook;
    /// Appends spool records to the `health:hooks` stream.
    fn push_hook_health(&mut self, entries: &[HookHealthEntry]) -> relay_core::Result<()>;
}

/// The real [`Bus`]: a [`Relay`] connection and this machine's name.
pub(crate) struct RelayBus {
    relay: Relay,
    machine: MachineName,
}

impl RelayBus {
    /// Connects to `url` under `prefix`; `timeout` bounds the connect and every
    /// call until [`Bus::set_timeout`] narrows it.
    pub(crate) fn connect(
        url: &str,
        prefix: &str,
        machine: &MachineName,
        timeout: Duration,
    ) -> relay_core::Result<Self> {
        Ok(Self {
            relay: Relay::connect(url, timeout)?.with_prefix(prefix),
            machine: machine.clone(),
        })
    }
}

impl Bus for RelayBus {
    fn set_timeout(&mut self, timeout: Duration) -> relay_core::Result<()> {
        self.relay.set_timeout(timeout)
    }

    fn probe_session(&mut self, session_id: &SessionId) -> relay_core::Result<SessionProbe> {
        self.relay.probe_session(session_id)
    }

    fn derive(
        &mut self,
        request: &HookRequest<'_>,
        probe: &SessionProbe,
        budget: &Budget<'_>,
    ) -> Result<HookIdentity, IdentityError> {
        identity::derive_for_hook(&mut self.relay, &self.machine, request, probe, budget)
    }

    fn mark_herdr_failed(&mut self, session_id: &SessionId) -> relay_core::Result<bool> {
        self.relay.mark_session_herdr_failed(session_id)
    }

    fn commit_identity(&mut self, commit: &IdentityCommit) -> relay_core::Result<CommitOutcome> {
        self.relay.commit_identity(commit)
    }

    fn pop_inbox(&mut self, name: &AgentName) -> relay_core::Result<BudgetPopResult> {
        self.relay.pop_inbox_budget(
            name,
            HOOK_BATCH,
            hook::HOOK_BYTE_BUDGET,
            Claim::new(Popper::Hook),
        )
    }

    fn ack(&mut self, name: &AgentName, batch: BatchId) -> relay_core::Result<bool> {
        self.relay.ack(name, batch, relay_core::Popper::Hook)
    }

    fn requeue_batch(&mut self, name: &AgentName, batch: BatchId) -> relay_core::Result<usize> {
        self.relay
            .requeue_batch(name, batch, relay_core::Popper::Hook)
    }

    fn address_book(&mut self) -> AddressBook {
        crate::address_book(&mut self.relay)
    }

    fn push_hook_health(&mut self, entries: &[HookHealthEntry]) -> relay_core::Result<()> {
        self.relay.push_hook_health(entries).map(drop)
    }
}

/// What a hook call was invoked with, apart from stdin.
pub(crate) struct HookCtx<'a> {
    /// This machine's bus name.
    pub(crate) machine: &'a MachineName,
}

/// Everything that can fail in a hook call. One value is one failure, and one
/// failure is one spool record ([`HookError::stage`], [`HookError::kind`]).
#[derive(Debug, thiserror::Error)]
pub(crate) enum HookError {
    /// Stdin could not be read.
    #[error("could not read the hook input: {0}")]
    ReadInput(#[source] io::Error),
    /// More than [`crate::stdin::MAX_INPUT_BYTES`] arrived on stdin.
    #[error(
        "the hook input is larger than {} bytes",
        crate::stdin::MAX_INPUT_BYTES
    )]
    InputTooLarge,
    /// Stdin neither closed nor completed a payload before the deadline; nothing
    /// was registered or popped (NFR-001-M-5).
    #[error("the hook input did not arrive before the deadline")]
    InputTimeout,
    /// Stdin was not the hook JSON.
    #[error("the hook input is not valid JSON: {0}")]
    ParseInput(#[source] serde_json::Error),
    /// The budget was spent before `step`.
    #[error("hook time budget exhausted before {step}")]
    BudgetExhausted {
        /// What would have run next.
        step: &'static str,
    },
    /// Connecting to Redis failed.
    #[error("could not connect to the bus: {0}")]
    Connect(#[source] relay_core::Error),
    /// Working out who this session is failed.
    #[error("could not derive this session's name: {0}")]
    Derive(#[source] relay_core::Error),
    /// The hook runs in a herdr pane and herdr failed (an error, or a call that
    /// timed out) before it could say who the session is, and nothing else named
    /// it. Nothing was registered.
    #[error("herdr could not say who this session is; not registering")]
    HerdrFailed,
    /// Reading or writing the registration, aliases or session cache failed.
    #[error("could not register: {0}")]
    Register(#[source] relay_core::Error),
    /// Popping the inbox failed. Messages may have been removed.
    #[error("could not pop the inbox: {0}")]
    Pop(#[source] relay_core::Error),
    /// The output could not be encoded.
    #[error("could not encode the hook output: {0}")]
    Encode(#[source] serde_json::Error),
    /// The output could not be written; the messages were put back.
    #[error("could not write the hook output: {0}")]
    Output(#[source] io::Error),
    /// The output could not be written and the messages could not be put back.
    /// They are still in flight, and return to the inbox when the lease
    /// expires; the spool keeps a copy too.
    #[error(
        "could not put {} popped message(s) back ({detail}); left in flight until the lease \
         expires, and kept in the spool",
        .messages.len()
    )]
    RequeueFailed {
        /// Why the requeue failed, as a code.
        kind: ErrorKind,
        /// Why, for stderr only.
        detail: String,
        /// A copy of the messages that are left in flight.
        messages: Vec<Message>,
    },
    /// The output was written but the acknowledgement did not complete: it may or
    /// may not have been applied, so the batch may be delivered again, marked,
    /// when its lease expires.
    #[error(
        "the batch was written but its acknowledgement did not complete ({detail}); \
         it may be redelivered"
    )]
    AckFailed {
        /// Why, as a code.
        kind: ErrorKind,
        /// Why, for stderr only.
        detail: String,
    },
    /// The output was written but the batch was no longer in flight when it was
    /// acknowledged: its lease had run out and the reap returned it, so the
    /// messages were, or are about to be, delivered again.
    #[error("the batch was no longer in flight when acknowledged; its messages are redelivered")]
    AckStale,
    /// A pop took nothing because the lead has too much in flight.
    #[error(
        "{pending} message(s) wait: the lead's in-flight cap is reached until a lease expires or a batch is acknowledged"
    )]
    InflightFull {
        /// How many messages were waiting.
        pending: usize,
    },
    /// Rows whose redeliveries are spent could not be dead-lettered.
    #[error("{stuck} message(s) could not be dead-lettered and stay in flight")]
    DeadLetterUnusable {
        /// How many rows.
        stuck: usize,
    },
    /// The hook panicked.
    #[error("the hook panicked")]
    Panicked,
}

impl HookError {
    /// Where this failed.
    pub(crate) fn stage(&self) -> Stage {
        match self {
            Self::ReadInput(_) | Self::InputTooLarge | Self::InputTimeout | Self::ParseInput(_) => {
                Stage::Decode
            }
            Self::BudgetExhausted { .. } => Stage::Budget,
            Self::Connect(_) => Stage::Connect,
            Self::Derive(_) | Self::HerdrFailed => Stage::Derive,
            Self::Register(_) => Stage::Register,
            Self::Pop(_) => Stage::Pop,
            Self::Encode(_) | Self::Output(_) => Stage::Output,
            Self::RequeueFailed { .. } => Stage::RequeueFailed,
            Self::AckFailed { .. } | Self::AckStale => Stage::Ack,
            Self::InflightFull { .. } | Self::DeadLetterUnusable { .. } => Stage::Pop,
            Self::Panicked => Stage::Panic,
        }
    }

    /// Why, as a stable code.
    pub(crate) fn kind(&self) -> ErrorKind {
        match self {
            Self::ReadInput(_) | Self::Output(_) => ErrorKind::Io,
            Self::ParseInput(_) | Self::Encode(_) => ErrorKind::Malformed,
            Self::InputTooLarge => ErrorKind::TooLarge,
            Self::InputTimeout => ErrorKind::StdinTimeout,
            Self::BudgetExhausted { .. } => ErrorKind::BudgetExhausted,
            Self::Connect(e) | Self::Derive(e) | Self::Register(e) | Self::Pop(e) => {
                ErrorKind::of(e)
            }

            Self::HerdrFailed => ErrorKind::HerdrFailed,
            Self::RequeueFailed { kind, .. } | Self::AckFailed { kind, .. } => *kind,
            Self::AckStale => ErrorKind::LeaseExpired,
            Self::InflightFull { .. } => ErrorKind::InflightFull,
            Self::DeadLetterUnusable { .. } => ErrorKind::Corrupt,
            Self::Panicked => ErrorKind::Panic,
        }
    }

    /// The derived name that collided, for the failure that has one.
    fn candidate(&self) -> Option<&AgentName> {
        match self {
            Self::ReadInput(_)
            | Self::ParseInput(_)
            | Self::InputTooLarge
            | Self::InputTimeout
            | Self::BudgetExhausted { .. }
            | Self::HerdrFailed
            | Self::Connect(_)
            | Self::Derive(_)
            | Self::Register(_)
            | Self::Pop(_)
            | Self::Encode(_)
            | Self::Output(_)
            | Self::RequeueFailed { .. }
            | Self::AckFailed { .. }
            | Self::AckStale
            | Self::InflightFull { .. }
            | Self::DeadLetterUnusable { .. }
            | Self::Panicked => None,
        }
    }

    fn messages(&self) -> &[Message] {
        match self {
            Self::RequeueFailed { messages, .. } => messages,
            Self::ReadInput(_)
            | Self::ParseInput(_)
            | Self::InputTooLarge
            | Self::InputTimeout
            | Self::BudgetExhausted { .. }
            | Self::HerdrFailed
            | Self::Connect(_)
            | Self::Derive(_)
            | Self::Register(_)
            | Self::Pop(_)
            | Self::Encode(_)
            | Self::Output(_)
            | Self::AckFailed { .. }
            | Self::AckStale
            | Self::InflightFull { .. }
            | Self::DeadLetterUnusable { .. }
            | Self::Panicked => &[],
        }
    }

    /// Whether this fault repeats on every call and happens before the hook
    /// could connect, so it can never rely on a later healthy hook of its own
    /// to ship the spool.
    fn is_persistent_input_fault(&self) -> bool {
        matches!(
            self,
            Self::ReadInput(_) | Self::InputTooLarge | Self::ParseInput(_)
        )
    }
}

/// Where failures go, and the context that makes a record useful.
struct Recorder<'a> {
    spool: Option<&'a Spool>,
    clock: &'a dyn Clock,
    subject: Subject,
    err: &'a mut dyn Write,
}

impl Recorder<'_> {
    /// One line on stderr. A closed or full stderr is ignored: reporting a
    /// failure must not become one.
    fn say(&mut self, args: fmt::Arguments<'_>) {
        let _ = writeln!(self.err, "{args}");
    }

    /// Records `error`: to the spool first (it is the durable copy, and holds
    /// the messages of a failed requeue), then to stderr. Neither can fail the
    /// hook.
    fn record(&mut self, error: &HookError) {
        self.write_spool(
            error.stage(),
            error.kind(),
            error.candidate(),
            error.messages(),
        );
        self.say(format_args!("agent-msg hook: {error}"));
    }

    /// Records a failure that does not end the hook (an inbox entry that would
    /// not decode, a refused demotion), with `text` for stderr.
    fn note(&mut self, stage: Stage, kind: ErrorKind, text: fmt::Arguments<'_>) {
        self.write_spool(stage, kind, None, &[]);
        self.say(text);
    }

    fn write_spool(
        &mut self,
        stage: Stage,
        kind: ErrorKind,
        candidate: Option<&AgentName>,
        messages: &[Message],
    ) {
        let Some(spool) = self.spool else { return };
        let mut record = SpoolRecord::new(self.clock.now_ms(), &self.subject, stage, kind);
        record.candidate = candidate.map(|name| name.as_str().to_owned());
        record.messages = messages.to_vec();
        if let Err(e) = spool.record(&record) {
            self.say(format_args!(
                "agent-msg hook: could not record the failure in the spool: {e}"
            ));
        }
    }
}

/// Records a failure that happens before a hook call exists (its arguments or
/// its config did not load), so it has no event, session or lead.
pub(crate) fn record_early(
    spool: Option<&Spool>,
    clock: &dyn Clock,
    stage: Stage,
    kind: ErrorKind,
) {
    let Some(spool) = spool else { return };
    // A host whose name is no usable machine name cannot register a lead, so
    // there is nothing to attribute the failure to.
    let Ok(machine) = relay_core::machine_name() else {
        return;
    };
    let subject = Subject::on(machine);
    let record = SpoolRecord::new(clock.now_ms(), &subject, stage, kind);
    if let Err(e) = spool.record(&record) {
        crate::report::line(format_args!(
            "agent-msg hook: could not record the failure in the spool: {e}"
        ));
    }
}

/// Runs one hook call. Never fails and never panics: every failure, a panic
/// included, is recorded (see the module docs) and the caller exits 0.
///
/// `connect` opens the bus with the timeout it is given (what is left of the
/// budget).
pub(crate) fn run<B: Bus>(
    _cx: &HookCtx<'_>,
    clock: &dyn Clock,
    spool: Option<&Spool>,
    (stdin, out, err): (&mut dyn InputSource, &mut dyn Write, &mut dyn Write),
    connect: &mut dyn FnMut(Duration) -> relay_core::Result<B>,
) {
    let mut recorder = Recorder {
        spool,
        clock,
        subject: Subject::on(_cx.machine.clone()),
        err,
    };
    // The deadline of the whole call starts here, before stdin is read.
    let started = clock.elapsed();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        try_run(
            _cx,
            (clock, started),
            &mut recorder,
            stdin,
            out,
            &mut *connect,
        )
    }));
    let error = match outcome {
        Ok(Ok(())) => return,
        Ok(Err(error)) => error,
        Err(_) => HookError::Panicked,
    };
    recorder.record(&error);
    if error.is_persistent_input_fault() {
        // Best effort, and never a second failure: the fault is already recorded.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            flush_after_failure(&mut recorder, (clock, started), &mut *connect);
        }));
    }
}

/// The first JSON value in `raw` as a hook payload. Bytes after it (a second
/// value, garbage, a NUL) are ignored: the read stops at the end of the first
/// object, so they may or may not have arrived. Invalid UTF-8 inside a string,
/// a value that is not an object, and an object that is not a hook payload are
/// all `malformed`.
pub(crate) fn decode_input(raw: &[u8]) -> Result<HookInput, serde_json::Error> {
    HookInput::deserialize(&mut serde_json::Deserializer::from_slice(raw))
}

fn try_run<B: Bus>(
    _cx: &HookCtx<'_>,
    (clock, started): (&dyn Clock, Duration),
    rec: &mut Recorder<'_>,
    stdin: &mut dyn InputSource,
    out: &mut dyn Write,
    connect: &mut dyn FnMut(Duration) -> relay_core::Result<B>,
) -> Result<(), HookError> {
    // The deadline started before stdin was read (NFR-001-M-5). The event is not
    // known until the payload is, so the read is bounded by 300 ms without
    // bytes (the idle window, which restarts as bytes arrive) and in total by the
    // largest budget of an event the harness kills at 2 s (900 ms), so a longer
    // wait could not leave a record. Both harnesses write the payload at once and
    // close stdin. The event's own budget then counts from the same start, so a
    // payload that arrived late leaves the event what is left of its budget.
    let read_budget = Budget::starting_at(clock, started, STDIN_TOTAL);
    let raw = stdin
        .read_input(&read_budget)
        .map_err(|fault| match fault {
            InputFault::Io(e) => HookError::ReadInput(e),
            InputFault::TooLarge => HookError::InputTooLarge,
            InputFault::Timeout => HookError::InputTimeout,
        })?;
    let input = decode_input(&raw).map_err(HookError::ParseInput)?;
    rec.subject.session_id.clone_from(&input.session_id);
    let Some(event) = Event::parse(&input.hook_event_name) else {
        return Ok(());
    };
    rec.subject.event = Some(event);
    if input.agent_id.is_some() {
        return Ok(());
    }

    // One budget for this *entire* hook call — connect, an optional identity
    // derivation, an optional register, and the pop each get whatever is left
    // of it (AGE-2018 PR review, finding 6). See [`hook_deadline_budget`] for
    // how it is sized per event.
    let budget = Budget::starting_at(clock, started, hook_deadline_budget(event));
    let connect_timeout = budget
        .remaining()
        .ok_or(HookError::BudgetExhausted { step: "connecting" })?;
    let connect_started = clock.elapsed();
    let mut bus = connect(connect_timeout).map_err(HookError::Connect)?;
    // Connecting sends no command, so this is only the TCP connect: a floor on
    // the round trip, not a measure of it. The first Redis request below adds one.
    let mut rtt = clock.elapsed().saturating_sub(connect_started);
    // Whether a request of the hook's own has been timed: without one `rtt` is
    // only the connect, which through a tunnel measures nothing.
    let mut sampled = false;

    let identity = {
        arm(&mut bus, &budget, "reading the session", HookError::Derive)?;
        // One request for every independent read about the session (suppressed,
        // cached queue). It is also the round trip the pop window
        // is sized from (see [`pop_window`]).
        let step_started = clock.elapsed();
        let probe = match input.session_id.as_ref() {
            Some(sid) => bus.probe_session(sid).map_err(HookError::Derive)?,
            None => SessionProbe::default(),
        };
        if input.session_id.is_some() {
            rtt = rtt.max(clock.elapsed().saturating_sub(step_started));
            sampled = true;
        }
        arm(
            &mut bus,
            &budget,
            "deriving the bus name",
            HookError::Derive,
        )?;
        let request = HookRequest {
            session_id: input.session_id.as_ref(),
            source: input.source.as_deref(),
            event,
        };
        match bus.derive(&request, &probe, &budget) {
            Ok(identity) => identity,
            Err(IdentityError::Relay(e)) => return Err(HookError::Derive(e)),
        }
    };
    let Derived { name, commit } = match identity {
        HookIdentity::NotALead => return Ok(()),
        HookIdentity::Suppress { session_id } => {
            if let Some(session_id) = session_id {
                let commit = IdentityCommit {
                    suppress_session: Some(session_id),
                    ..IdentityCommit::default()
                };
                apply(&mut bus, &budget, &commit, rec)?;
            }
            return Ok(());
        }
        HookIdentity::Lead(derived) => *derived,
        // In a herdr pane, but herdr could not name the session: a failure, so
        // it leaves a record (FR-030) instead of passing for a non-lead. Once
        // per session per window: the derivation is retried on every call.
        HookIdentity::HerdrFailed { budget_spent } => {
            return herdr_failed(&mut bus, &budget, input.session_id.as_ref(), budget_spent);
        }
    };
    rec.subject.lead = Some(name.as_str().to_owned());

    // Every identity write — registration, alias demotions, session cache — is
    // this one step, made only after everything that reads and waits is done,
    // so a call that ran out of time before here has written nothing.
    if commit != IdentityCommit::default() {
        apply(&mut bus, &budget, &commit, rec)?;
    }

    let window = if sampled {
        pop_window(rtt)
    } else {
        pop_window(rtt).max(UNSAMPLED_POP_WINDOW)
    };
    let Some(left) = budget.remaining().filter(|left| *left >= window) else {
        // A `UserPromptSubmit` deferral is expected and free: the mail is
        // delivered at the next `PostToolUse` or `Stop`, and recording it would
        // flood the health stream and cost every later hook a flush. Anywhere
        // else delivery really is delayed, so it is recorded.
        return match event {
            Event::UserPromptSubmit => Ok(()),
            Event::SessionStart | Event::PostToolUse | Event::Stop => {
                Err(HookError::BudgetExhausted {
                    step: "popping the inbox",
                })
            }
        };
    };
    bus.set_timeout(left).map_err(HookError::Pop)?;
    let popped = bus.pop_inbox(&name).map_err(HookError::Pop)?;
    for e in &popped.rejected {
        rec.note(
            Stage::Decode,
            ErrorKind::Corrupt,
            format_args!("agent-msg hook: dropped one inbox entry: {e}"),
        );
    }

    if popped.at_cap && event != Event::UserPromptSubmit {
        // Nothing was popped, and the mail is waiting: delayed, so recorded (a
        // skipped `UserPromptSubmit` pop is an expected, silent deferral).
        rec.record(&HookError::InflightFull {
            pending: popped.pending,
        });
    }
    if popped.stuck > 0 {
        rec.record(&HookError::DeadLetterUnusable {
            stuck: popped.stuck,
        });
    }

    // The messages are in flight, not gone: from here a failure that is known
    // puts the batch back (see `recover`), and any other end leaves it for the
    // lease to return. The sender addresses cost a round trip and the
    // acknowledgement after the write costs another, so the addresses are read
    // only when both fit in what is left (`window` is two round trips and a
    // margin); short of that bare bus names are printed, which costs a label,
    // where a missed acknowledgement costs a duplicate. Nothing between the write
    // and the acknowledgement needs the address book.
    let batch = popped.batch;
    let book = if popped.messages.is_empty()
        || !budget
            .remaining()
            .filter(|left| *left >= window)
            .is_some_and(|left| bus.set_timeout(left).is_ok())
    {
        AddressBook::default()
    } else {
        bus.address_book()
    };
    if let Some(output) = hook::respond(
        event,
        &popped.messages,
        &popped.redelivered,
        popped.pending,
        &book,
    ) {
        let bytes = match serde_json::to_vec(&output) {
            Ok(bytes) => bytes,
            Err(e) => {
                return Err(recover(
                    &mut bus,
                    &budget,
                    &name,
                    batch,
                    popped.messages,
                    HookError::Encode(e),
                ));
            }
        };
        if let Err(e) = out.write_all(&bytes).and_then(|()| out.flush()) {
            return Err(recover(
                &mut bus,
                &budget,
                &name,
                batch,
                popped.messages,
                HookError::Output(e),
            ));
        }
    }
    if let Some(batch) = batch {
        acknowledge(&mut bus, &budget, &name, batch, rec);
    }

    flush_health(&mut bus, &budget, rec);
    Ok(())
}

/// herdr could not name the session. The first failure of a session in a window
/// is returned as the failure to record (`budget` when a herdr call was refused
/// for want of time, else `derive`/`herdr_failed`); later ones are silent while
/// the mark stands. The mark limits only the record: nothing stops the next call
/// deriving again, so the lead registers as soon as herdr answers. When the mark
/// cannot be set (no time, or Redis fails) the failure is recorded, as before.
fn herdr_failed<B: Bus>(
    bus: &mut B,
    budget: &Budget<'_>,
    session_id: Option<&SessionId>,
    budget_spent: bool,
) -> Result<(), HookError> {
    let first = session_id.is_none_or(|sid| {
        arm(bus, budget, "marking the session", HookError::Derive).is_err()
            || bus.mark_herdr_failed(sid).unwrap_or(true)
    });
    match (first, budget_spent) {
        (false, _) => Ok(()),
        (true, true) => Err(HookError::BudgetExhausted {
            step: "asking herdr",
        }),
        (true, false) => Err(HookError::HerdrFailed),
    }
}

/// Bounds the bus to what is left of `budget`, or fails with the budget error
/// for `step`. A failure to set the timeout is `stage`'s failure.
fn arm<B: Bus>(
    bus: &mut B,
    budget: &Budget<'_>,
    step: &'static str,
    stage: fn(relay_core::Error) -> HookError,
) -> Result<(), HookError> {
    let left = budget
        .remaining()
        .ok_or(HookError::BudgetExhausted { step })?;
    bus.set_timeout(left).map_err(stage)
}

/// Applies `commit` inside the budget. A demotion the bus refused is recorded
/// (it is why an old name does not reach its owner), but does not fail the hook.
fn apply<B: Bus>(
    bus: &mut B,
    budget: &Budget<'_>,
    commit: &IdentityCommit,
    _rec: &mut Recorder<'_>,
) -> Result<(), HookError> {
    arm(bus, budget, "writing the registration", HookError::Register)?;
    bus.commit_identity(commit).map_err(HookError::Register)?;
    Ok(())
}

/// Acknowledges a delivered batch: the very next thing a hook does after its
/// output was written and flushed ("delivered" means written to the pipe: what
/// the harness then does with the bytes is beyond the hook). It needs nothing but
/// the batch id. Three ways it does not settle the batch, each recorded as what
/// it was: no time left or a request that did not complete (the acknowledgement
/// may or may not have been applied, so the batch may be redelivered), and a
/// batch that is no longer in flight (its lease ran out first: the messages are,
/// or will be, delivered again).
fn acknowledge<B: Bus>(
    bus: &mut B,
    budget: &Budget<'_>,
    name: &AgentName,
    batch: BatchId,
    rec: &mut Recorder<'_>,
) {
    let outcome = match budget.remaining() {
        None => Err(HookError::AckFailed {
            kind: ErrorKind::BudgetExhausted,
            detail: "no time left".to_owned(),
        }),
        Some(left) => match bus.set_timeout(left).and_then(|()| bus.ack(name, batch)) {
            Ok(true) => Ok(()),
            Ok(false) => Err(HookError::AckStale),
            Err(e) => Err(HookError::AckFailed {
                kind: ErrorKind::of(&e),
                detail: e.to_string(),
            }),
        },
    };
    if let Err(error) = outcome {
        rec.record(&error);
    }
}

/// Puts the in-flight `batch` of popped `messages` back after `cause` stopped
/// them being delivered, and returns what to record: `cause` when they are safe
/// in the inbox again, [`HookError::RequeueFailed`] — carrying a copy of the
/// messages, and `cause`'s text — when the put-back failed: they are then still
/// in flight, and the lease returns them. A requeue too is bounded by the
/// budget: with none left it is not attempted.
fn recover<B: Bus>(
    bus: &mut B,
    budget: &Budget<'_>,
    name: &AgentName,
    batch: Option<BatchId>,
    messages: Vec<Message>,
    cause: HookError,
) -> HookError {
    let requeued = match (batch, budget.remaining()) {
        (None, _) => Ok(()),
        (Some(_), None) => Err((ErrorKind::BudgetExhausted, "no time left".to_owned())),
        (Some(batch), Some(left)) => bus
            .set_timeout(left)
            .and_then(|()| bus.requeue_batch(name, batch).map(drop))
            .map_err(|e| (ErrorKind::of(&e), e.to_string())),
    };
    match requeued {
        Ok(()) => cause,
        Err((kind, detail)) => HookError::RequeueFailed {
            kind,
            detail: format!("{cause}; requeue: {detail}"),
            messages,
        },
    }
}

/// Ships the local spool to `health:hooks` when there is one and this hook has
/// time, in chunks that are each checked against the remaining budget, then
/// removes exactly what was shipped. A failure leaves what is unshipped in the
/// spool and is only logged: not recording it keeps a Redis outage from growing
/// the very file it cannot ship.
///
/// Each record is shipped at most once (FR-030): it carries an id minted when it was
/// appended, each chunk is one atomic request that adds only the records whose ids the bus
/// has not recorded as shipped, and the spool removes what was shipped by id. A crash
/// between a push and the removal, or a retry after a push that failed after the server
/// applied it, offers the same ids again and the bus skips them.
fn flush_health<B: Bus>(bus: &mut B, budget: &Budget<'_>, rec: &mut Recorder<'_>) {
    let Some(spool) = rec.spool else { return };
    if !spool.has_records()
        || budget
            .remaining()
            .is_none_or(|left| left < FLUSH_MIN_WINDOW)
    {
        return;
    }
    let flush = match spool.begin_flush() {
        Ok(Some(flush)) => flush,
        Ok(None) => return,
        Err(e) => {
            rec.say(format_args!(
                "agent-msg hook: could not read the spool: {e}"
            ));
            return;
        }
    };
    let mut shipped = std::collections::HashSet::new();
    for chunk in flush.entries().chunks(FLUSH_CHUNK) {
        let Some(left) = budget.remaining().filter(|left| *left >= FLUSH_MIN_WINDOW) else {
            break;
        };
        let batch: Vec<HookHealthEntry> = chunk
            .iter()
            .map(|entry| HookHealthEntry {
                id: Some(entry.id.clone()),
                machine: entry.machine.clone(),
                lead: entry.lead.clone(),
                record: entry.json.clone(),
            })
            .collect();
        if let Err(e) = bus
            .set_timeout(left)
            .and_then(|()| bus.push_hook_health(&batch))
        {
            rec.say(format_args!(
                "agent-msg hook: could not ship the spool, keeping the rest: {e}"
            ));
            break;
        }
        shipped.extend(chunk.iter().map(|entry| entry.id.clone()));
    }
    match flush.finish(&shipped) {
        Ok(0) => {}
        // Lines that were not records (an id outside its type's rule, or bytes no
        // binary wrote) went with the shipped ones: say so in the spool, which the
        // next flush ships, so doctor sees a count instead of a silent deletion.
        Ok(removed) => {
            let mut lost = SpoolRecord::new(
                rec.clock.now_ms(),
                &rec.subject,
                Stage::Decode,
                ErrorKind::Corrupt,
            );
            lost.dropped = u64::try_from(removed).unwrap_or(u64::MAX);
            if let Err(e) = spool.record(&lost) {
                rec.say(format_args!(
                    "agent-msg hook: could not record {removed} unreadable spool line(s): {e}"
                ));
            }
        }
        Err(e) => rec.say(format_args!(
            "agent-msg hook: could not update the spool after shipping it: {e}"
        )),
    }
}

/// A hook that failed for a reason that repeats on every call and before it
/// could connect has no later healthy hook of its own, so it connects once,
/// within [`HOOK_TIMEOUT`] of the start of the call, to ship the spool (its own
/// record included).
fn flush_after_failure<B: Bus>(
    rec: &mut Recorder<'_>,
    (clock, started): (&dyn Clock, Duration),
    connect: &mut dyn FnMut(Duration) -> relay_core::Result<B>,
) {
    // Counted from the start of the call, not a fresh 300 ms: a call that spent
    // its time reading stdin has that much less to ship the spool, and never
    // runs past the smallest event budget.
    let budget = Budget::starting_at(clock, started, HOOK_TIMEOUT);
    let Some(spool) = rec.spool else { return };
    if !spool.has_records() {
        return;
    }
    let Some(timeout) = budget.remaining() else {
        return;
    };
    // A refused connection is not a second failure: the spool keeps the record.
    if let Ok(mut bus) = connect(timeout) {
        flush_health(&mut bus, &budget, rec);
    }
}

#[cfg(test)]
mod tests;
