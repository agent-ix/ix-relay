// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Atomic, bounded reporting fields isolated from message inboxes.

use super::{decode, encode};
use crate::{
    Relay, Result,
    work::{AgentWork, WorkBinding, WorkError, WorkFocus, WorkReport, WorkScope, validate_text},
    work_history::{WorkChange, WorkEvent, WorkEventPage},
};
use serde::Serialize;
use std::collections::BTreeMap;

const WORK_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;
const IDENTITY_CHECK: &str = r#"
local stream_type = redis.call('TYPE', KEYS[2]).ok
local start_type = redis.call('TYPE', KEYS[3]).ok
local activity_type = redis.call('TYPE', KEYS[4]).ok
if (stream_type ~= 'none' and stream_type ~= 'stream') or
   (start_type ~= 'none' and start_type ~= 'string') or (activity_type ~= 'none' and activity_type ~= 'string') then return -4 end
if redis.call('HSTRLEN', KEYS[1], 'binding') > 65536 then return -3 end
local binding = redis.call('HGET', KEYS[1], 'binding')
if binding then
  local old = cjson.decode(binding)
  local new = cjson.decode(ARGV[1])
  if old.machine ~= new.machine or old.pane ~= new.pane or old.session ~= new.session
    or old.process_id ~= new.process_id or old.kind ~= new.kind then return -1 end
end
"#;
// Exact count cap and per-event input bounds keep this separate from the mail log.
// Server time establishes ordering even when producer clocks disagree.
const HISTORY: &str = r#"
if ARGV[5] ~= '' then
  local t = redis.call('TIME')
  local ms = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
  redis.call('SET', KEYS[3], tostring(ms), 'NX')
  redis.call('XADD', KEYS[2], 'MAXLEN', '=', 4096, '*',
    'at_ms', tostring(ms), 'binding', ARGV[1], 'change', ARGV[5])
  redis.call('XTRIM', KEYS[2], 'MINID', '=', tostring(math.max(0, ms - 604800000)) .. '-0')
  redis.call('EXPIRE', KEYS[2], 604800)
end
"#;
const UPDATE: &str = r#"
local agents = redis.call('HLEN', KEYS[1]) - redis.call('HEXISTS', KEYS[1], 'binding')
  - redis.call('HEXISTS', KEYS[1], 'scope') - redis.call('HEXISTS', KEYS[1], 'focus')
if string.sub(ARGV[2], 1, 6) == 'agent:' and redis.call('HEXISTS', KEYS[1], ARGV[2]) == 0
  and agents >= 128 then return -2 end
if ARGV[2] == 'activity' then
  redis.call('HSET', KEYS[1], 'binding', ARGV[1])
  redis.call('SET', KEYS[4], ARGV[3], 'EX', ARGV[4])
else
  redis.call('HSET', KEYS[1], 'binding', ARGV[1], ARGV[2], ARGV[3])
end
redis.call('EXPIRE', KEYS[1], ARGV[4])
return 1
"#;

const REMOVE: &str = r#"
redis.call('HDEL', KEYS[1], ARGV[2])
"#;

const READ: &str = r#"
if redis.call('HLEN', KEYS[1]) > 131 then return {-1, {}} end
local fields = redis.call('HKEYS', KEYS[1])
for _, field in ipairs(fields) do
  if string.len(field) > 134 or redis.call('HSTRLEN', KEYS[1], field) > 65536 then
    return {-1, {}}
  end
end
return {1, redis.call('HGETALL', KEYS[1])}
"#;

impl Relay {
    fn update_work<T: Serialize>(
        &mut self,
        binding: &WorkBinding,
        field: &str,
        value: Option<&T>,
        change: Option<&WorkChange>,
    ) -> Result<()> {
        binding.validate()?;
        let key = self
            .keys
            .work_key(&binding.machine, &binding.session, binding.process_id);
        let event = change.map(encode).unwrap_or_default();
        if event.len() + encode(binding).len() > 8192 {
            return Err(WorkError::ReportLimit.into());
        }
        // Remove UPDATE's return so state and history commit in one script.
        let mutation = if value.is_some() {
            UPDATE.replace("return 1", "")
        } else {
            REMOVE.into()
        };
        let reply: i64 =
            redis::Script::new(&format!("{IDENTITY_CHECK}{mutation}{HISTORY}return 1"))
                .key(&key)
                .key(self.keys.work_events_key())
                .key(self.keys.work_events_start_key())
                .key(self.keys.work_activity_key(
                    &binding.machine,
                    &binding.session,
                    binding.process_id,
                ))
                .arg(encode(binding))
                .arg(field)
                .arg(encode(&value))
                .arg(WORK_TTL_SECONDS)
                .arg(event)
                .invoke(&mut self.conn)?;
        match reply {
            1 => Ok(()),
            -1 => Err(WorkError::IdentityMismatch.into()),
            -2 => Err(WorkError::AgentLimit.into()),
            -3 => Err(WorkError::ReportLimit.into()),
            _ => Err(crate::Error::UnexpectedScriptReply {
                script: "work-update",
                reply: reply.to_string(),
            }),
        }
    }

