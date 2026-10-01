// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Pure dashboard view built from independent tracker and explicit assignment data.

use ix_board::{ScopeSelection, ScopeSummary, TeamKey, WorkflowStateType, WorkspaceSnapshot};
use relay_core::work::{AgentWork, WorkReport, WorkScope};
use std::collections::{BTreeMap, BTreeSet};

/// Tracker source snapshot; its timestamp belongs to the source fetch.
pub type TrackerState = WorkspaceSnapshot;

/// Progress within one scope-associated project or initiative.
#[derive(Clone, Debug)]
pub struct ScopeProgress {
    /// Project or initiative display name.
    pub name: String,
    /// Associated initiative names for a project row.
    pub initiatives: Vec<String>,
    /// Tracker project status; initiatives have no project workflow status.
    pub status: Option<String>,
    /// Remaining scoped leaves by tracker workflow stage.
    pub remaining_by_stage: BTreeMap<String, usize>,
    /// Scoped leaves in the authoritative ready queue.
    pub ready: usize,
    /// Unfinished scoped leaves with unfinished or unknown prerequisites.
    pub blocked: usize,
    /// Completed leaf tasks.
    pub done: usize,
    /// Total non-canceled, non-duplicate leaves.
    pub total: usize,
}

/// One ticket enriched with reported agent assignments.
#[derive(Clone, Debug)]
pub struct TicketSummary {
    /// Native Linear identifier.
    pub identifier: String,
    /// Full title retained for the detail view.
    pub title: String,
    /// Project name, when assigned.
    pub project: String,
    /// Linear workflow stage, distinct from agent activity.
    pub state: String,
    /// Numeric priority displayed only in details.
    pub priority: f64,
    /// Direct unfinished prerequisites, including unknown references.
    pub blockers: Vec<String>,
    /// All explicit assignments for this ticket.
    pub agents: Vec<AgentWork>,
    /// Whether this ticket belongs to the explicit scope.
    pub in_scope: bool,
    /// Whether ix-board's ordered queue includes this ticket.
    pub ready: bool,
}

/// Complete presentation model for one explicit scope.
#[derive(Clone, Debug)]
pub struct DashboardView {
    /// Resolved leaf-task membership and progress.
    pub scope: ScopeSummary,
    /// Explicit ticket subtree roots, resolved to native identifiers.
    pub ticket_roots: Vec<String>,
    /// Initiative progress restricted to this reported scope.
    pub initiatives: Vec<ScopeProgress>,
    /// Project totals for the detail view.
    pub projects: Vec<ScopeProgress>,
    /// Open tickets and explicitly reported assignments, in deterministic order.
    pub tickets: Vec<TicketSummary>,
    /// Ready identifiers in deterministic team order, preserving each team's tracker queue.
    pub ready_queue: Vec<String>,
}

/// Uses the same selection representation as the authoritative board library.
pub fn selection(scope: &WorkScope) -> ScopeSelection {
    ScopeSelection {
        initiatives: scope.initiatives.clone(),
        projects: scope.projects.clone(),
        tickets: scope.tickets.clone(),
        ..Default::default()
    }
}

