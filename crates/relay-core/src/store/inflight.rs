//! The in-flight list: a popped batch stays recorded until its delivery is
//! acknowledged (FR-020, FR-040, AGE-2032).
//!
//! # Scope
//!
//! Covers what `agent-msg hook` and `relay-watcher` deliver. The opencode
//! plugin's `agent-msg inbox --json` still pops outright and is at-most-once
//! (AGE-2051 gives it a claim and an acknowledgement).
//!
//! # Keys
//!
//! | Key | Type | Holds |
//! |---|---|---|
//! | `<prefix>inflight:<lead>` | list | one row per popped entry, in the order the entries were first popped: `<batch id> <popper> <deadline ms> <entry>` |
//! | `<prefix>deadletter:<lead>` | list | entries whose redeliveries ran out, oldest first, capped |
//! | `<prefix>inflight-seq` | integer | counter batch ids are drawn from |
//!
//! Every operation is one `EVAL` (or one pipeline of them), so the pop, the
//! move to the in-flight list, the reap and the acknowledgement are each atomic.
//! Lease deadlines are on the Redis server's clock, so machines whose clocks
//! differ agree on what has expired.
//!
//! # Redelivery counts
//!
//! An entry redelivered `n` times is stored with `n` trailing spaces (a stored
//! [`crate::Message`] is JSON and never ends in one, and `serde_json` ignores
//! trailing whitespace, so every reader still decodes it). The count therefore
//! travels with the entry through the inbox, a requeue and a re-pop, and an
//! identical entry sent afresh starts at zero. Nothing else stores counts, so
//! nothing can leak or expire.
//!
//! # Failure discipline
//!
//! Redis does not undo what a script wrote before it errored. Every script
//! therefore checks the type of every key it may write before its first write,
//! and refuses with [`Error::InflightUnreadable`] having changed nothing; and it
//! writes the destination of each move before it deletes the source, so nothing
//! is ever deleted that was not first written elsewhere. A dead-letter key of the
//! wrong type does not make a lead unreadable: rows that would be dead-lettered
//! stay in flight and are counted as stuck ([`Reaped::stuck`]).
//!
//! # Order
//!
//! Messages keep their order within a batch and from one pop to the next. A
//! redelivered batch is marked and comes back on its own lease, so it may arrive
//! after newer mail (and a batch with a longer lease after one with a shorter): the
//! marker is what tells the recipient. Due rows are returned in the order the
//! entries were first popped.

use std::{collections::BTreeMap, fmt, num::NonZeroUsize, sync::LazyLock, time::Duration};

use redis::Value;

use super::{
    BudgetPopResult, LOG_CAP, MAX_INBOX_PEEK, PEEK_SCRIPT, PopResult, Relay, decode_entries,
};
use crate::{
    error::{Error, Result},
    name::AgentName,
};

/// How long a hook may hold a batch before it is redelivered.
///
/// A hook's whole budget is at most 3.3 s (`agent-msg`'s `hook_deadline_budget`), so
/// 20 s is six times the longest a live hook holds a batch: a slow delivery is not
/// redelivered, and one whose hook was killed returns by the lead's next hook after
/// at most 20 s. Longer only delays recovery; shorter risks a duplicate for a hook
/// that is merely slow.
pub const HOOK_LEASE: Duration = Duration::from_secs(20);

/// How long the watcher may hold a batch before it is redelivered.
///
/// `herdr agent prompt` times out after 10 s, so 60 s is six times the longest a
/// live delivery takes, and a watcher that crashed between the pop and the
/// prompt returns its batch to the sweep (every 15 s) within about 75 s.
pub const WATCHER_LEASE: Duration = Duration::from_secs(60);

/// One pending message decoded by a read-only inbox preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekedMessage {
    /// The message body and routing fields.
    pub message: crate::Message,
    /// Whether this message had already been delivered and requeued at least once.
    pub redelivered: bool,
}

/// A bounded, non-destructive inbox preview.
#[derive(Debug)]
pub struct PeekResult {
    /// The oldest pending messages up to the caller's bound, oldest first.
    pub messages: Vec<PeekedMessage>,
    /// Entries that were present but could not be decoded; they remain in the inbox.
    pub rejected: Vec<Error>,
    /// Total pending entries at the time of the snapshot.
    pub pending: usize,
    /// Whether some pending entries were omitted by the bound.
    pub truncated: bool,
}

