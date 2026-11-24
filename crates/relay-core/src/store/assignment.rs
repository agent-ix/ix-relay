// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The ordered per-lane assignment lists and their bounded transition history (FR-072).

use std::sync::LazyLock;

use super::{
    Relay, Result, decode, encode,
    inflight::{CachedScript, slim_script},
};
use crate::{
    Address, Error,
    assignment::{
        ASSIGN_EVENTS_MAX, ASSIGN_EVENTS_MAX_AGE_MS, AssignError, AssignEventPage,
        AssignTransition, AssignWrite, AssignWriter, AssignedTicket, AssignmentList, FollowOutcome,
        MAX_ASSIGNED_LANES, MAX_ASSIGNED_TICKETS, MoveDefect, ProbeDefect, lane_address,
        validate_order_id,
    },
    fleet::fleet_registry,
};

/// Seconds the transition stream and the defect records keep after the last write.
const EVENTS_TTL_SECS: u64 = 604_800;

fn script(own: &str) -> CachedScript {
    CachedScript::new(slim_script(&format!(
        "{}\n{own}",
        include_str!("lua/assign_common.lua")
    )))
}

static WRITE: LazyLock<CachedScript> = LazyLock::new(|| script(include_str!("lua/assign.lua")));
static MOVE: LazyLock<CachedScript> = LazyLock::new(|| script(include_str!("lua/assign_move.lua")));

/// What the write script answers: status, version, write time, the list's entries.
type WriteReply = (i64, i64, i64, Vec<String>);

fn refusal(reply: &WriteReply, lane: &Address) -> Error {
    let first = reply.3.first().cloned().unwrap_or_default();
    let reason = match reply.0 {
        -1 => AssignError::TooManyTickets,
        -2 => AssignError::HostRegistryStale(lane.to_string()),
        -3 => AssignError::DuplicateTicket(first),
        -4 => AssignError::NoSuchTicket(first),
        -5 => AssignError::ReorderMismatch,
        -6 => AssignError::LaneLimit,
        -7 => AssignError::UnreadableStorage,
        other => {
            return Error::UnexpectedScriptReply {
                script: "assign",
                reply: other.to_string(),
            };
        }
    };
    reason.into()
}

impl Relay {
    /// Applies `write` to `lane`'s list and records the transition, atomically.
    /// `order_id` names the order that carried the write and is stored in the
    /// transition. Refused, with nothing changed, unless `writer` holds the
    /// orchestrator role and is not the lane itself (the guard lives here, so every
    /// caller has it); for a lane that no registry lists; for a lane of a host whose
    /// lease has expired (the refusal raises the host's probe defect, see
    /// [`Relay::assignment_probe_defects`]); and for a write that breaks a bound of
    /// FR-072. The list returned is the one this write left, as the script saw it.
    ///
    /// # Errors
    /// [`Error::Assign`] with the reason; [`Error::Redis`] when Redis fails.
    pub fn assign(
        &mut self,
        writer: &AssignWriter,
        lane: &str,
        write: &AssignWrite,
        order_id: Option<&str>,
    ) -> Result<AssignmentList> {
        let lane = lane_address(lane)?;
        writer.validate(&lane)?;
        write.validate()?;
        validate_order_id(order_id)?;
        // The registry's spelling of the address names the list, so case never splits one.
        let (lane, stale) = self
            .registry_lane(&lane)?
            .ok_or_else(|| AssignError::UnknownLane(lane.to_string()))?;
        let mut reply = self.write_once(writer, &lane, write, order_id, stale)?;
        if reply.0 == -6 {
            // The lane limit counts lists of lanes that left the registry: drop the
            // ones past retention and try once more.
            self.prune_assignments()?;
            reply = self.write_once(writer, &lane, write, order_id, stale)?;
        }
        if reply.0 != 1 {
            return Err(refusal(&reply, &lane));
        }
        let tickets = reply
            .3
            .iter()
            .map(|e| decode::<AssignedTicket>("assignment list", e))
            .collect::<Result<Vec<_>>>()?;
        Ok(AssignmentList {
            lane: lane.to_string(),
            version: u64::try_from(reply.1).unwrap_or(0),
            written_at_ms: reply.2,
            tickets,
        })
    }

