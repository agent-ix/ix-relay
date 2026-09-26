//! Reads and writes of the relay's own health: the watcher heartbeat, the
//! numbers `agent-msg doctor` reports and the registration liveness refresh.
//!
//! Nothing here pops or moves a message. Key scans go through the one bounded
//! scan in `scan.rs`, so a Redis shared with something large costs a health
//! check a bounded number of round trips, not a stall, and a cut is reported.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use redis::Commands;

use super::scan::ScanCut;
use super::{AgentName, MachineName, Registration, Relay, Result, decode_bytes, encode};
use crate::health::{HeartbeatRead, MachineHeartbeat, WatcherHeartbeat};

/// The most heartbeat keys one [`Relay::heartbeats`] scan returns. A bus has a
/// handful of machines; this only bounds a pathological keyspace.
pub const MAX_HEARTBEAT_KEYS: usize = 256;

/// The most in-flight rows of one lead [`Relay::overdue_leases`] reads.
const MAX_LEASE_ROWS: usize = 200;

/// What one recipient's queue and machine looked like right after a send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientProbe {
    /// Messages waiting in the recipient's inbox, the one just sent included.
    pub inbox_len: usize,
    /// The recipient's machine's heartbeat key: `None` when there is none.
    pub heartbeat: Option<HeartbeatRead>,
}

/// One lead's inbox as `doctor` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxOverview {
    /// Messages waiting.
    pub len: usize,
    /// When the oldest was sent (the sender's clock, Unix milliseconds), if the
    /// inbox is not empty and its oldest entry decodes.
    pub oldest_sent_at_ms: Option<i64>,
}

/// One lead's queue lengths, for the watcher's metrics: the inbox, its oldest
/// message, and the in-flight and dead-letter lists, read together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeadOverview {
    /// Messages waiting.
    pub len: usize,
    /// When the oldest was sent (the sender's clock, Unix milliseconds), if the
    /// inbox is not empty and its oldest entry decodes.
    pub oldest_sent_at_ms: Option<i64>,
    /// Entries (rows, not batches) popped and not yet acknowledged.
    pub in_flight: usize,
    /// Dead-lettered entries.
    pub dead: usize,
}

/// What one lead's in-flight rows say about their leases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseState {
    /// The lead.
    pub lead: AgentName,
    /// Entries (rows) in flight, all of them.
    pub entries: usize,
    /// Distinct batches among the rows read.
    pub batches: usize,
    /// Rows read whose lease deadline is before the server's clock.
    pub overdue: usize,
    /// How far past its deadline the most overdue row is, in milliseconds.
    pub most_overdue_ms: i64,
    /// Whether more rows exist than were read, so `overdue` and `batches` may be
    /// short.
    pub rows_cut: bool,
}

/// Every machine's heartbeat, the last time each machine's watcher ticked, and
/// whether the scan was cut.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Heartbeats {
    /// Sorted by machine.
    pub found: Vec<MachineHeartbeat>,
    /// Which bound cut the key scan, if one did: a machine that was not asked for
    /// by name may be missing. [`ScanCut::Deadline`] means nothing was read at all
    /// (`found` and `last_ticks` are empty).
    pub truncated: Option<ScanCut>,
    /// The last tick each machine's watcher wrote to the non-expiring marker
    /// (`health:watcher-seen`), by machine, in Unix milliseconds. Unlike the
    /// heartbeat, it outlives a dead watcher, so `doctor` can tell a watcher that
    /// stopped an hour ago from one that was never there.
    pub last_ticks: BTreeMap<MachineName, i64>,
    /// Heartbeat keys and `health:watcher-seen` fields whose machine is not a
    /// machine name: left out above, and counted here.
    pub unreadable: usize,
}

/// The oldest message's `sent_at_ms`, read without decoding a whole
/// [`crate::Message`]: an entry this binary cannot fully decode still has an age.
fn sent_at_of(raw: &[u8]) -> Option<i64> {
    #[derive(serde::Deserialize)]
    struct Stamp {
        sent_at_ms: i64,
    }
    serde_json::from_slice::<Stamp>(raw)
        .ok()
        .map(|s| s.sent_at_ms)
}

/// `<batch id> <popper> <deadline ms> <entry>`: the batch id and the deadline of
/// an in-flight row, when it has them (the entry may be any bytes).
fn lease_fields(row: &[u8]) -> Option<(u64, i64)> {
    let mut parts = row.splitn(4, |b| *b == b' ');
    let batch = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
    parts.next()?;
    let deadline = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
    Some((batch, deadline))
}