/// The longest lease a [`Claim`] can hold: a longer one is clamped to it, so a
/// huge value can neither overflow a deadline nor make a row that cannot be read.
pub const MAX_LEASE: Duration = Duration::from_secs(24 * 60 * 60);

/// How many times an entry whose lease expired is returned to the inbox before
/// the next expiry sends it to the dead-letter list: at most four deliveries.
pub const MAX_REDELIVERIES: u32 = 3;

/// The most entries a lead's dead-letter list keeps (the newest).
pub const DEAD_LETTER_CAP: usize = 1_000;

/// The most unacknowledged batches one lead may have in flight: a pop that would
/// make one more takes nothing.
pub const MAX_INFLIGHT_BATCHES: usize = 16;

/// The most bytes of in-flight rows one lead may have: a pop that would take it
/// past this takes nothing. What a pop would add is counted as each entry's
/// length plus [`INFLIGHT_ROW_OVERHEAD`], which is never less than the rows it
/// would write.
pub const MAX_INFLIGHT_BYTES: usize = 1024 * 1024;

/// What the byte cap counts on top of each entry's length: the row header
/// (`<batch id> <popper> <deadline> `) is at most 20 + 1 + 1 + 1 + 20 + 1 bytes.
pub const INFLIGHT_ROW_OVERHEAD: usize = 64;

/// The most keys a scan for stranded in-flight rows looks at, per pattern.
pub const BACKLOG_SCAN_BOUND: usize = 1_000;

/// A script body as sent: the shared helpers and the script's own text, with the
/// comments, indentation and blank lines removed (a hook sends the pop's body on
/// every call, so the bytes are worth saving; Lua does not care).
fn script(common: &str, own: &str) -> String {
    common
        .lines()
        .chain(own.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

static POP_SCRIPT: LazyLock<String> =
    LazyLock::new(|| script(include_str!("lua/common.lua"), include_str!("lua/pop.lua")));
static ACK_SCRIPT: LazyLock<String> =
    LazyLock::new(|| script(include_str!("lua/common.lua"), include_str!("lua/ack.lua")));
pub(super) static REQUEUE_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    script(
        include_str!("lua/common.lua"),
        include_str!("lua/requeue.lua"),
    )
});
static DEAD_LETTER_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    script(
        include_str!("lua/common.lua"),
        include_str!("lua/deadletter.lua"),
    )
});
static RENEW_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    script(
        include_str!("lua/common.lua"),
        include_str!("lua/renew.lua"),
    )
});
static REAP_SCRIPT: LazyLock<String> =
    LazyLock::new(|| script(include_str!("lua/common.lua"), include_str!("lua/reap.lua")));

/// Who popped a batch: recorded with it, and picks its default lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popper {
    /// `agent-msg hook`, delivering on stdout.
    Hook,
    /// `relay-watcher`, typing into the pane.
    Watcher,
}

impl Popper {
    fn letter(self) -> char {
        match self {
            Self::Hook => 'h',
            Self::Watcher => 'w',
        }
    }

    /// How long this popper holds a batch by default.
    #[must_use]
    pub const fn lease(self) -> Duration {
        match self {
            Self::Hook => HOOK_LEASE,
            Self::Watcher => WATCHER_LEASE,
        }
    }
}

/// The clock a lease is measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Now {
    /// The Redis server's own clock (`TIME`): what every real caller uses.
    #[default]
    Server,
    /// This Unix time in milliseconds: how a test expires a lease without
    /// waiting for it.
    AtMs(i64),
}

impl Now {
    /// The script argument: empty for the server's clock.
    fn arg(self) -> String {
        match self {
            Self::Server => String::new(),
            Self::AtMs(ms) => ms.to_string(),
        }
    }
}

/// How a pop claims what it takes: who is popping, for how long it may hold the
/// batch, and on which clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claim {
    popper: Popper,
    lease: Duration,
    now: Now,
}

impl Claim {
    /// `popper`'s own lease, on the server's clock.
    #[must_use]
    pub const fn new(popper: Popper) -> Self {
        Self {
            popper,
            lease: popper.lease(),
            now: Now::Server,
        }
    }

    /// This claim with another lease, clamped to [`MAX_LEASE`].
    #[must_use]
    pub fn with_lease(mut self, lease: Duration) -> Self {
        self.lease = lease.min(MAX_LEASE);
        self
    }

