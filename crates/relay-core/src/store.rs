//! The Redis-backed bus.
//!
//! Keys, all under one prefix:
//!
//! | Key | Type | Holds |
//! |---|---|---|
//! | `<prefix>inbox:<agent>` | list | pending [`Message`] JSON, oldest first |
//! | `<prefix>inflight:<agent>`, `deadletter:<agent>`, `inflight-seq` | list, list, integer | popped-but-unacknowledged batches and what becomes of them, see the `inflight` module |
//! | `<prefix>agents` | hash | agent name → [`Registration`] JSON |
//! | `<prefix>log` | stream, capped | every sent [`Message`] JSON under field `msg` |
//! | `<prefix>esc:seq` | integer | counter [`Relay::escalate`] draws ids from |
//! | `<prefix>esc:<id>` | hash | one [`Escalation`]: `meta` (JSON), plus plain `status`/`decision` fields a transition script mutates without touching the JSON |
//! | `<prefix>esc:index` | sorted set | escalation id → `created_at_ms`, for [`Relay::list`] |
//! | `<prefix>health:watcher:<machine>` | string with expiry | the machine's [`crate::health::WatcherHeartbeat`] JSON; absent means no watcher has ticked there recently |
//! | `<prefix>health:moves` | stream, capped | one [`crate::moves::MoveEvent`] per relink or rename, written by the script that changes the registration: fields `machine`, `lead`, `rec` (FR-043) |
//! | `<prefix>health:replaced:<lead>` | string, expiring | one lead's [`crate::moves::PaneReplaced`] JSON: the lead's pane now hosts another session (FR-032) |
//! | `<prefix>health:hooks` | stream, capped | hook failure records flushed from each machine's local spool: fields `machine`, `lead`, `rec` (see [`Relay::push_hook_health`]) |

use std::{cell::Cell, collections::HashMap, num::NonZeroUsize, time::Duration};

use redis::{Commands, IntoConnectionInfo, streams::StreamRangeReply};

use crate::{
    error::{Error, Result},
    escalation::{
        Escalation, EscalationFiling, EscalationId, EscalationList, EscalationMeta,
        EscalationStatus,
    },
    ids::{MachineName, PaneId, PaneRef, SessionId},
    message::{Message, Registration},
    name::{AgentName, Target},
};

/// Key prefix used in production. Tests use their own so they can share one Redis.
pub const DEFAULT_PREFIX: &str = "ixr:v3:";

/// Roughly how many entries the history stream keeps (`XADD MAXLEN ~`).
const LOG_CAP: usize = 10_000;

/// Roughly how many entries the hook health stream keeps (`XADD MAXLEN ~`).
const HEALTH_HOOKS_CAP: usize = 1_000;

/// How long a session's remembered name or suppression lasts: 30 days, in
/// seconds. A harness session id is one-shot (never reused across restarts),
/// so entries would otherwise accumulate forever with nothing to clean them up.
const SESSION_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// How long a recorded Herdr failure is deduplicated: five minutes.
/// This never stops session verification or enrollment.
pub const HERDR_FAILED_TTL_SECS: u64 = 300;

mod commit;
mod health;
mod inflight;
#[cfg(test)]
mod inflight_fault_tests;
mod keys;
mod moves;
#[cfg(test)]
mod pop_budget_tests;
mod scan;
mod work;

use keys::Keys;

pub use commit::{CommitOutcome, IdentityCommit};
pub use health::{
    Heartbeats, InboxOverview, LeadOverview, LeaseState, MAX_HEARTBEAT_KEYS, RecipientProbe,
};
pub use inflight::{
    BACKLOG_SCAN_BOUND, Backlog, BatchId, Claim, DEAD_LETTER_CAP, HOOK_LEASE,
    INFLIGHT_ROW_OVERHEAD, LeadBacklog, MAX_INFLIGHT_BATCHES, MAX_INFLIGHT_BYTES, MAX_LEASE,
    MAX_REDELIVERIES, Now, Popper, Reaped, Sweep, WATCHER_LEASE,
};
pub use moves::{Moves, PanesReplaced, PlaceRefresh};
pub use scan::ScanCut;

/// Largest `text` a single [`Message`] may carry, in bytes. Enforced by
/// [`Relay::send`]. Keeps one message small enough that a budgeted pop (see
/// [`Relay::pop_inbox_budget`]) can always deliver at least one message
/// whole, never split mid-text.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024;

/// A connection to the bus.
pub struct Relay {
    conn: redis::Connection,
    keys: Keys,
    /// The read/write timeout currently armed on `conn`, so a blocking call
    /// can widen it and put it back.
    timeout: Cell<Duration>,
    /// When a key scan must stop, if its caller has a fixed budget (see
    /// [`Relay::set_scan_deadline`]).
    scan_deadline: Cell<Option<std::time::Instant>>,
}

impl Relay {
    /// Connects to `url` (e.g. `redis://127.0.0.1:6379`). `timeout` bounds the
    /// connect and every later read and write, so a dead tunnel fails fast instead
    /// of stalling the hook that called us.
    ///
    /// Connecting sends no command: redis-rs's `CLIENT SETINFO` handshake is
    /// skipped, because it is a whole extra round trip on every connection and
    /// a hook connects on every call (AGE-2028). The only cost is that the server's
    /// `CLIENT LIST` shows no library name.
    pub fn connect(url: &str, timeout: Duration) -> Result<Self> {
        let mut info = url.into_connection_info()?;
        let settings = info.redis_settings().clone().set_skip_set_lib_name();
        info = info.set_redis_settings(settings);
        let conn = redis::Client::open(info)?.get_connection_with_timeout(timeout)?;
        conn.set_read_timeout(Some(timeout))?;
        conn.set_write_timeout(Some(timeout))?;
        Ok(Self {
            conn,
            keys: Keys::new(DEFAULT_PREFIX),
            timeout: Cell::new(timeout),
            scan_deadline: Cell::new(None),
        })
    }

