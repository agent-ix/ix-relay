//! [`SocketDeliverer`]: posts a batch to a lead's Claude Code inbox socket and
//! decides, from what comes back, what the caller may do about the batch.
//!
//! The outcome is one of the four the watcher acts on:
//!
//! - [`SocketError::NotSent`]: certain nothing was delivered (no socket, refused
//!   connection, a receipt saying refused or dropped). The caller falls back to
//!   herdr.
//! - [`SocketError::Held`]: Claude Code holds it behind an approval prompt.
//!   Sending it again, by any transport, would duplicate it.
//! - [`SocketError::Deferred`]: not tried, because the lead's mail is held. It goes
//!   back to the inbox unmarked.
//! - [`SocketError::Ambiguous`]: it may have arrived. The batch stays in flight
//!   and is redelivered, marked, when its lease expires (FR-020); that redelivery
//!   goes through herdr, because the socket has shown it cannot say.
//!
//! `Ok` is returned only when the batch's `msg_id` is in the lead's transcript.
//!
//! A receipt can arrive long after a delivery returned (a held message the user
//! approves or denies minutes later). A thread keeps the reply socket for the life
//! of the deliverer and queues what it hears; [`SocketDeliverer::settle`] routes
//! each receipt, by the message id it names, to the batch it is about.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use claude_wire::{
    ControlLine, Envelope, Receipt, TranscriptMatch, Verdict, parse_control_line, wrap,
};
use relay_core::{
    BatchId,
    socket_status::{
        self, DeadLetterNote, HeldBatch, LeadSocket, OwedSettlement, SocketStatus, WireDrift,
    },
    text::sanitise_line,
};

use super::{
    session::{self, Procs},
    transcript::{self, Watch},
};
use crate::deliver::{BatchTarget, DeliverError, RegisteredLead, SettleAction, Settlement};

/// How long a delivery waits, after posting, for a receipt or the transcript to
/// show the message. An idle session records it within a second or two; what it
/// has not shown by then is carried as pending state and settled by later ticks.
pub const DEFAULT_ACK_WAIT: Duration = Duration::from_secs(3);

/// Claude Code drops an identical body from the same sender within 30 seconds;
/// one body is never posted twice inside this (30 s and a margin).
pub const DEDUPE_WINDOW: Duration = Duration::from_secs(35);

/// How long a lead whose mail is held is left alone before the socket is tried
/// again: each retry would add another held message behind the same prompt.
pub const HELD_QUIET: Duration = Duration::from_secs(10 * 60);

/// How long a lead whose socket delivery was ambiguous, or was refused after the
/// fact, goes through herdr before the socket is tried again.
pub const SUSPECT_FOR: Duration = Duration::from_secs(10 * 60);

/// How long a connect may take. A session that has stopped accepting fills its
/// socket's backlog, and a connect to it then blocks without end.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// The whole deadline for writing one line to the socket.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// The wait between looks at the transcript starts here and doubles.
const POLL_START: Duration = Duration::from_millis(25);

/// The longest wait between looks at the transcript.
const POLL_MAX: Duration = Duration::from_millis(200);

/// How long the receipt thread sleeps when nothing is waiting on the socket.
const RECEIVER_IDLE: Duration = Duration::from_millis(20);

/// How long one receipt connection may take to send its line.
const RECEIPT_READ_TIMEOUT: Duration = Duration::from_millis(100);

/// The most bytes read from one receipt connection.
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;

/// The most receipts queued between settlements; the oldest is dropped past it.
const MAX_QUEUED_RECEIPTS: usize = 1024;

/// The most sends whose outcome is still open. The oldest is forgotten past it.
const MAX_PENDING: usize = 256;

/// How long a send's outcome is kept open.
const PENDING_FOR: Duration = Duration::from_secs(60 * 60);

/// How long a probe connect to a socket that may be live may take.
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// The first wait before the reply socket is bound again, doubling to the last.
const REBIND_START: Duration = Duration::from_secs(2);

/// The longest wait before the reply socket is bound again.
const REBIND_MAX: Duration = Duration::from_secs(5 * 60);

/// What an unconfirmed socket delivery may be down to, for the note `doctor` shows.
const AMBIGUOUS_CAUSES: &str = "the cause may be a transcript under another CLAUDE_CONFIG_DIR, \
    a socket that wants authentication, a session that was cleared or busy, or a changed \
    wire format";

/// How long a held batch's lease is extended by each renewal. It must outlast the
/// longest gap between two ticks (the sweep is every 15 s), so a batch held behind an
/// approval prompt is never returned to the inbox, or popped by the lead's own hook,
/// while Claude Code still holds the message.
pub const HELD_RENEW_LEASE: Duration = Duration::from_secs(120);

/// The longest a batch is kept in flight for an approval that never comes. At the
/// end of it the batch is dead-lettered (reason `hold_expired`), never posted again.
pub const MAX_HOLD: Duration = Duration::from_secs(30 * 60);

/// The most time one tick spends on sockets, across all leads: what is spent (resolving,
/// connecting, writing and waiting, and the settlement's transcript checks) plus the worst
/// case of a post (a connect, a write and a whole ack wait, 7 s) never passes it, so its
/// worst case is a little over the budget only for what is not counted (the directory scan
/// that finds a transcript). A batch is posted only when what is left of it covers that; otherwise it is deferred to the
/// next tick, unposted, so many wedged leads cannot hold the tick past the heartbeat's
/// expiry and a budget cut is never mistaken for a receiver that failed to confirm.
pub const TICK_BUDGET: Duration = Duration::from_secs(8);

// One place for the timing ratio: a batch is posted only when the tick's remaining
// budget covers a whole ack wait, so the budget must hold at least one.
const _: () = assert!(
    (DEFAULT_ACK_WAIT.as_millis() + CONNECT_TIMEOUT.as_millis() + WRITE_TIMEOUT.as_millis())
        <= TICK_BUDGET.as_millis()
);

/// How long, after Claude Code reports a message delivered, the transcript is waited
/// for. When it never shows the message the batch is acknowledged anyway (Claude Code
/// itself reported delivery) and the doctor is told (`delivered_unconfirmed`).
pub const DELIVERED_GRACE: Duration = Duration::from_secs(5 * 60);

/// How long the reason for a dead letter the watcher made is kept in its status.
pub const DEAD_NOTE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// How many failed writes of one settlement raise the alert (a doctor note and one
/// notification). The settlement is never given up on: it is kept and its batch renewed.
const SETTLE_ALERT_AFTER: u32 = 5;

/// The wait after a first failed settlement write; it doubles per failure.
pub const SETTLE_BACKOFF: Duration = Duration::from_secs(5);

/// The longest wait between two tries of a failing settlement write.
const SETTLE_BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);

/// The most open sends whose transcript is looked at per settlement, taken in turn.
const TRANSCRIPT_CHECKS_PER_SETTLE: usize = 32;

/// The longest sender label put in `from-name`, in characters.
const MAX_FROM_NAME_CHARS: usize = 200;

/// Where everything lives and how long things take.
#[derive(Debug, Clone)]
pub struct SocketConfig {
    /// Claude Code's session registry (`~/.claude/sessions`), read only.
    pub sessions_dir: PathBuf,
    /// Claude Code's transcripts (`~/.claude/projects`), read only.
    pub projects_dir: PathBuf,
    /// Where Claude Code's sockets live (`$XDG_RUNTIME_DIR/cc-socks`); the
    /// watcher's own reply socket is bound here.
    pub socket_dir: PathBuf,
    /// Where the status `doctor` reads is written (one file per watcher), `None`
    /// for nowhere.
    pub status_dir: Option<PathBuf>,
    /// See [`DEFAULT_ACK_WAIT`].
    pub ack_wait: Duration,
    /// See [`DEDUPE_WINDOW`].
    pub dedupe_window: Duration,
    /// See [`HELD_QUIET`].
    pub held_quiet: Duration,
    /// See [`SUSPECT_FOR`].
    pub suspect_for: Duration,
    /// See [`REBIND_START`].
    pub rebind_start: Duration,
    /// See [`MAX_HOLD`].
    pub max_hold: Duration,
    /// See [`TICK_BUDGET`].
    pub tick_budget: Duration,
    /// See [`DELIVERED_GRACE`].
    pub delivered_grace: Duration,
    /// See [`DEAD_NOTE_AGE`].
    pub dead_note_age: Duration,
    /// How long after a failed settlement write the next try waits; doubles per failure, up to five minutes.
    pub settle_backoff: Duration,
}