    /// Replaces only this session's explicit scope, preserving agent and focus reports.
    pub fn report_work_scope(&mut self, binding: &WorkBinding, scope: &WorkScope) -> Result<()> {
        scope.validate()?;
        self.update_work(
            binding,
            "scope",
            Some(scope),
            Some(&WorkChange::Scope {
                scope: scope.into(),
            }),
        )
    }

    /// Replaces only this session's focus; it does not change tracker state.
    pub fn report_work_focus(&mut self, binding: &WorkBinding, focus: &WorkFocus) -> Result<()> {
        focus.validate()?;
        self.update_work(binding, "focus", Some(focus), None)
    }

    /// Upserts one agent assignment atomically, without overwriting other agents.
    pub fn report_work_agent(&mut self, binding: &WorkBinding, agent: &AgentWork) -> Result<()> {
        agent.validate()?;
        self.update_work(
            binding,
            &format!("agent:{}", agent.agent_id),
            Some(agent),
            Some(&WorkChange::Agent {
                agent: agent.into(),
            }),
        )
    }

    /// Removes a finished assignment when the lead no longer needs it displayed.
    pub fn remove_work_agent(&mut self, binding: &WorkBinding, agent_id: &str) -> Result<()> {
        validate_text(agent_id, "agent_id", 128)?;
        self.update_work::<()>(
            binding,
            &format!("agent:{agent_id}"),
            None,
            Some(&WorkChange::Remove {
                agent_id: agent_id.to_owned(),
            }),
        )
    }

    /// Corrects activity intent without claiming execution or completing any ticket.
    pub fn report_work_activity(&mut self, binding: &WorkBinding, paused: bool) -> Result<()> {
        self.update_work(
            binding,
            "activity",
            Some(&paused),
            Some(&WorkChange::Activity { paused }),
        )
    }

    /// Reads immutable transitions after a Redis stream cursor, without Herdr or inbox access.
    pub fn work_events(&mut self, after: &str, limit: usize) -> Result<WorkEventPage> {
        let valid = after
            .split_once('-')
            .is_some_and(|(ms, seq)| ms.parse::<u64>().is_ok() && seq.parse::<u64>().is_ok());
        if !valid || !(1..=1000).contains(&limit) {
            return Err(WorkError::InvalidField {
                field: "history cursor/limit",
            }
            .into());
        }
        let script = redis::Script::new(
            r#"
local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
local cutoff = math.max(0, now - 604800000)
local floor = string.format('%.0f', cutoff) .. '-0'
local lower = '(' .. ARGV[1]
if tonumber(string.match(ARGV[1], '^(%d+)%-')) < cutoff then lower = floor end
local first = redis.call('XRANGE', KEYS[1], floor, '+', 'COUNT', 1)
local rows = redis.call('XRANGE', KEYS[1], lower, '+', 'COUNT', ARGV[2])
return {redis.call('GET', KEYS[2]) or '', first, rows,
  string.format('%.0f', now)}
"#,
        );
        let (start, first, rows, now): (
            String,
            redis::streams::StreamRangeReply,
            redis::streams::StreamRangeReply,
            String,
        ) = script
            .key(self.keys.work_events_key())
            .key(self.keys.work_events_start_key())
            .arg(after)
            .arg(limit + 1)
            .invoke(&mut self.conn)?;
        let observed_at_ms: i64 = now.parse().map_err(|_| WorkError::InvalidField {
            field: "history server time",
        })?;
        let cutoff_ms = observed_at_ms.saturating_sub(604800000).max(0);
        let floor_cursor = format!("{cutoff_ms}-0");
        let oldest = first.ids.first().map(|row| row.id.as_str());
        let tuple = |s: &str| -> (u64, u64) {
            s.split_once('-')
                .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
                .unwrap_or_default()
        };
        let oldest_ms = oldest
            .map(|id| tuple(id).0)
            .and_then(|ms| i64::try_from(ms).ok());
        let available_from_ms = start
            .parse::<i64>()
            .ok()
            .map(|start| start.max(oldest_ms.unwrap_or(start)).max(cutoff_ms));
        let retained_lower = oldest.unwrap_or(&floor_cursor);
        let truncated =
            after != "0-0" && available_from_ms.is_some() && tuple(after) < tuple(retained_lower);
        let has_more = rows.ids.len() > limit;
        let mut events = Vec::new();
        for row in rows.ids.into_iter().take(limit) {
            let field = |name: &str| -> Result<String> {
                row.map
                    .get(name)
                    .ok_or(
                        WorkError::InvalidField {
                            field: "stored history",
                        }
                        .into(),
                    )
                    .and_then(|v| {
                        redis::from_redis_value(v.clone()).map_err(|_| {
                            WorkError::InvalidField {
                                field: "stored history value",
                            }
                            .into()
                        })
                    })
            };
            let binding: WorkBinding = decode("work history", &field("binding")?)?;
            binding.validate()?;
            let change: WorkChange = decode("work history", &field("change")?)?;
            let at_ms = field("at_ms")?
                .parse::<i64>()
                .map_err(|_| WorkError::InvalidField {
                    field: "history timestamp",
                })?;
            if u64::try_from(at_ms).ok() != Some(tuple(&row.id).0) {
                return Err(WorkError::InvalidField {
                    field: "history timestamp",
                }
                .into());
            }
            events.push(WorkEvent {
                id: row.id.clone(),
                at_ms,
                binding: (&binding).into(),
                change,
            });
        }
        Ok(WorkEventPage {
            version: 1,
            available_from_ms,
            observed_at_ms,
            truncated,
            next_cursor: events.last().map_or_else(
                || {
                    if tuple(after) < tuple(&floor_cursor) {
                        floor_cursor
                    } else {
                        after.to_owned()
                    }
                },
                |e| e.id.clone(),
            ),
            has_more,
            events,
        })
    }