    /// This claim measured on another clock.
    #[must_use]
    pub const fn at(mut self, now: Now) -> Self {
        self.now = now;
        self
    }

    /// How long the batch may stay unacknowledged.
    #[must_use]
    pub const fn lease(&self) -> Duration {
        self.lease
    }
}

/// The id of one popped batch: what an acknowledgement names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatchId(u64);

impl BatchId {
    /// The batch numbered `id`, as an acknowledgement or a test names one.
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// The number, for logs.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for BatchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What one reap did to one lead.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reaped {
    /// Entries returned to the front of the inbox.
    pub requeued: usize,
    /// Entries moved to the dead-letter list, their redeliveries spent.
    pub dead: usize,
    /// Entries whose redeliveries are spent and that could not be dead-lettered
    /// (the dead-letter key holds the wrong type): left in flight.
    pub stuck: usize,
    /// Older dead letters dropped to keep the list at [`DEAD_LETTER_CAP`].
    pub evicted: usize,
}

/// What a sweep found ([`Relay::reap_and_inbox_lens`]).
#[derive(Debug)]
pub struct Sweep {
    /// What the reap did to each lead it was asked about, in that order.
    pub reaped: Vec<Result<Reaped>>,
    /// Each asked-about inbox's length after the reap, in the order given.
    pub lens: Vec<Result<usize>>,
}

/// One lead's in-flight and dead-letter entry counts, as found by
/// [`Relay::backlog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeadBacklog {
    /// The lead's key suffix: a registered name, or one that no longer is.
    pub lead: String,
    /// Entries in flight.
    pub in_flight: usize,
    /// Dead letters.
    pub dead: usize,
    /// Whether one of its two keys holds a value of the wrong type, so the counts
    /// above cannot be trusted (shown as unreadable, not as zero).
    pub unreadable: bool,
}

/// Every lead with in-flight entries or dead letters, registered or not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backlog {
    /// The leads, by name.
    pub leads: Vec<LeadBacklog>,
    /// Whether the scan stopped at [`BACKLOG_SCAN_BOUND`] keys and may have
    /// missed some.
    pub truncated: bool,
}

impl Relay {
    /// Reads up to `max` pending messages without removing or acknowledging them.
    ///
    /// Requests are capped at [`MAX_INBOX_PEEK`]. Redelivery state is taken from
    /// the count encoded with each inbox row; a corrupt message is reported but
    /// remains pending for an explicit consumer or operator repair.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] for a failed read, or [`Error::InflightUnreadable`] when
    /// this lead's inbox key has the wrong Redis type.
    pub fn peek_inbox(&mut self, agent: &AgentName, max: NonZeroUsize) -> Result<PeekResult> {
        let requested = max.get().min(MAX_INBOX_PEEK);
        let (status, pending, raw): (i64, usize, Vec<Vec<u8>>) = redis::cmd("EVAL")
            .arg(PEEK_SCRIPT)
            .arg(1)
            .arg(self.keys.inbox_key(agent))
            .arg(requested)
            .query(&mut self.conn)?;
        if status < 0 {
            return Err(Error::InflightUnreadable {
                lead: agent.clone(),
            });
        }
        let key = self.keys.inbox_key(agent);
        let mut messages = Vec::new();
        let mut rejected = Vec::new();
        for entry in raw {
            let core_len = entry
                .iter()
                .rposition(|byte| *byte != b' ')
                .map_or(0, |index| index + 1);
            match serde_json::from_slice::<crate::Message>(&entry[..core_len]) {
                Ok(message) => messages.push(PeekedMessage {
                    message,
                    redelivered: core_len < entry.len(),
                }),
                Err(source) => rejected.push(Error::Decode {
                    key: key.clone(),
                    source,
                }),
            }
        }
        Ok(PeekResult {
            messages,
            rejected,
            pending,
            truncated: pending > requested,
        })
    }

