// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Read-only SWM project and ticket summaries from ix-board's structured API.

use std::collections::HashMap;

use ix_board::{Issue, Project, ReadyItem, WorkspaceSnapshot};

/// Project and ticket information shown in the monitor's primary pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackerState {
    /// The two selected projects, including an unavailable row when missing.
    pub projects: Vec<ProjectSummary>,
    /// Open SWM tickets in either selected project, excluding the next ticket.
    pub current: Vec<TicketSummary>,
    /// First selected-project candidate in ix-board's deterministic ready queue.
    pub next: Option<TicketSummary>,
}

/// One selected Linear project and its issue counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectSummary {
    /// Stable project label.
    pub name: String,
    /// Current Linear project status, or `unavailable` if absent.
    pub status: String,
    /// Issues not Completed or Canceled.
    pub active_count: usize,
    /// Issues in Completed state.
    pub done_count: usize,
    /// Whether the project record was present in the complete snapshot.
    pub available: bool,
}

/// Selected work item enriched from the ix-board snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TicketSummary {
    /// Linear issue identifier.
    pub identifier: String,
    /// Ticket title.
    pub title: String,
    /// Selected project name.
    pub project: String,
    /// Linear workflow state.
    pub state: String,
    /// Display label for Linear's numeric priority.
    pub priority: String,
    /// Direct blocker summary based on structured issue relationships.
    pub blockers: BlockerState,
}

/// Whether an issue has an unresolved direct prerequisite.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BlockerState {
    /// Every direct prerequisite is Completed, or there are no prerequisites.
    #[default]
    Unblocked,
    /// One or more direct prerequisites are still open or canceled.
    Blocked(Vec<String>),
    /// At least one referenced prerequisite was missing from the snapshot.
    Unknown(Vec<String>),
}

/// Builds the displayed summaries from ix-board's complete structured read.
pub fn summarize(snapshot: &WorkspaceSnapshot, queue: &[ReadyItem]) -> TrackerState {
    let selected: Vec<_> = ["ix-board", "ix-projects"]
        .into_iter()
        .map(|wanted| {
            snapshot
                .projects
                .iter()
                .find(|project| project_matches(project, wanted))
        })
        .collect();
    let projects = ["ix-board", "ix-projects"]
        .into_iter()
        .zip(selected.iter().copied())
        .map(|(wanted, project)| match project {
            Some(project) => summarize_project(snapshot, project),
            None => ProjectSummary {
                name: wanted.to_owned(),
                status: "unavailable".to_owned(),
                available: false,
                ..ProjectSummary::default()
            },
        })
        .collect();

    let project_ids: Vec<_> = selected
        .iter()
        .flatten()
        .map(|project| &project.id)
        .collect();
    let issues: HashMap<_, _> = snapshot
        .issues
        .iter()
        .map(|issue| (issue.identifier.as_str(), issue))
        .collect();
    let next = queue
        .iter()
        .filter_map(|ready| issues.get(ready.identifier.as_str()).copied())
        .find(|issue| {
            selected
                .iter()
                .flatten()
                .any(|project| issue.project_id.as_ref() == Some(&project.id))
                && is_actionable_issue(issue, snapshot)
        })
        .map(|issue| ticket_summary(snapshot, issue));
    let mut current: Vec<_> = snapshot
        .issues
        .iter()
        .filter(|issue| {
            issue.team_key.0.eq_ignore_ascii_case("SWM")
                && is_open_state(&issue.state)
                && next
                    .as_ref()
                    .is_none_or(|next| next.identifier != issue.identifier)
                && issue
                    .project_id
                    .as_ref()
                    .is_some_and(|project_id| project_ids.contains(&project_id))
        })
        .collect();
    current.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    let current = current
        .into_iter()
        .map(|issue| ticket_summary(snapshot, issue))
        .collect();

    TrackerState {
        projects,
        current,
        next,
    }
}

