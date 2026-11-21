// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Reference resolution for `focuses create` and `focuses update` (FR-061 "Reference
//! resolution and the bounded snapshot read").
//!
//! Resolution is one call of the shared ix-board read service,
//! `ReadService::canonicalize_named_selection_bounded`. One deadline (120 seconds by default,
//! 1 to 600 with `--read-timeout`) starts after the credentials are ready and before the cache
//! lookup, and covers the cache, the source read and every resolution scan. Cached entries up
//! to five minutes old are accepted; an older one is refreshed under the same deadline. No
//! document is returned after a timeout or failure. This module starts no thread and makes no
//! second read; nothing here reads Linear, handles a token or computes membership.

use std::time::Duration;

#[cfg(test)]
use ix_board::{Clock, WorkspaceSource};
use ix_board::{
    LinearGraphQlSource, NamedMember, NamedMemberInput, NamedSelection, NamedSelectionInput,
    ReadPolicy, ReadService,
};
use relay_core::named_focus::{
    FocusContent, FocusError, FocusResult, Member, MemberKind, SourceReason,
};

use super::file::{DefinitionFile, FileMember, FileMembers};

/// The default overall deadline of the resolution.
pub(crate) const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Entries of the shared cache live five minutes; an older snapshot never validates a selection.
pub(crate) const MAX_SNAPSHOT_AGE: Duration = Duration::from_secs(300);

/// What names the snapshot source in a `source_unavailable` refusal.
const SOURCE: &str = "ix-board workspace snapshot";

/// Where canonical selections come from.
pub(crate) trait SnapshotSource {
    /// Resolves `input` to a canonical selection under `policy`: one deadline over the cache,
    /// the read and the resolution.
    ///
    /// The credentials are resolved before the deadline starts and the wait for them is not
    /// counted: obtaining them can wait for the user's keychain.
    fn canonicalize(
        &mut self,
        policy: &ReadPolicy,
        input: &NamedSelectionInput,
    ) -> Result<NamedSelection, ix_board::Error>;
}

/// The production source: the user's normal credentials through the shared ix-board service.
pub(crate) struct LinearSnapshots;

impl SnapshotSource for LinearSnapshots {
    fn canonicalize(
        &mut self,
        policy: &ReadPolicy,
        input: &NamedSelectionInput,
    ) -> Result<NamedSelection, ix_board::Error> {
        let source = LinearGraphQlSource::from_env()?;
        let mut service = ReadService::open_default(source)?;
        service.canonicalize_named_selection_bounded(policy, input)
    }
}

/// A source over an already opened service, for a caller that supplies its own source and clock.
#[cfg(test)]
pub(crate) struct ServiceSnapshots<S: WorkspaceSource, C: Clock>(pub(crate) ReadService<S, C>);

#[cfg(test)]
impl<S: WorkspaceSource, C: Clock> SnapshotSource for ServiceSnapshots<S, C> {
    fn canonicalize(
        &mut self,
        policy: &ReadPolicy,
        input: &NamedSelectionInput,
    ) -> Result<NamedSelection, ix_board::Error> {
        self.0.canonicalize_named_selection_bounded(policy, input)
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
        E::MissingApiKey | E::Credential(_) | E::CredentialInput(_) | E::EmptyCredentialInput => {
            SourceReason::Missing
        }
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

/// The members with their canonical ID as the display label.
///
/// The bounded method returns the canonical document only, not the snapshot, and a second
/// read is not allowed, so a name cannot be looked up here. The label only displays.
fn typed(members: &[NamedMember]) -> Vec<Member> {
    members
        .iter()
        .map(|member| {
            let label = member.id().to_owned();
            match member {
                NamedMember::Initiative { id } => Member::new(MemberKind::Initiative, id, label),
                NamedMember::Project { id } => Member::new(MemberKind::Project, id, label),
                NamedMember::Milestone { id, project_id } => {
                    Member::milestone(id, project_id, label)
                }
                NamedMember::Issue { id } => Member::new(MemberKind::Issue, id, label),
            }
        })
        .collect()
}

/// The content of `file` with every reference resolved to a canonical stable ID.
///
/// Nothing is written by this function. A missing, partial, failed or timed-out read, or an
/// unresolvable reference, returns a typed error and no content.
pub(crate) fn resolve(
    file: &DefinitionFile,
    snapshots: &mut dyn SnapshotSource,
    read_timeout: Duration,
) -> FocusResult<FocusContent> {
    let policy = ReadPolicy {
        max_age: MAX_SNAPSHOT_AGE,
        deadline: read_timeout,
    };
    let selection: NamedSelection = snapshots
        .canonicalize(
            &policy,
            &NamedSelectionInput {
                team: file.team.clone(),
                include: input_list(&file.include),
                exclude: input_list(&file.exclude),
            },
        )
        .map_err(unresolved)?;
    Ok(FocusContent {
        team: selection.team.0.clone(),
        name: file.name.clone(),
        description: file.description.clone(),
        include: typed(&selection.include),
        exclude: typed(&selection.exclude),
    })
}