    /// Removes and returns pending messages for `agent`, oldest first, keeping
    /// the running total of their *rendered* size at or under `max_bytes` —
    /// except the first message, which is always taken even if it alone exceeds
    /// the budget, so a caller is never stuck behind one oversized entry.
    /// Never takes more than `max_count`.
    ///
    /// What it takes is not gone: the same script records the batch as in
    /// flight under `claim`, and it stays there until [`Relay::ack`] names its
    /// [`BudgetPopResult::batch`], or [`Relay::requeue_batch`] puts it back, or
    /// its lease runs out (see the module docs). Before it pops, the script
    /// returns every due batch of `agent` to the front of the inbox, so those
    /// are what this pop takes first. A pop that would put more than
    /// [`MAX_INFLIGHT_BATCHES`] batches or [`MAX_INFLIGHT_BYTES`] bytes in
    /// flight takes nothing and sets [`BudgetPopResult::at_cap`].
    ///
    /// The budget exists to bound what an agent is shown, so the batch is
    /// counted the way it is printed: each message weighs at least
    /// `Message::rendered_len_max` (its signature with the longest sender
    /// label and the redelivery marker, its text with every line feed
    /// indented, and the byte that joins it to the next). The script cannot
    /// call Rust, so it computes a weight from the stored JSON that is never
    /// smaller: the JSON length, plus the fixed overhead the rendering can add
    /// over the smallest stored form, plus what the text grows by when rendered
    /// (`\n` is 2 bytes stored and 5 printed; `\r`, `\b`, `\f`, DEL, other C1
    /// and raw control bytes and U+061C are shorter than the 3-byte replacement
    /// they print as). Text this over-counts only makes a batch a little
    /// smaller. So a batch printed with [`crate::AddressBook::render_marked`]
    /// and joined by one newline per gap is at most `max_bytes` long, unless its
    /// first message alone is longer.
    ///
    /// One Lua script does the reap, the peek, the budget walk and the move, so
    /// it is a single atomic Redis command: concurrent consumers never see the
    /// same message twice, and nothing leaves the inbox that is not recorded in
    /// flight. A call that fails *before* Redis runs it changes nothing; one
    /// whose reply is lost or late (a read timeout) has already recorded the
    /// batch, which the lease returns. It is always exactly one round trip.
    ///
    /// [`BudgetPopResult::pending`] is how many messages remain in the inbox
    /// right after the pop, so a caller can tell the agent more is waiting.
    ///
    /// An undecodable entry is taken and reported in `rejected`, in the same
    /// batch: acknowledging the batch drops it, as before.
    ///
    /// # Errors
    ///
    /// [`Error::InflightUnreadable`] when the inbox, the in-flight list or the
    /// batch counter holds a value of the wrong type: nothing was changed.
    pub fn pop_inbox_budget(
        &mut self,
        agent: &AgentName,
        max_count: NonZeroUsize,
        max_bytes: usize,
        claim: Claim,
    ) -> Result<BudgetPopResult> {
        let key = self.keys.inbox_key(agent);
        let lease_ms = u64::try_from(claim.lease.as_millis()).unwrap_or(u64::MAX);
        // `EVAL` with the body, never `EVALSHA`: `Script::invoke` falls back to
        // `SCRIPT LOAD` plus a second `EVALSHA` on a cold server, three
        // sequential replies where a hook's pop-safety window
        // (`agent-msg`'s `pop_window`) assumes one.
        let (status, pending, id, raw, counts, stuck, evicted): PopReply = redis::cmd("EVAL")
            .arg(POP_SCRIPT.as_str())
            .arg(4)
            .arg(&key)
            .arg(self.keys.inflight_key(agent))
            .arg(self.keys.dead_letter_key(agent))
            .arg(self.keys.inflight_seq_key())
            .arg(max_count.get())
            .arg(max_bytes)
            .arg(crate::message::WEIGHT_SLACK)
            .arg(claim.popper.letter().to_string())
            .arg(lease_ms)
            .arg(claim.now.arg())
            .arg(MAX_REDELIVERIES)
            .arg(DEAD_LETTER_CAP)
            .arg(MAX_INFLIGHT_BATCHES)
            .arg(MAX_INFLIGHT_BYTES)
            .arg(INFLIGHT_ROW_OVERHEAD)
            .query(&mut self.conn)?;
        if status < 0 {
            return Err(Error::InflightUnreadable {
                lead: agent.clone(),
            });
        }
        let (messages, redelivered, rejected) = decode_claimed(&key, raw, &counts);
        Ok(BudgetPopResult {
            messages,
            redelivered,
            rejected,
            pending,
            batch: (id != 0).then_some(BatchId(id)),
            at_cap: status == 1,
            stuck,
            evicted,
        })
    }