    fn write_once(
        &mut self,
        writer: &AssignWriter,
        lane: &Address,
        write: &AssignWrite,
        order_id: Option<&str>,
        stale: bool,
    ) -> Result<WriteReply> {
        let (op, payload): (&str, Vec<String>) = match (stale, write) {
            (true, _) => ("stale", Vec::new()),
            (false, AssignWrite::Set(t) | AssignWrite::Append(t)) => {
                (write.op(), t.iter().map(encode).collect())
            }
            (false, AssignWrite::Remove(id)) => (write.op(), vec![id.clone()]),
            (false, AssignWrite::Reorder(ids)) => (write.op(), ids.clone()),
        };
        let name = lane.to_string();
        Ok(WRITE.run(&mut self.conn, |cmd| {
            cmd.arg(7)
                .arg(self.keys.assign_list_key(lane))
                .arg(self.keys.assign_meta_key(lane))
                .arg(self.keys.assign_lanes_key())
                .arg(self.keys.assign_events_key())
                .arg(self.keys.assign_events_start_key())
                .arg(self.keys.assign_defects_key())
                .arg(self.keys.assign_move_defects_key())
                .arg(op)
                .arg(&name)
                .arg(&writer.name)
                .arg(order_id.unwrap_or_default())
                .arg(MAX_ASSIGNED_TICKETS)
                .arg(MAX_ASSIGNED_LANES)
                .arg(ASSIGN_EVENTS_MAX)
                .arg(ASSIGN_EVENTS_MAX_AGE_MS)
                .arg(EVENTS_TTL_SECS)
                .arg(lane.server());
            for item in &payload {
                cmd.arg(item);
            }
        })?)
    }

    /// `lane` as the fleet registry (the unexpired host lists and the registrations)
    /// spells it, folding case as [`Address`] equality does, and whether only an expired
    /// host lists it; `None` when no registry lists it.
    fn registry_lane(&mut self, lane: &Address) -> Result<Option<(Address, bool)>> {
        let lists = self.lane_lists()?;
        let registrations: Vec<String> = self
            .agents()?
            .registrations
            .iter()
            .filter_map(|r| {
                r.verified_place()
                    .map(|p| p.address(&r.machine).to_string())
            })
            .collect();
        Ok(fleet_registry(&lists.lists, &registrations, lists.now_ms)
            .into_iter()
            .find_map(|l| {
                let listed = lane_address(&l.address).ok()?;
                (listed == *lane).then_some((listed, l.stale))
            }))
    }

    /// The list of `lane` under whatever spelling, or `None` when none was ever written.
    /// The address is resolved through the registry as a write resolves it, and read as
    /// given when no registry lists it. Changes nothing.
    ///
    /// # Errors
    /// [`Error::Assign`] for an address that is not a lane; [`Error::Redis`] or
    /// [`Error::Decode`] when Redis fails or a stored entry does not decode.
    pub fn assignment(&mut self, lane: &str) -> Result<Option<AssignmentList>> {
        let asked = lane_address(lane)?;
        let lane = self.registry_lane(&asked)?.map_or(asked, |(lane, _)| lane);
        let rows: Vec<ListRow> = redis::Script::new(READ_ONE)
            .key(self.keys.assign_list_key(&lane))
            .key(self.keys.assign_meta_key(&lane))
            .arg(lane.to_string())
            .invoke(&mut self.conn)?;
        rows.into_iter()
            .next()
            .map(read_list)
            .transpose()
            .map(Option::flatten)
    }

    /// Every lane's list that holds tickets, sorted by lane. Changes nothing.
    ///
    /// # Errors
    /// [`Error::Redis`] or [`Error::Decode`] when Redis fails or a stored entry does
    /// not decode.
    pub fn assignments(&mut self) -> Result<Vec<AssignmentList>> {
        let rows: Vec<ListRow> = redis::Script::new(READ_ALL)
            .key(self.keys.assign_lanes_key())
            .arg(self.keys.assign_list_stem())
            .arg(self.keys.assign_meta_stem())
            .invoke(&mut self.conn)?;
        let mut lists = rows
            .into_iter()
            .filter_map(|row| read_list(row).transpose())
            .collect::<Result<Vec<_>>>()?;
        lists.sort_by(|a, b| a.lane.cmp(&b.lane));
        Ok(lists)
    }

