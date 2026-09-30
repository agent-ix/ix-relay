// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Atomic, bounded reporting fields isolated from message inboxes.

use super::{decode, encode};
use crate::{
    Relay, Result,
    work::{AgentWork, WorkBinding, WorkError, WorkFocus, WorkReport, WorkScope, validate_text},
};
use serde::Serialize;
use std::collections::BTreeMap;

const WORK_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;
const IDENTITY_CHECK: &str = r#"
if redis.call('HSTRLEN', KEYS[1], 'binding') > 65536 then return -3 end
local binding = redis.call('HGET', KEYS[1], 'binding')
if binding then
  local old = cjson.decode(binding)
  local new = cjson.decode(ARGV[1])
  if old.machine ~= new.machine or old.pane ~= new.pane or old.session ~= new.session
    or old.process_id ~= new.process_id or old.kind ~= new.kind then return -1 end
end
"#;
const UPDATE: &str = r#"
local agents = redis.call('HLEN', KEYS[1]) - redis.call('HEXISTS', KEYS[1], 'binding')
  - redis.call('HEXISTS', KEYS[1], 'scope') - redis.call('HEXISTS', KEYS[1], 'focus')
if string.sub(ARGV[2], 1, 6) == 'agent:' and redis.call('HEXISTS', KEYS[1], ARGV[2]) == 0
  and agents >= 128 then return -2 end
redis.call('HSET', KEYS[1], 'binding', ARGV[1], ARGV[2], ARGV[3])
redis.call('EXPIRE', KEYS[1], ARGV[4])
return 1
"#;

const REMOVE: &str = r#"
redis.call('HDEL', KEYS[1], ARGV[2])
return 1
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
    ) -> Result<()> {
        binding.validate()?;
        let key = self
            .keys
            .work_key(&binding.machine, &binding.session, binding.process_id);
        let reply: i64 = redis::Script::new(&format!(
            "{IDENTITY_CHECK}{}",
            if value.is_some() { UPDATE } else { REMOVE }
        ))
        .key(&key)
        .arg(encode(binding))
        .arg(field)
        .arg(encode(&value))
        .arg(WORK_TTL_SECONDS)
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
        self.update_work(binding, "scope", Some(scope))
    }

    /// Replaces only this session's focus; it does not change tracker state.
    pub fn report_work_focus(&mut self, binding: &WorkBinding, focus: &WorkFocus) -> Result<()> {
        focus.validate()?;
        self.update_work(binding, "focus", Some(focus))
    }

    /// Upserts one agent assignment atomically, without overwriting other agents.
    pub fn report_work_agent(&mut self, binding: &WorkBinding, agent: &AgentWork) -> Result<()> {
        agent.validate()?;
        self.update_work(binding, &format!("agent:{}", agent.agent_id), Some(agent))
    }

    /// Removes a finished assignment when the lead no longer needs it displayed.
    pub fn remove_work_agent(&mut self, binding: &WorkBinding, agent_id: &str) -> Result<()> {
        validate_text(agent_id, "agent_id", 128)?;
        self.update_work::<()>(binding, &format!("agent:{agent_id}"), None)
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