    /// Acknowledges a delivery: deletes exactly the in-flight batch `batch` that `popper`
    /// popped (another consumer's batch under the same id is left alone).
    /// Returns whether it deleted anything: `false` for a batch already
    /// acknowledged, requeued or returned by its lease expiring, which an
    /// acknowledgement must not disturb (a stale acknowledgement never deletes a
    /// newer batch: ids are never reused).
    ///
    /// # Errors
    ///
    /// [`Error::InflightUnreadable`] when the in-flight key holds a value of
    /// the wrong type.
    pub fn ack(&mut self, agent: &AgentName, batch: BatchId, popper: Popper) -> Result<bool> {
        let removed: i64 = redis::cmd("EVAL")
            .arg(ACK_SCRIPT.as_str())
            .arg(1)
            .arg(self.keys.inflight_key(agent))
            .arg(batch.0)
            .arg(popper.letter().to_string())
            .query(&mut self.conn)?;
        if removed < 0 {
            return Err(Error::InflightUnreadable {
                lead: agent.clone(),
            });
        }
        Ok(removed > 0)
    }

    /// Puts the in-flight batch `batch` back at the front of `agent`'s inbox in
    /// its original order, for a consumer that knows the delivery did not
    /// happen. Not a redelivery: the entries keep the counts they had. Returns
    /// how many entries moved: none for a batch that is no longer in flight.
    ///
    /// Also appends a wake entry (field `requeued`, no `msg`) to the message log
    /// in the same script, so a watcher blocked on the log looks at the inbox now
    /// instead of at its next sweep; [`Relay::log`] skips it.
    ///
    /// # Errors
    ///
    /// [`Error::InflightUnreadable`] when the in-flight list, the inbox or the
    /// log holds a value of the wrong type: nothing was changed.
    pub fn requeue_batch(
        &mut self,
        agent: &AgentName,
        batch: BatchId,
        popper: Popper,
    ) -> Result<usize> {
        let moved: i64 = redis::cmd("EVAL")
            .arg(REQUEUE_SCRIPT.as_str())
            .arg(3)
            .arg(self.keys.inflight_key(agent))
            .arg(self.keys.inbox_key(agent))
            .arg(self.keys.log_key())
            .arg(batch.0)
            .arg(agent.as_str())
            .arg(LOG_CAP)
            .arg(popper.letter().to_string())
            .query(&mut self.conn)?;
        usize::try_from(moved).map_err(|_| Error::InflightUnreadable {
            lead: agent.clone(),
        })
    }

    /// Moves the in-flight batch `batch` (popped by `popper`; another consumer's batch
    /// under the same id is left alone) to `agent`'s dead-letter list, for a
    /// consumer that knows it will never be delivered (a receiver that denied it).
    /// The entries keep their text; the caller records why. Returns how many
    /// entries moved: none for a batch that is no longer in flight.
    ///
    /// # Errors
    ///
    /// [`Error::InflightUnreadable`] when the in-flight or dead-letter key holds a
    /// value of the wrong type: nothing was changed.
    pub fn dead_letter_batch(
        &mut self,
        agent: &AgentName,
        batch: BatchId,
        popper: Popper,
    ) -> Result<usize> {
        let moved: i64 = redis::cmd("EVAL")
            .arg(DEAD_LETTER_SCRIPT.as_str())
            .arg(2)
            .arg(self.keys.inflight_key(agent))
            .arg(self.keys.dead_letter_key(agent))
            .arg(batch.0)
            .arg(DEAD_LETTER_CAP)
            .arg(popper.letter().to_string())
            .query(&mut self.conn)?;
        usize::try_from(moved).map_err(|_| Error::InflightUnreadable {
            lead: agent.clone(),
        })
    }

    /// Extends the lease of the in-flight batch `batch` (popped by `popper`; another
    /// consumer's batch under the same id is left alone) to `lease` from `now`
    /// (never shortening it, and at most [`MAX_LEASE`]), for a consumer that knows the
    /// batch is being held elsewhere (a receiver's approval prompt) and must not have
    /// it returned to the inbox or popped by another consumer meanwhile. Returns how
    /// many entries it extended: none for a batch that is no longer in flight, which
    /// is left untouched.
    ///
    /// # Errors
    ///
    /// [`Error::InflightUnreadable`] when the in-flight key holds a value of the wrong
    /// type: nothing was changed.
    pub fn renew_batch(
        &mut self,
        agent: &AgentName,
        batch: BatchId,
        lease: Duration,
        now: Now,
        popper: Popper,
    ) -> Result<usize> {
        let moved: i64 = redis::cmd("EVAL")
            .arg(RENEW_SCRIPT.as_str())
            .arg(1)
            .arg(self.keys.inflight_key(agent))
            .arg(batch.0)
            .arg(now.arg())
            .arg(u64::try_from(lease.as_millis()).unwrap_or(u64::MAX))
            .arg(u64::try_from(MAX_LEASE.as_millis()).unwrap_or(u64::MAX))
            .arg(popper.letter().to_string())
            .query(&mut self.conn)?;
        usize::try_from(moved).map_err(|_| Error::InflightUnreadable {
            lead: agent.clone(),
        })
    }