fn summarize_project(snapshot: &WorkspaceSnapshot, project: &Project) -> ProjectSummary {
    let project_issues = snapshot
        .issues
        .iter()
        .filter(|issue| issue.project_id.as_ref() == Some(&project.id));
    let mut active_count = 0;
    let mut done_count = 0;
    for issue in project_issues {
        if issue.state.eq_ignore_ascii_case("Completed") {
            done_count += 1;
        } else if !issue.state.eq_ignore_ascii_case("Canceled") {
            active_count += 1;
        }
    }
    ProjectSummary {
        name: project.name.clone(),
        status: project.status.clone(),
        active_count,
        done_count,
        available: true,
    }
}

fn ticket_summary(snapshot: &WorkspaceSnapshot, issue: &Issue) -> TicketSummary {
    let project = issue
        .project_id
        .as_ref()
        .and_then(|id| snapshot.projects.iter().find(|project| &project.id == id))
        .map_or_else(
            || "unknown project".to_owned(),
            |project| project.name.clone(),
        );
    let issue_by_id: HashMap<_, _> = snapshot
        .issues
        .iter()
        .map(|candidate| (&candidate.id, candidate))
        .collect();
    let mut unresolved = Vec::new();
    let mut missing = Vec::new();
    for blocker_id in &issue.blocked_by {
        match issue_by_id.get(blocker_id) {
            Some(blocker) if blocker.state.eq_ignore_ascii_case("Completed") => {}
            Some(blocker) => unresolved.push(blocker.identifier.clone()),
            None => missing.push(blocker_id.0.clone()),
        }
    }
    let blockers = if !missing.is_empty() {
        BlockerState::Unknown(missing)
    } else if !unresolved.is_empty() {
        BlockerState::Blocked(unresolved)
    } else {
        BlockerState::Unblocked
    };
    TicketSummary {
        identifier: issue.identifier.clone(),
        title: issue.title.clone(),
        project,
        state: issue.state.clone(),
        priority: priority_label(issue.priority),
        blockers,
    }
}

fn project_matches(project: &Project, wanted: &str) -> bool {
    project.name.eq_ignore_ascii_case(wanted)
        || project
            .identifier
            .as_deref()
            .is_some_and(|identifier| identifier.eq_ignore_ascii_case(wanted))
}

fn is_open_state(state: &str) -> bool {
    !state.eq_ignore_ascii_case("Completed") && !state.eq_ignore_ascii_case("Canceled")
}

/// A backlog ticket is deferred for this monitor. ix-board's queue guarantees
/// recursive readiness; this additional check rejects a candidate whose direct
/// blocker state contradicts the queue record.
fn is_actionable_issue(issue: &Issue, snapshot: &WorkspaceSnapshot) -> bool {
    !issue.state.eq_ignore_ascii_case("Backlog")
        && issue.blocked_by.iter().all(|blocker_id| {
            snapshot.issues.iter().any(|blocker| {
                blocker.id == *blocker_id && blocker.state.eq_ignore_ascii_case("Completed")
            })
        })
}