impl SocketConfig {
    /// The most one post can take: a connect, a write and a whole ack wait.
    #[must_use]
    pub fn post_worst_case(&self) -> Duration {
        CONNECT_TIMEOUT + WRITE_TIMEOUT + self.ack_wait
    }

    /// These locations with the production timings (the constants of this module).
    #[must_use]
    pub fn with_dirs(
        sessions_dir: PathBuf,
        projects_dir: PathBuf,
        socket_dir: PathBuf,
        status_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            sessions_dir,
            projects_dir,
            socket_dir,
            status_dir,
            ack_wait: DEFAULT_ACK_WAIT,
            dedupe_window: DEDUPE_WINDOW,
            held_quiet: HELD_QUIET,
            suspect_for: SUSPECT_FOR,
            rebind_start: REBIND_START,
            max_hold: MAX_HOLD,
            tick_budget: TICK_BUDGET,
            delivered_grace: DELIVERED_GRACE,
            dead_note_age: DEAD_NOTE_AGE,
            settle_backoff: SETTLE_BACKOFF,
        }
    }

    /// The locations for this user: `$IX_RELAY_CLAUDE_DIR` (else `~/.claude`)
    /// and `$XDG_RUNTIME_DIR/cc-socks` (default `/tmp/cc-socks` on macOS),
    /// with the default timings.
    ///
    /// # Errors
    ///
    /// Why a location cannot be told (no `HOME`, or no `XDG_RUNTIME_DIR` on Linux).
    pub fn from_env() -> Result<Self, String> {
        let claude = match std::env::var_os("IX_RELAY_CLAUDE_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(
                std::env::var_os("HOME").ok_or("neither IX_RELAY_CLAUDE_DIR nor HOME is set")?,
            )
            .join(".claude"),
        };
        #[cfg(target_os = "macos")]
        let socket_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(|runtime| PathBuf::from(runtime).join("cc-socks"))
            .unwrap_or_else(|| PathBuf::from("/tmp/cc-socks"));
        #[cfg(not(target_os = "macos"))]
        let socket_dir =
            PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?)
                .join("cc-socks");
        Ok(Self::with_dirs(
            claude.join("sessions"),
            claude.join("projects"),
            socket_dir,
            relay_core::config::state_dir().map(|d| socket_status::dir_in(&d)),
        ))
    }
}

/// Why a batch did not (certainly) arrive; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketError {
    /// Certain nothing was delivered.
    NotSent(String),
    /// Held behind an approval prompt.
    Held(String),
    /// Not tried: the lead's mail is held.
    Deferred(String),
    /// Not posted this tick: its socket time is spent. Nothing to do with the receiver.
    Postponed(String),
    /// May have been delivered.
    Ambiguous(String),
    /// The receiver's user denied it.
    Denied(String),
}

impl From<SocketError> for DeliverError {
    fn from(error: SocketError) -> Self {
        match error {
            SocketError::NotSent(why) => Self::NotSent(why),
            SocketError::Held(why) => Self::Held(why),
            SocketError::Deferred(why) => Self::Deferred(why),
            SocketError::Postponed(why) => Self::Postponed(why),
            SocketError::Ambiguous(why) => Self::Ambiguous(why),
            SocketError::Denied(why) => Self::Denied(why),
        }
    }
}

/// What the receipt thread hands over: receipts, oldest first.
#[derive(Default)]
struct Shared {
    receipts: Mutex<VecDeque<Receipt>>,
    arrived: Condvar,
    stop: AtomicBool,
    /// Reply-socket lines that were JSON in a shape not known (drift).
    unrecognised: AtomicU64,
    /// Reply-socket lines that were not JSON (drift).
    not_json: AtomicU64,
}

/// Adds one to `counter` (saturating) and returns what it was: `0` means this is
/// the first of an episode, so the caller logs once.
fn bump(counter: &AtomicU64) -> u64 {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            Some(n.saturating_add(1))
        })
        .unwrap_or(u64::MAX)
}

impl Shared {
    fn queue(&self) -> MutexGuard<'_, VecDeque<Receipt>> {
        self.receipts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The watcher's own socket, where Claude Code sends receipts and which it
/// checks the sender of a message against, and the thread that reads it.
struct Reply {
    path: PathBuf,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Reply {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // Best effort: a watcher killed by SIGKILL, or that lost the race with a crash,
        // leaves it for the next start to remove (SIGTERM and SIGINT are handled).
        let _ = fs::remove_file(&self.path);
    }
}

/// Whether the reply socket is bound, and when to try again if not.
enum ReplyState {
    Bound(Reply),
    Unbound {
        why: String,
        next_try: Instant,
        backoff: Duration,
        /// Set while a probe of a wedged listener at our socket name is blocked:
        /// one flag for every retry, so retries do not each leave a thread behind.
        probe: Arc<AtomicBool>,
    },
}

/// What is to be done to a send's batch, once it is known: kept until the write to
/// the bus has succeeded, so a failed write is retried and nothing is forgotten.
struct Resolution {
    action: SettleAction,
    why: String,
    notice: Option<(String, String)>,
    /// For a dead letter, the reason to record once it is written.
    reason: Option<&'static str>,
}

/// What is known of a send whose outcome is not final.
struct Pending {
    lead: String,
    session: String,
    batch: BatchId,
    msg_id: String,
    at: Instant,
    /// When Claude Code first held the message, while it is held (or approved and
    /// not yet in the transcript): the batch's lease is renewed until it is settled.
    held_since: Option<Instant>,
    /// When Claude Code reported the message delivered (the user approved it).
    delivered_at: Option<Instant>,
    /// The transcript poll for this message.
    watch: Option<Watch>,
    resolution: Option<Resolution>,
    /// Failed writes of the resolution so far.
    attempts: u32,
    /// Restored from a dead watcher's status file.
    adopted: bool,
    /// When the resolution's write may next be tried (after a failure).
    next_try: Option<Instant>,
    /// Whether the alert for a resolution that keeps failing has been raised.
    alerted: bool,
}

/// What the watcher knows of a lead's resolved Claude Code process.
struct ResolvedLead {
    /// What the status file shows.
    socket: LeadSocket,
    /// The process's start time when it was resolved; a pid that now starts at another
    /// time is another process (FR-055).
    started: String,
}

/// A lead whose mail is held.
struct Held {
    session: String,
    since: Instant,
}

/// A lead whose socket cannot be relied on for now.
struct Suspect {
    session: String,
    since: Instant,
}

struct State {
    reply: ReplyState,
    /// Bodies posted, by target session, and when.
    sent: HashMap<(String, u64), Instant>,
    /// Leads whose Claude Code holds their mail.
    held: HashMap<String, Held>,
    /// Leads that go through herdr for now.
    suspect: HashMap<String, Suspect>,
    /// Sends whose outcome is open, by message id.
    pending: HashMap<String, Pending>,
    /// The socket each lead last resolved to, with the start time of its process then.
    leads: BTreeMap<String, ResolvedLead>,
    /// Why a lead cannot use the socket, or what happened to it.
    notes: BTreeMap<String, String>,
    /// One flag per socket path with a connect helper thread: set while it runs.
    connecting: HashMap<PathBuf, Arc<AtomicBool>>,
    /// The batches the watcher dead-lettered on its own account, and why, newest last.
    dead_notes: VecDeque<DeadLetterNote>,
    /// Time spent waiting on sockets since the last settlement (the tick's start).
    tick_spent: Duration,
    /// Where the next settlement's round of transcript checks starts.
    settle_cursor: usize,
    /// See [`SocketConfig::max_hold`].
    max_hold: Duration,
    /// Notifications waiting to be taken by [`SocketDeliverer::alerts`].
    alerts: Vec<(String, String)>,
    /// The `settlement_failed` note of each lead whose decision cannot be written: kept
    /// apart from the lead's other note, so neither overwrites the other.
    failed_notes: BTreeMap<(String, u64), String>,
    /// Whether writing the status file is failing (the directory is not trusted, say): the
    /// failure is logged and alerted once per episode, not on every write.
    status_failing: bool,
    /// Drift counted on this side of the reply socket: unknown receipt statuses and
    /// transcript lines of an unrecognised shape. The reply socket's own counts live
    /// in [`Shared`] and are joined at publish.
    drift: WireDrift,
}

impl State {
    /// Counts a receipt whose status is not known (drift), and logs the first of them.
    fn count_unknown_status(&mut self) {
        self.drift.unknown_status_receipts = self.drift.unknown_status_receipts.saturating_add(1);
        if self.drift.unknown_status_receipts == 1 {
            eprintln!(
                "relay-watcher: socket_wire_drift: Claude Code sent a receipt status this build does not know \
                 (claude-wire captured on {}); counted, further ones are not logged",
                claude_wire::CAPTURED_ON
            );
        }
    }

