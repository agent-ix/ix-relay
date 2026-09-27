//! Relinks, renames and replaced panes: the registration changes that follow
//! herdr's ids (FR-043, FR-032), each with the record that says it happened.
//!
//! Every change of a registration here is one script: it swaps the stored JSON
//! only if it is still exactly the JSON that was read, and in the same
//! transaction appends the [`MoveEvent`] to the capped stream, so a change and
//! its event stand or fall together. The new JSON is spliced into what was read
//! (not re-encoded from a [`Registration`]), so members a newer binary wrote
//! survive.
//!
//! WRITE DISCIPLINE (as `store/lua/common.lua`). Redis does not roll back what a
//! script wrote before it errored, so the script first checks the type of every
//! key it may write and refuses, having written nothing, with
//! [`Error::MoveUnreadable`]; only then does it write, the registration last. A
//! caller is therefore never told a move failed after the registration moved.

use std::{collections::BTreeMap, sync::LazyLock};

use redis::{Commands, streams::StreamRangeReply};

use super::{Relay, Result, decode_bytes, encode};
use crate::{
    address::Place,
    error::Error,
    ids::PaneId,
    message::Registration,
    moves::{MoveEvent, MoveKind, PaneReplaced},
    name::AgentName,
};

/// Roughly how many entries the moves stream keeps (`XADD MAXLEN ~`), like the
/// hook health stream.
const HEALTH_MOVES_CAP: usize = 1_000;

/// How long one lead's replaced-pane marker lives after it was written, in
/// seconds: a marker of a lead that never came back goes away by itself, and the
/// keys are bounded by the leads that ever were.
const REPLACED_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// What [`Relay::recent_moves`] read: the usable events, and how many it could not use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Moves {
    /// The events, newest first.
    pub entries: Vec<MoveEvent>,
    /// Entries the read left out: a missing field or a record that does not decode.
    pub unreadable: usize,
}

/// What [`Relay::panes_replaced`] read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanesReplaced {
    /// The markers, by the lead's bus name.
    pub found: BTreeMap<AgentName, PaneReplaced>,
    /// Markers the read left out because they do not decode.
    pub unreadable: usize,
}

/// What [`Relay::refresh_place`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceRefresh {
    /// The registration is absent, has that place already, or was registered again
    /// meanwhile: nothing changed.
    Unchanged,
    /// The verified place or its freshness changed without a workspace/tab label
    /// change, the caller asked for no event, or the lead had no place before.
    Refreshed,
    /// A workspace or tab label of the lead changed: the place is updated and this
    /// event is recorded.
    Renamed(Box<MoveEvent>),
}

/// The one script every recorded change runs: refuses (returns `2`) when a key it
/// may write has the wrong type, else swaps the registration if it is still
/// `ARGV[2]` (`0` when it is not), appends the event when `ARGV[6]` is not empty,
/// and (`ARGV[7]` is `1`) removes the lead's replaced-pane marker. The
/// registration is written last.
static SWAP_AND_RECORD: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        r"
    local function readable(key, want)
        local kind = redis.call('TYPE', key)['ok']
        return kind == 'none' or kind == want
    end
    if not readable(KEYS[1], 'hash') or not readable(KEYS[2], 'stream')
        or not readable(KEYS[3], 'string') then
        return 2
    end
    if redis.call('HGET', KEYS[1], ARGV[1]) ~= ARGV[2] then
        return 0
    end
    if ARGV[6] ~= '' then
        redis.call('XADD', KEYS[2], 'MAXLEN', '~', ARGV[4], '*',
                   'machine', ARGV[5], 'lead', ARGV[1], 'rec', ARGV[6])
    end
    if ARGV[7] == '1' then
        redis.call('DEL', KEYS[3])
    end
    redis.call('HSET', KEYS[1], ARGV[1], ARGV[3])
    return 1",
    )
});

impl Relay {
    /// Sets `name`'s recorded [`Place`] to `place` — how `relay-watcher`, which
    /// sees herdr's live state, keeps `server:workspace[:tab[:pane]]` addresses true
    /// after a tab closes (herdr renumbers unlabelled tabs) or a workspace or
    /// tab is renamed.
    ///
    /// When the lead's own workspace or tab *label* changed and `record` is set,
    /// a [`MoveKind::Rename`] event at `at_ms` is recorded in the same script
    /// ([`PlaceRefresh::Renamed`]). A change to stable identity or freshness with
    /// the same workspace/tab labels is not a rename ([`PlaceRefresh::Refreshed`]). Nothing changes, and
    /// [`PlaceRefresh::Unchanged`] is returned, when `name` is not registered,
    /// already has `place`, or re-registered while this ran (that registration
    /// carries a fresher place of its own, so it is dropped rather than
    /// overwritten).
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out; [`Error::Decode`] when the
    /// stored record is not a registration; [`Error::MoveUnreadable`] when a key
    /// the script writes has the wrong type (nothing changed).
    pub fn refresh_place(
        &mut self,
        name: &AgentName,
        place: &Place,
        at_ms: i64,
        record: bool,
    ) -> Result<PlaceRefresh> {
        self.refresh_place_between(name, place, at_ms, record, |_| Ok(()))
    }