    /// The reap for `agent` alone, on `now`'s clock.
    ///
    /// # Errors
    ///
    /// [`Error::InflightUnreadable`] when the inbox or the in-flight list holds
    /// a value of the wrong type: nothing was changed.
    pub fn reap_expired(&mut self, agent: &AgentName, now: Now) -> Result<Reaped> {
        let replies: Vec<ReapReply> = self.reap_cmd(&[agent], now).query(&mut self.conn)?;
        reaped_of(agent, replies.first().copied())
    }

    /// One `EVAL` that reaps every lead of `agents`, its three keys each.
    fn reap_cmd(&self, agents: &[&AgentName], now: Now) -> redis::Cmd {
        let mut cmd = redis::cmd("EVAL");
        cmd.arg(REAP_SCRIPT.as_str()).arg(agents.len() * 3);
        for agent in agents {
            cmd.arg(self.keys.inbox_key(agent))
                .arg(self.keys.inflight_key(agent))
                .arg(self.keys.dead_letter_key(agent));
        }
        cmd.arg(now.arg())
            .arg(MAX_REDELIVERIES)
            .arg(DEAD_LETTER_CAP);
        cmd
    }

    /// One request that reaps every lead in `reap` and then reads the length of
    /// every inbox in `lens`: the watcher's sweep pays one round trip for both,
    /// and the lengths already include what the reap returned. A lead whose keys
    /// hold a value of the wrong type is that entry's [`Error::InflightUnreadable`]
    /// (nothing of it was changed); only a failure of the connection itself fails
    /// the call.
    pub fn reap_and_inbox_lens(
        &mut self,
        reap: &[&AgentName],
        lens: &[&AgentName],
        now: Now,
    ) -> Result<Sweep> {
        let mut pipe = redis::pipe();
        pipe.ignore_errors();
        if !reap.is_empty() {
            pipe.add_command(self.reap_cmd(reap, now));
        }
        for agent in lens {
            pipe.cmd("LLEN").arg(self.keys.inbox_key(agent));
        }
        if reap.is_empty() && lens.is_empty() {
            return Ok(Sweep {
                reaped: Vec::new(),
                lens: Vec::new(),
            });
        }
        let mut values: Vec<redis::RedisResult<Value>> = pipe.query(&mut self.conn)?;
        let lens_values = values.split_off(usize::from(!reap.is_empty()));
        let reaped = match values.pop() {
            None => Vec::new(),
            Some(reply) => match convert::<Vec<ReapReply>>(reply) {
                Ok(replies) => reap
                    .iter()
                    .zip(replies.into_iter().map(Some).chain(std::iter::repeat(None)))
                    .map(|(agent, reply)| reaped_of(agent, reply))
                    .collect(),
                // The whole script failed (not one lead): the first lead reports
                // why, and the rest that they could not be read.
                Err(e) => {
                    let mut first = Some(e);
                    reap.iter()
                        .map(|agent| {
                            first.take().map_or_else(
                                || {
                                    Err(Error::InflightUnreadable {
                                        lead: (*agent).clone(),
                                    })
                                },
                                Err,
                            )
                        })
                        .collect()
                }
            },
        };
        Ok(Sweep {
            reaped,
            lens: lens_values.into_iter().map(convert).collect(),
        })
    }

    /// The most recent `max` dead letters of `agent`, oldest first, without
    /// removing them. An entry that does not decode is reported in `rejected`.
    pub fn dead_letters(&mut self, agent: &AgentName, max: NonZeroUsize) -> Result<PopResult> {
        let key = self.keys.dead_letter_key(agent);
        let stop = i64::try_from(max.get()).unwrap_or(i64::MAX);
        let raw: Vec<Vec<u8>> = redis::cmd("LRANGE")
            .arg(&key)
            .arg(-stop)
            .arg(-1)
            .query(&mut self.conn)?;
        let (messages, rejected) = decode_entries(&key, raw);
        Ok(PopResult { messages, rejected })
    }