    /// Counts a transcript look that found our message id without a structured
    /// `origin.msg_id` (drift), and logs the first of them.
    fn count_transcript_shape(&mut self) {
        self.drift.transcript_shape_unrecognised =
            self.drift.transcript_shape_unrecognised.saturating_add(1);
        if self.drift.transcript_shape_unrecognised == 1 {
            eprintln!(
                "relay-watcher: socket_transcript_shape_unrecognised: a transcript line names a message id \
                 without a structured origin.msg_id (claude-wire captured on {}); the message is \
                 acknowledged as received but unconfirmed, counted, further ones are not logged",
                claude_wire::CAPTURED_ON
            );
        }
    }

    fn max_hold_ms(&self) -> u128 {
        self.max_hold.as_millis()
    }

    /// Drops the dead-letter reasons older than `age` and keeps the newest cap of them.
    fn prune_dead_notes(&mut self, age: Duration) {
        let now = now_ms();
        let age_ms = i64::try_from(age.as_millis()).unwrap_or(i64::MAX);
        // A time from another process's file: checked, and anything that does not fit
        // (or is from the future) is dropped.
        self.dead_notes.retain(|n| {
            now.checked_sub(n.at_ms)
                .is_some_and(|d| (0..=age_ms).contains(&d))
        });
        while self.dead_notes.len() > socket_status::MAX_DEAD_LETTER_NOTES {
            self.dead_notes.pop_front();
        }
    }
}

/// Delivers batches through Claude Code's inbox sockets.
pub struct SocketDeliverer {
    config: SocketConfig,
    procs: Box<dyn Procs + Send + Sync>,
    state: Mutex<State>,
}

impl SocketDeliverer {
    /// Removes what an earlier, dead watcher of this machine left, and binds the
    /// watcher's reply socket at `<socket_dir>/<pid>.sock` (0600).
    ///
    /// Never fails: a reply socket that cannot be bound (the directory may not
    /// exist yet at login) leaves a deliverer that sends nothing (every batch is
    /// [`SocketError::NotSent`], so it goes to herdr), says why in its status and
    /// binds again, with a growing wait, on a later delivery.
    pub fn start(config: SocketConfig, procs: Box<dyn Procs + Send + Sync>) -> Self {
        let adopted = remove_stale(&config, procs.as_ref());
        let probe = Arc::new(AtomicBool::new(false));
        let reply = match bind_reply(&config.socket_dir, &probe) {
            Ok(reply) => ReplyState::Bound(reply),
            Err(why) => {
                eprintln!(
                    "relay-watcher: reply socket not bound ({why}); batches go through herdr \
                     until it binds"
                );
                ReplyState::Unbound {
                    why,
                    next_try: Instant::now() + config.rebind_start,
                    backoff: config.rebind_start,
                    probe,
                }
            }
        };
        let mut state = State {
            reply,
            sent: HashMap::new(),
            held: HashMap::new(),
            suspect: HashMap::new(),
            pending: HashMap::new(),
            leads: BTreeMap::new(),
            notes: BTreeMap::new(),
            connecting: HashMap::new(),
            dead_notes: adopted.dead_lettered.into(),
            max_hold: config.max_hold,
            tick_spent: Duration::ZERO,
            settle_cursor: 0,
            alerts: Vec::new(),
            failed_notes: BTreeMap::new(),
            status_failing: false,
            drift: WireDrift::default(),
        };
        state.prune_dead_notes(config.dead_note_age);
        // A batch held across a restart is still held: keep renewing it, and post
        // nothing more for that lead until the hold is over. The file is another
        // process's word: every field is checked, and the bus checks the batch itself
        // (a renewal names its lead and popper, and matches only a live in-flight row).
        let now = now_ms();
        for h in adopted.held_batches.into_iter().take(MAX_PENDING) {
            let plain = |t: &str| !t.is_empty() && t.len() <= 128 && transcript::is_plain_id(t);
            // Ages from the file, checked: a time that overflows, is in the future or is
            // older than the maximum hold makes the entry invalid.
            let age_of = |since_ms: i64| {
                now.checked_sub(since_ms)
                    .filter(|a| *a >= 0)
                    .and_then(|a| u64::try_from(a).ok())
                    .filter(|a| u128::from(*a) <= state.max_hold_ms())
            };
            let Some(age_ms) = age_of(h.since_ms) else {
                continue;
            };
            let delivered_age_ms = match h.delivered_since_ms {
                None => None,
                Some(d) => match age_of(d) {
                    Some(a) => Some(a),
                    None => continue,
                },
            };
            let valid = h.batch > 0
                && h.lead.parse::<relay_core::AgentName>().is_ok()
                && plain(&h.session)
                && plain(&h.msg_id);
            if !valid {
                continue;
            }
            let ago = |ms: u64| {
                Instant::now()
                    .checked_sub(Duration::from_millis(ms))
                    .unwrap_or_else(Instant::now)
            };
            let since = ago(age_ms);
            state.held.insert(
                h.lead.clone(),
                Held {
                    session: h.session.clone(),
                    since,
                },
            );
            state.pending.insert(
                h.msg_id.clone(),
                Pending {
                    lead: h.lead,
                    session: h.session,
                    batch: BatchId::new(h.batch),
                    msg_id: h.msg_id,
                    at: Instant::now(),
                    held_since: Some(since),
                    delivered_at: delivered_age_ms.map(ago),
                    watch: None,
                    resolution: None,
                    attempts: 0,
                    adopted: true,
                    next_try: None,
                    alerted: false,
                },
            );
        }
        // Decisions the dead watcher still owed a write: applied by this one, validated like
        // the held batches. One for a batch already adopted as held replaces its plain hold.
        for o in adopted.owed.into_iter().take(MAX_PENDING) {
            let action = match o.action.as_str() {
                "ack" => SettleAction::Ack,
                "requeue" => SettleAction::Requeue,
                "dead_letter" => SettleAction::DeadLetter,
                _ => continue,
            };
            let reason: Option<&'static str> = match o.reason.as_deref() {
                None => None,
                Some("denied") => Some("denied"),
                Some("hold_expired") => Some("hold_expired"),
                Some(_) => continue,
            };
            let age_ok = now.checked_sub(o.since_ms).is_some_and(|a| {
                (0..=i64::try_from(config.dead_note_age.as_millis()).unwrap_or(i64::MAX))
                    .contains(&a)
            });
            if o.batch == 0 || !age_ok || o.lead.parse::<relay_core::AgentName>().is_err() {
                continue;
            }
            let resolution = Resolution {
                action,
                why: "an owed decision restored from the previous watcher".to_owned(),
                notice: None,
                reason,
            };
            let batch = BatchId::new(o.batch);
            if let Some(p) = state
                .pending
                .values_mut()
                .find(|p| p.lead == o.lead && p.batch == batch)
            {
                p.resolution = Some(resolution);
                continue;
            }
            state.pending.insert(
                format!("owed-{}-{}", o.lead, o.batch),
                Pending {
                    lead: o.lead,
                    session: String::new(),
                    batch,
                    msg_id: format!("owed-{}", o.batch),
                    at: Instant::now(),
                    held_since: None,
                    delivered_at: None,
                    watch: None,
                    resolution: Some(resolution),
                    attempts: 0,
                    adopted: true,
                    next_try: None,
                    alerted: false,
                },
            );
        }
        let deliverer = Self {
            state: Mutex::new(state),
            config,
            procs,
        };
        deliverer.publish(&mut deliverer.lock());
        deliverer
    }

    /// The reply socket's path, when it is bound.
    #[must_use]
    pub fn reply_path(&self) -> Option<PathBuf> {
        match &self.lock().reply {
            ReplyState::Bound(reply) => Some(reply.path.clone()),
            ReplyState::Unbound { .. } => None,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The directories a session's socket may be in.
    fn socket_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.config.socket_dir.clone()];
        #[cfg(target_os = "macos")]
        dirs.push(PathBuf::from("/tmp/cc-socks"));
        if let Some(uid) = self.procs.uid() {
            dirs.push(PathBuf::from(format!("/tmp/cc-socks-{uid}")));
        }
        dirs
    }