    /// Removes cached address evidence when the watcher can no longer verify
    /// this exact registration against Herdr. The whole stored record is used
    /// as a compare-and-set guard so a concurrent re-registration is untouched.
    pub fn invalidate_place(&mut self, seen: &Registration) -> Result<bool> {
        let key = self.keys.agents_key();
        let raw: Option<Vec<u8>> = self.conn.hget(&key, seen.name.as_str())?;
        let Some(raw) = raw else {
            return Ok(false);
        };
        let current: Registration = decode_bytes(&key, &raw)?;
        if current != *seen || current.place.is_none() {
            return Ok(false);
        }
        self.swap_recorded(
            &raw,
            &current.name,
            &current.machine,
            None,
            false,
            |_| Ok(()),
            |members| {
                members.insert("place".to_owned(), serde_json::Value::Null);
                Ok(())
            },
        )
    }

    /// [`Relay::refresh_place`] running `between` after the read and before the
    /// script: the seam a test uses to change the registration in exactly the
    /// window the script guards.
    fn refresh_place_between(
        &mut self,
        name: &AgentName,
        place: &Place,
        at_ms: i64,
        record: bool,
        between: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<PlaceRefresh> {
        let key = self.keys.agents_key();
        let raw: Option<Vec<u8>> = self.conn.hget(&key, name.as_str())?;
        let Some(raw) = raw else {
            return Ok(PlaceRefresh::Unchanged);
        };
        let reg: Registration = decode_bytes(&key, &raw)?;
        if reg.place.as_ref() == Some(place) {
            return Ok(PlaceRefresh::Unchanged);
        }
        let relabelled = reg
            .place
            .as_ref()
            .is_some_and(|old| old.workspace != place.workspace || old.tab != place.tab);
        let event = (record && relabelled).then(|| MoveEvent {
            lead: reg.name.clone(),
            machine: reg.machine.clone(),
            kind: MoveKind::Rename,
            session_id: reg.session_id.clone(),
            previous_pane: None,
            pane: reg.herdr_pane.clone(),
            old_place: reg.place.clone(),
            new_place: Some(place.clone()),
            at_ms,
        });
        let swapped = self.swap_recorded(
            &raw,
            &reg.name,
            &reg.machine,
            event.as_ref(),
            false,
            between,
            |members| {
                members.insert("place".to_owned(), to_value(&key, place)?);
                Ok(())
            },
        )?;
        Ok(match (swapped, event) {
            (false, _) => PlaceRefresh::Unchanged,
            (true, Some(event)) => PlaceRefresh::Renamed(Box::new(event)),
            (true, None) => PlaceRefresh::Refreshed,
        })
    }

    /// Points `seen`'s registration at `to`, with `place`, stamps its
    /// `last_seen_ms` with `at_ms` (herdr has just shown its session live there),
    /// and records a [`MoveKind::Relink`] event at `at_ms`, in one script
    /// (FR-043): the lead's session moved to another pane.
    ///
    /// Compare-and-set against the whole registration the caller read: nothing
    /// changes, and `None` is returned, when the lead is gone (a pruned lead is
    /// never brought back), when what is stored is not exactly `seen` (it was
    /// registered again, re-placed or refreshed meanwhile), when `seen` has no
    /// session id (nothing to follow) or when `to` is the pane it already names.
    /// It never touches another machine's registration: `seen` is compared
    /// whole, machine included.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out; [`Error::Decode`] when the
    /// stored record is not a registration; [`Error::MoveUnreadable`] when a key
    /// the script writes has the wrong type (nothing changed).
    pub fn relink_pane(
        &mut self,
        seen: &Registration,
        to: &PaneId,
        place: &Place,
        at_ms: i64,
    ) -> Result<Option<MoveEvent>> {
        self.relink_pane_between(seen, to, place, at_ms, |_| Ok(()))
    }

    /// [`Relay::relink_pane`] running `between` after the read and before the
    /// script: the seam a test uses to change the registration in exactly the
    /// window the script guards.
    fn relink_pane_between(
        &mut self,
        seen: &Registration,
        to: &PaneId,
        place: &Place,
        at_ms: i64,
        between: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Option<MoveEvent>> {
        if seen.session_id.is_none() || seen.herdr_pane.as_ref() == Some(to) {
            return Ok(None);
        }
        let key = self.keys.agents_key();
        let raw: Option<Vec<u8>> = self.conn.hget(&key, seen.name.as_str())?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let stored: Registration = decode_bytes(&key, &raw)?;
        if stored != *seen {
            return Ok(None);
        }
        let event = MoveEvent {
            lead: seen.name.clone(),
            machine: seen.machine.clone(),
            kind: MoveKind::Relink,
            session_id: seen.session_id.clone(),
            previous_pane: seen.herdr_pane.clone(),
            pane: Some(to.clone()),
            old_place: seen.place.clone(),
            new_place: Some(place.clone()),
            at_ms,
        };
        let stamp = at_ms.max(seen.last_seen_ms);
        let swapped = self.swap_recorded(
            &raw,
            &seen.name,
            &seen.machine,
            Some(&event),
            true,
            between,
            |members| {
                members.insert("herdr_pane".to_owned(), to_value(&key, to)?);
                members.insert("place".to_owned(), to_value(&key, place)?);
                members.insert("last_seen_ms".to_owned(), serde_json::Value::from(stamp));
                Ok(())
            },
        )?;
        Ok(swapped.then_some(event))
    }

    /// Runs [`SWAP_AND_RECORD`]: `edit` splices the change into the stored JSON
    /// `raw`, and the script applies it only if `raw` is still what is stored.
    /// Returns whether it did.
    fn swap_recorded(
        &mut self,
        raw: &[u8],
        lead: &AgentName,
        machine: &crate::ids::MachineName,
        event: Option<&MoveEvent>,
        clear_marker: bool,
        between: impl FnOnce(&mut Self) -> Result<()>,
        edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> Result<()>,
    ) -> Result<bool> {
        let key = self.keys.agents_key();
        let mut stored: serde_json::Value = decode_bytes(&key, raw)?;
        let Some(members) = stored.as_object_mut() else {
            return Ok(false);
        };
        edit(members)?;
        between(self)?;
        let reply: i64 = SWAP_AND_RECORD
            .key(&key)
            .key(self.keys.health_moves_key())
            .key(self.keys.health_replaced_key(lead))
            .arg(lead.as_str())
            .arg(raw)
            .arg(encode(&stored))
            .arg(HEALTH_MOVES_CAP)
            .arg(machine.as_str())
            .arg(event.map_or_else(String::new, encode))
            .arg(if clear_marker { "1" } else { "0" })
            .invoke(&mut self.conn)?;
        match reply {
            1 => Ok(true),
            0 => Ok(false),
            2 => Err(Error::MoveUnreadable { lead: lead.clone() }),
            other => Err(Error::UnexpectedScriptReply {
                script: "swap_and_record",
                reply: other.to_string(),
            }),
        }
    }

    /// Up to `max` events from the moves stream, newest first.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn recent_moves(&mut self, max: usize) -> Result<Moves> {
        let reply: StreamRangeReply =
            self.conn
                .xrevrange_count(self.keys.health_moves_key(), "+", "-", max)?;
        Ok(moves_of(&reply))
    }

    /// The events at or after `since_ms` (Unix milliseconds), newest first, at most
    /// `max` of them, and whether more than `max` matched: the stream is read by
    /// its id, which is a millisecond timestamp, so the count is not cut at some
    /// other bound first. An event's own `at_ms` is used when it decodes.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn moves_since(&mut self, since_ms: i64, max: usize) -> Result<(Moves, bool)> {
        let start = since_ms.max(0).to_string();
        let reply: StreamRangeReply = self.conn.xrevrange_count(
            self.keys.health_moves_key(),
            "+",
            &start,
            max.saturating_add(1),
        )?;
        let mut moves = moves_of(&reply);
        let more = reply.ids.len() > max;
        if more {
            moves.entries.truncate(max.saturating_sub(moves.unreadable));
        }
        Ok((moves, more))
    }