    /// Moves the list of `from` to `to` after a lane's move or rename (FR-043) and
    /// records one transition naming both addresses. The addresses are compared as
    /// written: a case-only rename moves the list.
    ///
    /// A list that cannot follow (`to` already holds one, as when two lanes swap tab
    /// names, or the store fails) stays under `from`: the outcome is
    /// [`FollowOutcome::Failed`], a `move_failed` transition and a [`MoveDefect`] are
    /// recorded when the store takes them, and no list is overwritten.
    ///
    /// # Errors
    /// [`Error::Assign`] for an address that is not a lane.
    pub fn assignment_follow(&mut self, from: &str, to: &str) -> Result<FollowOutcome> {
        let (from, to) = (lane_address(from)?, lane_address(to)?);
        let (from, to) = (from.to_string(), to.to_string());
        if from == to {
            return Ok(FollowOutcome::Unchanged);
        }
        let status = match self.run_move(&from, &to, "move", "") {
            Ok(status) => status,
            Err(error) => return Ok(self.move_failed(&from, &to, &error.to_string())),
        };
        Ok(match status {
            1 => FollowOutcome::Moved,
            0 => FollowOutcome::NoList,
            -8 => FollowOutcome::Failed("target_occupied".into()),
            -7 => self.move_failed(&from, &to, "unreadable_storage"),
            other => FollowOutcome::Failed(format!("unexpected move reply {other}")),
        })
    }

    /// Records a failed move the caller saw, as far as the store takes it.
    fn move_failed(&mut self, from: &str, to: &str, reason: &str) -> FollowOutcome {
        let _unrecorded = self.run_move(from, to, "fail", reason);
        FollowOutcome::Failed(reason.to_owned())
    }

    fn run_move(&mut self, from: &str, to: &str, mode: &str, reason: &str) -> Result<i64> {
        let (from_lane, to_lane) = (lane_address(from)?, lane_address(to)?);
        Ok(MOVE.run(&mut self.conn, |cmd| {
            cmd.arg(8)
                .arg(self.keys.assign_list_key(&from_lane))
                .arg(self.keys.assign_meta_key(&from_lane))
                .arg(self.keys.assign_lanes_key())
                .arg(self.keys.assign_events_key())
                .arg(self.keys.assign_events_start_key())
                .arg(self.keys.assign_list_key(&to_lane))
                .arg(self.keys.assign_meta_key(&to_lane))
                .arg(self.keys.assign_move_defects_key())
                .arg(from)
                .arg(to)
                .arg("relay")
                .arg("")
                .arg(ASSIGN_EVENTS_MAX)
                .arg(ASSIGN_EVENTS_MAX_AGE_MS)
                .arg(EVENTS_TTL_SECS)
                .arg(mode)
                .arg(reason);
        })?)
    }