    /// How many entries (not batches) are in flight for `agent`.
    pub fn inflight_len(&mut self, agent: &AgentName) -> Result<usize> {
        let len: usize = redis::cmd("LLEN")
            .arg(self.keys.inflight_key(agent))
            .query(&mut self.conn)?;
        Ok(len)
    }

    /// Every lead with in-flight entries or dead letters, found by scanning the
    /// keys, so that a lead that was pruned or renamed while it had a batch in
    /// flight is listed too. The scan looks at no more than
    /// [`BACKLOG_SCAN_BOUND`] keys per kind; past that the answer is marked
    /// [`Backlog::truncated`].
    pub fn backlog(&mut self) -> Result<Backlog> {
        let mut found: BTreeMap<String, (usize, usize, bool)> = BTreeMap::new();
        let mut truncated = false;
        for dead in [false, true] {
            let head = if dead {
                self.keys.dead_letter_stem()
            } else {
                self.keys.inflight_stem()
            };
            let scanned = self.scan_prefixed(&head, BACKLOG_SCAN_BOUND)?;
            truncated |= scanned.cut.is_some();
            let keys = scanned.keys;
            let mut pipe = redis::pipe();
            pipe.ignore_errors();
            for key in &keys {
                pipe.cmd("LLEN").arg(key);
            }
            let lens: Vec<redis::RedisResult<usize>> = if keys.is_empty() {
                Vec::new()
            } else {
                pipe.query(&mut self.conn)?
            };
            for (key, len) in keys.iter().zip(lens) {
                let Some(lead) = key.strip_prefix(&head) else {
                    continue;
                };
                let entry = found.entry(lead.to_owned()).or_default();
                let Ok(len) = len else {
                    entry.2 = true;
                    continue;
                };
                if dead {
                    entry.1 = len;
                } else {
                    entry.0 = len;
                }
            }
        }
        Ok(Backlog {
            leads: found
                .into_iter()
                .filter(|(_, (in_flight, dead, unreadable))| in_flight + dead > 0 || *unreadable)
                .map(|(lead, (in_flight, dead, unreadable))| LeadBacklog {
                    lead,
                    in_flight,
                    dead,
                    unreadable,
                })
                .collect(),
            truncated,
        })
    }
}

/// The pop script's reply: status, pending, batch id, entries, redelivery
/// counts, stuck rows, evicted dead letters.
type PopReply = (i64, usize, u64, Vec<Vec<u8>>, Vec<u32>, usize, usize);

/// One lead's reply of the reap script: requeued (`-1` for an unreadable lead),
/// dead-lettered, stuck, evicted.
type ReapReply = (i64, usize, usize, usize);

/// [`decode_entries`] for a claimed batch, keeping each decoded message's
/// redelivery flag aligned with it.
fn decode_claimed(
    key: &str,
    raw: Vec<Vec<u8>>,
    counts: &[u32],
) -> (Vec<crate::Message>, Vec<bool>, Vec<Error>) {
    let mut messages = Vec::new();
    let mut redelivered = Vec::new();
    let mut rejected = Vec::new();
    for (entry, count) in raw.into_iter().zip(counts) {
        match serde_json::from_slice(&entry) {
            Ok(m) => {
                messages.push(m);
                redelivered.push(*count > 0);
            }
            Err(source) => rejected.push(Error::Decode {
                key: key.to_owned(),
                source,
            }),
        }
    }
    (messages, redelivered, rejected)
}

/// One lead's reply of the reap script, as this crate's answer.
fn reaped_of(agent: &AgentName, reply: Option<ReapReply>) -> Result<Reaped> {
    reply
        .and_then(|(requeued, dead, stuck, evicted)| {
            Some(Reaped {
                requeued: usize::try_from(requeued).ok()?,
                dead,
                stuck,
                evicted,
            })
        })
        .ok_or_else(|| Error::InflightUnreadable {
            lead: agent.clone(),
        })
}

/// One entry of a pipelined reply, decoded, its failure as this crate's error.
fn convert<T: redis::FromRedisValue>(value: redis::RedisResult<Value>) -> Result<T> {
    redis::from_redis_value(value?).map_err(|e| Error::Redis(redis::RedisError::from(e)))
}