    /// Checks that `url` is a well-formed Redis URL without connecting, so a
    /// long-running caller can refuse a bad one at start-up instead of failing
    /// every connection attempt forever.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] (an invalid-client-config error) if `url` does not parse.
    pub fn validate_url(url: &str) -> Result<()> {
        redis::Client::open(url)?;
        Ok(())
    }

    /// Uses `prefix` instead of [`DEFAULT_PREFIX`] for every key.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.keys = Keys::new(prefix);
        self
    }

    /// Re-bounds every read and write from this point on to `timeout`. Lets a
    /// caller with a fixed total budget (e.g. a hook) shrink the timeout for each
    /// later call to whatever is left, instead of paying the full timeout again
    /// per operation.
    pub fn set_timeout(&self, timeout: Duration) -> Result<()> {
        self.conn.set_read_timeout(Some(timeout))?;
        self.conn.set_write_timeout(Some(timeout))?;
        self.timeout.set(timeout);
        Ok(())
    }

    /// Adds or refreshes a session-bound lead registration.
    pub fn register(&mut self, reg: &Registration) -> Result<()> {
        let (): () = redis::cmd("HSET")
            .arg(self.keys.agents_key())
            .arg(reg.name.as_str())
            .arg(encode(reg))
            .query(&mut self.conn)?;
        Ok(())
    }

    /// Every registered lead that decodes, sorted by name, plus a report of the
    /// records that did not.
    ///
    /// One undecodable record (written by a binary whose format this one cannot
    /// read, or by hand) must not hide every other lead from a mixed-version
    /// fleet, so it is reported in [`AgentList::skipped`] and the rest come back.
    pub fn agents(&mut self) -> Result<AgentList> {
        let key = self.keys.agents_key();
        // Bytes, not `String`: a record (or field) that is not UTF-8 must be
        // skipped and reported like any other undecodable record, not fail
        // the read of every lead (FR-008-AC-4).
        let raw: Vec<(Vec<u8>, Vec<u8>)> = self.conn.hgetall(&key)?;
        let mut registrations: Vec<Registration> = Vec::with_capacity(raw.len());
        let mut skipped = Vec::new();
        for (field, json) in raw {
            match serde_json::from_slice(&json) {
                Ok(reg) => registrations.push(reg),
                Err(source) => skipped.push(SkippedRegistration {
                    field: String::from_utf8_lossy(&field).into_owned(),
                    error: Error::Decode {
                        key: key.clone(),
                        source,
                    },
                    raw: json,
                }),
            }
        }
        registrations.sort_by(|a, b| a.name.cmp(&b.name));
        skipped.sort_by(|a, b| a.field.cmp(&b.field));
        Ok(AgentList {
            registrations,
            skipped,
        })
    }

    /// One registered lead's entry, or `None` if `name` has never
    /// registered. Used by AGE-2018's derived-name collision check: before
    /// registering under a name derived from herdr location, look up
    /// whether it's already live under a different machine.
    pub fn agent(&mut self, name: &AgentName) -> Result<Option<Registration>> {
        let key = self.keys.agents_key();
        let raw: Option<Vec<u8>> = self.conn.hget(&key, name.as_str())?;
        raw.map(|json| decode_bytes(&key, &json)).transpose()
    }

    /// [`Relay::agent`] that tells "registered but unreadable" from an error:
    /// a record this binary refuses to decode (an id an older writer stored that
    /// breaks its type's rule, AGE-2027) is [`AgentSlot::Unreadable`], not a
    /// failure. The name is taken, and an older binary may still deliver to it.
    ///
    /// # Errors
    ///
    /// Any error other than [`Error::Decode`]: Redis failing or timing out.
    pub fn agent_slot(&mut self, name: &AgentName) -> Result<AgentSlot> {
        match self.agent(name) {
            Ok(None) => Ok(AgentSlot::Absent),
            Ok(Some(reg)) => Ok(AgentSlot::Registered(Box::new(reg))),
            Err(Error::Decode { .. }) => Ok(AgentSlot::Unreadable),
            Err(e) => Err(e),
        }
    }

    /// Every registration on `machine` whose `herdr_pane` is `pane`,
    /// most-recently-seen first (AGE-2018 PR review, finding 4): a command run
    /// directly in the agent's own shell, rather than through a hook, has no
    /// session transcript to re-derive a `/rename` title from — this is how it
    /// finds what the hook last registered *this pane* as instead, which does
    /// see one.
    ///
    /// Pane ids (`w1:p1`) are only unique within one machine's herdr, so the
    /// machine is part of the key: without it, two machines with the same pane
    /// id would resolve to each other's lead. A [`PaneRef`] is that pair, and
    /// its machine is normalised by construction. Ordinarily at most one registration
    /// matches; more than one (shouldn't happen — each pane hosts one lead at
    /// a time) resolves to whichever registered most recently. Records that
    /// fail to decode are skipped (see [`Relay::agents`]).
    pub fn registrations_for_pane(&mut self, pane: &PaneRef) -> Result<Vec<Registration>> {
        let mut regs = self.agents()?.registrations;
        regs.retain(|r| r.is_on(pane));
        regs.sort_by_key(|r| std::cmp::Reverse(r.last_seen_ms));
        Ok(regs)
    }

    /// Remove a registration only when its last-seen value still matches.
    /// Concurrent session refreshes are preserved.
    pub fn prune(
        &mut self,
        name: &AgentName,
        seen_last_seen_ms: i64,
    ) -> Result<Option<Vec<AgentName>>> {
        // Returns false on a changed record, otherwise an empty success payload.
        const SCRIPT: &str = r"
            local raw = redis.call('HGET', KEYS[1], ARGV[1])
            if not raw then return false end
            local ok, reg = pcall(cjson.decode, raw)
            if not ok or type(reg) ~= 'table'
                or reg.last_seen_ms ~= tonumber(ARGV[2]) then
                return false
            end
            redis.call('HDEL', KEYS[1], ARGV[1])
            return {}";
        let removed: Option<Vec<String>> = redis::Script::new(SCRIPT)
            .key(self.keys.agents_key())
            .arg(name.as_str())
            .arg(seen_last_seen_ms)
            .invoke(&mut self.conn)?;
        Ok(removed.map(|removed| {
            let mut ids: Vec<AgentName> =
                removed.into_iter().filter_map(|a| a.parse().ok()).collect();
            ids.sort();
            ids
        }))
    }

    /// Read suppression and the session queue in one atomic request/reply.
    /// A cache miss never adopts a registry record or generates a name.
    ///
    /// Reading a session also renews the expiry of its remembered name and its
    /// suppression mark (30 days from now, in the same round trip): what is in
    /// use does not expire mid-session, and what is abandoned still does.
    ///
    /// A key of the wrong type reads as absent, as the `MGET` this replaced did,
    /// so one corrupt key cannot make every hook of a session fail; only the
    /// connection failing fails the call.
    pub fn probe_session(&mut self, session_id: &SessionId) -> Result<SessionProbe> {
        // One Redis reply is essential: a pipeline with five ignored errors
        // can consume five read timeouts when the peer accepts but never answers.
        const SCRIPT: &str = r#"
local cached = redis.pcall('GET', KEYS[2])
if type(cached) == 'table' and cached.err then cached = false end
local suppressed = redis.call('EXISTS', KEYS[1])
redis.call('EXPIRE', KEYS[1], ARGV[1])
redis.call('EXPIRE', KEYS[2], ARGV[1])
return {suppressed, cached}
"#;
        let (suppressed, name): (bool, Option<String>) = redis::cmd("EVAL")
            .arg(SCRIPT)
            .arg(2)
            .arg(self.keys.suppressed_key(session_id))
            .arg(self.keys.session_key(session_id))
            .arg(SESSION_TTL_SECS)
            .query(&mut self.conn)?;
        Ok(SessionProbe {
            suppressed,
            name: name.and_then(|s| s.parse().ok()),
        })
    }

    /// Marks `session_id` as having had its herdr failure recorded for
    /// [`HERDR_FAILED_TTL_SECS`], unless it already is. Returns whether this
    /// call set the mark (record the failure) or an earlier one is still
    /// remembered (do not). Verification continues on every uncached hook, so a
    /// Herdr outage never prevents enrollment after it recovers.
    pub fn mark_session_herdr_failed(&mut self, session_id: &SessionId) -> Result<bool> {
        let key = self.keys.herdr_failed_key(session_id);
        let set: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(HERDR_FAILED_TTL_SECS)
            .query(&mut self.conn)?;
        Ok(set.is_some())
    }

    /// Appends hook failure records to the capped `<prefix>health:hooks`
    /// stream, one entry per record, in one pipelined round trip. Each entry
    /// carries the record's JSON under `rec` and, as plain fields, the
    /// machine and the lead (when known) it came from, so a reader can
    /// attribute one without parsing it. The stream keeps roughly the newest
    /// thousand.
    ///
    /// This is at-least-once: a caller that crashes after this returns but
    /// before clearing its spool sends the same records again.
    ///
    /// Record text is untrusted: a record can carry a lead's message text
    /// (see `agent-msg`'s failed-requeue record), so anything that renders
    /// this stream must neutralise it first ([`crate::text::neutralise`]).
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out. Then nothing is known
    /// to have been appended and the caller keeps its records.
    pub fn push_hook_health(&mut self, entries: &[HookHealthEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let key = self.keys.health_hooks_key();
        let mut pipe = redis::pipe();
        for entry in entries {
            pipe.cmd("XADD")
                .arg(&key)
                .arg("MAXLEN")
                .arg("~")
                .arg(HEALTH_HOOKS_CAP)
                .arg("*")
                .arg("machine")
                .arg(entry.machine.as_str())
                .arg("lead")
                .arg(entry.lead.as_deref().unwrap_or(""))
                .arg("rec")
                .arg(&entry.record)
                .ignore();
        }
        let () = pipe.query(&mut self.conn)?;
        Ok(())
    }

    /// Up to `max` entries from the hook health stream, oldest first: what
    /// [`Relay::push_hook_health`] appended.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn hook_health(&mut self, max: usize) -> Result<HookHealth> {
        let reply: StreamRangeReply =
            self.conn
                .xrange_count(self.keys.health_hooks_key(), "-", "+", max)?;
        Ok(hook_health_of(&reply))
    }

    /// Up to `max` entries from the hook health stream, **newest first**. The stream
    /// is capped approximately (`MAXLEN ~`), so it can hold more than `max`; a reader
    /// that wants the latest failures reads from this end.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn hook_health_newest(&mut self, max: usize) -> Result<HookHealth> {
        let reply: StreamRangeReply =
            self.conn
                .xrevrange_count(self.keys.health_hooks_key(), "+", "-", max)?;
        Ok(hook_health_of(&reply))
    }

    /// Queues `msg` for its target and records it in the log, in one transaction.
    /// Returns the recipient queue identities. A broadcast skips the sender.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownAgent`] when a single queue target
    /// has never registered, so a typo fails loudly instead of filling an
    /// inbox nobody reads.
    ///
    /// [`Error::MessageTooLarge`] when `msg.text` exceeds [`MAX_MESSAGE_BYTES`],
    /// checked before anything is queued.
    ///
    /// [`Error::UnsafeMessageText`] when `msg.text` holds a character
    /// [`crate::text::is_unsafe_char`] rejects, also checked before anything
    /// is queued.
    pub fn send(&mut self, msg: &Message) -> Result<Vec<AgentName>> {
        if msg.text.len() > MAX_MESSAGE_BYTES {
            return Err(Error::MessageTooLarge {
                len: msg.text.len(),
                max: MAX_MESSAGE_BYTES,
            });
        }
        if let Some((byte_offset, c)) = crate::text::first_unsafe(&msg.text) {
            return Err(Error::UnsafeMessageText {
                codepoint: u32::from(c),
                byte_offset,
            });
        }
        let agents_key = self.keys.agents_key();
        let recipients = match &msg.to {
            Target::Agent(name) => {
                let canonical = name.clone();
                let known: bool = self.conn.hexists(&agents_key, canonical.as_str())?;
                if !known {
                    return Err(Error::UnknownAgent(name.clone()));
                }
                vec![canonical]
            }
            Target::All => {
                let names: Vec<Vec<u8>> = self.conn.hkeys(&agents_key)?;
                let mut names = names
                    .into_iter()
                    .filter_map(|n| String::from_utf8(n).ok()?.parse::<AgentName>().ok())
                    .filter(|n| *n != msg.from)
                    .collect::<Vec<_>>();
                let registrations = self.agents()?.registrations;
                names.retain(|id| {
                    !registrations.iter().any(|r| {
                        &r.name == id
                            && r.full_address().is_some_and(|a| {
                                a.to_string().eq_ignore_ascii_case(msg.from.as_str())
                            })
                    })
                });
                names.sort();
                names
            }
        };

        // Snapshot the public recipient while retaining the immutable queue ID
        // solely for enqueueing. History must not change when a tab is renamed.
        let mut stored = msg.clone();
        if let Target::Agent(id) = &msg.to
            && id.as_str().starts_with("session/")
            && let Some(address) = self.agent(id)?.and_then(|r| r.full_address())
        {
            stored.to = Target::Agent(address.to_string().parse()?);
        }
        let json = encode(&stored);
        let mut pipe = redis::pipe();
        pipe.atomic();
        for name in &recipients {
            pipe.rpush(self.keys.inbox_key(name), &json).ignore();
        }
        pipe.cmd("XADD")
            .arg(self.keys.log_key())
            .arg("MAXLEN")
            .arg("~")
            .arg(LOG_CAP)
            .arg("*")
            .arg("msg")
            .arg(&json)
            .ignore();
        let () = pipe.query(&mut self.conn)?;
        Ok(recipients)
    }

    /// How many messages are waiting in `agent`'s inbox.
    ///
    /// Lets a poller (e.g. `relay-watcher`) skip the `herdr agent get` round trip
    /// for leads with nothing queued, and lets a caller (a hook, a plugin) check
    /// whether there is anything to deliver before doing something disruptive
    /// like starting a new turn.
    pub fn inbox_len(&mut self, agent: &AgentName) -> Result<usize> {
        let key = self.keys.inbox_key(agent);
        let len: usize = self.conn.llen(&key)?;
        Ok(len)
    }

    /// How many messages wait in each of `agents`' inboxes, in one pipelined
    /// request, in the order given (AGE-2028): a poller checking every lead pays
    /// one round trip, not one per lead. A failure on one inbox (a key of the
    /// wrong type) is that entry's error and leaves the others readable; only a
    /// failure of the connection itself fails the call.
    pub fn inbox_lens(&mut self, agents: &[&AgentName]) -> Result<Vec<Result<usize>>> {
        if agents.is_empty() {
            return Ok(Vec::new());
        }
        let mut pipe = redis::pipe();
        pipe.ignore_errors();
        for agent in agents {
            pipe.cmd("LLEN").arg(self.keys.inbox_key(agent));
        }
        let lens: Vec<redis::RedisResult<usize>> = pipe.query(&mut self.conn)?;
        Ok(lens
            .into_iter()
            .map(|len| len.map_err(Error::from))
            .collect())
    }

    /// The id of the newest entry of the message log, or `0-0` when there is
    /// none: where [`Relay::wait_for_log`] starts listening from. Read it
    /// *before* looking at the inboxes, so a message sent in between wakes the
    /// next wait instead of being missed.
    pub fn log_tail_id(&mut self) -> Result<String> {
        let reply: StreamRangeReply =
            self.conn
                .xrevrange_count(self.keys.log_key(), "+", "-", 1)?;
        Ok(reply
            .ids
            .into_iter()
            .next()
            .map_or_else(|| "0-0".to_owned(), |entry| entry.id))
    }

    /// Blocks up to `block` for an entry newer than `after` on the message log
    /// (`XREAD BLOCK`): every send appends one. Returns whether one arrived.
    /// The log is capped and shared by every machine's traffic, so an arrival
    /// says "look at your inboxes", not "your inbox has mail".
    ///
    /// The connection's read timeout is widened for the wait, so `block` may
    /// exceed it, and put back afterwards.
    pub fn wait_for_log(&mut self, after: &str, block: Duration) -> Result<bool> {
        /// Slack over `block` before the socket itself gives up.
        const SLACK: Duration = Duration::from_secs(1);
        let armed = self.timeout.get();
        self.conn.set_read_timeout(Some(block + SLACK))?;
        let reply = redis::cmd("XREAD")
            .arg("BLOCK")
            // `BLOCK 0` waits forever, so a sub-millisecond wait is one millisecond.
            .arg(u64::try_from(block.as_millis()).unwrap_or(u64::MAX).max(1))
            .arg("COUNT")
            .arg(1)
            .arg("STREAMS")
            .arg(self.keys.log_key())
            .arg(after)
            .query::<redis::Value>(&mut self.conn);
        // Put the timeout back even when the wait failed: a caller that keeps
        // the connection after a data error must not keep a widened timeout.
        let restored = self.conn.set_read_timeout(Some(armed));
        let reply = reply?;
        restored?;
        Ok(!matches!(reply, redis::Value::Nil))
    }

    /// Puts `messages` back at the front of `agent`'s inbox, in their original
    /// order, so a later pop sees them exactly as if they had never been
    /// popped. For a caller holding messages that no in-flight record covers (a
    /// message popped with [`Relay::pop_inbox`]); a claimed batch goes back with
    /// [`Relay::requeue_batch`], which does not depend on the caller still
    /// holding the messages.
    ///
    /// Also appends a wake entry (field `requeued`, no `msg`) to the message log
    /// in the same transaction, so a watcher blocked on the log looks at the
    /// inbox now instead of at its next sweep; [`Relay::log`] skips it.
    pub fn requeue_front(&mut self, agent: &AgentName, messages: &[Message]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let key = self.keys.inbox_key(agent);
        // LPUSH pushes each argument onto the head in turn, so the last
        // argument ends up first; pushing in reverse restores the original
        // front-to-back order.
        let jsons: Vec<String> = messages.iter().rev().map(encode).collect();
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.lpush(key, jsons).ignore();
        pipe.cmd("XADD")
            .arg(self.keys.log_key())
            .arg("MAXLEN")
            .arg("~")
            .arg(LOG_CAP)
            .arg("*")
            .arg("requeued")
            .arg(agent.as_str())
            .ignore();
        let () = pipe.query(&mut self.conn)?;
        Ok(())
    }

    /// Removes and returns up to `max` pending messages for `agent`, oldest first.
    /// The pop is one Redis command, so two consumers never get the same message.
    /// This is the plain pop behind `agent-msg inbox` (a human's read, and the
    /// opencode plugin's `inbox --json` delivery): it records nothing in flight, so
    /// what it pops is at-most-once. The opencode delivery needs a claim and an
    /// acknowledgement (AGE-2051); the hook and the watcher pop with
    /// [`Relay::pop_inbox_budget`].
    ///
    /// One undecodable entry (someone wrote to the inbox key by hand) does not
    /// lose the rest of the batch: it is popped and reported in `rejected`, while
    /// every entry that decoded cleanly still comes back in `messages`.
    pub fn pop_inbox(&mut self, agent: &AgentName, max: NonZeroUsize) -> Result<PopResult> {
        let key = self.keys.inbox_key(agent);
        // Bytes, not `String`: the server has already removed the batch when
        // the reply is converted, so an entry that is not UTF-8 must be
        // reported as rejected, not fail the pop and lose its neighbours.
        let raw: Option<Vec<Vec<u8>>> = self.conn.lpop(&key, Some(max))?;
        let (messages, rejected) = decode_entries(&key, raw.unwrap_or_default());
        Ok(PopResult { messages, rejected })
    }

    /// Up to `max` logged messages sent at or after `since_ms` (Unix milliseconds,
    /// the log's own clock), oldest first. When more than `max` were sent in the
    /// window, returns the most recent `max` of them.
    pub fn log(&mut self, since_ms: i64, max: usize) -> Result<Vec<Message>> {
        let key = self.keys.log_key();
        let start = since_ms.max(0).to_string();
        // XREVRANGE walks newest-first, so a COUNT cap keeps the most recent
        // `max` entries instead of XRANGE's oldest `max`. Reverse back to
        // oldest-first for callers.
        let reply: StreamRangeReply = self.conn.xrevrange_count(&key, "+", start, max)?;
        reply
            .ids
            .iter()
            .rev()
            .filter_map(|entry| entry.get::<String>("msg"))
            .map(|json| decode(&key, &json))
            .collect()
    }

    /// Every retained logged message at or after `since_ms` (Unix milliseconds,
    /// the log's Redis stream clock), oldest first. The stream is already
    /// retention-capped, so this reads the complete retained window without a
    /// second caller-chosen limit.
    pub fn log_since(&mut self, since_ms: i64) -> Result<Vec<Message>> {
        let key = self.keys.log_key();
        let start = format!("{}-0", since_ms.max(0));
        let reply: StreamRangeReply = redis::cmd("XRANGE")
            .arg(&key)
            .arg(start)
            .arg("+")
            .query(&mut self.conn)?;
        reply
            .ids
            .iter()
            .filter_map(|entry| entry.get::<String>("msg"))
            .map(|json| decode(&key, &json))
            .collect()
    }

    /// Files a new escalation and returns it, status [`EscalationStatus::Waiting`].
    /// The id is drawn from an `INCR` counter (`e1`, `e2`, ...): short and
    /// human-typeable, unique for this bus regardless of prefix collisions
    /// between test runs sharing one Redis (each test uses its own prefix).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidSessionId`] when `new.session_id` doesn't look like a
    /// harness session id — it's eventually passed verbatim to `claude
    /// --resume <sid>` by `investigate`.
    ///
    /// [`Error::UnsafeEscalationText`] when `new.context`, one of
    /// `new.options`, or `new.recommendation` contains a control character
    /// other than `\n`/`\t` (see `escalation.rs`'s `validate_safe_text`
    /// for why). Checked before anything is written: a rejected escalation
    /// is never partially stored.
    pub fn escalate(&mut self, new: EscalationFiling) -> Result<Escalation> {
        crate::escalation::validate_session_id(new.session_id.as_str())?;
        crate::escalation::validate_safe_text("context", &new.context)?;
        for option in &new.options {
            crate::escalation::validate_safe_text("option", option)?;
        }
        crate::escalation::validate_safe_text("recommendation", &new.recommendation)?;

        let n: u64 = self.conn.incr(self.keys.esc_seq_key(), 1)?;
        let id = EscalationId::from_counter(n);
        let meta = EscalationMeta {
            id: id.clone(),
            filing: new,
        };
        let created_at_ms = meta.filing.created_at_ms;
        let meta_json = encode(&meta);
        let fields: [(&str, &str); 3] = [
            ("meta", meta_json.as_str()),
            ("status", EscalationStatus::Waiting.as_str()),
            ("decision", ""),
        ];
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.hset_multiple(self.keys.esc_key(&id), &fields).ignore();
        pipe.zadd(self.keys.esc_index_key(), id.as_str(), created_at_ms)
            .ignore();
        let () = pipe.query(&mut self.conn)?;
        Ok(meta.into_escalation(EscalationStatus::Waiting, None))
    }

    /// Looks up one escalation by id.
    ///
    /// # Errors
    ///
    /// [`Error::EscalationNotFound`] when no such id exists.
    pub fn get(&mut self, id: &EscalationId) -> Result<Escalation> {
        let key = self.keys.esc_key(id);
        let fields: HashMap<String, String> = self.conn.hgetall(&key)?;
        if fields.is_empty() {
            return Err(Error::EscalationNotFound(id.to_string()));
        }
        decode_escalation(id, &fields)
    }

    /// Every escalation, oldest first, optionally filtered to one `status`.
    ///
    /// One entry that fails to decode (a hand-edited or half-written hash;
    /// shouldn't happen — the index and the hash are written together in
    /// `escalate`'s atomic pipe and nothing ever deletes one without the
    /// other) does not lose the rest of the listing: it's reported in
    /// [`EscalationList::rejected`] instead, same as a bad inbox entry is in
    /// [`PopResult::rejected`].
    pub fn list(&mut self, status: Option<EscalationStatus>) -> Result<EscalationList> {
        let raw_ids: Vec<String> = self.conn.zrange(self.keys.esc_index_key(), 0, -1)?;
        // An index entry that is not a valid id could alias another key
        // (`seq`, `index`): report it, never look it up.
        let mut ids = Vec::with_capacity(raw_ids.len());
        let mut rejected = Vec::new();
        for raw in raw_ids {
            match raw.parse::<EscalationId>() {
                Ok(id) => ids.push(id),
                Err(e) => rejected.push((raw, e)),
            }
        }
        if ids.is_empty() && rejected.is_empty() {
            return Ok(EscalationList::default());
        }
        // One pipelined round trip for every hash, rather than one HGETALL
        // per id: a listing with N escalations costs one round trip instead
        // of N.
        let mut pipe = redis::pipe();
        for id in &ids {
            pipe.hgetall(self.keys.esc_key(id));
        }
        let all_fields: Vec<HashMap<String, String>> = pipe.query(&mut self.conn)?;

        let mut escalations = Vec::with_capacity(ids.len());
        for (id, fields) in ids.into_iter().zip(all_fields) {
            if fields.is_empty() {
                rejected.push((
                    id.to_string(),
                    Error::CorruptEscalation {
                        id: id.to_string(),
                        reason: "indexed but the hash is empty or missing".to_owned(),
                    },
                ));
                continue;
            }
            match decode_escalation(&id, &fields) {
                Ok(esc) if status.is_none_or(|s| s == esc.status) => escalations.push(esc),
                Ok(_) => {}
                Err(e) => rejected.push((id.to_string(), e)),
            }
        }
        Ok(EscalationList {
            escalations,
            rejected,
        })
    }

    /// The next attempt number for investigating `id`, starting at 1. Backs
    /// `investigate`'s herdr agent name (`esc-<id>-<attempt>`): herdr agent
    /// names must be unique among live agents, and `investigate` can be run
    /// more than once on the same escalation (a second investigation is
    /// allowed — see [`Relay::set_investigating`]), so a fixed name would
    /// collide with the first attempt's still-open pane.
    pub fn next_investigation_attempt(&mut self, id: &EscalationId) -> Result<u64> {
        let key = self.keys.esc_attempts_key(id);
        let n: u64 = self.conn.incr(key, 1)?;
        Ok(n)
    }

    /// Delivers a notice from `from` to `human`, bypassing the
    /// registered-recipient check [`Relay::send`] enforces for every other
    /// target. Deliberately narrow — takes a specific `human` `AgentName`
    /// rather than a whole [`Message`] with a [`Target`] that could be
    /// [`Target::All`], so there's no broadcast-to-everyone case to refuse at
    /// runtime. The only caller is `escalate`/`resolve` notifying the operator, who
    /// runs no hook and so never registers on the bus the way a lead does.
    /// Anything else that wants to reach an unregistered name should not use
    /// this.
    ///
    /// Still enforces [`MAX_MESSAGE_BYTES`], and still logs the message, same
    /// as [`Relay::send`].
    ///
    /// # Errors
    ///
    /// [`Error::MessageTooLarge`] when `text` exceeds [`MAX_MESSAGE_BYTES`].
    pub fn notify_human(
        &mut self,
        from: &AgentName,
        human: &AgentName,
        text: &str,
        sent_at_ms: i64,
    ) -> Result<()> {
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(Error::MessageTooLarge {
                len: text.len(),
                max: MAX_MESSAGE_BYTES,
            });
        }
        let msg = Message {
            from: from.clone(),
            to: Target::Agent(human.clone()),
            text: text.to_owned(),
            sent_at_ms,
        };
        let json = encode(&msg);
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.rpush(self.keys.inbox_key(human), &json).ignore();
        pipe.cmd("XADD")
            .arg(self.keys.log_key())
            .arg("MAXLEN")
            .arg("~")
            .arg(LOG_CAP)
            .arg("*")
            .arg("msg")
            .arg(&json)
            .ignore();
        let () = pipe.query(&mut self.conn)?;
        Ok(())
    }

    /// Marks `id` as [`EscalationStatus::Investigating`]. Allowed from
    /// `Waiting` or `Investigating` (a second investigation is fine — the operator
    /// starting a second look doesn't change anything); refused once
    /// resolved.
    ///
    /// # Errors
    ///
    /// [`Error::EscalationNotFound`] when no such id exists.
    /// [`Error::EscalationAlreadyResolved`] when it was already resolved.
    pub fn set_investigating(&mut self, id: &EscalationId) -> Result<Escalation> {
        self.transition(
            id,
            &[EscalationStatus::Waiting, EscalationStatus::Investigating],
            EscalationStatus::Investigating,
            None,
        )
    }

    /// Marks `id` as [`EscalationStatus::Resolved`] with `decision`, atomically.
    /// Allowed from `Waiting` or `Investigating`; resolution is terminal.
    ///
    /// The transition runs as one Lua script server-side, so two concurrent
    /// `resolve` calls on the same id can never both succeed: Redis executes
    /// scripts one at a time, so whichever runs first wins and the second
    /// observes the already-`resolved` status and gets
    /// [`Error::EscalationAlreadyResolved`].
    ///
    /// # Errors
    ///
    /// [`Error::EmptyDecision`] when `decision` is empty or only whitespace (a
    /// resolved escalation always records a real one). Checked before Redis is touched.
    /// [`Error::EscalationNotFound`] when no such id exists.
    /// [`Error::EscalationAlreadyResolved`] when it was already resolved.
    pub fn resolve(&mut self, id: &EscalationId, decision: &str) -> Result<Escalation> {
        if decision.trim().is_empty() {
            return Err(Error::EmptyDecision { id: id.to_string() });
        }
        self.transition(
            id,
            &[EscalationStatus::Waiting, EscalationStatus::Investigating],
            EscalationStatus::Resolved,
            Some(("decision", decision)),
        )
    }

    /// The one atomic status transition both `set_investigating` and
    /// `resolve` build on: refuses unless the escalation's current status is
    /// in `allowed`, then sets `status` to `to` and, when `extra` is given,
    /// one more hash field — all in a single Lua script, so no other caller
    /// can observe or race past the check.
    fn transition(
        &mut self,
        id: &EscalationId,
        allowed: &[EscalationStatus],
        to: EscalationStatus,
        extra: Option<(&str, &str)>,
    ) -> Result<Escalation> {
        // Returns "ok", "not_found", or "blocked:<current status>".
        const SCRIPT: &str = r"
            if redis.call('EXISTS', KEYS[1]) == 0 then return 'not_found' end
            local status = redis.call('HGET', KEYS[1], 'status')
            local allowed = {}
            for s in string.gmatch(ARGV[1], '[^,]+') do allowed[s] = true end
            if not allowed[status] then return 'blocked:' .. status end
            redis.call('HSET', KEYS[1], 'status', ARGV[2])
            if ARGV[3] ~= '' then redis.call('HSET', KEYS[1], ARGV[3], ARGV[4]) end
            return 'ok'
        ";
        let key = self.keys.esc_key(id);
        let allowed_arg = allowed
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let (field, value) = extra.unwrap_or(("", ""));
        let outcome: String = redis::Script::new(SCRIPT)
            .key(&key)
            .arg(&allowed_arg)
            .arg(to.as_str())
            .arg(field)
            .arg(value)
            .invoke(&mut self.conn)?;
        match outcome.as_str() {
            "ok" => self.get(id),
            "not_found" => Err(Error::EscalationNotFound(id.to_string())),
            // Only `resolved` is ever excluded from `allowed` today (both
            // callers allow Waiting and Investigating), so this is always a
            // double resolve/investigate; kept as a match on the prefix
            // rather than assuming, so a future third status is refused
            // loudly instead of silently mis-reported.
            other => {
                let blocked_status = other.strip_prefix("blocked:").unwrap_or(other);
                if blocked_status == EscalationStatus::Resolved.as_str() {
                    Err(Error::EscalationAlreadyResolved(id.to_string()))
                } else {
                    Err(Error::CorruptEscalation {
                        id: id.to_string(),
                        reason: format!("unexpected transition script reply {other:?}"),
                    })
                }
            }
        }
    }
}

