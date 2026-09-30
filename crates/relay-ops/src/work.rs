// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Measured session binding and authoritative scope resolution for work reports.

use crate::{OpsError, Result};
use herdr_cli::{ProcessRunner, call, with_machine};
use relay_core::{
    Address, MachineName, PaneId, SessionId, TabId, WorkspaceId,
    work::{WorkBinding, WorkScope},
};
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path, time::Duration};

// These are selected fields of Herdr's externally owned payload, as in find.rs.
// Unknown fields are not a version negotiation or a second wire schema.
#[derive(Debug, Deserialize)]
struct SnapshotResult {
    snapshot: Snapshot,
}
#[derive(Debug, Deserialize)]
struct Snapshot {
    panes: Vec<Pane>,
    workspaces: Vec<Workspace>,
    tabs: Vec<Tab>,
}
#[derive(Debug, Deserialize)]
struct Workspace {
    workspace_id: WorkspaceId,
    label: String,
}
#[derive(Debug, Deserialize)]
struct Tab {
    tab_id: TabId,
    label: String,
}
#[derive(Debug, Deserialize)]
struct Pane {
    pane_id: PaneId,
    workspace_id: WorkspaceId,
    tab_id: TabId,
    agent: Option<String>,
    agent_status: String,
    agent_session: Option<NativeSession>,
}
#[derive(Debug, Deserialize)]
struct NativeSession {
    kind: String,
    value: String,
}
#[derive(Debug, Deserialize)]
struct ProcessResult {
    process_info: ProcessInfo,
}
#[derive(Debug, Deserialize)]
struct ProcessInfo {
    foreground_processes: Vec<ForegroundProcess>,
}
#[derive(Debug, Deserialize)]
struct ForegroundProcess {
    name: String,
    pid: u32,
    argv: Vec<String>,
}

/// One observation of the bound lead and other pane agents on its machine.
#[derive(Clone, Debug)]
pub struct WorkObservation {
    /// Exact session incarnation observed.
    pub binding: WorkBinding,
    /// Herdr's lead lifecycle state, distinct from ticket workflow.
    pub status: String,
    /// Other observable pane agent states, paired with verified native sessions.
    pub panes: Vec<PaneActivity>,
}

/// Session-qualified runtime activity of one pane agent.
#[derive(Clone, Debug)]
pub struct PaneActivity {
    /// Pane hosting the agent.
    pub pane: PaneId,
    /// Native session, when Herdr reports it.
    pub session: Option<SessionId>,
    /// Observed lifecycle state.
    pub status: String,
}

fn snapshot(runner: &impl ProcessRunner, remote: Option<&str>) -> Result<Snapshot> {
    let response: SnapshotResult = call(
        runner,
        "herdr",
        &with_machine(remote, &["api", "snapshot"]),
        Duration::from_secs(5),
    )?;
    Ok(response.snapshot)
}

fn process_identity(
    runner: &impl ProcessRunner,
    remote: Option<&str>,
    pane: &Pane,
) -> Result<(SessionId, u32)> {
    let response: ProcessResult = call(
        runner,
        "herdr",
        &with_machine(
            remote,
            &["pane", "process-info", "--pane", pane.pane_id.as_str()],
        ),
        Duration::from_secs(5),
    )?;
    let kind = pane.agent.as_deref().ok_or(OpsError::WorkAgentAbsent)?;
    let processes: Vec<_> = response
        .process_info
        .foreground_processes
        .iter()
        .filter(|p| {
            p.name == kind
                || Path::new(&p.name)
                    .file_name()
                    .is_some_and(|name| name == kind)
        })
        .collect();
    let [process] = processes.as_slice() else {
        return Err(OpsError::WorkProcessAmbiguous);
    };
    let observed = pane
        .agent_session
        .as_ref()
        .filter(|s| s.kind == "id")
        .map(|s| s.value.parse())
        .transpose()?;
    let resumed = if kind == "codex" {
        process
            .argv
            .windows(2)
            .find(|args| args[0] == "resume" && !args[1].starts_with('-'))
            .map(|args| args[1].parse())
            .transpose()?
    } else {
        None
    };
    if observed.is_some() && resumed.is_some() && observed != resumed {
        return Err(OpsError::WorkIdentityConflict);
    }
    let session = observed
        .or(resumed)
        .ok_or(OpsError::WorkSessionUnavailable)?;
    Ok((session, process.pid))
}