    /// Reads current verified reporting state without inventing transition timestamps.
    pub fn work_snapshot(
        &mut self,
        binding: &WorkBinding,
        observed_at_ms: i64,
    ) -> Result<relay_work_wire::Snapshot> {
        let report = self.work_report(binding)?;
        let key = self
            .keys
            .work_key(&binding.machine, &binding.session, binding.process_id);
        let paused: Option<String> = redis::cmd("GET")
            .arg(self.keys.work_activity_key(
                &binding.machine,
                &binding.session,
                binding.process_id,
            ))
            .query(&mut self.conn)?;
        let paused = paused.map(|text| decode(&key, &text)).transpose()?;
        Ok(relay_work_wire::Snapshot {
            version: 1,
            observed_at_ms,
            binding: binding.into(),
            scope: report
                .as_ref()
                .and_then(|r| r.scope.as_ref())
                .map(Into::into),
            assignments: report
                .as_ref()
                .map_or_else(Vec::new, |r| r.agents.iter().map(Into::into).collect()),
            paused,
        })
    }

    /// Reads a bounded snapshot without registering anyone or consuming messages.
    pub fn work_report(&mut self, binding: &WorkBinding) -> Result<Option<WorkReport>> {
        binding.validate()?;
        let key = self
            .keys
            .work_key(&binding.machine, &binding.session, binding.process_id);
        let (status, fields): (i64, BTreeMap<String, String>) =
            redis::Script::new(READ).key(&key).invoke(&mut self.conn)?;
        if status != 1 {
            return Err(WorkError::ReportLimit.into());
        }
        if fields.is_empty() {
            return Ok(None);
        }
        let identity: WorkBinding = decode(
            &key,
            fields.get("binding").ok_or(WorkError::IdentityMismatch)?,
        )?;
        identity.validate()?;
        if !identity.same_incarnation(binding) {
            return Err(WorkError::IdentityMismatch.into());
        }
        let mut report = WorkReport {
            binding: identity,
            scope: None,
            focus: None,
            agents: Vec::new(),
        };
        for (field, value) in fields {
            match field.as_str() {
                "binding" => {}
                "activity" => {
                    let _: bool = decode(&key, &value)?;
                }
                "scope" => {
                    let scope: WorkScope = decode(&key, &value)?;
                    scope.validate()?;
                    report.scope = Some(scope);
                }
                "focus" => {
                    let focus: WorkFocus = decode(&key, &value)?;
                    focus.validate()?;
                    report.focus = Some(focus);
                }
                _ => {
                    let id = field
                        .strip_prefix("agent:")
                        .ok_or(WorkError::InvalidField {
                            field: "stored field",
                        })?;
                    let agent: AgentWork = decode(&key, &value)?;
                    agent.validate()?;
                    if agent.agent_id != id {
                        return Err(WorkError::InvalidField {
                            field: "stored agent_id",
                        }
                        .into());
                    }
                    report.agents.push(agent);
                }
            }
        }
        Ok(Some(report))
    }
}