/// Decodes one escalation hash (`meta` JSON plus plain `status`/`decision`
/// fields) into an [`Escalation`].
fn decode_escalation(id: &EscalationId, fields: &HashMap<String, String>) -> Result<Escalation> {
    let meta_json = fields.get("meta").ok_or_else(|| Error::CorruptEscalation {
        id: id.to_string(),
        reason: "missing meta field".to_owned(),
    })?;
    let key = format!("esc:{id}");
    let meta: EscalationMeta = decode(&key, meta_json)?;
    let status_raw = fields
        .get("status")
        .ok_or_else(|| Error::CorruptEscalation {
            id: id.to_string(),
            reason: "missing status field".to_owned(),
        })?;
    let status: EscalationStatus = status_raw.parse()?;
    let decision = fields.get("decision").filter(|d| !d.is_empty()).cloned();
    Ok(meta.into_escalation(status, decision))
}

/// What [`Relay::agents`] found: the registrations that decoded, and the
/// records that did not.
#[derive(Debug, Default)]
pub struct AgentList {
    /// Registered leads, sorted by name.
    pub registrations: Vec<Registration>,
    /// Records skipped because they did not decode, sorted by hash field.
    pub skipped: Vec<SkippedRegistration>,
}

/// One registry record [`Relay::agents`] could not decode.
#[derive(Debug)]
pub struct SkippedRegistration {
    /// The registry hash field (the name the record was stored under).
    pub field: String,
    /// Why it failed to decode.
    pub error: Error,
    /// The record's stored bytes, for [`SkippedRegistration::salvage`].
    pub raw: Vec<u8>,
}