    /// Binds the reply socket if it is not bound and the wait since the last try
    /// has passed.
    fn ensure_bound(&self, state: &mut State) {
        let ReplyState::Unbound {
            why,
            next_try,
            backoff,
            probe,
        } = &mut state.reply
        else {
            return;
        };
        if Instant::now() < *next_try {
            return;
        }
        match bind_reply(&self.config.socket_dir, probe) {
            Ok(reply) => {
                eprintln!(
                    "relay-watcher: reply socket bound at {}; socket delivery is available",
                    reply.path.display()
                );
                state.reply = ReplyState::Bound(reply);
            }
            Err(now_why) => {
                // One log line per episode: the reason is repeated only when it changes.
                if *why != now_why {
                    eprintln!("relay-watcher: reply socket not bound ({now_why})");
                    *why = now_why;
                }
                *backoff = (*backoff * 2).min(REBIND_MAX);
                *next_try = Instant::now() + *backoff;
            }
        }
    }

    /// Whether the socket is holding mail for `lead`'s `session`, so none should
    /// be popped for it.
    pub fn holds(&self, lead: &str, session: Option<&relay_core::SessionId>) -> bool {
        let mut state = self.lock();
        let Some(session) = session.map(relay_core::SessionId::as_str) else {
            return false;
        };
        match state.held.get(lead) {
            Some(held)
                if held.session == session && held.since.elapsed() < self.config.held_quiet =>
            {
                true
            }
            Some(_) => {
                state.held.remove(lead);
                false
            }
            None => false,
        }
    }