    /// Records that `name`'s pane now hosts another session (FR-032), keeping the
    /// first `since_ms` when the same finding is already recorded. Returns whether
    /// a new finding was written. The marker is its own key with an expiry.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out, and when the key holds a
    /// value of the wrong type.
    pub fn mark_pane_replaced(&mut self, name: &AgentName, found: &PaneReplaced) -> Result<bool> {
        let key = self.keys.health_replaced_key(name);
        let held: Option<Vec<u8>> = self.conn.get(&key)?;
        let same = held
            .and_then(|raw| serde_json::from_slice::<PaneReplaced>(&raw).ok())
            .is_some_and(|old| {
                old.pane == found.pane && old.seen_last_seen_ms == found.seen_last_seen_ms
            });
        if same {
            return Ok(false);
        }
        let () = redis::cmd("SET")
            .arg(&key)
            .arg(encode(found))
            .arg("EX")
            .arg(REPLACED_TTL_SECS)
            .query(&mut self.conn)?;
        Ok(true)
    }

    /// Removes `name`'s replaced-pane marker, if any. Returns whether there was one.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn clear_pane_replaced(&mut self, name: &AgentName) -> Result<bool> {
        let removed: i64 = self.conn.del(self.keys.health_replaced_key(name))?;
        Ok(removed > 0)
    }

    /// The replaced-pane markers of `names`, in one request. A key of the wrong
    /// type reads as no marker, as every `MGET` here does.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn panes_replaced(&mut self, names: &[&AgentName]) -> Result<PanesReplaced> {
        let mut out = PanesReplaced::default();
        if names.is_empty() {
            return Ok(out);
        }
        let keys: Vec<String> = names
            .iter()
            .map(|n| self.keys.health_replaced_key(n))
            .collect();
        let raws: Vec<Option<Vec<u8>>> = redis::cmd("MGET").arg(&keys).query(&mut self.conn)?;
        for (name, raw) in names.iter().zip(raws) {
            let Some(raw) = raw else {
                continue;
            };
            match serde_json::from_slice::<PaneReplaced>(&raw) {
                Ok(marker) => {
                    out.found.insert((*name).clone(), marker);
                }
                Err(_) => out.unreadable += 1,
            }
        }
        Ok(out)
    }
}