/// What can be read from a registry record this binary refuses to decode, enough
/// for the watcher to apply its prune rule to it (AGE-2027): a record an older
/// binary wrote with an id outside its type's rule must not live forever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Salvage {
    /// The registry field, as a bus name.
    pub name: AgentName,
    /// When the lead last registered or was seen, Unix milliseconds.
    pub last_seen_ms: i64,
    /// The machine the record names, normalised.
    pub machine: MachineName,
    /// What the record holds for `herdr_pane`.
    pub pane: Salvaged<PaneId>,
    /// What the record holds for `session_id`.
    pub session: Salvaged<SessionId>,
}

/// One optional id of a [`Salvage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Salvaged<T> {
    /// The record has none (absent or `null`).
    Absent,
    /// The record has one and it is a valid id.
    Valid(T),
    /// The record has one that breaks its type's rule (or is not text): no
    /// herdr can have issued it.
    Refused,
}

impl SkippedRegistration {
    /// The fields the prune rule needs, read from the raw JSON without the id
    /// rules, or `None` when the record is not an object with a bus name field, a
    /// machine that is a machine name and a numeric `last_seen_ms`.
    #[must_use]
    pub fn salvage(&self) -> Option<Salvage> {
        let value: serde_json::Value = serde_json::from_slice(&self.raw).ok()?;
        let object = value.as_object()?;
        Some(Salvage {
            name: self.field.parse().ok()?,
            last_seen_ms: object.get("last_seen_ms")?.as_i64()?,
            machine: MachineName::new(object.get("machine")?.as_str()?).ok()?,
            pane: salvage_id(object.get("herdr_pane")),
            session: salvage_id(object.get("session_id")),
        })
    }
}