    /// Starts a tick's view of the registry: drops every resolved-lead entry whose lead has
    /// no registration in `registered`, is registered on another machine, is not a Claude
    /// Code lead, or whose process is gone or was replaced by one that started at another
    /// time (FR-055). Nothing else is touched (a held batch, a pending decision, a
    /// dead-letter reason, the drift counts have their own rules), and nothing waits: this
    /// is a few table lookups. The status file is written when an entry was dropped.
    pub fn registry_read(&self, registered: &[RegisteredLead<'_>]) {
        let mut state = self.lock();
        let before = state.leads.len();
        state.leads.retain(|lead, resolved| {
            let registration_ok = registered
                .iter()
                .any(|r| r.name == lead && r.here && relay_core::tries_socket(r.kind));
            registration_ok
                && self.procs.start_time(resolved.socket.pid).as_deref()
                    == Some(resolved.started.as_str())
        });
        if state.leads.len() != before {
            self.publish(&mut state);
        }
    }

    /// Forgets what the status file said about `lead`'s socket (its resolved entry, its note
    /// and its suspicion) because the lead does not try the socket: a lead whose
    /// registration changed `kind` is judged by the registration the tick read (FR-054).
    /// A held batch, a pending decision and a dead-letter reason are not touched.
    pub fn not_tried(&self, lead: &str) {
        let mut state = self.lock();
        let had = state.leads.remove(lead).is_some()
            | state.notes.remove(lead).is_some()
            | state.suspect.remove(lead).is_some();
        if had {
            self.publish(&mut state);
        }
    }

    /// Posts `text` for `target` and waits for the outcome.
    ///
    /// # Errors
    ///
    /// [`SocketError`], as the module docs describe.
    pub fn deliver(&self, target: &BatchTarget<'_>, text: &str) -> Result<(), SocketError> {
        let result = self.attempt(target, text);
        let mut state = self.lock();
        match &result {
            Ok(()) => {
                state.notes.remove(target.lead);
                state.suspect.remove(target.lead);
            }
            Err(SocketError::NotSent(why)) => {
                state.leads.remove(target.lead);
                state
                    .notes
                    .insert(target.lead.to_owned(), sanitise_line(why));
            }
            Err(SocketError::Ambiguous(why)) => {
                state
                    .notes
                    .insert(target.lead.to_owned(), sanitise_line(why));
            }
            Err(
                SocketError::Held(_)
                | SocketError::Deferred(_)
                | SocketError::Postponed(_)
                | SocketError::Denied(_),
            ) => {}
        }
        self.publish(&mut state);
        drop(state);
        result
    }

    fn attempt(&self, target: &BatchTarget<'_>, text: &str) -> Result<(), SocketError> {
        let session = target
            .session
            .map(relay_core::SessionId::as_str)
            .ok_or_else(|| SocketError::NotSent("the lead registered no session id".to_owned()))?;
        if !transcript::is_plain_id(session) {
            return Err(SocketError::NotSent(
                "the session id is not a plain id".to_owned(),
            ));
        }
        // Everything from here to the end of the wait counts against the tick's budget.
        let started = Instant::now();
        let posted = self.post(target, text, session, started);
        self.lock().tick_spent += started.elapsed();
        let (shared, msg_id, deadline_watch) = posted?;
        let waiting = Instant::now();
        let outcome = self.await_outcome(&shared, target, session, &msg_id, deadline_watch);
        self.lock().tick_spent += waiting.elapsed();
        outcome
    }

    /// Resolves the target and posts the line: everything before the wait. Returns what
    /// the wait needs.
    fn post(
        &self,
        target: &BatchTarget<'_>,
        text: &str,
        session: &str,
        started: Instant,
    ) -> Result<(Arc<Shared>, String, Option<Watch>), SocketError> {
        let lead = target.lead;
        let posted = {
            let mut state = self.lock();
            self.ensure_bound(&mut state);
            let (shared, reply_path) = match &state.reply {
                ReplyState::Bound(reply) => (Arc::clone(&reply.shared), reply.path.clone()),
                ReplyState::Unbound { why, .. } => {
                    return Err(SocketError::NotSent(format!(
                        "reply socket not bound: {why}"
                    )));
                }
            };
            let resolved = session::resolve(
                &self.config.sessions_dir,
                session,
                self.procs.as_ref(),
                &self.socket_dirs(),
            )
            .map_err(|unresolved| SocketError::NotSent(unresolved.to_string()))?;
            state.leads.insert(
                lead.to_owned(),
                ResolvedLead {
                    socket: LeadSocket {
                        lead: lead.to_owned(),
                        pid: resolved.pid,
                        socket: resolved.socket.display().to_string(),
                        version: resolved.version.clone(),
                    },
                    started: resolved.proc_start.clone(),
                },
            );
            if let Some(held) = state.held.get(lead) {
                if held.session == session && held.since.elapsed() < self.config.held_quiet {
                    return Err(SocketError::Deferred(
                        "mail for this lead is held for approval; not tried".to_owned(),
                    ));
                }
                state.held.remove(lead);
            }
            if let Some(suspect) = state.suspect.get(lead) {
                if suspect.session == session && suspect.since.elapsed() < self.config.suspect_for {
                    return Err(SocketError::NotSent(format!(
                        "an earlier socket delivery to this lead could not be confirmed, so \
                         herdr types its mail for now; {AMBIGUOUS_CAUSES}"
                    )));
                }
                state.suspect.remove(lead);
            }
            // Posted only when what is left of the tick's budget covers the worst case of
            // the post: a connect, a write and a whole ack wait (what was spent before,
            // this attempt's resolving included, is counted).
            if state.tick_spent + started.elapsed() + self.config.post_worst_case()
                > self.config.tick_budget
            {
                return Err(SocketError::Postponed(
                    "this tick's time on sockets is spent; not posted, tried on the next tick"
                        .to_owned(),
                ));
            }
            let content = self.content(&reply_path, target, text);
            let key = (session.to_owned(), hash_of(&content));
            let window = self.config.dedupe_window;
            state.sent.retain(|_, at| at.elapsed() < window);
            if state.sent.contains_key(&key) {
                return Err(SocketError::NotSent(
                    "an identical body was posted to this session in the last 30 s".to_owned(),
                ));
            }
            let msg_id = new_msg_id()
                .map_err(|e| SocketError::NotSent(format!("no random id available: {e}")))?;
            let from = format!("uds:{}", reply_path.display());
            let line = Envelope::new(&msg_id, session, &from, &content)
                .line()
                .ok_or_else(|| {
                    SocketError::NotSent("the batch does not fit one line".to_owned())
                })?;
            let busy = Arc::clone(state.connecting.entry(resolved.socket.clone()).or_default());
            let mut stream = connect_bounded(&busy, &resolved.socket, CONNECT_TIMEOUT)
                .map_err(|e| SocketError::NotSent(format!("cannot connect: {e}")))?;
            // The transcript is watched from where it ends *before* the line is written:
            // a receiver may record the message the instant it reads it.
            let watch = transcript::find(&self.config.projects_dir, session).map(Watch::new);
            state.sent.insert(key, Instant::now());
            if let Err(e) = write_within(&mut stream, line.as_bytes(), WRITE_TIMEOUT) {
                // Part of the line may have been read: unconfirmed, and a receiver that
                // reads this slowly goes through herdr for a while.
                state.suspect.insert(
                    lead.to_owned(),
                    Suspect {
                        session: session.to_owned(),
                        since: Instant::now(),
                    },
                );
                return Err(SocketError::Ambiguous(format!(
                    "the line was not written within {}s: {e}; this lead goes through herdr \
                     for now",
                    WRITE_TIMEOUT.as_secs()
                )));
            }
            if state.pending.len() >= MAX_PENDING
                && let Some(oldest) = state
                    .pending
                    .iter()
                    .filter(|(_, p)| p.held_since.is_none())
                    .min_by_key(|(_, p)| p.at)
                    .map(|(id, _)| id.clone())
            {
                state.pending.remove(&oldest);
            }
            state.pending.insert(
                msg_id.clone(),
                Pending {
                    lead: lead.to_owned(),
                    session: session.to_owned(),
                    batch: target.batch,
                    msg_id: msg_id.clone(),
                    at: Instant::now(),
                    held_since: None,
                    delivered_at: None,
                    watch: None,
                    resolution: None,
                    attempts: 0,
                    adopted: false,
                    next_try: None,
                    alerted: false,
                },
            );
            (shared, msg_id, watch)
        };
        Ok(posted)
    }

    /// The wrapped text, with the sender label bounded.
    fn content(&self, reply: &Path, target: &BatchTarget<'_>, text: &str) -> String {
        let name: String = sanitise_line(target.senders)
            .chars()
            .take(MAX_FROM_NAME_CHARS)
            .collect();
        let name = if name.is_empty() { "ix-relay" } else { &name };
        wrap(&format!("uds:{}", reply.display()), name, text)
    }

    /// Waits the whole ack wait for a receipt about `msg_id` or the transcript to show
    /// it (a batch is only posted when the tick's budget covers that). What is still
    /// open at the end stays in the pending table, and [`SocketDeliverer::settle`]
    /// finishes it. A wait that ends with neither is a receiver the socket cannot
    /// confirm: that lead goes through herdr for a while.
    fn await_outcome(
        &self,
        shared: &Shared,
        target: &BatchTarget<'_>,
        session: &str,
        msg_id: &str,
        mut watch: Option<Watch>,
    ) -> Result<(), SocketError> {
        let deadline = Instant::now() + self.config.ack_wait;
        let mut step = POLL_START;
        loop {
            if let Some(receipt) = take_receipt(shared, msg_id) {
                // `None`: delivered, so the transcript is next.
                if let Some(outcome) = self.verdict(target, session, msg_id, receipt.verdict) {
                    return outcome;
                }
            }
            // Created after the post: the transcript came into being since, so all of it is new.
            watch = watch.or_else(|| {
                transcript::find(&self.config.projects_dir, session).map(Watch::from_start)
            });
            match watch.as_mut().map(|w| w.records(msg_id)) {
                Some(TranscriptMatch::Acked) => {
                    let mut state = self.lock();
                    state.pending.remove(msg_id);
                    state.held.remove(target.lead);
                    return Ok(());
                }
                Some(TranscriptMatch::ShapeUnrecognised) => {
                    // Our fresh id is in the transcript, so Claude Code received the message;
                    // the shape it recorded it in is not one this build knows. Acknowledge it as
                    // received but unconfirmed: no herdr fallback, no redelivery loop.
                    let mut state = self.lock();
                    state.count_transcript_shape();
                    state.pending.remove(msg_id);
                    state.held.remove(target.lead);
                    return Ok(());
                }
                Some(TranscriptMatch::Absent) | None => {}
            }
            let now = Instant::now();
            if now >= deadline {
                let mut state = self.lock();
                let delivered = state.pending.get_mut(msg_id).is_some_and(|p| {
                    p.watch = watch.take();
                    p.delivered_at.is_some()
                });
                if delivered {
                    // Claude Code said delivered: the transcript is waited for, on the
                    // pending entry, until the grace ends.
                    return Err(SocketError::Ambiguous(
                        "Claude Code reported the message delivered; its transcript does not \
                         show it yet"
                            .to_owned(),
                    ));
                }
                let seen = if state.pending.get(msg_id).is_some_and(|p| p.watch.is_some()) {
                    "its transcript does not show the message yet"
                } else {
                    "its transcript cannot be found"
                };
                state.suspect.insert(
                    target.lead.to_owned(),
                    Suspect {
                        session: session.to_owned(),
                        since: now,
                    },
                );
                return Err(SocketError::Ambiguous(format!(
                    "the socket took the line but sent no receipt and {seen}; {AMBIGUOUS_CAUSES}; \
                     this lead goes through herdr for now"
                )));
            }
            let guard = shared.queue();
            let _ = shared
                .arrived
                .wait_timeout(guard, step.min(deadline - now))
                .unwrap_or_else(PoisonError::into_inner);
            step = (step * 2).min(POLL_MAX);
        }
    }

    /// What a receipt about the message being waited on means for the delivery:
    /// `Some(outcome)` ends it, `None` keeps waiting (for the transcript).
    fn verdict(
        &self,
        target: &BatchTarget<'_>,
        session: &str,
        msg_id: &str,
        verdict: Verdict,
    ) -> Option<Result<(), SocketError>> {
        let mut state = self.lock();
        match verdict {
            Verdict::Held => {
                state.held.insert(
                    target.lead.to_owned(),
                    Held {
                        session: session.to_owned(),
                        since: Instant::now(),
                    },
                );
                if let Some(p) = state.pending.get_mut(msg_id) {
                    p.held_since.get_or_insert_with(Instant::now);
                }
                Some(Err(SocketError::Held(
                    "Claude Code holds the message for approval (bypass mode)".to_owned(),
                )))
            }
            Verdict::Delivered => {
                if let Some(p) = state.pending.get_mut(msg_id) {
                    p.delivered_at.get_or_insert_with(Instant::now);
                    p.held_since.get_or_insert_with(Instant::now);
                }
                state.held.remove(target.lead);
                None
            }
            Verdict::Denied => {
                // The caller dead-letters the batch; the resolution is kept until the
                // write is confirmed by the next settlement (which finds it done).
                state.held.remove(target.lead);
                if let Some(p) = state.pending.get_mut(msg_id) {
                    p.resolution = Some(Resolution {
                        action: SettleAction::DeadLetter,
                        why: "denied by the receiver's user".to_owned(),
                        notice: None,
                        reason: Some("denied"),
                    });
                }
                Some(Err(SocketError::Denied(
                    "the receiver's user denied the message".to_owned(),
                )))
            }
            Verdict::Refused => {
                state.pending.remove(msg_id);
                Some(Err(SocketError::NotSent(
                    "Claude Code refuses cross-session messages there".to_owned(),
                )))
            }
            Verdict::Dropped => {
                state.pending.remove(msg_id);
                Some(Err(SocketError::NotSent(
                    "Claude Code dropped the message".to_owned(),
                )))
            }
            Verdict::HoldExpired => {
                state.pending.remove(msg_id);
                state.held.remove(target.lead);
                Some(Err(SocketError::Deferred(
                    "the hold on the message timed out unapproved".to_owned(),
                )))
            }
            Verdict::Duplicate => Some(Err(SocketError::Ambiguous(
                "Claude Code dropped it as a duplicate of an earlier copy".to_owned(),
            ))),
            Verdict::Expired => Some(Err(SocketError::Ambiguous(
                "Claude Code reports the message expired".to_owned(),
            ))),
            Verdict::Unknown => {
                state.count_unknown_status();
                state.notes.insert(
                    target.lead.to_owned(),
                    "Claude Code sent a receipt status this watcher does not know".to_owned(),
                );
                Some(Err(SocketError::Ambiguous(
                    "Claude Code sent a receipt this watcher does not know".to_owned(),
                )))
            }
        }
    }

    /// Records that the watcher dead-lettered `batch` of `lead`, and why. A dead letter
    /// is a bare message, so the reason lives here, where later notes do not overwrite it.
    fn note_dead_letter(&self, state: &mut State, lead: &str, batch: BatchId, reason: &str) {
        state.dead_notes.push_back(DeadLetterNote {
            lead: lead.to_owned(),
            batch: batch.get(),
            reason: reason.to_owned(),
            at_ms: now_ms(),
        });
        state.prune_dead_notes(self.config.dead_note_age);
    }

    /// The notifications waiting to be shown, taken once.
    pub fn alerts(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.lock().alerts)
    }

