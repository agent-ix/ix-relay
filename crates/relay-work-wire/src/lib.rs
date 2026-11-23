// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Version-one work history contract shared by the relay producer and metrics readers.
use serde::{Deserialize, Serialize};

/// Verified source session incarnation; pane names never substitute for session identity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// Native host identity.
    pub machine: String,
    /// Native pane identity.
    pub pane: String,
    /// Harness session identity.
    pub session: String,
    /// Process incarnation.
    pub process_id: u32,
    /// Harness kind.
    pub kind: String,
    /// Canonical reporting address.
    pub address: String,
}
impl Binding {
    /// Stable process-incarnation identity, distinct from display address.
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.machine, self.session, self.process_id)
    }
}
/// Source scope payload; its clock is preserved but ordering uses server time.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReportedScope {
    /// Native initiatives.
    pub initiatives: Vec<String>,
    /// Native projects.
    pub projects: Vec<String>,
    /// Native tickets.
    pub tickets: Vec<String>,
    /// Source report timestamp.
    pub reported_at_ms: i64,
}
/// Lead-reported assignment state; distinct from observed execution.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    /// Reported active assignment.
    Running,
    /// Reported wait.
    Waiting,
    /// Stage completed, not necessarily its ticket.
    Done,
    /// Stage failed.
    Failed,
    /// Stage canceled.
    Canceled,
}
/// An explicit assignment; names and identities are never Prometheus labels.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    /// Native assignment identity.
    pub agent_id: String,
    /// Display name retained only in the bounded journal.
    pub name: String,
    /// Native ticket identifier.
    pub ticket: String,
    /// Reported stage/role; reduced to a bounded stage enum for export.
    pub role: String,
    /// Reported state.
    pub status: AssignmentStatus,
    /// Separate pane, if present.
    pub pane: Option<String>,
    /// Separate pane's verified session, required when pane is present.
    pub session: Option<String>,
    /// Original producer timestamp.
    pub reported_at_ms: i64,
}
/// Immutable transition emitted atomically with current work state.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    /// Scope selectors changed.
    Scope {
        /// New scope.
        scope: ReportedScope,
    },
    /// Assignment/stage update.
    Agent {
        /// Explicit assignment.
        agent: Assignment,
    },
    /// Removal never proves ticket completion.
    Remove {
        /// Removed native ID.
        agent_id: String,
    },
    /// Explicit pause/resume correction.
    Activity {
        /// True for a pause.
        paused: bool,
    },
}
/// Transition with stable source event identity and server timestamp.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Stable Redis stream ID.
    pub id: String,
    /// Source server time.
    pub at_ms: i64,
    /// Verified reporting incarnation.
    pub binding: Binding,
    /// Typed transition.
    pub change: Change,
}
/// Read-only history page; same schema as relay-core WorkEventPage.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventPage {
    /// Protocol version.
    pub version: u32,
    /// Retained complete-history lower bound.
    pub available_from_ms: Option<i64>,
    /// Redis server observation instant.
    pub observed_at_ms: i64,
    /// Cursor preceded retained history.
    pub truncated: bool,
    /// Next exclusive cursor.
    pub next_cursor: String,
    /// More events remain.
    pub has_more: bool,
    /// Ordered transitions.
    pub events: Vec<Event>,
}

/// Verified current report used to bootstrap a collector without inventing history.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// Protocol version.
    pub version: u32,
    /// Observation instant; this is not a historical assignment start.
    pub observed_at_ms: i64,
    /// Live verified process incarnation.
    pub binding: Binding,
    /// Published scope, absent when the lead has not reported one.
    pub scope: Option<ReportedScope>,
    /// Current assignments, which may predate transition instrumentation.
    pub assignments: Vec<Assignment>,
    /// Explicit pause correction, absent when none was reported.
    pub paused: Option<bool>,
}
