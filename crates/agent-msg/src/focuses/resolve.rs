// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Reference resolution for `focuses create` and `focuses update` (FR-061 "Reference
//! resolution and the bounded snapshot read").
//!
//! Resolution is one call of the shared ix-board read service,
//! `ReadService::resolve_named_selection_bounded`. One deadline (120 seconds by default,
//! 1 to 600 with `--read-timeout`) starts after the credentials are ready and before the cache
//! lookup, and covers the cache, the source read and every resolution scan. Cached entries up
//! to five minutes old are accepted; an older one is refreshed under the same deadline. No
//! document and no association is returned after a timeout or failure. This module starts no
//! thread and makes no second read; nothing here reads Linear, handles a token or computes
//! membership.
//!
//! The call returns the canonical selection and, for every input occurrence, the member it
//! resolved to, identified by `(list, index)` of the input as it was sent. Labels are joined
//! to members by that association only, never by position in the canonical lists.
//!
//! Label rule, per list and per canonical member (the labels are display only):
//! 1. the label the caller wrote in the file for an occurrence, else
//! 2. the reference text the caller wrote for it, else (an empty reference) the canonical ID;
//! 3. when several occurrences collapse to one member (aliases, repeats), an explicit label
//!    beats a reference text, and among equals the first occurrence in the file's order wins;
//!    a later conflicting label is not kept.
//!
//! The text is then sanitized to one line and cut to 64 bytes on a UTF-8 boundary by
//! `FocusContent::normalize`.

use std::{collections::BTreeMap, time::Duration};

#[cfg(test)]
use ix_board::{Clock, WorkspaceSource};
use ix_board::{
    LinearGraphQlSource, ListKind, NamedMember, NamedMemberInput, NamedSelectionInput, ReadPolicy,
    ReadService, ResolvedNamedSelection,
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
    /// Resolves `input` to a canonical selection and the association of every input occurrence
    /// under `policy`: one deadline over the cache, the read and the resolution.
    ///
    /// The credentials are resolved before the deadline starts and the wait for them is not
    /// counted: obtaining them can wait for the user's keychain.
    fn resolve_named(
        &mut self,
        policy: &ReadPolicy,
        input: &NamedSelectionInput,
    ) -> Result<ResolvedNamedSelection, ix_board::Error>;
}

/// The production source: the user's normal credentials through the shared ix-board service.
pub(crate) struct LinearSnapshots;

impl SnapshotSource for LinearSnapshots {
    fn resolve_named(
        &mut self,
        policy: &ReadPolicy,
        input: &NamedSelectionInput,
    ) -> Result<ResolvedNamedSelection, ix_board::Error> {
        let source = LinearGraphQlSource::from_env()?;
        let mut service = ReadService::open_default(source)?;
        service.resolve_named_selection_bounded(policy, input)
    }
}

/// A source over an already opened service, for a caller that supplies its own source and clock.
#[cfg(test)]
pub(crate) struct ServiceSnapshots<S: WorkspaceSource, C: Clock>(pub(crate) ReadService<S, C>);

#[cfg(test)]
impl<S: WorkspaceSource, C: Clock> SnapshotSource for ServiceSnapshots<S, C> {
    fn resolve_named(
        &mut self,
        policy: &ReadPolicy,
        input: &NamedSelectionInput,
    ) -> Result<ResolvedNamedSelection, ix_board::Error> {
        self.0.resolve_named_selection_bounded(policy, input)
    }
}

fn unavailable(reason: SourceReason, error: &ix_board::Error) -> FocusError {
    FocusError::SourceUnavailable {
        reason,
        origin: SOURCE.to_owned(),
        message: error.to_string(),
    }
}