    /// The busy flag of the probe of a listener at the reply socket's name, while the
    /// socket is unbound.
    #[cfg(test)]
    pub(crate) fn probe_flag(&self) -> Option<Arc<AtomicBool>> {
        match &self.lock().reply {
            ReplyState::Unbound { probe, .. } => Some(Arc::clone(probe)),
            ReplyState::Bound(_) => None,
        }
    }

    /// Tells the transport what became of a settlement it returned. The entry it is
    /// about is forgotten only when the write succeeded (or the batch is gone); a failed
    /// write is retried by the next settlement, a bounded number of times.
    pub fn applied(&self, settlement: &Settlement, result: &Result<usize, String>) {
        let mut state = self.lock();
        let found = state
            .pending
            .iter()
            .find(|(_, p)| p.lead == settlement.lead && p.batch == settlement.batch)
            .map(|(id, _)| id.clone());
        let Some(id) = found else {
            return;
        };
        match result {
            Ok(moved) => {
                if matches!(settlement.action, SettleAction::Renew(_)) {
                    if *moved == 0 {
                        // No longer in flight (acknowledged or reaped): stop renewing it.
                        state.pending.remove(&id);
                    }
                } else {
                    state
                        .failed_notes
                        .remove(&(settlement.lead.clone(), settlement.batch.get()));
                    if let Some(reason) = state
                        .pending
                        .remove(&id)
                        .and_then(|p| p.resolution)
                        .and_then(|r| r.reason)
                    {
                        self.note_dead_letter(
                            &mut state,
                            &settlement.lead,
                            settlement.batch,
                            reason,
                        );
                    }
                }
            }
            Err(why) => {
                if matches!(settlement.action, SettleAction::Renew(_)) {
                    return;
                }
                let backoff_start = self.config.settle_backoff;
                let mut alert = None;
                if let Some(p) = state.pending.get_mut(&id) {
                    p.attempts += 1;
                    // Never given up on: kept, renewed, and tried again after a growing wait.
                    let doublings = p.attempts.saturating_sub(1).min(16);
                    let wait = backoff_start
                        .saturating_mul(1_u32 << doublings)
                        .min(SETTLE_BACKOFF_MAX);
                    p.next_try = Some(Instant::now() + wait);
                    if p.attempts == 1 {
                        eprintln!(
                            "relay-watcher: {}: settling batch {} ({:?}) failed: {why}; kept, \
                             renewed and retried",
                            settlement.lead,
                            settlement.batch.get(),
                            settlement.action
                        );
                    }
                    if p.attempts >= SETTLE_ALERT_AFTER && !p.alerted {
                        p.alerted = true;
                        alert = Some(sanitise_line(&format!(
                            "settlement_failed: batch {} ({:?}) cannot be written to the bus ({why})",
                            settlement.batch.get(),
                            settlement.action
                        )));
                    }
                }
                if let Some(note) = alert {
                    state.alerts.push((
                        format!("ix-relay: a decision about {}'s mail cannot be written", settlement.lead),
                        format!(
                            "{note}. The batch is kept in flight and its lease renewed, and the write \
                             is retried; nothing is posted again meanwhile."
                        ),
                    ));
                    state
                        .failed_notes
                        .insert((settlement.lead.clone(), settlement.batch.get()), note);
                }
            }
        }
        self.publish(&mut state);
    }

    /// Starts a tick: routes every receipt that arrived since the last call to the
    /// batch it is about, decides what is to be done with each open send, and returns
    /// what the caller must now do. Every send with a decision is returned again on
    /// each tick until [`SocketDeliverer::applied`] reports the write done, and every
    /// held batch has its lease renewed on each.
    pub fn settle(&self) -> Vec<Settlement> {
        let started = Instant::now();
        let mut out = Vec::new();
        let shared = {
            let mut state = self.lock();
            state.tick_spent = Duration::ZERO;
            self.ensure_bound(&mut state);
            match &state.reply {
                ReplyState::Bound(reply) => Some(Arc::clone(&reply.shared)),
                ReplyState::Unbound { .. } => None,
            }
        };
        let receipts: Vec<Receipt> = shared
            .map(|s| s.queue().drain(..).collect())
            .unwrap_or_default();
        let mut state = self.lock();
        state.prune_dead_notes(self.config.dead_note_age);
        state.pending.retain(|_, p| {
            p.at.elapsed() < PENDING_FOR || p.held_since.is_some() || p.resolution.is_some()
        });
        for receipt in receipts {
            let ids: Vec<String> = state
                .pending
                .keys()
                .filter(|id| receipt.concerns(id))
                .cloned()
                .collect();
            for id in ids {
                self.apply_receipt(&mut state, &id, receipt.verdict);
            }
        }
        // Rotating transcript checks over the sends still undecided, oldest first.
        let mut ids: Vec<(Instant, String)> = state
            .pending
            .iter()
            .filter(|(_, p)| p.resolution.is_none())
            .map(|(id, p)| (p.at, id.clone()))
            .collect();
        ids.sort();
        let total = ids.len();
        let start = if total == 0 {
            0
        } else {
            state.settle_cursor % total
        };
        let picked: Vec<String> = ids
            .into_iter()
            .map(|(_, id)| id)
            .cycle()
            .skip(start)
            .take(TRANSCRIPT_CHECKS_PER_SETTLE.min(total))
            .collect();
        state.settle_cursor = state
            .settle_cursor
            .wrapping_add(TRANSCRIPT_CHECKS_PER_SETTLE);
        let projects = self.config.projects_dir.clone();
        let mut shape_seen: Vec<String> = Vec::new();
        for id in picked {
            let Some(p) = state.pending.get_mut(&id) else {
                continue;
            };
            if p.watch.is_none() {
                // Posted by this watcher, the transcript came into being since; posted
                // before a restart, the message's place in the file is unknown.
                p.watch = transcript::find(&projects, &p.session).map(|f| {
                    if p.adopted {
                        Watch::from_tail(f)
                    } else {
                        Watch::from_start(f)
                    }
                });
            }
            match p.watch.as_mut().map(|w| w.records(&id)) {
                Some(TranscriptMatch::Acked) => {
                    p.resolution = Some(Resolution {
                        action: SettleAction::Ack,
                        why: "the transcript shows the message".to_owned(),
                        notice: None,
                        reason: None,
                    });
                }
                // The id is in the transcript: received, in a shape not known. Acknowledged
                // like `delivered_unconfirmed`, with a drift note; never left to redeliver.
                Some(TranscriptMatch::ShapeUnrecognised) => {
                    p.resolution = Some(Resolution {
                        action: SettleAction::Ack,
                        why: "transcript_shape_unrecognised: the transcript names the message \
                              without a structured origin.msg_id; acknowledged as received"
                            .to_owned(),
                        notice: None,
                        reason: None,
                    });
                    shape_seen.push(p.lead.clone());
                }
                Some(TranscriptMatch::Absent) | None => {}
            }
        }
        for lead in shape_seen {
            state.count_transcript_shape();
            state.notes.insert(
                lead,
                "transcript_shape_unrecognised: the transcript names a message without a \
                 structured origin.msg_id; the batch was acknowledged as received but unconfirmed"
                    .to_owned(),
            );
        }
        // Decisions by time: an approved message whose transcript line never shows up is
        // acknowledged after the grace (Claude Code itself said delivered); a hold nobody
        // answers is dead-lettered at the maximum hold.
        let (grace, max_hold) = (self.config.delivered_grace, self.config.max_hold);
        let mut unconfirmed: Vec<String> = Vec::new();
        for p in state
            .pending
            .values_mut()
            .filter(|p| p.resolution.is_none())
        {
            if p.delivered_at.is_some_and(|at| at.elapsed() >= grace) {
                unconfirmed.push(p.lead.clone());
                p.resolution = Some(Resolution {
                    action: SettleAction::Ack,
                    why: "delivered_unconfirmed: Claude Code reported it delivered; the \
                          transcript never showed it"
                        .to_owned(),
                    notice: None,
                    reason: None,
                });
            } else if p.delivered_at.is_none()
                && p.held_since.is_some_and(|s| s.elapsed() >= max_hold)
            {
                let lead = p.lead.clone();
                p.resolution = Some(Resolution {
                    action: SettleAction::DeadLetter,
                    why: "hold_expired: nobody answered the approval prompt in time".to_owned(),
                    notice: Some((
                        format!("ix-relay: a message to {lead} was never approved"),
                        "Claude Code held it behind an approval prompt that nobody answered; it \
                         was dead-lettered (reason hold_expired) and will not be posted again. \
                         Set crossSessionInbound to accept, or delivery = herdr."
                            .to_owned(),
                    )),
                    reason: Some("hold_expired"),
                });
            }
        }
        for lead in unconfirmed {
            state.notes.insert(
                lead,
                "delivered_unconfirmed: Claude Code reported a held message delivered but its \
                 transcript never showed it (another CLAUDE_CONFIG_DIR?); the batch was acknowledged"
                    .to_owned(),
            );
        }
        for p in state.pending.values() {
            // Renewed while held, and while a decision about it cannot be written.
            if p.held_since.is_some() || (p.resolution.is_some() && p.attempts > 0) {
                out.push(Settlement {
                    lead: p.lead.clone(),
                    batch: p.batch,
                    action: SettleAction::Renew(HELD_RENEW_LEASE),
                    why: "held behind an approval prompt".to_owned(),
                    notice: None,
                });
            }
            let due = p.next_try.is_none_or(|t| Instant::now() >= t);
            if let Some(r) = p.resolution.as_ref().filter(|_| due) {
                out.push(Settlement {
                    lead: p.lead.clone(),
                    batch: p.batch,
                    action: r.action,
                    why: r.why.clone(),
                    notice: r.notice.clone(),
                });
            }
        }
        // The checks above count against the tick's budget.
        state.tick_spent = started.elapsed();
        if !out.is_empty() || !state.pending.is_empty() {
            self.publish(&mut state);
        }
        out
    }

