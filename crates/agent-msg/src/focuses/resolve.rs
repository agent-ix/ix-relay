// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Reference resolution for `focuses create` and `focuses update` (FR-061 "Reference
//! resolution and the bounded snapshot read").
//!
//! The snapshot comes only from the shared ix-board read service, read once under one policy:
//! entries up to five minutes old are accepted, and an overall deadline (120 seconds by
//! default, 1 to 600 with `--read-timeout`) starts after the credentials are ready and before
//! the cache lookup. The read is enforced at the source boundary, which answers with a typed
//! timeout; this module starts no thread and no read that outlives the command. References
//! resolve to canonical stable IDs through ix-board's own resolver; nothing here reads Linear,
//! handles a token or computes membership.

use std::time::{Duration, Instant};

use ix_board::{
    LinearGraphQlSource, NamedMember, NamedMemberInput, NamedSelection, NamedSelectionInput,
    ReadPolicy, ReadService, WorkspaceSnapshot, canonicalize_named_selection,
};
#[cfg(test)]
use ix_board::{Clock, WorkspaceSource};
use relay_core::named_focus::{
    FocusContent, FocusError, FocusResult, Member, MemberKind, SourceReason,
};

use super::file::{DefinitionFile, FileMember, FileMembers};

/// The default overall deadline of the snapshot read.
pub(crate) const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Entries of the shared cache live five minutes; an older snapshot never validates a selection.
pub(crate) const MAX_SNAPSHOT_AGE: Duration = Duration::from_secs(300);

/// What names the snapshot source in a `source_unavailable` refusal.
const SOURCE: &str = "ix-board workspace snapshot";

/// Where a bounded snapshot comes from.
pub(crate) trait SnapshotSource {
    /// Reads one snapshot under `policy`.
    ///
    /// The credentials are resolved before the deadline starts and the wait for them is not
    /// counted: obtaining them can wait for the user's keychain.
    fn snapshot(&mut self, policy: &ReadPolicy) -> Result<WorkspaceSnapshot, ix_board::Error>;
}

/// The production source: the user's normal credentials through the shared ix-board service.
pub(crate) struct LinearSnapshots;

impl SnapshotSource for LinearSnapshots {
    fn snapshot(&mut self, policy: &ReadPolicy) -> Result<WorkspaceSnapshot, ix_board::Error> {
        let source = LinearGraphQlSource::from_env()?;
        let mut service = ReadService::open_default(source)?;
        service.workspace_snapshot_bounded(policy)
    }
}

/// A source over an already opened service, for a caller that supplies its own source and clock.
#[cfg(test)]
pub(crate) struct ServiceSnapshots<S: WorkspaceSource, C: Clock>(pub(crate) ReadService<S, C>);

#[cfg(test)]
impl<S: WorkspaceSource, C: Clock> SnapshotSource for ServiceSnapshots<S, C> {
    fn snapshot(&mut self, policy: &ReadPolicy) -> Result<WorkspaceSnapshot, ix_board::Error> {
        self.0.workspace_snapshot_bounded(policy)
    }
}

fn unavailable(reason: SourceReason, error: &ix_board::Error) -> FocusError {
    FocusError::SourceUnavailable {
        reason,
        origin: SOURCE.to_owned(),
        message: error.to_string(),
    }
}

/// How a failed read is reported: which of the five reasons the contract names.
pub(crate) fn source_error(error: &ix_board::Error) -> FocusError {
    use ix_board::Error as E;
    let reason = match error {
        E::ReadTimeout { .. } => SourceReason::Timeout,
        E::MissingApiKey
        | E::Credential(_)
        | E::CredentialInput(_)
        | E::EmptyCredentialInput => SourceReason::Missing,
        E::MissingWorkspaceData
        | E::PaginationLimit { .. }
        | E::NestedPaginationLimit { .. }
        | E::MissingCursor
        | E::InvalidCursor { .. }
        | E::RelationsLimit { .. }
        | E::MissingParent { .. }
        | E::MissingDependency { .. }
        | E::ParentCycle { .. }
        | E::DependencyCycle { .. } => SourceReason::Partial,
        _ => SourceReason::Failed,
    };
    unavailable(reason, error)
}

fn unresolved(error: ix_board::Error) -> FocusError {
    use ix_board::Error as E;
    match error {
        E::ScopeSelection {
            kind,
            identifier,
            matches,
        } => FocusError::UnresolvedReference {
            message: format!("{kind} matched {matches} records, not exactly one"),
            reference: identifier,
        },
        E::AmbiguousMilestone {
            selector,
            candidates,
        } => FocusError::UnresolvedReference {
            message: format!(
                "the milestone name is ambiguous; qualify it with its project: {candidates}"
            ),
            reference: selector,
        },
        E::NamedMemberRejected {
            kind,
            identifier,
            outcome,
        } => FocusError::UnresolvedReference {
            message: format!("{kind} cannot be a member of this selection: {outcome:?}"),
            reference: identifier,
        },
        E::InvalidTeam => FocusError::invalid("team", "must not be empty"),
        other => source_error(&other),
    }
}

