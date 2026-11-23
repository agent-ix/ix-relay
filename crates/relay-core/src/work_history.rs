// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Shared work-history contract and conversions from validated reporting identities.
use crate::work::{AgentWork, WorkBinding, WorkScope, WorkStatus};
pub use relay_work_wire::{
    Binding, Change as WorkChange, Event as WorkEvent, EventPage as WorkEventPage,
};
impl From<&WorkBinding> for Binding {
    fn from(b: &WorkBinding) -> Self {
        Self {
            machine: b.machine.to_string(),
            pane: b.pane.to_string(),
            session: b.session.to_string(),
            process_id: b.process_id,
            kind: b.kind.clone(),
            address: b.address.clone(),
        }
    }
}
impl From<&WorkScope> for relay_work_wire::ReportedScope {
    fn from(s: &WorkScope) -> Self {
        Self {
            initiatives: s.initiatives.clone(),
            projects: s.projects.clone(),
            tickets: s.tickets.clone(),
            reported_at_ms: s.reported_at_ms,
        }
    }
}
impl From<&AgentWork> for relay_work_wire::Assignment {
    fn from(a: &AgentWork) -> Self {
        Self {
            agent_id: a.agent_id.clone(),
            name: a.name.clone(),
            ticket: a.ticket.to_string(),
            role: a.role.clone(),
            status: match a.status {
                WorkStatus::Running => relay_work_wire::AssignmentStatus::Running,
                WorkStatus::Waiting => relay_work_wire::AssignmentStatus::Waiting,
                WorkStatus::Done => relay_work_wire::AssignmentStatus::Done,
                WorkStatus::Failed => relay_work_wire::AssignmentStatus::Failed,
                WorkStatus::Canceled => relay_work_wire::AssignmentStatus::Canceled,
            },
            pane: a.pane.as_ref().map(ToString::to_string),
            session: a.session.as_ref().map(ToString::to_string),
            reported_at_ms: a.reported_at_ms,
        }
    }
}
