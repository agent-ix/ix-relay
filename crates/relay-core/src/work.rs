// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Explicit assignments belonging to one observed agent session incarnation.

use crate::{MachineName, PaneId, SessionId, TicketId};
use serde::{Deserialize, Serialize};

/// Identity measured from a live Herdr pane and its agent process.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkBinding {
    /// Machine hosting the session.
    pub machine: MachineName,
    /// Stable pane handle.
    pub pane: PaneId,
    /// Native harness session ID.
    pub session: SessionId,
    /// Process incarnation; restarting the same native session requires rebinding.
    pub process_id: u32,
    /// Native harness kind.
    pub kind: String,
    /// Human-readable Herdr location.
    pub address: String,
}

/// Explicit assignment selectors, resolved to UUIDs by the reporting command.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkScope {
    /// Selected initiative UUIDs.
    pub initiatives: Vec<String>,
    /// Selected project UUIDs.
    pub projects: Vec<String>,
    /// Selected ticket UUIDs, including descendants.
    pub tickets: Vec<String>,
    /// Time the lead reported this assignment, in Unix milliseconds.
    pub reported_at_ms: i64,
}

/// Current focus explicitly reported by the lead.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkFocus {
    /// Plain-language action currently being pursued.
    pub summary: String,
    /// Related ticket identifiers.
    pub tickets: Vec<TicketId>,
    /// Last report time in Unix milliseconds.
    pub reported_at_ms: i64,
}

/// Status reported by the lead, separate from observed runtime activity.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    /// The agent has been dispatched.
    Running,
    /// The agent is waiting on something.
    Waiting,
    /// The assigned stage finished, without implying the Linear ticket is done.
    Done,
    /// The assigned stage failed.
    Failed,
    /// The assignment was canceled.
    Canceled,
}

/// One agent and its assigned ticket/role.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentWork {
    /// Native subagent ID or human-readable assignment ID.
    pub agent_id: String,
    /// Agent display name.
    pub name: String,
    /// Ticket being worked on.
    pub ticket: TicketId,
    /// Assigned stage/role, such as coder or reviewer.
    pub role: String,
    /// Lead-reported status.
    pub status: WorkStatus,
    /// Herdr pane when this is a separate pane agent.
    pub pane: Option<PaneId>,
    /// Native session of a pane agent; required with pane to detect replacement.
    pub session: Option<SessionId>,
    /// Last report time in Unix milliseconds.
    pub reported_at_ms: i64,
}

/// Snapshot of explicit work reports; absence is not an empty assignment.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkReport {
    /// Session owning the report.
    pub binding: WorkBinding,
    /// Explicit scope, if reported.
    pub scope: Option<WorkScope>,
    /// Explicit current focus, if reported.
    pub focus: Option<WorkFocus>,
    /// Agent assignments, in deterministic ID order.
    pub agents: Vec<AgentWork>,
}

/// Typed validation failures for work reporting.
#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    /// A field was empty, unsafe, or exceeded its bound.
    #[error("invalid work report field: {field}")]
    InvalidField {
        /// Field rejected by validation.
        field: &'static str,
    },
    /// The session already has the maximum number of tracked agents.
    #[error("work report has reached its 128-agent limit; remove finished assignments")]
    AgentLimit,
    /// Stored report exceeds its bounded read envelope.
    #[error("stored work report exceeds its field or byte limit")]
    ReportLimit,
    /// Stored identity differs from the requested identity.
    #[error("stored work report does not belong to this session incarnation")]
    IdentityMismatch,
}

pub(crate) fn validate_text(value: &str, field: &'static str, max: usize) -> Result<(), WorkError> {
    if value.trim().is_empty() || value.len() > max || value != crate::text::sanitise_line(value) {
        return Err(WorkError::InvalidField { field });
    }
    Ok(())
}

impl WorkScope {
    pub(crate) fn validate(&self) -> Result<(), WorkError> {
        let values: Vec<_> = self
            .initiatives
            .iter()
            .chain(&self.projects)
            .chain(&self.tickets)
            .collect();
        if values.is_empty() || values.len() > 100 {
            return Err(WorkError::InvalidField { field: "scope" });
        }
        for value in values {
            validate_text(value, "scope selector", 128)?;
        }
        Ok(())
    }
}

impl AgentWork {
    pub(crate) fn validate(&self) -> Result<(), WorkError> {
        validate_text(&self.agent_id, "agent_id", 128)?;
        validate_text(&self.name, "name", 128)?;
        validate_text(&self.role, "role", 64)?;
        if self.pane.is_some() != self.session.is_some() {
            return Err(WorkError::InvalidField {
                field: "pane/session",
            });
        }
        Ok(())
    }
}

impl WorkBinding {
    pub(crate) fn validate(&self) -> Result<(), WorkError> {
        validate_text(&self.kind, "harness kind", 32)?;
        validate_text(&self.address, "address", 256)?;
        if self.process_id == 0 {
            return Err(WorkError::InvalidField {
                field: "process_id",
            });
        }
        Ok(())
    }

    /// Compares session incarnations independently of mutable display labels.
    pub fn same_incarnation(&self, other: &Self) -> bool {
        self.machine == other.machine
            && self.pane == other.pane
            && self.session == other.session
            && self.process_id == other.process_id
            && self.kind == other.kind
    }
}

impl WorkFocus {
    pub(crate) fn validate(&self) -> Result<(), WorkError> {
        validate_text(&self.summary, "focus", 512)?;
        if self.tickets.len() > 100 {
            return Err(WorkError::InvalidField {
                field: "focus tickets",
            });
        }
        Ok(())
    }
}