/// One optional id member of a record, read for [`Salvage`].
fn salvage_id<T: std::str::FromStr>(member: Option<&serde_json::Value>) -> Salvaged<T> {
    match member {
        None | Some(serde_json::Value::Null) => Salvaged::Absent,
        Some(serde_json::Value::String(s)) => s.parse().map_or(Salvaged::Refused, Salvaged::Valid),
        Some(_) => Salvaged::Refused,
    }
}

impl std::fmt::Display for SkippedRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "skipped registry record {:?}: {}",
            self.field, self.error
        )
    }
}

impl AgentList {
    /// The decodable registrations, after reporting each skipped record to
    /// `report` — for a caller that wants to log them and carry on.
    pub fn into_registrations_reporting(
        self,
        mut report: impl FnMut(&SkippedRegistration),
    ) -> Vec<Registration> {
        self.skipped.iter().for_each(&mut report);
        self.registrations
    }
}

/// What [`Relay::agent_slot`] found under a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSlot {
    /// Nobody has registered the name.
    Absent,
    /// A registration that decodes.
    Registered(Box<Registration>),
    /// A record is there but this binary refuses to decode it.
    Unreadable,
}

/// One entry of the hook health stream: a failure record and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookHealthEntry {
    /// The machine that spooled it, normalised.
    pub machine: MachineName,
    /// The lead's bus name, when the hook knew it.
    pub lead: Option<String>,
    /// The record, as one line of JSON.
    pub record: String,
}