    /// What a late receipt decides for the send it names.
    fn apply_receipt(&self, state: &mut State, id: &str, verdict: Verdict) {
        let Some(p) = state.pending.get(id) else {
            return;
        };
        let (lead, session) = (p.lead.clone(), p.session.clone());
        let resolve = |state: &mut State, action, why: &str, reason| {
            if let Some(p) = state.pending.get_mut(id) {
                p.resolution = Some(Resolution {
                    action,
                    why: why.to_owned(),
                    notice: None,
                    reason,
                });
            }
        };
        match verdict {
            Verdict::Held => {
                state.held.insert(
                    lead,
                    Held {
                        session,
                        since: Instant::now(),
                    },
                );
                if let Some(p) = state.pending.get_mut(id) {
                    p.held_since.get_or_insert_with(Instant::now);
                }
            }
            Verdict::Delivered => {
                // Approved: the hold is over; the transcript will show it, and the
                // batch's lease is renewed until it does (or the grace ends).
                if let Some(p) = state.pending.get_mut(id) {
                    p.delivered_at.get_or_insert_with(Instant::now);
                    p.held_since.get_or_insert_with(Instant::now);
                }
                state.held.remove(&lead);
            }
            Verdict::Denied => {
                state.held.remove(&lead);
                state.notes.insert(
                    lead,
                    "the receiver's user denied a held message; it was dead-lettered as denied"
                        .to_owned(),
                );
                resolve(
                    state,
                    SettleAction::DeadLetter,
                    "denied by the receiver's user",
                    Some("denied"),
                );
            }
            Verdict::Refused | Verdict::Dropped => {
                state.suspect.insert(
                    lead,
                    Suspect {
                        session,
                        since: Instant::now(),
                    },
                );
                resolve(
                    state,
                    SettleAction::Requeue,
                    "a late receipt says it was refused or dropped",
                    None,
                );
            }
            Verdict::HoldExpired => {
                state.held.remove(&lead);
                resolve(
                    state,
                    SettleAction::Requeue,
                    "the hold timed out unapproved",
                    None,
                );
            }
            Verdict::Duplicate | Verdict::Expired => {
                state.notes.insert(
                    lead,
                    "a late receipt says the message expired or was a duplicate; it may or may \
                     not have been delivered"
                        .to_owned(),
                );
            }
            Verdict::Unknown => {
                state.count_unknown_status();
                state.notes.insert(
                    lead,
                    "Claude Code sent a receipt status this watcher does not know".to_owned(),
                );
            }
        }
    }

    /// Writes this watcher's status file, the one `doctor` reads. A failure is
    /// not a delivery failure.
    fn publish(&self, state: &mut State) {
        let Some(dir) = &self.config.status_dir else {
            return;
        };
        let path = socket_status::path_for(dir, std::process::id());
        let mut notes: Vec<String> = state
            .notes
            .iter()
            .map(|(l, n)| format!("{l}: {n}"))
            .chain(
                state
                    .failed_notes
                    .iter()
                    .map(|((l, _), n)| format!("{l}: {n}")),
            )
            .collect();
        let reply_socket = match &state.reply {
            ReplyState::Bound(reply) => Some(reply.path.display().to_string()),
            ReplyState::Unbound { why, .. } => {
                notes.push(format!(
                    "reply socket not bound, every batch goes to herdr until it binds: {why}"
                ));
                None
            }
        };
        let mut drift = state.drift;
        if let ReplyState::Bound(reply) = &state.reply {
            drift.unrecognised_lines = reply.shared.unrecognised.load(Ordering::Relaxed);
            drift.not_json_lines = reply.shared.not_json.load(Ordering::Relaxed);
        }
        let mut held: Vec<String> = state.held.keys().cloned().collect();
        held.sort();
        let now = now_ms();
        let held_batches = state
            .pending
            .values()
            .filter_map(|p| {
                let since = p.held_since?;
                Some(HeldBatch {
                    lead: p.lead.clone(),
                    batch: p.batch.get(),
                    msg_id: p.msg_id.clone(),
                    session: p.session.clone(),
                    since_ms: ms_ago(now, since),
                    delivered_since_ms: p.delivered_at.map(|d| ms_ago(now, d)),
                })
            })
            .collect();
        let status = SocketStatus {
            pid: std::process::id(),
            start_time: self.procs.start_time(std::process::id()),
            updated_at_ms: now,
            reply_socket,
            leads: state.leads.values().map(|l| l.socket.clone()).collect(),
            held,
            held_batches,
            owed: state
                .pending
                .values()
                .filter_map(|p| {
                    let r = p.resolution.as_ref()?;
                    Some(OwedSettlement {
                        lead: p.lead.clone(),
                        batch: p.batch.get(),
                        action: match r.action {
                            SettleAction::Ack => "ack",
                            SettleAction::Requeue => "requeue",
                            SettleAction::DeadLetter => "dead_letter",
                            SettleAction::Renew(_) => return None,
                        }
                        .to_owned(),
                        reason: r.reason.map(str::to_owned),
                        since_ms: ms_ago(now, p.at),
                    })
                })
                .collect(),
            dead_lettered: state.dead_notes.iter().cloned().collect(),
            drift,
            notes,
        };
        match status.write(&path) {
            Ok(()) => state.status_failing = false,
            Err(e) => {
                // Once per episode: delivery goes on, but what a restart needs (held batches,
                // owed decisions) is not saved, and the operator is told so.
                if !state.status_failing {
                    state.status_failing = true;
                    eprintln!("relay-watcher: cannot write {}: {e}", path.display());
                    state.alerts.push((
                        "ix-relay: the socket status directory is not usable".to_owned(),
                        format!(
                            "socket status directory untrusted or unwritable ({e}): held batches \
                             and owed decisions will not survive a restart. Delivery goes on."
                        ),
                    ));
                }
            }
        }
    }
}

impl Drop for SocketDeliverer {
    /// A clean exit removes the status file, unless a batch held behind a prompt, or a decision
    /// still owed a write, must survive it: that stays for the next watcher to adopt and then remove. The
    /// dead-letter reasons alone do not keep it (they age out instead).
    fn drop(&mut self) {
        let Some(dir) = &self.config.status_dir else {
            return;
        };
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        let keeps = state
            .pending
            .values()
            .any(|p| p.held_since.is_some() || p.resolution.is_some());
        if !keeps {
            let _ = fs::remove_file(socket_status::path_for(dir, std::process::id()));
        }
    }
}

/// Removes the first queued receipt about `msg_id`, if any.
fn take_receipt(shared: &Shared, msg_id: &str) -> Option<Receipt> {
    let mut queue = shared.queue();
    let at = queue.iter().position(|r| r.concerns(msg_id))?;
    queue.remove(at)
}