fn observe_pane(
    runner: &impl ProcessRunner,
    remote: Option<&str>,
    view: &Snapshot,
    pane: &Pane,
    machine: MachineName,
) -> Result<WorkObservation> {
    let workspace = view
        .workspaces
        .iter()
        .find(|w| w.workspace_id == pane.workspace_id)
        .ok_or(OpsError::WorkTargetUnavailable)?;
    let tab = view
        .tabs
        .iter()
        .find(|t| t.tab_id == pane.tab_id)
        .ok_or(OpsError::WorkTargetUnavailable)?;
    let (session, process_id) = process_identity(runner, remote, pane)?;
    let address = Address::new(machine.as_str(), &workspace.label, Some(&tab.label)).to_string();
    Ok(WorkObservation {
        binding: WorkBinding {
            machine,
            pane: pane.pane_id.clone(),
            session,
            process_id,
            kind: pane.agent.clone().ok_or(OpsError::WorkAgentAbsent)?,
            address,
        },
        status: pane.agent_status.clone(),
        panes: view
            .panes
            .iter()
            .filter(|p| p.agent.is_some())
            .map(|p| PaneActivity {
                pane: p.pane_id.clone(),
                session: p
                    .agent_session
                    .as_ref()
                    .filter(|s| s.kind == "id")
                    .and_then(|s| s.value.parse().ok()),
                status: p.agent_status.clone(),
            })
            .collect(),
    })
}

/// Resolves an explicit Herdr address, or the calling pane when omitted.
/// Refuses multiple occupants and unavailable native identity; never uses titles.
pub fn resolve_work_target(
    runner: &impl ProcessRunner,
    target: Option<&str>,
    local_machine: &MachineName,
) -> Result<WorkObservation> {
    let address = target.map(str::parse::<Address>).transpose()?;
    let machine: MachineName = address
        .as_ref()
        .map_or(local_machine.as_str(), Address::server)
        .parse()?;
    let remote = (machine != *local_machine).then_some(machine.as_str());
    let view = snapshot(runner, remote)?;
    let caller = if address.is_none() {
        Some(
            std::env::var("HERDR_PANE_ID")
                .map_err(|_| OpsError::WorkTargetUnavailable)?
                .parse::<PaneId>()?,
        )
    } else {
        None
    };
    let candidates: Vec<_> = view
        .panes
        .iter()
        .filter(|pane| {
            if pane.agent.is_none() {
                return false;
            }
            if let Some(caller) = &caller {
                return &pane.pane_id == caller;
            }
            let Some(workspace) = view
                .workspaces
                .iter()
                .find(|w| w.workspace_id == pane.workspace_id)
            else {
                return false;
            };
            let Some(tab) = view.tabs.iter().find(|t| t.tab_id == pane.tab_id) else {
                return false;
            };
            address.as_ref().is_some_and(|a| {
                a.selects(&Address::with_pane(
                    machine.as_str(),
                    &workspace.label,
                    &tab.label,
                    pane.pane_id.as_str(),
                ))
            })
        })
        .collect();
    let [pane] = candidates.as_slice() else {
        return Err(OpsError::WorkTargetCardinality {
            matches: candidates.len(),
        });
    };
    observe_pane(runner, remote, &view, pane, machine.clone())
}

/// Observes the original pane and refuses to inherit another session's reports.
pub fn observe_work_binding(
    runner: &impl ProcessRunner,
    binding: &WorkBinding,
    local_machine: &MachineName,
) -> Result<WorkObservation> {
    let remote = (binding.machine != *local_machine).then_some(binding.machine.as_str());
    let view = snapshot(runner, remote)?;
    let pane = view
        .panes
        .iter()
        .find(|p| p.pane_id == binding.pane)
        .ok_or(OpsError::WorkSessionReplaced)?;
    let observation = observe_pane(runner, remote, &view, pane, binding.machine.clone())?;
    if observation.binding.session != binding.session
        || observation.binding.process_id != binding.process_id
        || observation.binding.kind != binding.kind
    {
        return Err(OpsError::WorkSessionReplaced);
    }
    Ok(observation)
}

/// Resolves selectors into stable UUIDs, keeping initiative selectors dynamic.
pub fn resolve_work_scope(
    snapshot: &ix_board::WorkspaceSnapshot,
    selection: &ix_board::ScopeSelection,
    reported_at_ms: i64,
) -> Result<WorkScope> {
    ix_board::select_scope(snapshot, selection)?;
    let mut projects = BTreeSet::new();
    for project in &selection.projects {
        let selected = ix_board::select_scope(
            snapshot,
            &ix_board::ScopeSelection {
                projects: vec![project.clone()],
                ..Default::default()
            },
        )?;
        projects.extend(selected.project_ids.into_iter().map(|id| id.0));
    }
    let initiatives = snapshot
        .projects
        .iter()
        .flat_map(|p| &p.initiatives)
        .filter(|i| {
            selection
                .initiatives
                .iter()
                .any(|s| i.id == *s || i.name.eq_ignore_ascii_case(s))
        })
        .map(|i| i.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let tickets = snapshot
        .issues
        .iter()
        .filter(|i| {
            selection
                .tickets
                .iter()
                .any(|s| i.id.0 == *s || i.identifier.eq_ignore_ascii_case(s))
        })
        .map(|i| i.id.0.clone())
        .collect();
    Ok(WorkScope {
        projects: projects.into_iter().collect(),
        initiatives,
        tickets,
        reported_at_ms,
    })
}