    /// The hosts whose stale lanes were written to and that have not refreshed since.
    ///
    /// # Errors
    /// [`Error::Redis`] when Redis fails.
    pub fn assignment_probe_defects(&mut self) -> Result<Vec<ProbeDefect>> {
        let raw: std::collections::BTreeMap<String, String> = redis::cmd("HGETALL")
            .arg(self.keys.assign_defects_key())
            .query(&mut self.conn)?;
        Ok(raw
            .iter()
            .filter(|(field, _)| !field.starts_with("count:"))
            .map(|(host, last)| ProbeDefect {
                host: host.clone(),
                refused: raw
                    .get(&format!("count:{host}"))
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0),
                last_ms: last.parse().unwrap_or(0),
            })
            .collect())
    }

    /// The lists that did not follow their lane and have not been settled by a write.
    ///
    /// # Errors
    /// [`Error::Redis`] when Redis fails.
    pub fn assignment_move_defects(&mut self) -> Result<Vec<MoveDefect>> {
        let raw: std::collections::BTreeMap<String, String> = redis::cmd("HGETALL")
            .arg(self.keys.assign_move_defects_key())
            .query(&mut self.conn)?;
        // One record per (from, to) pair, under the field `from>to`.
        let mut found: Vec<MoveDefect> = raw
            .values()
            .filter_map(|json| serde_json::from_str(json).ok())
            .collect();
        found.sort_by(|a: &MoveDefect, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
        found.dedup();
        Ok(found)
    }

    /// A move defect ends when the list is no longer under `from`; while it is, the defect
    /// stays, however long ago it was raised.
    fn drop_resolved_move_defects(&mut self) -> Result<()> {
        let key = self.keys.assign_move_defects_key();
        let raw: std::collections::BTreeMap<String, String> =
            redis::cmd("HGETALL").arg(&key).query(&mut self.conn)?;
        for (field, json) in raw {
            let Ok(defect) = serde_json::from_str::<MoveDefect>(&json) else {
                continue;
            };
            let Ok(from) = lane_address(&defect.from) else {
                continue;
            };
            let held: bool = redis::cmd("EXISTS")
                .arg(self.keys.assign_list_key(&from))
                .query(&mut self.conn)?;
            if !held {
                redis::cmd("HDEL")
                    .arg(&key)
                    .arg(&field)
                    .query::<()>(&mut self.conn)?;
            }
        }
        Ok(())
    }

    /// Ends `machine`'s probe defect: its lanes are listed again.
    pub(super) fn clear_assignment_defect(&mut self, machine: &str) -> Result<()> {
        let key = self.keys.assign_defects_key();
        redis::pipe()
            .hdel(&key, machine)
            .hdel(&key, format!("count:{machine}"))
            .query::<()>(&mut self.conn)?;
        Ok(())
    }

    /// Removes the lists of lanes that no registry lists and that were last written
    /// longer ago than the FR-066 retention (seven days), and the index entries whose
    /// list is gone, so lists of lanes that left never fill the lane limit. Returns how
    /// many lanes it dropped.
    ///
    /// # Errors
    /// [`Error::Redis`] when Redis fails.
    pub fn prune_assignments(&mut self) -> Result<usize> {
        let lanes_key = self.keys.assign_lanes_key();
        let members: Vec<String> = redis::cmd("SMEMBERS")
            .arg(&lanes_key)
            .query(&mut self.conn)?;
        let lists = self.lane_lists()?;
        let registrations: Vec<String> = self
            .agents()?
            .registrations
            .iter()
            .filter_map(|r| {
                r.verified_place()
                    .map(|p| p.address(&r.machine).to_string())
            })
            .collect();
        let registry = fleet_registry(&lists.lists, &registrations, lists.now_ms);
        let mut dropped = 0;
        for member in members {
            let Ok(lane) = lane_address(&member) else {
                let () = redis::cmd("SREM")
                    .arg(&lanes_key)
                    .arg(&member)
                    .query(&mut self.conn)?;
                continue;
            };
            let listed = registry
                .iter()
                .any(|l| lane_address(&l.address).is_ok_and(|a| a == lane));
            let written: Option<String> = redis::cmd("HGET")
                .arg(self.keys.assign_meta_key(&lane))
                .arg("at_ms")
                .query(&mut self.conn)?;
            let aged = written
                .and_then(|ms| ms.parse::<i64>().ok())
                .is_none_or(|ms| lists.now_ms.saturating_sub(ms) >= ASSIGN_EVENTS_MAX_AGE_MS);
            if !listed && aged {
                redis::pipe()
                    .del(self.keys.assign_list_key(&lane))
                    .del(self.keys.assign_meta_key(&lane))
                    .srem(&lanes_key, &member)
                    .query::<()>(&mut self.conn)?;
                dropped += 1;
            }
        }
        self.drop_resolved_move_defects()?;
        Ok(dropped)
    }

    /// Transitions after the exclusive cursor `after` (`0-0` for the start), at most
    /// `limit` (1 to 1000), under the bounds of FR-066. Changes nothing.
    ///
    /// # Errors
    /// [`Error::Assign`] for a malformed cursor or a limit out of range; [`Error::Redis`]
    /// or [`Error::Decode`] when Redis fails or a stored record does not decode.
    pub fn assignment_events(&mut self, after: &str, limit: usize) -> Result<AssignEventPage> {
        let tuple = |s: &str| -> Option<(u64, u64)> {
            let (ms, seq) = s.split_once('-')?;
            Some((ms.parse().ok()?, seq.parse().ok()?))
        };
        let Some(cursor) = tuple(after).filter(|_| (1..=1000).contains(&limit)) else {
            return Err(AssignError::InvalidField("history cursor/limit").into());
        };
        let (start, first, rows, now): (
            String,
            redis::streams::StreamRangeReply,
            redis::streams::StreamRangeReply,
            String,
        ) = redis::Script::new(READ_EVENTS)
            .key(self.keys.assign_events_key())
            .key(self.keys.assign_events_start_key())
            .arg(after)
            .arg(limit + 1)
            .invoke(&mut self.conn)?;
        let observed_at_ms: i64 = now
            .parse()
            .map_err(|_| AssignError::InvalidField("history server time"))?;
        let cutoff_ms = observed_at_ms
            .saturating_sub(ASSIGN_EVENTS_MAX_AGE_MS)
            .max(0);
        let floor = format!("{cutoff_ms}-0");
        let oldest = first.ids.first().map(|row| row.id.as_str());
        let oldest_ms = oldest
            .and_then(tuple)
            .and_then(|(ms, _)| i64::try_from(ms).ok());
        let available_from_ms = start
            .parse::<i64>()
            .ok()
            .map(|start| start.max(oldest_ms.unwrap_or(start)).max(cutoff_ms));
        let lower = oldest.unwrap_or(&floor);
        let truncated = after != "0-0"
            && available_from_ms.is_some()
            && tuple(lower).is_some_and(|lower| cursor < lower);
        let has_more = rows.ids.len() > limit;
        let mut events = Vec::new();
        for row in rows.ids.into_iter().take(limit) {
            let text: String = row
                .get("record")
                .ok_or(AssignError::InvalidField("stored history"))?;
            let mut event: AssignTransition = decode("assignment history", &text)?;
            event.id = row.id;
            events.push(event);
        }
        let next_cursor = events.last().map_or_else(
            || {
                if cursor < tuple(&floor).unwrap_or_default() {
                    floor.clone()
                } else {
                    after.to_owned()
                }
            },
            |e| e.id.clone(),
        );
        Ok(AssignEventPage {
            version: 1,
            available_from_ms,
            observed_at_ms,
            truncated,
            next_cursor,
            has_more,
            events,
        })
    }
}