/// What popping an inbox found: the messages that decoded cleanly, and any raw
/// entries that did not.
#[derive(Debug, Default)]
pub struct PopResult {
    /// Messages ready for the caller, oldest first. Already removed from Redis.
    pub messages: Vec<Message>,
    /// Entries popped alongside `messages` that failed to decode. Also already
    /// removed from Redis; the caller decides how to surface them.
    pub rejected: Vec<Error>,
}

/// What the session cache holds for one session ([`Relay::probe_session`]). The
/// default is a session nothing is known about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionProbe {
    /// Whether the session is suppressed: it never joins the bus.
    pub suppressed: bool,
    /// The bus name remembered for the session, if any.
    pub name: Option<AgentName>,
}

/// The usable entries of a hook health stream reply, in reply order, and how many
/// it could not use.
fn hook_health_of(reply: &StreamRangeReply) -> HookHealth {
    let mut entries = Vec::with_capacity(reply.ids.len());
    let mut unreadable = 0;
    for entry in &reply.ids {
        // An entry a reader cannot use (a missing field, or a machine outside
        // its type's rule) is counted, never silently dropped.
        let read = (|| {
            Some(HookHealthEntry {
                machine: entry.get::<String>("machine")?.parse().ok()?,
                lead: entry.get::<String>("lead").filter(|l| !l.is_empty()),
                record: entry.get::<String>("rec")?,
            })
        })();
        match read {
            Some(e) => entries.push(e),
            None => unreadable += 1,
        }
    }
    HookHealth {
        entries,
        unreadable,
    }
}