/// `now` minus how long ago `at` was, in Unix milliseconds (saturating).
fn ms_ago(now: i64, at: Instant) -> i64 {
    now.saturating_sub(i64::try_from(at.elapsed().as_millis()).unwrap_or(i64::MAX))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn hash_of(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// A random version 4 UUID: Claude Code records a message's id in the transcript
/// only when it is one.
fn new_msg_id() -> io::Result<String> {
    let mut b = [0_u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        hex[0..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..16].concat()
    ))
}

/// Connects to `path`, giving up after `timeout`, with at most one helper thread
/// per socket: while `busy` is set, an earlier connect to it is still blocked
/// and no other is started.
pub(super) fn connect_bounded(
    busy: &Arc<AtomicBool>,
    path: &Path,
    timeout: Duration,
) -> io::Result<UnixStream> {
    let path = path.to_path_buf();
    run_within(busy, move || UnixStream::connect(path), timeout)
}

/// Writes all of `bytes` within `total`: one deadline for the whole line. The socket's
/// write timeout restarts for every wait inside one `write` call, so a receiver that reads
/// a little every few seconds could stretch a blocking write far past it; the stream is
/// made non-blocking instead, and the loop itself keeps the deadline.
pub(super) fn write_within(
    stream: &mut UnixStream,
    bytes: &[u8],
    total: Duration,
) -> io::Result<()> {
    /// How long to wait before trying again when the socket's buffer is full.
    const RETRY: Duration = Duration::from_millis(2);
    let deadline = Instant::now() + total;
    stream.set_nonblocking(true)?;
    let mut written = 0;
    while written < bytes.len() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the write deadline passed",
            ));
        }
        match stream.write(&bytes[written..]) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(n) => written += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(RETRY),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Runs `connect` on its own thread and waits `timeout` for its stream.
///
/// `UnixStream::connect` has no timeout, and blocks without end when a session
/// has stopped accepting and filled its socket's backlog. The thread only
/// connects and hands the stream over: if the caller has given up it drops the
/// stream unwritten, so a timeout means nothing was sent. `busy` is set while the
/// thread lives, so a wedged socket costs one blocked thread, not one per try.
pub(super) fn run_within(
    busy: &Arc<AtomicBool>,
    connect: impl FnOnce() -> io::Result<UnixStream> + Send + 'static,
    timeout: Duration,
) -> io::Result<UnixStream> {
    if busy.swap(true, Ordering::AcqRel) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "an earlier connect to this socket is still blocked",
        ));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let flag = Arc::clone(busy);
    thread::spawn(move || {
        let result = connect();
        flag.store(false, Ordering::Release);
        // The receiver is gone when the caller timed out: the stream is dropped.
        let _ = tx.send(result);
    });
    rx.recv_timeout(timeout)
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the connect timed out"))?
}

/// Binds the reply socket at `<socket_dir>/<pid>.sock` (0600) and starts the
/// thread that reads it.
///
/// An existing `<pid>.sock` is removed only when it is a socket nothing answers
/// on: a live listener there (a session in a sibling pid namespace that shares
/// this directory can hold the same pid) is a collision and is not touched.
fn bind_reply(socket_dir: &Path, probe_busy: &Arc<AtomicBool>) -> Result<Reply, String> {
    let path = socket_dir.join(format!("{}.sock", std::process::id()));
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() => {
            let probe = run_within(
                probe_busy,
                {
                    let path = path.clone();
                    move || UnixStream::connect(path)
                },
                PROBE_TIMEOUT,
            );
            if probe_is_live(&probe) {
                return Err(format!(
                    "{} is held by a live listener (a session in another pid namespace?)",
                    path.display()
                ));
            }
            let _ = fs::remove_file(&path);
        }
        Ok(_) => return Err(format!("{} exists and is not a socket", path.display())),
        Err(_) => {}
    }
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ready = fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .and_then(|()| listener.set_nonblocking(true));
    if let Err(e) = ready {
        let _ = fs::remove_file(&path);
        return Err(format!("{}: {e}", path.display()));
    }
    let shared = Arc::new(Shared::default());
    let thread = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || receive(&listener, &shared))
    };
    Ok(Reply {
        path,
        shared,
        thread: Some(thread),
    })
}

/// Whether a probe connect shows something listening: it connected, or it timed
/// out (a full backlog is a live listener). Refused or not found is stale.
fn probe_is_live(probe: &io::Result<UnixStream>) -> bool {
    match probe {
        Ok(_) => true,
        Err(e) => !matches!(
            e.kind(),
            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
        ),
    }
}

/// The receipt thread: reads one line from each connection to the reply socket
/// and queues the receipts, until told to stop. A connection that sends nothing (a
/// liveness check) or junk queues nothing.
fn receive(listener: &UnixListener, shared: &Shared) {
    while !shared.stop.load(Ordering::Relaxed) {
        let Ok((conn, _)) = listener.accept() else {
            thread::sleep(RECEIVER_IDLE);
            continue;
        };
        // On macOS accepted sockets inherit O_NONBLOCK; the bounded read
        // below needs blocking mode for SO_RCVTIMEO to take effect.
        if conn.set_nonblocking(false).is_err() {
            continue;
        }
        if conn.set_read_timeout(Some(RECEIPT_READ_TIMEOUT)).is_err() {
            // macOS may reject SO_RCVTIMEO when the sender already closed.
            // Drain queued bytes without ever blocking in that case.
            if conn.set_nonblocking(true).is_err() {
                continue;
            }
        }
        // Bytes, not a `String`: a line that is not UTF-8 is drift to count, not a read
        // error to skip past.
        let mut line = Vec::new();
        if BufReader::new(conn.take(MAX_RECEIPT_BYTES))
            .read_until(b'\n', &mut line)
            .is_err()
        {
            continue;
        }
        match parse_control_line(&String::from_utf8_lossy(&line)) {
            ControlLine::Receipt(receipt) => {
                let mut queue = shared.queue();
                if queue.len() >= MAX_QUEUED_RECEIPTS {
                    queue.pop_front();
                }
                queue.push_back(receipt);
                drop(queue);
                shared.arrived.notify_all();
            }
            ControlLine::KnownIgnored(_) => {}
            ControlLine::Unrecognised { reason } => {
                if bump(&shared.unrecognised) == 0 {
                    eprintln!(
                        "relay-watcher: socket_wire_drift: a reply-socket line is JSON in a shape \
                         this build does not know ({reason:?}; claude-wire captured on {}); \
                         counted, further ones are not logged",
                        claude_wire::CAPTURED_ON
                    );
                }
            }
            ControlLine::NotJson => {
                if bump(&shared.not_json) == 0 {
                    eprintln!(
                        "relay-watcher: socket_wire_drift: a reply-socket line is not JSON \
                         (claude-wire captured on {}); counted, further ones are not logged",
                        claude_wire::CAPTURED_ON
                    );
                }
            }
        }
    }
}

/// What a dead watcher left that its successor takes over.
#[derive(Default)]
struct Adopted {
    held_batches: Vec<HeldBatch>,
    owed: Vec<OwedSettlement>,
    dead_lettered: Vec<DeadLetterNote>,
}

/// Whether the watcher that wrote `status` is gone: its pid has no process, or the
/// process there started at another time (the pid was reused).
fn watcher_is_gone(status: &SocketStatus, procs: &dyn Procs) -> bool {
    match (&status.start_time, procs.start_time(status.pid)) {
        (_, None) => true,
        (Some(then), Some(now)) => *then != now,
        // A status without a start time is judged by the pid alone.
        (None, Some(_)) => false,
    }
}

/// Removes what dead watchers left: for each status file of another watcher that is
/// gone, the reply socket it recorded (only when the name is that watcher's
/// `<pid>.sock` in this socket directory and it is a socket) and the file. Claude
/// Code's own sockets share the directory and are never touched. What the dead
/// watcher was holding is returned for this one to take over. A watcher killed
/// by SIGKILL leaves its socket until this runs at the next start.
fn remove_stale(config: &SocketConfig, procs: &dyn Procs) -> Adopted {
    let mut adopted = Adopted::default();
    let Some(dir) = config.status_dir.as_deref() else {
        return adopted;
    };
    for (file, read) in socket_status::read_all(dir) {
        let Ok(previous) = read else { continue };
        if previous.pid == std::process::id() || !watcher_is_gone(&previous, procs) {
            continue;
        }
        if let Some(path) = previous.reply_socket.as_ref().map(PathBuf::from) {
            let is_theirs = path.parent() == Some(config.socket_dir.as_path())
                && path
                    .file_name()
                    .is_some_and(|n| n == format!("{}.sock", previous.pid).as_str());
            let is_socket = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_socket());
            if is_theirs && is_socket {
                let _ = fs::remove_file(&path);
            }
        }
        adopted.held_batches.extend(previous.held_batches);
        adopted.owed.extend(previous.owed);
        adopted.dead_lettered.extend(previous.dead_lettered);
        let _ = fs::remove_file(file);
    }
    adopted
}