fn priority_label(priority: f64) -> String {
    if priority == 1.0 {
        "Urgent".to_owned()
    } else if priority == 2.0 {
        "High".to_owned()
    } else if priority == 3.0 {
        "Medium".to_owned()
    } else if priority == 4.0 {
        "Low".to_owned()
    } else if priority == 0.0 {
        "None".to_owned()
    } else {
        "Unknown".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use ix_board::{IssueId, NextStep, ProjectId, TeamKey};

    use super::*;

    fn project(id: &str, name: &str, status: &str) -> Project {
        Project {
            id: ProjectId(id.to_owned()),
            identifier: None,
            name: name.to_owned(),
            description: String::new(),
            content: None,
            status: status.to_owned(),
            priority: 2,
            sort_order: 0.0,
            team_keys: vec![TeamKey("SWM".to_owned())],
            milestones: Vec::new(),
        }
    }

    fn issue(id: &str, project_id: &str, state: &str, blockers: Vec<&str>) -> Issue {
        Issue {
            id: IssueId(id.to_owned()),
            identifier: id.to_owned(),
            title: format!("{id} title"),
            description: None,
            state: state.to_owned(),
            team_key: TeamKey("SWM".to_owned()),
            project_id: Some(ProjectId(project_id.to_owned())),
            priority: 2.0,
            sort_order: 0.0,
            blocked_by: blockers
                .into_iter()
                .map(|id| IssueId(id.to_owned()))
                .collect(),
        }
    }

    #[test]
    fn summarizes_counts_current_next_and_dependency_state() {
        let snapshot = WorkspaceSnapshot {
            projects: vec![
                project("board", "ix-board", "Started"),
                project("projects", "ix-projects", "Paused"),
            ],
            issues: vec![
                issue("SWM-10", "board", "Coding", vec!["SWM-9"]),
                issue("SWM-11", "board", "Review", vec![]),
                issue("SWM-9", "board", "Started", vec![]),
                issue("SWM-8", "board", "Completed", vec![]),
                issue("SWM-7", "board", "Canceled", vec![]),
                issue("SWM-20", "projects", "Completed", vec![]),
            ],
        };
        let queue = vec![ReadyItem {
            project: "ix-projects".to_owned(),
            identifier: "SWM-20".to_owned(),
            title: "next".to_owned(),
            next_step: NextStep::Coding,
            reason: "ready".to_owned(),
        }];

        let state = summarize(&snapshot, &queue);
        assert_eq!(
            (state.projects[0].active_count, state.projects[0].done_count),
            (3, 1)
        );
        assert_eq!(
            (state.projects[1].active_count, state.projects[1].done_count),
            (0, 1)
        );
        assert_eq!(state.current.len(), 3);
        assert_eq!(state.current[0].identifier, "SWM-10");
        assert_eq!(state.current[1].identifier, "SWM-11");
        assert_eq!(state.current[1].state, "Review");
        assert_eq!(state.current[2].identifier, "SWM-9");
        assert_eq!(
            state.current[0].blockers,
            BlockerState::Blocked(vec!["SWM-9".to_owned()])
        );
        assert_eq!(
            state.next.as_ref().map(|ticket| ticket.identifier.as_str()),
            Some("SWM-20")
        );
    }

    #[test]
    fn missing_projects_and_blocker_references_are_not_reported_as_ready() {
        let snapshot = WorkspaceSnapshot {
            projects: vec![project("board", "ix-board", "Started")],
            issues: vec![issue("SWM-10", "board", "Coding", vec!["missing-id"])],
        };
        let state = summarize(&snapshot, &[]);
        assert!(!state.projects[1].available);
        assert_eq!(state.current.len(), 1);
        assert_eq!(
            state.current[0].blockers,
            BlockerState::Unknown(vec!["missing-id".to_owned()])
        );
    }

    #[test]
    fn backlog_and_directly_blocked_queue_items_are_not_presented_as_ready() {
        let snapshot = WorkspaceSnapshot {
            projects: vec![project("board", "ix-board", "Started")],
            issues: vec![
                issue("SWM-8", "board", "Backlog", vec!["SWM-7"]),
                issue("SWM-7", "external", "Done", vec![]),
                issue("SWM-9", "board", "Specced", vec![]),
            ],
        };
        let queue = vec![
            ReadyItem {
                project: "ix-board".to_owned(),
                identifier: "SWM-8".to_owned(),
                title: "deferred".to_owned(),
                next_step: NextStep::Specify,
                reason: "queue record".to_owned(),
            },
            ReadyItem {
                project: "ix-board".to_owned(),
                identifier: "SWM-9".to_owned(),
                title: "actionable".to_owned(),
                next_step: NextStep::Coding,
                reason: "queue record".to_owned(),
            },
        ];

        let state = summarize(&snapshot, &queue);
        assert_eq!(
            state.next.as_ref().map(|ticket| ticket.identifier.as_str()),
            Some("SWM-9")
        );
        let deferred = state
            .current
            .iter()
            .find(|ticket| ticket.identifier == "SWM-8")
            .expect("backlog ticket remains visible in the open list");
        assert_eq!(deferred.state, "Backlog");
        assert_eq!(
            deferred.blockers,
            BlockerState::Blocked(vec!["SWM-7".to_owned()])
        );
    }
}