/// What [`Relay::hook_health`] read: the usable entries, and how many it could not use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookHealth {
    /// The entries, oldest first.
    pub entries: Vec<HookHealthEntry>,
    /// Entries the read left out: a missing field, or a machine that is not a
    /// machine name.
    pub unreadable: usize,
}

/// What a budgeted pop ([`Relay::pop_inbox_budget`]) found: the messages that
/// decoded cleanly, any raw entries that did not, how many messages are still
/// queued in the inbox right after this pop, and the batch to acknowledge.
#[derive(Debug, Default)]
pub struct BudgetPopResult {
    /// Messages ready for the caller, oldest first. Recorded in flight until
    /// [`Relay::ack`]ed (see [`Relay::pop_inbox_budget`]).
    pub messages: Vec<Message>,
    /// For each of `messages`, whether it is a redelivery: it was popped before
    /// and its lease expired without an acknowledgement. Same length as `messages`.
    pub redelivered: Vec<bool>,
    /// Whether nothing was taken because the lead already has too much in
    /// flight for this pop (see [`Relay::pop_inbox_budget`]): mail is waiting and
    /// the leases will free the room.
    pub at_cap: bool,
    /// Entries whose redeliveries are spent and that could not be dead-lettered
    /// because the lead's dead-letter key has the wrong type: left in flight.
    pub stuck: usize,
    /// Older dead letters dropped by this pop's reap to keep the list capped.
    pub evicted: usize,
    /// Entries popped alongside `messages` that failed to decode. They are in
    /// the same batch: acknowledging it drops them.
    pub rejected: Vec<Error>,
    /// How many messages are left in the inbox immediately after this pop.
    pub pending: usize,
    /// The in-flight batch to acknowledge once the messages are delivered;
    /// `None` when nothing was popped.
    pub batch: Option<BatchId>,
}