/// The events of one stream reply, newest first as read, and how many entries did
/// not decode.
fn moves_of(reply: &StreamRangeReply) -> Moves {
    let mut entries = Vec::with_capacity(reply.ids.len());
    let mut unreadable = 0;
    for entry in &reply.ids {
        let read = entry
            .get::<String>("rec")
            .and_then(|rec| serde_json::from_str::<MoveEvent>(&rec).ok());
        match read {
            Some(event) => entries.push(event),
            None => unreadable += 1,
        }
    }
    Moves {
        entries,
        unreadable,
    }
}

/// `value` as JSON, for splicing into a stored record.
fn to_value<T: serde::Serialize>(key: &str, value: &T) -> Result<serde_json::Value> {
    serde_json::to_value(value).map_err(|source| Error::Decode {
        key: key.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

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

    fn registration(last_seen_ms: i64, pane: &str) -> Registration {
        Registration {
            name: "lead-a".parse().unwrap(),
            machine: "hub".parse().unwrap(),
            kind: "claude".into(),
            session_id: Some("sess-1".parse().unwrap()),
            cwd: None,
            herdr_pane: Some(pane.parse().unwrap()),
            place: Some(place("wsA")),
            last_seen_ms,
        }
    }

    fn place(workspace: &str) -> Place {
        Place {
            workspace: workspace.into(),
            tab: "1".into(),
            herdr_ids: None,
        }
    }

    /// The window between the read and the script: a prune, or a registration
    /// that arrives there, defeats the write, and the script leaves no event.
    ///
    /// Tracing: TC-043-04
    /// ACs: FR-043-AC-4
    #[test]
    fn tc_043_04_a_prune_or_a_registration_between_the_read_and_the_script_defeats_the_write() {
        let Some(mut r) = relay("moves-window") else {
            return;
        };
        let seen = registration(100, "w1:p3");
        let to: PaneId = "w2:p2".parse().unwrap();
        let lead = seen.name.clone();
        let cleanup = |r: &mut Relay| {
            let _ = r.prune(&lead, 100);
            let _ = r.prune(&lead, 200);
        };

        // Pruned in the window: not brought back, no event.
        r.register(&seen).unwrap();
        let moved = r
            .relink_pane_between(&seen, &to, &place("wsB"), 300, |r| {
                r.prune(&lead, 100).map(|_| ())
            })
            .unwrap();
        assert_eq!(moved, None);
        assert_eq!(r.agent(&lead).unwrap(), None);

        // Registered again in the window: the newer registration stays.
        r.register(&seen).unwrap();
        let newer = registration(200, "w1:p4");
        let moved = r
            .relink_pane_between(&seen, &to, &place("wsB"), 300, |r| r.register(&newer))
            .unwrap();
        assert_eq!(moved, None);
        assert_eq!(r.agent(&lead).unwrap(), Some(newer.clone()));

        // A rename in the same window loses the same way.
        let moved = r
            .refresh_place_between(&lead, &place("wsC"), 400, true, |r: &mut Relay| {
                r.register(&registration(250, "w1:p4"))
            })
            .unwrap();
        assert_eq!(moved, PlaceRefresh::Unchanged);
        assert_eq!(r.agent(&lead).unwrap().unwrap().last_seen_ms, 250);
        assert!(r.recent_moves(10).unwrap().entries.is_empty());
        cleanup(&mut r);
    }
}