/// Which of the five reasons the contract names a failed read has.
///
/// Every variant of the SDK error is listed, with no catch-all and no `_`: the SDK enum is not
/// `non_exhaustive`, so a variant added by a later pin stops this from compiling and is placed
/// by a person, instead of becoming `failed` unnoticed.
fn reason_of(error: &ix_board::Error) -> SourceReason {
    use ix_board::Error as E;
    match error {
        E::ReadTimeout { .. } => SourceReason::Timeout,
        // No credential was available to read with.
        E::MissingApiKey | E::Credential(_) | E::CredentialInput(_) | E::EmptyCredentialInput => {
            SourceReason::Missing
        }
        // The data the read returned is incomplete, so nothing may be validated against it.
        E::MissingWorkspaceData
        | E::PaginationLimit { .. }
        | E::NestedPaginationLimit { .. }
        | E::MissingCursor
        | E::InvalidCursor { .. }
        | E::RelationsLimit { .. }
        | E::MissingParent { .. }
        | E::MissingDependency { .. }
        | E::MissingProjectMilestone { .. }
        | E::ParentCycle { .. }
        | E::DependencyCycle { .. }
        | E::FocusPlanIncomplete { .. }
        | E::FocusHygieneIncomplete { .. } => SourceReason::Partial,
        // The read itself failed, or the failure belongs to no read of a snapshot.
        E::Http(_)
        | E::LinearRateLimited { .. }
        // The shared quota protection refused or could not run the read; no snapshot was read.
        | E::LinearLocalBudgetExhausted { .. }
        // The shared quota protection refused admission with a typed decision; the bounded read
        // returns no snapshot, so the read failed whatever the decision's hold.
        | E::QuotaAdmissionRefused { .. }
        | E::RefreshLockTimeout
        | E::LinearQueryComplexityExceeded { .. }
        | E::InvalidQuotaConfig { .. }
        | E::InvalidQuotaReservation { .. }
        | E::InvalidCacheTtl
        | E::InvalidConfiguration
        // A successful response that broke the typed wire contract is not a usable snapshot.
        | E::LinearDecode(_)
        | E::GraphQl { .. }
        | E::CacheDirectoryUnavailable
        | E::CacheDirectory(_)
        | E::Cache(_)
        | E::CachePermissions(_)
        | E::CacheTimestamp
        | E::CacheDecode(_)
        | E::CacheEncode(_)
        | E::BoundUnsupported
        | E::InvalidReadPolicy { .. }
        | E::ProgramReview { .. }
        | E::WorkMilestoneNotFound { .. }
        | E::InvalidQuery
        | E::DriftInputRead { .. }
        | E::DriftInputDecode { .. }
        | E::Drift(_)
        | E::Retro(_)
        | E::Agents(_)
        | E::ProjectNotFound { .. }
        | E::InvalidProjectStatus { .. }
        | E::InvalidProjectPriority { .. }
        | E::InvalidProjectSortOrder
        | E::ProjectStatusUnavailable { .. }
        | E::ProjectStatusAmbiguous { .. }
        | E::ProjectUpdateRejected { .. }
        | E::EmptyFocus
        | E::FocusHygieneResourceLimit { .. }
        | E::InvalidFocusHygieneTime
        | E::UnsupportedSchemaVersion { .. }
        | E::InvalidNamedSelection { .. }
        | E::DuplicateSelectionKey { .. }
        // A fault of the caller's input: `unresolved` reports it before a read is blamed.
        | E::ScopeSelection { .. }
        | E::AmbiguousMilestone { .. }
        | E::NamedMemberRejected { .. }
        | E::InvalidTeam => SourceReason::Failed,
    }
}

/// How a failed read is reported: a `source_unavailable` with the reason of [`reason_of`].
pub(crate) fn source_error(error: &ix_board::Error) -> FocusError {
    unavailable(reason_of(error), error)
}