/// Counts scoped leaves and attaches explicit agent reports without inferring activity.
pub fn summarize(
    snapshot: &TrackerState,
    report: Option<&WorkReport>,
    team: &str,
) -> Result<DashboardView, ix_board::Error> {
    let whole_team = WorkScope::default();
    let assignment = report.and_then(|r| r.scope.as_ref()).unwrap_or(&whole_team);
    let agents = report.map_or(&[][..], |r| r.agents.as_slice());
    let mut selectors = selection(assignment);
    selectors.team = Some(TeamKey(team.to_owned()));
    let narrowed = !assignment.initiatives.is_empty()
        || !assignment.projects.is_empty()
        || !assignment.tickets.is_empty();
    let scope = ix_board::select_scope(snapshot, &selectors)?;
    let teams: BTreeSet<_> = snapshot
        .issues
        .iter()
        .filter(|i| scope.issue_ids.contains(&i.id))
        .map(|i| &i.team_key.0)
        .collect();
    let mut ready = Vec::new();
    for team in teams {
        ready.extend(
            ix_board::ready_from_snapshot(snapshot, team)?
                .into_iter()
                .filter(|q| {
                    snapshot
                        .issues
                        .iter()
                        .any(|i| i.identifier == q.identifier && scope.issue_ids.contains(&i.id))
                }),
        );
    }
    let ready_ids: BTreeSet<_> = ready.iter().map(|q| q.identifier.as_str()).collect();
    let by_id: BTreeMap<_, _> = snapshot.issues.iter().map(|i| (&i.id, i)).collect();
    let mut ticket_roots = BTreeSet::new();
    let mut initiative_projects: BTreeMap<_, (String, BTreeSet<_>)> = BTreeMap::new();
    let mut projects = Vec::new();
    for project in snapshot.projects.iter().filter(|p| {
        scope.project_ids.contains(&p.id)
            || snapshot
                .issues
                .iter()
                .any(|i| scope.issue_ids.contains(&i.id) && i.project_id.as_ref() == Some(&p.id))
    }) {
        for initiative in project
            .initiatives
            .iter()
            .filter(|i| !narrowed || assignment.initiatives.contains(&i.id))
        {
            initiative_projects
                .entry(&initiative.id)
                .or_insert_with(|| (initiative.name.clone(), BTreeSet::new()))
                .1
                .insert(&project.id);
        }
        let leaves: Vec<_> = snapshot
            .issues
            .iter()
            .filter(|i| {
                scope.leaf_ids.contains(&i.id) && i.project_id.as_ref() == Some(&project.id)
            })
            .collect();
        let mut project_progress = progress(
            &by_id,
            &leaves,
            &ready_ids,
            &project.name,
            Some(&project.status),
        );
        project_progress.initiatives = project.initiatives.iter().map(|i| i.name.clone()).collect();
        projects.push(project_progress);
    }
    for id in &assignment.tickets {
        if let Some(issue) = snapshot
            .issues
            .iter()
            .find(|i| i.id.0 == *id && scope.issue_ids.contains(&i.id))
        {
            ticket_roots.insert(issue.identifier.clone());
        }
    }
    let mut initiatives: Vec<_> = initiative_projects
        .into_values()
        .map(|(name, project_ids)| {
            let leaves: Vec<_> = snapshot
                .issues
                .iter()
                .filter(|issue| {
                    scope.leaf_ids.contains(&issue.id)
                        && issue
                            .project_id
                            .as_ref()
                            .is_some_and(|id| project_ids.contains(id))
                })
                .collect();
            progress(&by_id, &leaves, &ready_ids, &name, None)
        })
        .collect();
    initiatives.sort_by(|a, b| a.name.cmp(&b.name));
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    let mut tickets = Vec::new();
    for issue in &snapshot.issues {
        let agents: Vec<_> = agents
            .iter()
            .filter(|a| a.ticket.as_str() == issue.identifier)
            .cloned()
            .collect();
        let in_scope = scope.issue_ids.contains(&issue.id);
        let open = !matches!(
            issue.state_type,
            WorkflowStateType::Completed
                | WorkflowStateType::Canceled
                | WorkflowStateType::Duplicate
        );
        if (in_scope && open) || !agents.is_empty() {
            let blockers = issue
                .blocked_by
                .iter()
                .filter_map(|id| match by_id.get(id) {
                    Some(i) if i.state_type == WorkflowStateType::Completed => None,
                    Some(i) => Some(i.identifier.clone()),
                    None => Some(format!("unknown {}", id.0)),
                })
                .collect();
            tickets.push(TicketSummary {
                identifier: issue.identifier.clone(),
                title: issue.title.clone(),
                project: snapshot
                    .projects
                    .iter()
                    .find(|p| issue.project_id.as_ref() == Some(&p.id))
                    .map_or_else(|| "No project".into(), |p| p.name.clone()),
                state: issue.state.clone(),
                priority: issue.priority,
                blockers,
                agents,
                in_scope,
                ready: ready_ids.contains(issue.identifier.as_str()),
            });
        }
    }
    // A reported assignment absent from Linear remains visible as unknown.
    for agent in agents {
        if !snapshot
            .issues
            .iter()
            .any(|i| i.identifier == agent.ticket.as_str())
        {
            if let Some(ticket) = tickets
                .iter_mut()
                .find(|t| t.identifier == agent.ticket.as_str())
            {
                ticket.agents.push(agent.clone());
            } else {
                tickets.push(TicketSummary {
                    identifier: agent.ticket.as_str().into(),
                    title: "Ticket unavailable in Linear".into(),
                    project: "unknown".into(),
                    state: "unknown".into(),
                    priority: 0.0,
                    blockers: vec![],
                    agents: vec![agent.clone()],
                    in_scope: false,
                    ready: false,
                });
            }
        }
    }
    tickets.sort_by(|a, b| {
        a.agents
            .is_empty()
            .cmp(&b.agents.is_empty())
            .then_with(|| a.identifier.cmp(&b.identifier))
    });
    Ok(DashboardView {
        scope,
        ticket_roots: ticket_roots.into_iter().collect(),
        initiatives,
        projects,
        tickets,
        ready_queue: ready.into_iter().map(|item| item.identifier).collect(),
    })
}

fn progress(
    by_id: &BTreeMap<&ix_board::IssueId, &ix_board::Issue>,
    leaves: &[&ix_board::Issue],
    ready_ids: &BTreeSet<&str>,
    name: &str,
    status: Option<&str>,
) -> ScopeProgress {
    let mut remaining_by_stage = BTreeMap::new();
    let mut done = 0;
    let mut blocked = 0;
    let mut ready = 0;
    for issue in leaves {
        if issue.state_type == WorkflowStateType::Completed {
            done += 1;
            continue;
        }
        *remaining_by_stage.entry(issue.state.clone()).or_default() += 1;
        ready += usize::from(ready_ids.contains(issue.identifier.as_str()));
        blocked += usize::from(issue.blocked_by.iter().any(|id| {
            by_id
                .get(id)
                .is_none_or(|prerequisite| prerequisite.state_type != WorkflowStateType::Completed)
        }));
    }
    ScopeProgress {
        name: name.into(),
        initiatives: Vec::new(),
        status: status.map(str::to_owned),
        remaining_by_stage,
        done,
        total: leaves.len(),
        ready,
        blocked,
    }
}