impl Relay {
    /// One `PING`: whether the server answers, and (timed by the caller) how
    /// fast.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when Redis fails or times out.
    pub fn ping(&mut self) -> Result<()> {
        let _: String = redis::cmd("PING").query(&mut self.conn)?;
        Ok(())
    }

    /// Writes `heartbeat` under its machine's key with `ttl` as its expiry
    /// (`SET .. PX`) and the same tick time to the non-expiring marker
    /// (`HSET`), in one request. The expiry is the point of the heartbeat: see
    /// [`crate::health::heartbeat_ttl`]; the marker is what remains when it has
    /// lapsed.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when Redis fails or times out.
    pub fn write_heartbeat(&mut self, heartbeat: &WatcherHeartbeat, ttl: Duration) -> Result<()> {
        let millis = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX).max(1);
        let () = redis::pipe()
            .cmd("SET")
            .arg(self.keys.health_watcher_key(&heartbeat.machine))
            .arg(encode(heartbeat))
            .arg("PX")
            .arg(millis)
            .ignore()
            .cmd("HSET")
            .arg(self.keys.health_seen_key())
            .arg(heartbeat.machine.as_str())
            .arg(heartbeat.last_tick_ms)
            .ignore()
            .query(&mut self.conn)?;
        Ok(())
    }

    /// The heartbeat of `machine`'s watcher, `None` when there is no key: the
    /// watcher is down, expired or never ran.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when Redis fails or times out.
    pub fn heartbeat(&mut self, machine: &MachineName) -> Result<Option<HeartbeatRead>> {
        let raw: Option<Vec<u8>> = self.conn.get(self.keys.health_watcher_key(machine))?;
        Ok(raw.map(|bytes| read_heartbeat(&bytes)))
    }

    /// The heartbeat of each machine in `named` (a direct `GET` each, so a big
    /// keyspace cannot hide a machine the caller cares about) and of every other
    /// machine a bounded key scan finds, sorted by machine.
    /// [`Heartbeats::truncated`] says the scan was cut.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when Redis fails or times out.
    pub fn heartbeats(&mut self, named: &[MachineName]) -> Result<Heartbeats> {
        let stem = self.keys.health_watcher_stem();
        let scanned = self.scan_prefixed(&stem, MAX_HEARTBEAT_KEYS)?;
        if scanned.cut == Some(ScanCut::Deadline) {
            // The scan ran out of the caller's time in the middle of a call: the
            // connection may still owe a reply, so nothing more is read from it.
            // The caller sees `truncated` and knows the heartbeats were not read.
            return Ok(Heartbeats {
                found: Vec::new(),
                truncated: Some(ScanCut::Deadline),
                last_ticks: BTreeMap::new(),
                unreadable: 0,
            });
        }
        // A key whose suffix is not a machine name this crate could have written
        // is not a heartbeat: it is left out rather than listed under a name it
        // cannot be read back by.
        let mut unreadable = 0;
        let mut machines: BTreeSet<MachineName> = named.iter().cloned().collect();
        for key in &scanned.keys {
            // A watcher writes its key under the normalised name. A suffix that
            // only normalises to a machine (`Hub`) would be read back as the
            // other key, so its own content is never read: counted, not merged.
            match key.strip_prefix(&stem) {
                Some(suffix) => match suffix.parse::<MachineName>() {
                    Ok(machine) if machine.as_str() == suffix => {
                        machines.insert(machine);
                    }
                    _ => unreadable += 1,
                },
                None => unreadable += 1,
            }
        }
        if machines.is_empty() {
            let (last_ticks, bad_ticks) = self.last_ticks()?;
            return Ok(Heartbeats {
                found: Vec::new(),
                truncated: scanned.cut,
                last_ticks,
                unreadable: unreadable + bad_ticks,
            });
        }
        let mut pipe = redis::pipe();
        for machine in &machines {
            pipe.cmd("GET").arg(self.keys.health_watcher_key(machine));
        }
        // A key that expired between the scan and the read is simply gone.
        let values: Vec<Option<Vec<u8>>> = pipe.query(&mut self.conn)?;
        let found = machines
            .into_iter()
            .zip(values)
            .filter_map(|(machine, value)| {
                Some(MachineHeartbeat {
                    machine,
                    read: read_heartbeat(&value?),
                })
            })
            .collect();
        let (last_ticks, bad_ticks) = self.last_ticks()?;
        Ok(Heartbeats {
            found,
            truncated: scanned.cut,
            last_ticks,
            unreadable: unreadable + bad_ticks,
        })
    }

    /// The non-expiring last-tick marker of every machine that ever wrote one,
    /// and how many fields were left out.
    ///
    /// The watcher writes the field as the normalised machine name. Fields that
    /// normalise to the same machine (`Hub` and `hub`, written by something
    /// else) are one machine: the newest tick wins. A field that is not a machine
    /// name is counted, not merged.
    fn last_ticks(&mut self) -> Result<(BTreeMap<MachineName, i64>, usize)> {
        let raw: Vec<(String, i64)> = redis::cmd("HGETALL")
            .arg(self.keys.health_seen_key())
            .query(&mut self.conn)?;
        let mut ticks: BTreeMap<MachineName, i64> = BTreeMap::new();
        let mut unreadable = 0;
        for (field, ms) in raw {
            match field.parse::<MachineName>() {
                Ok(machine) => {
                    let newest = ticks.entry(machine).or_insert(ms);
                    *newest = (*newest).max(ms);
                }
                Err(_) => unreadable += 1,
            }
        }
        Ok((ticks, unreadable))
    }

    /// What `agent-msg send` needs to describe a recipient right after queueing:
    /// its inbox length and its machine's heartbeat, in ONE request with ONE
    /// reply (a read-only `EVAL` of `LLEN` and `GET`), so the connection's
    /// timeout is the whole probe's bound: two replies would each get it, and a
    /// stalled Redis would cost twice as much.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when Redis fails or times out.
    pub fn recipient_probe(
        &mut self,
        agent: &AgentName,
        machine: &MachineName,
    ) -> Result<RecipientProbe> {
        /// Reads only: the length of the inbox and the heartbeat key.
        const SCRIPT: &str = "return {redis.call('LLEN', KEYS[1]), redis.call('GET', KEYS[2])}";
        let (inbox_len, heartbeat): (usize, Option<Vec<u8>>) = redis::cmd("EVAL")
            .arg(SCRIPT)
            .arg(2)
            .arg(self.keys.inbox_key(agent))
            .arg(self.keys.health_watcher_key(machine))
            .query(&mut self.conn)?;
        Ok(RecipientProbe {
            inbox_len,
            heartbeat: heartbeat.map(|bytes| read_heartbeat(&bytes)),
        })
    }

    /// Each of `agents`' inbox length and oldest message's send time, in one
    /// pipelined request, in the order given. Reads only (`LLEN`, `LINDEX 0`).
    /// An inbox that cannot be read (a key of the wrong type) is that entry's
    /// error; only a failure of the connection fails the call.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when the connection fails or times out.
    pub fn inbox_overviews(&mut self, agents: &[&AgentName]) -> Result<Vec<Result<InboxOverview>>> {
        if agents.is_empty() {
            return Ok(Vec::new());
        }
        let mut pipe = redis::pipe();
        pipe.ignore_errors();
        for agent in agents {
            let key = self.keys.inbox_key(agent);
            pipe.cmd("LLEN").arg(&key);
            pipe.cmd("LINDEX").arg(&key).arg(0);
        }
        let replies: Vec<redis::RedisResult<redis::Value>> = pipe.query(&mut self.conn)?;
        let mut replies = replies.into_iter();
        let mut overviews = Vec::with_capacity(agents.len());
        while let (Some(len), Some(oldest)) = (replies.next(), replies.next()) {
            overviews.push(overview_of(len, oldest));
        }
        Ok(overviews)
    }

    /// Each of `agents`' inbox length, oldest message's send time, in-flight length
    /// and dead-letter length, in **one** pipelined request (`LLEN`, `LINDEX 0`,
    /// `LLEN`, `LLEN` per lead), in the order given. Reads only, and no key scan:
    /// the keys are the leads' own. A lead whose keys cannot be read (a key of the
    /// wrong type) is that entry's error; only a failure of the connection fails the
    /// call.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when the connection fails or times out.
    pub fn lead_overviews(&mut self, agents: &[&AgentName]) -> Result<Vec<Result<LeadOverview>>> {
        if agents.is_empty() {
            return Ok(Vec::new());
        }
        let mut pipe = redis::pipe();
        pipe.ignore_errors();
        for agent in agents {
            let inbox = self.keys.inbox_key(agent);
            pipe.cmd("LLEN").arg(&inbox);
            pipe.cmd("LINDEX").arg(&inbox).arg(0);
            pipe.cmd("LLEN").arg(self.keys.inflight_key(agent));
            pipe.cmd("LLEN").arg(self.keys.dead_letter_key(agent));
        }
        let replies: Vec<redis::RedisResult<redis::Value>> = pipe.query(&mut self.conn)?;
        let mut replies = replies.into_iter();
        let mut out = Vec::with_capacity(agents.len());
        while let (Some(len), Some(oldest), Some(in_flight), Some(dead)) = (
            replies.next(),
            replies.next(),
            replies.next(),
            replies.next(),
        ) {
            out.push(overview_of(len, oldest).and_then(|inbox| {
                let count = |v: redis::RedisResult<redis::Value>| -> Result<usize> {
                    Ok(redis::from_redis_value(v?).map_err(redis::RedisError::from)?)
                };
                Ok(LeadOverview {
                    len: inbox.len,
                    oldest_sent_at_ms: inbox.oldest_sent_at_ms,
                    in_flight: count(in_flight)?,
                    dead: count(dead)?,
                })
            }));
        }
        Ok(out)
    }

    /// The lease state of each of `leads`' in-flight lists (the first
    /// 200 rows of each), against the server's clock. Reads only.
    /// A list that cannot be read (a key of the wrong type) is left out.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when the connection fails or times out.
    pub fn overdue_leases(&mut self, leads: &[AgentName]) -> Result<Vec<LeaseState>> {
        if leads.is_empty() {
            return Ok(Vec::new());
        }
        let (secs, micros): (i64, i64) = redis::cmd("TIME").query(&mut self.conn)?;
        let now_ms = secs.saturating_mul(1000).saturating_add(micros / 1000);
        let mut pipe = redis::pipe();
        pipe.ignore_errors();
        for lead in leads {
            let key = self.keys.inflight_key(lead);
            pipe.cmd("LLEN").arg(&key);
            pipe.cmd("LRANGE").arg(&key).arg(0).arg(MAX_LEASE_ROWS - 1);
        }
        let replies: Vec<redis::RedisResult<redis::Value>> = pipe.query(&mut self.conn)?;
        let mut replies = replies.into_iter();
        let mut out = Vec::new();
        for lead in leads {
            let (Some(len), Some(rows)) = (replies.next(), replies.next()) else {
                break;
            };
            let (Ok(len), Ok(rows)) = (len, rows) else {
                continue;
            };
            let (Ok(entries), Ok(rows)) = (
                redis::from_redis_value::<usize>(len),
                redis::from_redis_value::<Vec<Vec<u8>>>(rows),
            ) else {
                continue;
            };
            let mut batches = BTreeSet::new();
            let (mut overdue, mut most) = (0, 0);
            for (batch, deadline) in rows.iter().filter_map(|r| lease_fields(r)) {
                batches.insert(batch);
                if deadline < now_ms {
                    overdue += 1;
                    most = most.max(now_ms - deadline);
                }
            }
            out.push(LeaseState {
                lead: lead.clone(),
                entries,
                batches: batches.len(),
                overdue,
                most_overdue_ms: most,
                rows_cut: entries > rows.len(),
            });
        }
        Ok(out)
    }

    /// Sets `name`'s `last_seen_ms` to `now_ms`, but only if the registration is
    /// still exactly the one the caller read (its `last_seen_ms` was
    /// `seen_last_seen_ms`) and `now_ms` is newer. Returns whether it changed
    /// anything: `false` when `name` is gone (a pruned lead is never brought
    /// back), was registered again meanwhile (that registration is fresher) or
    /// is already as fresh.
    ///
    /// Compare-and-set on the stored JSON, like [`Relay::refresh_place`]: the
    /// new value is spliced into what was read, so members a newer binary wrote
    /// survive. The script is the guard against a prune or a re-registration
    /// between the read and the write.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Redis`] when Redis fails or times out; [`crate::Error::Decode`] when the
    /// stored record is not a registration.
    pub fn touch_last_seen(
        &mut self,
        name: &AgentName,
        seen_last_seen_ms: i64,
        now_ms: i64,
    ) -> Result<bool> {
        self.touch_last_seen_between(name, seen_last_seen_ms, now_ms, |_| Ok(()))
    }

    /// [`Relay::touch_last_seen`] running `between` after the read and before the
    /// script: the seam a test uses to change the registration in exactly the
    /// window the script guards.
    fn touch_last_seen_between(
        &mut self,
        name: &AgentName,
        seen_last_seen_ms: i64,
        now_ms: i64,
        between: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<bool> {
        /// Replaces the field only if it still holds the JSON we read.
        const SCRIPT: &str = r"
            if redis.call('HGET', KEYS[1], ARGV[1]) == ARGV[2] then
                redis.call('HSET', KEYS[1], ARGV[1], ARGV[3])
                return 1
            end
            return 0";
        if now_ms <= seen_last_seen_ms {
            return Ok(false);
        }
        let key = self.keys.agents_key();
        let raw: Option<Vec<u8>> = self.conn.hget(&key, name.as_str())?;
        let Some(raw) = raw else {
            return Ok(false);
        };
        let reg: Registration = decode_bytes(&key, &raw)?;
        if reg.last_seen_ms != seen_last_seen_ms {
            return Ok(false);
        }
        let mut stored: serde_json::Value = decode_bytes(&key, &raw)?;
        let Some(members) = stored.as_object_mut() else {
            return Ok(false);
        };
        members.insert("last_seen_ms".to_owned(), serde_json::Value::from(now_ms));
        between(self)?;
        let swapped: i64 = redis::Script::new(SCRIPT)
            .key(&key)
            .arg(name.as_str())
            .arg(&raw)
            .arg(encode(&stored))
            .invoke(&mut self.conn)?;
        Ok(swapped == 1)
    }
}