impl Relay {
    /// [`Relay::publish_lane_list`]'s companion: a host that lists its lanes again is
    /// no longer stale, so its probe defect ends.
    pub(super) fn host_refreshed(&mut self, machine: &str) -> Result<()> {
        self.clear_assignment_defect(machine)
    }
}

fn read_list(row: ListRow) -> Result<Option<AssignmentList>> {
    let (lane, version, at_ms, entries) = row;
    let Some(version) = version else {
        return Ok(None);
    };
    let tickets = entries
        .iter()
        .map(|e| decode::<AssignedTicket>("assignment list", e))
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(AssignmentList {
        lane,
        version: version.parse().unwrap_or(0),
        written_at_ms: at_ms.and_then(|n| n.parse().ok()).unwrap_or(0),
        tickets,
    }))
}

/// One list as the read scripts return it: lane, version, write time, entries.
type ListRow = (String, Option<String>, Option<String>, Vec<String>);

const READ_ONE: &str = r"
local meta = redis.call('HMGET', KEYS[2], 'version', 'at_ms')
return {{ARGV[1], meta[1] or false, meta[2] or false, redis.call('LRANGE', KEYS[1], 0, -1)}}
";

const READ_ALL: &str = r"
local out = {}
for _, lane in ipairs(redis.call('SMEMBERS', KEYS[1])) do
  local meta = redis.call('HMGET', ARGV[2] .. lane, 'version', 'at_ms')
  out[#out + 1] = {lane, meta[1] or false, meta[2] or false, redis.call('LRANGE', ARGV[1] .. lane, 0, -1)}
end
return out
";

const READ_EVENTS: &str = r"
local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
local cutoff = math.max(0, now - 604800000)
local floor = string.format('%.0f', cutoff) .. '-0'
local lower = '(' .. ARGV[1]
if tonumber(string.match(ARGV[1], '^(%d+)%-')) < cutoff then lower = floor end
local first = redis.call('XRANGE', KEYS[1], floor, '+', 'COUNT', 1)
local rows = redis.call('XRANGE', KEYS[1], lower, '+', 'COUNT', ARGV[2])
return {redis.call('GET', KEYS[2]) or '', first, rows, string.format('%.0f', now)}
";

impl Relay {
    /// A lane whose place changed keeps its list under the new address (FR-072). The
    /// registration has moved already and stays moved, so a follow that cannot happen
    /// is not an error of the move: it is logged here, recorded as a `move_failed`
    /// transition and a standing defect that `agent-msg doctor` reports, and the list
    /// stays under the old address until an orchestrator settles it.
    pub(super) fn follow_assignments(
        &mut self,
        old: Option<&crate::Place>,
        new: &crate::Place,
        machine: &crate::MachineName,
    ) {
        let Some(old) = old else { return };
        let (from, to) = (
            old.address(machine).to_string(),
            new.address(machine).to_string(),
        );
        match self.assignment_follow(&from, &to) {
            Ok(FollowOutcome::Failed(reason)) => {
                eprintln!(
                    "ix-relay: assignment list of {from} did not follow its lane to {to}: {reason}"
                );
            }
            Err(error) => {
                eprintln!(
                    "ix-relay: assignment list of {from} did not follow its lane to {to}: {error}"
                );
            }
            Ok(_) => {}
        }
    }
}