/// How the bounded canonicalization failed: a fault of the caller's reference, or of the read.
pub(crate) fn unresolved(error: ix_board::Error) -> FocusError {
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
        // Every other variant is a failure of the read and is placed by `reason_of`.
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

/// The members of one list in the order they are sent to the SDK: by kind, then file order.
/// The position in this list is the `index` of the occurrence's association.
fn ordered(members: &FileMembers) -> Vec<(MemberKind, &FileMember)> {
    [
        (MemberKind::Initiative, &members.initiatives),
        (MemberKind::Project, &members.projects),
        (MemberKind::Milestone, &members.milestones),
        (MemberKind::Issue, &members.tickets),
    ]
    .into_iter()
    .flat_map(|(kind, list)| list.iter().map(move |member| (kind, member)))
    .collect()
}

fn input_list(sent: &[(MemberKind, &FileMember)]) -> Vec<NamedMemberInput> {
    sent.iter()
        .map(|(kind, member)| input_member(*kind, member))
        .collect()
}

/// The refusal for a result of the shared read that does not match what was sent. The SDK
/// promises one association per occurrence and a member for every canonical entry; a result
/// that breaks it is a failed read and is never stored.
fn mismatch(detail: &str) -> FocusError {
    FocusError::SourceUnavailable {
        reason: SourceReason::Failed,
        origin: SOURCE.to_owned(),
        message: format!(
            "the shared read returned an association that does not match the input: {detail}"
        ),
    }
}

/// A display label and whether the caller wrote it.
struct Chosen<'a> {
    explicit: bool,
    text: &'a str,
}

/// The relay member of a canonical member, with its display label.
fn member_of(member: &NamedMember, label: &str) -> Member {
    match member {
        NamedMember::Initiative { id } => Member::new(MemberKind::Initiative, id, label),
        NamedMember::Project { id } => Member::new(MemberKind::Project, id, label),
        NamedMember::Milestone { id, project_id } => Member::milestone(id, project_id, label),
        NamedMember::Issue { id } => Member::new(MemberKind::Issue, id, label),
    }
}

/// The members of one canonical list with their labels, joined to the input occurrences by
/// `(list, index)` through the SDK's own association.
///
/// Every occurrence of `list` is looked up by its index and must carry the input that was
/// sent at that index; a canonical member no occurrence resolved to is a mismatch. Nothing is
/// matched by position in `members`, which is sorted and shorter when occurrences collapse.
fn labelled(
    list: ListKind,
    sent: &[(MemberKind, &FileMember)],
    inputs: &[NamedMemberInput],
    resolved: &ResolvedNamedSelection,
    members: &[NamedMember],
) -> FocusResult<Vec<Member>> {
    let mut chosen: BTreeMap<&NamedMember, Chosen<'_>> = BTreeMap::new();
    for (index, ((_, file), supplied)) in sent.iter().zip(inputs).enumerate() {
        let entry = resolved
            .association(list, index)
            .filter(|entry| entry.input == *supplied)
            .ok_or_else(|| {
                mismatch(&format!(
                    "{list:?} occurrence {index} has none or another input"
                ))
            })?;
        let own = file
            .label
            .as_deref()
            .filter(|label| !label.trim().is_empty());
        let candidate = match own {
            Some(label) => Chosen {
                explicit: true,
                text: label,
            },
            None => Chosen {
                explicit: false,
                text: &file.reference,
            },
        };
        let keep_first = chosen
            .get(&&entry.member)
            .is_some_and(|kept| kept.explicit || !candidate.explicit);
        if !keep_first {
            chosen.insert(&entry.member, candidate);
        }
    }
    members
        .iter()
        .map(|member| {
            let kept = chosen.get(&member).ok_or_else(|| {
                mismatch(&format!("a canonical {list:?} member has no occurrence"))
            })?;
            let text = if kept.text.trim().is_empty() {
                member.id()
            } else {
                kept.text
            };
            Ok(member_of(member, text))
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
    let (include, exclude) = (ordered(&file.include), ordered(&file.exclude));
    let input = NamedSelectionInput {
        team: file.team.clone(),
        include: input_list(&include),
        exclude: input_list(&exclude),
    };
    let resolved = snapshots
        .resolve_named(&policy, &input)
        .map_err(unresolved)?;
    let sent = input.include.len() + input.exclude.len();
    if resolved.associations.len() != sent {
        return Err(mismatch(&format!(
            "{} associations for {sent} inputs",
            resolved.associations.len()
        )));
    }
    let selection = &resolved.selection;
    Ok(FocusContent {
        team: selection.team.0.clone(),
        name: file.name.clone(),
        description: file.description.clone(),
        include: labelled(
            ListKind::Include,
            &include,
            &input.include,
            &resolved,
            &selection.include,
        )?,
        exclude: labelled(
            ListKind::Exclude,
            &exclude,
            &input.exclude,
            &resolved,
            &selection.exclude,
        )?,
    })
}