/// Decodes popped inbox entries: the messages that decode, in order, and an
/// error for each entry that does not (not JSON, an unknown member, or not
/// UTF-8).
fn decode_entries(key: &str, raw: Vec<Vec<u8>>) -> (Vec<Message>, Vec<Error>) {
    let mut messages = Vec::new();
    let mut rejected = Vec::new();
    for entry in raw {
        match serde_json::from_slice(&entry) {
            Ok(m) => messages.push(m),
            Err(source) => rejected.push(Error::Decode {
                key: key.to_owned(),
                source,
            }),
        }
    }
    (messages, rejected)
}

fn encode<T: serde::Serialize>(value: &T) -> String {
    // Our types are plain structs of strings and integers; serializing them to a
    // String cannot fail.
    #[expect(
        clippy::expect_used,
        reason = "every encoded type is plain strings, integers and vecs with string keys: serialization is infallible, and a Result here would be an error variant with no producer"
    )]
    serde_json::to_string(value).expect("relay types always serialize")
}

/// [`decode`] for bytes read from Redis, which need not be UTF-8: those fail
/// as a corrupt record like any other undecodable one.
fn decode_bytes<T: serde::de::DeserializeOwned>(key: &str, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|source| Error::Decode {
        key: key.to_owned(),
        source,
    })
}

fn decode<T: serde::de::DeserializeOwned>(key: &str, json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|source| Error::Decode {
        key: key.to_owned(),
        source,
    })
}