/// One inbox's `LLEN` and `LINDEX 0` replies as an [`InboxOverview`].
fn overview_of(
    len: redis::RedisResult<redis::Value>,
    oldest: redis::RedisResult<redis::Value>,
) -> Result<InboxOverview> {
    let len: usize = redis::from_redis_value(len?).map_err(redis::RedisError::from)?;
    let oldest: Option<Vec<u8>> =
        redis::from_redis_value(oldest?).map_err(redis::RedisError::from)?;
    Ok(InboxOverview {
        len,
        oldest_sent_at_ms: oldest.as_deref().and_then(sent_at_of),
    })
}

/// Decodes one heartbeat key's bytes.
fn read_heartbeat(bytes: &[u8]) -> HeartbeatRead {
    serde_json::from_slice::<WatcherHeartbeat>(bytes).map_or(HeartbeatRead::Undecodable, |hb| {
        HeartbeatRead::Live(Box::new(hb))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(test: &str) -> Option<Relay> {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping {test}: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return None;
        };
        Some(
            Relay::connect(&url, Duration::from_secs(5))
                .unwrap()
                .with_prefix(format!("test:{}:{test}:", std::process::id())),
        )
    }

    fn registration(name: &str, last_seen_ms: i64) -> Registration {
        Registration {
            name: name.parse().unwrap(),
            machine: "node-a".parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: None,
            last_seen_ms,
        }
    }

    #[test]
    fn the_oldest_entry_has_an_age_even_when_it_is_not_a_whole_message() {
        assert_eq!(sent_at_of(br#"{"sent_at_ms":42,"from":"x"}"#), Some(42));
        assert_eq!(sent_at_of(b"not json"), None);
    }

    #[test]
    fn a_lease_row_gives_its_batch_and_deadline_whatever_the_entry_holds() {
        assert_eq!(lease_fields(b"7 hook 1500 {\"a\": 1}"), Some((7, 1500)));
        assert_eq!(lease_fields(b"7 hook 1500 \xff\xfe"), Some((7, 1500)));
        assert_eq!(lease_fields(b"seven hook 1500 x"), None);
        assert_eq!(lease_fields(b"7 hook"), None);
    }

    /// Tracing: TC-042-05
    /// ACs: FR-042-AC-5
    ///
    /// The script is the only guard against a change between the read and the
    /// write, and every case the Rust pre-check refuses never reaches it. So the
    /// registration is changed in exactly that window, by `between`: pruned, and
    /// registered again. A script that always wrote (`if true then`) resurrects
    /// the pruned lead and overwrites the newer registration here.
    #[test]
    fn tc_042_05_the_script_refuses_a_prune_or_a_registration_between_the_read_and_the_write() {
        let Some(mut r) = relay("touch-interleaved") else {
            return;
        };
        let lead: AgentName = "lead".parse().unwrap();

        r.register(&registration("lead", 1000)).unwrap();
        let moved = r
            .touch_last_seen_between(&lead, 1000, 5000, |r| r.prune(&lead, 1000).map(|_| ()))
            .unwrap();
        assert!(!moved, "a lead pruned in the window is not brought back");
        assert!(r.agent(&lead).unwrap().is_none());

        r.register(&registration("lead", 1000)).unwrap();
        let moved = r
            .touch_last_seen_between(&lead, 1000, 5000, |r| {
                r.register(&registration("lead", 4000))
            })
            .unwrap();
        assert!(!moved, "a registration in the window is not overwritten");
        assert_eq!(r.agent(&lead).unwrap().unwrap().last_seen_ms, 4000);

        // And with nothing in the window it does write.
        let moved = r
            .touch_last_seen_between(&lead, 4000, 6000, |_| Ok(()))
            .unwrap();
        assert!(moved);
        assert_eq!(r.agent(&lead).unwrap().unwrap().last_seen_ms, 6000);
        r.prune(&lead, 6000).unwrap();
    }
}