fn input_member(kind: MemberKind, member: &FileMember) -> NamedMemberInput {
    match kind {
        MemberKind::Initiative => NamedMemberInput::Initiative(member.reference.clone()),
        MemberKind::Project => NamedMemberInput::Project(member.reference.clone()),
        MemberKind::Milestone => NamedMemberInput::Milestone {
            selector: member.reference.clone(),
            project: member.project.clone(),
        },
        MemberKind::Issue => NamedMemberInput::Ticket(member.reference.clone()),
    }
}

fn input_list(members: &FileMembers) -> Vec<NamedMemberInput> {
    [
        (MemberKind::Initiative, &members.initiatives),
        (MemberKind::Project, &members.projects),
        (MemberKind::Milestone, &members.milestones),
        (MemberKind::Issue, &members.tickets),
    ]
    .into_iter()
    .flat_map(|(kind, list)| list.iter().map(move |member| input_member(kind, member)))
    .collect()
}

/// The display label of a canonical member, from the snapshot it resolved in.
fn label(snapshot: &WorkspaceSnapshot, member: &NamedMember) -> String {
    match member {
        NamedMember::Initiative { id } => snapshot
            .projects
            .iter()
            .flat_map(|project| &project.initiatives)
            .find(|initiative| initiative.id == *id)
            .map(|initiative| initiative.name.clone()),
        NamedMember::Project { id } => snapshot
            .projects
            .iter()
            .find(|project| project.id.0 == *id)
            .map(|project| project.name.clone()),
        NamedMember::Milestone { id, project_id } => snapshot
            .projects
            .iter()
            .filter(|project| project.id.0 == *project_id)
            .flat_map(|project| &project.milestones)
            .find(|milestone| milestone.id == *id)
            .map(|milestone| milestone.name.clone()),
        NamedMember::Issue { id } => snapshot
            .issues
            .iter()
            .find(|issue| issue.id.0 == *id)
            .map(|issue| issue.identifier.clone()),
    }
    .unwrap_or_else(|| member.id().to_owned())
}

fn typed(snapshot: &WorkspaceSnapshot, members: &[NamedMember]) -> Vec<Member> {
    members
        .iter()
        .map(|member| {
            let label = label(snapshot, member);
            match member {
                NamedMember::Initiative { id } => Member::new(MemberKind::Initiative, id, label),
                NamedMember::Project { id } => Member::new(MemberKind::Project, id, label),
                NamedMember::Milestone { id, project_id } => Member::milestone(id, project_id, label),
                NamedMember::Issue { id } => Member::new(MemberKind::Issue, id, label),
            }
        })
        .collect()
}

/// KNOWN DEFECT, PROVISIONAL (SWM-128): the deadline is not one budget over the whole
/// operation. The snapshot read is bounded, but `canonicalize_named_selection` runs
/// unbounded afterwards and is only checked against the deadline once it returns. This
/// does not satisfy FR-061-AC-19/20 and is to be replaced by the corrected composed-input
/// `ReadService` seam of the reopened ix-board work when its accepted API is handed off.
///
/// The content of `file` with every reference resolved to a canonical stable ID and labelled.
///
/// Nothing is written by this function. An older, missing, partial or failed snapshot, or one
/// whose read outlived its deadline, never validates a selection.
pub(crate) fn resolve(
    file: &DefinitionFile,
    snapshots: &mut dyn SnapshotSource,
    read_timeout: Duration,
    now_ms: i64,
) -> FocusResult<FocusContent> {
    let policy = ReadPolicy {
        max_age: MAX_SNAPSHOT_AGE,
        deadline: read_timeout,
    };
    let started = Instant::now();
    let snapshot = snapshots.snapshot(&policy).map_err(|error| source_error(&error))?;
    let age_seconds = now_ms / 1_000 - snapshot.fetched_at;
    if age_seconds > i64::try_from(MAX_SNAPSHOT_AGE.as_secs()).unwrap_or(i64::MAX) {
        return Err(FocusError::SourceUnavailable {
            reason: SourceReason::Stale,
            origin: SOURCE.to_owned(),
            message: format!("the snapshot was fetched {age_seconds} seconds ago"),
        });
    }
    let selection: NamedSelection = canonicalize_named_selection(
        &snapshot,
        &NamedSelectionInput {
            team: file.team.clone(),
            include: input_list(&file.include),
            exclude: input_list(&file.exclude),
        },
    )
    .map_err(unresolved)?;
    if started.elapsed() > read_timeout {
        return Err(FocusError::SourceUnavailable {
            reason: SourceReason::Timeout,
            origin: SOURCE.to_owned(),
            message: format!("resolution outlived the {read_timeout:?} deadline"),
        });
    }
    Ok(FocusContent {
        team: selection.team.0.clone(),
        name: file.name.clone(),
        description: file.description.clone(),
        include: typed(&snapshot, &selection.include),
        exclude: typed(&snapshot, &selection.exclude),
    })
}
