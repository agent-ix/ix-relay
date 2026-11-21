// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! What `focuses` commands print: text for a person, or with `--json` one compact JSON value
//! and one newline through the shared encoder, and on failure one typed error object.
//!
//! `show --json` prints the definition under `definition` in exactly the shape of the
//! definition file, so it can be saved and used as an `update` file once the expected revision
//! is supplied separately.

use std::io::{self, Write};

use relay_core::named_focus::{
    ChangeKind, DefinitionState, FocusError, FocusId, Member, MemberKind, Meta, Owner,
    OwnerBinding, OwnerChange, Revision, SessionState, Summary,
};
use serde::Serialize;

use crate::json;

/// One member as the file and `show --json` spell it.
#[derive(Serialize)]
pub(crate) struct MemberJson {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    label: String,
}

/// The four lists of one side of a selection.
#[derive(Default, Serialize)]
pub(crate) struct MemberSetJson {
    initiatives: Vec<MemberJson>,
    projects: Vec<MemberJson>,
    milestones: Vec<MemberJson>,
    tickets: Vec<MemberJson>,
}

fn grouped(members: Vec<Member>) -> MemberSetJson {
    let mut set = MemberSetJson::default();
    for member in members {
        let entry = MemberJson {
            reference: member.id,
            project: member.project_id,
            label: member.label,
        };
        match member.kind {
            MemberKind::Initiative => set.initiatives.push(entry),
            MemberKind::Project => set.projects.push(entry),
            MemberKind::Milestone => set.milestones.push(entry),
            MemberKind::Issue => set.tickets.push(entry),
        }
    }
    set
}

/// A revision's content in the shape of the definition file.
#[derive(Serialize)]
pub(crate) struct DefinitionJson {
    team: String,
    name: String,
    description: String,
    include: MemberSetJson,
    exclude: MemberSetJson,
}

/// The definition file's shape of `revision`, or `None` when its selection does not decode.
pub(crate) fn definition_json(revision: &Revision) -> Option<DefinitionJson> {
    let (include, exclude) = revision.members().ok()?;
    Some(DefinitionJson {
        team: revision.team.clone(),
        name: revision.name.clone(),
        description: revision.description.clone(),
        include: grouped(include),
        exclude: grouped(exclude),
    })
}

#[derive(Serialize)]
struct OwnerJson<'a> {
    owner_id: &'a str,
    epoch: u64,
    state: &'static str,
    binding: &'a OwnerBinding,
    change: &'a OwnerChange,
}

fn owner_json(owner: &Owner, state: SessionState) -> OwnerJson<'_> {
    OwnerJson {
        owner_id: &owner.owner_id,
        epoch: owner.epoch,
        state: state.as_str(),
        binding: &owner.binding,
        change: &owner.change,
    }
}

#[derive(Serialize)]
struct ShowJson<'a> {
    focus_id: &'a FocusId,
    state: DefinitionState,
    latest_revision: u64,
    kept_revisions: &'a [u64],
    revision: u64,
    written_at_ms: i64,
    owner_epoch_at_write: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    archived_at_ms: Option<i64>,
    owner: OwnerJson<'a>,
    empty_selection: bool,
    definition: Option<DefinitionJson>,
}

/// Prints one definition at one revision.
pub(crate) fn show(
    out: &mut impl Write,
    json: bool,
    summary: &Summary,
    kept: &[u64],
    revision: &Revision,
    owner_state: SessionState,
) -> io::Result<()> {
    let meta = &summary.meta;
    let definition = definition_json(revision);
    let empty = revision
        .members()
        .is_ok_and(|(include, _)| include.is_empty());
    if json {
        return json::write_line(
            out,
            &ShowJson {
                focus_id: &meta.focus_id,
                state: meta.state,
                latest_revision: meta.latest_revision,
                kept_revisions: kept,
                revision: revision.revision,
                written_at_ms: revision.written_at_ms,
                owner_epoch_at_write: revision.owner_epoch,
                archived_at_ms: meta.archived_at_ms,
                owner: owner_json(&summary.owner, owner_state),
                empty_selection: empty,
                definition,
            },
        );
    }
    writeln!(
        out,
        "{} {} {:?}: revision {} of {} ({}), kept {:?}",
        meta.focus_id,
        meta.team,
        revision.name,
        revision.revision,
        meta.latest_revision,
        state_word(meta.state),
        kept
    )?;
    if !revision.description.is_empty() {
        writeln!(out, "description: {}", revision.description)?;
    }
    let owner = &summary.owner;
    writeln!(
        out,
        "owner: {} epoch {} ({}) at {}",
        owner.owner_id, owner.epoch, owner_state, owner.binding.address
    )?;
    match revision.members() {
        Ok((include, exclude)) => {
            if include.is_empty() {
                writeln!(out, "selects nothing: the inclusion list is empty")?;
            }
            members_text(out, "include", &include)?;
            members_text(out, "exclude", &exclude)
        }
        Err(reason) => writeln!(out, "selection unavailable: {reason}"),
    }
}

fn members_text(out: &mut impl Write, side: &str, members: &[Member]) -> io::Result<()> {
    for member in members {
        let project = member
            .project_id
            .as_deref()
            .map_or_else(String::new, |project| format!(" in {project}"));
        writeln!(
            out,
            "{side} {} {}{project} ({})",
            member.kind.wire(),
            member.id,
            member.label
        )?;
    }
    Ok(())
}

fn state_word(state: DefinitionState) -> &'static str {
    match state {
        DefinitionState::Active => "active",
        DefinitionState::Archived => "archived",
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum ListItemJson<'a> {
    Available {
        focus_id: &'a FocusId,
        team: &'a str,
        name: &'a str,
        state: DefinitionState,
        latest_revision: u64,
        owner_id: &'a str,
        owner_epoch: u64,
        owner_state: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        archived_at_ms: Option<i64>,
    },
    Unavailable {
        focus_id: &'a FocusId,
        available: bool,
        reason: &'static str,
    },
}

/// One listed definition.
pub(crate) enum ListRow<'a> {
    /// A definition that decodes, with its owner's state.
    Available(&'a Summary, SessionState),
    /// A definition that cannot be used.
    Unavailable(&'a FocusId, &'static str),
}

/// Prints a listing.
pub(crate) fn list(out: &mut impl Write, json: bool, rows: &[ListRow<'_>]) -> io::Result<()> {
    if json {
        #[derive(Serialize)]
        struct Listing<'a> {
            definitions: Vec<ListItemJson<'a>>,
        }
        let definitions = rows
            .iter()
            .map(|row| match row {
                ListRow::Available(summary, state) => ListItemJson::Available {
                    focus_id: &summary.meta.focus_id,
                    team: &summary.meta.team,
                    name: &summary.meta.name,
                    state: summary.meta.state,
                    latest_revision: summary.meta.latest_revision,
                    owner_id: &summary.owner.owner_id,
                    owner_epoch: summary.owner.epoch,
                    owner_state: state.as_str(),
                    archived_at_ms: summary.meta.archived_at_ms,
                },
                ListRow::Unavailable(focus, reason) => ListItemJson::Unavailable {
                    focus_id: focus,
                    available: false,
                    reason: *reason,
                },
            })
            .collect();
        return json::write_line(out, &Listing { definitions });
    }
    if rows.is_empty() {
        return writeln!(out, "no named focus definitions");
    }
    for row in rows {
        match row {
            ListRow::Available(summary, state) => writeln!(
                out,
                "{} {} {:?} r{} {} owner {} epoch {} ({})",
                summary.meta.focus_id,
                summary.meta.team,
                summary.meta.name,
                summary.meta.latest_revision,
                state_word(summary.meta.state),
                summary.owner.owner_id,
                summary.owner.epoch,
                state
            )?,
            ListRow::Unavailable(focus, reason) => {
                writeln!(out, "{focus} unavailable_definition: {reason}")?;
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct CreatedJson<'a> {
    focus_id: &'a FocusId,
    team: &'a str,
    name: &'a str,
    revision: u64,
    owner_id: &'a str,
    owner_epoch: u64,
    removed: Option<&'a FocusId>,
}

/// Prints the result of a creation.
pub(crate) fn created(
    out: &mut impl Write,
    json: bool,
    meta: &Meta,
    owner: &Owner,
    removed: Option<&FocusId>,
) -> io::Result<()> {
    if json {
        return json::write_line(
            out,
            &CreatedJson {
                focus_id: &meta.focus_id,
                team: &meta.team,
                name: &meta.name,
                revision: meta.latest_revision,
                owner_id: &owner.owner_id,
                owner_epoch: owner.epoch,
                removed,
            },
        );
    }
    writeln!(
        out,
        "created {} {:?} revision {} owned by {} at epoch {}",
        meta.focus_id, meta.name, meta.latest_revision, owner.owner_id, owner.epoch
    )?;
    if let Some(removed) = removed {
        writeln!(out, "removed archived definition {removed} to make room")?;
    }
    Ok(())
}

#[derive(Serialize)]
struct UpdatedJson<'a> {
    focus_id: &'a FocusId,
    revision: u64,
    changed: bool,
}

/// Prints the result of an update.
pub(crate) fn updated(
    out: &mut impl Write,
    json: bool,
    focus: &FocusId,
    revision: u64,
    changed: bool,
) -> io::Result<()> {
    if json {
        return json::write_line(
            out,
            &UpdatedJson {
                focus_id: focus,
                revision,
                changed,
            },
        );
    }
    if changed {
        writeln!(out, "updated {focus} to revision {revision}")
    } else {
        writeln!(out, "{focus} already holds this content at revision {revision}; nothing written")
    }
}

#[derive(Serialize)]
struct ArchivedJson<'a> {
    focus_id: &'a FocusId,
    state: DefinitionState,
    latest_revision: u64,
    archived_at_ms: Option<i64>,
}

/// Prints the result of an archive.
pub(crate) fn archived(out: &mut impl Write, json: bool, meta: &Meta) -> io::Result<()> {
    if json {
        return json::write_line(
            out,
            &ArchivedJson {
                focus_id: &meta.focus_id,
                state: meta.state,
                latest_revision: meta.latest_revision,
                archived_at_ms: meta.archived_at_ms,
            },
        );
    }
    writeln!(out, "archived {} at revision {}", meta.focus_id, meta.latest_revision)
}

#[derive(Serialize)]
struct OwnedJson<'a> {
    focus_id: &'a FocusId,
    owner_id: &'a str,
    owner_epoch: u64,
    change: &'a OwnerChange,
}

/// Prints the result of a claim or a transfer.
pub(crate) fn owned(
    out: &mut impl Write,
    json: bool,
    focus: &FocusId,
    owner: &Owner,
) -> io::Result<()> {
    if json {
        return json::write_line(
            out,
            &OwnedJson {
                focus_id: focus,
                owner_id: &owner.owner_id,
                owner_epoch: owner.epoch,
                change: &owner.change,
            },
        );
    }
    let how = match owner.change.kind {
        ChangeKind::Create => "created by",
        ChangeKind::Claim => "claimed by",
        ChangeKind::ForcedClaim => "claimed by force by",
        ChangeKind::Transfer => "transferred to",
    };
    writeln!(out, "{focus} {how} {} at epoch {}", owner.owner_id, owner.epoch)
}

/// A typed error object: the stable code, a message and whatever is relevant to it.
#[derive(Serialize)]
pub(crate) struct ErrorJson {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_owner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_owner_epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bound: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

/// The typed error object of `error`.
pub(crate) fn error_json(error: &FocusError) -> ErrorJson {
    let mut body = ErrorJson {
        code: error.code().as_str(),
        message: error.to_string(),
        focus_id: None,
        current_revision: None,
        current_owner_id: None,
        current_owner_epoch: None,
        reason: None,
        bound: None,
        max: None,
        field: None,
        reference: None,
        name: None,
        source: None,
    };
    match error {
        FocusError::RevisionConflict { focus, latest } => {
            body.focus_id = Some(focus.to_string());
            body.current_revision = Some(*latest);
        }
        FocusError::OwnerEpochConflict {
            focus,
            owner_id,
            epoch,
        }
        | FocusError::NotOwner {
            focus,
            owner_id,
            epoch,
            ..
        } => {
            body.focus_id = Some(focus.to_string());
            body.current_owner_id = Some(owner_id.clone());
            body.current_owner_epoch = Some(*epoch);
        }
        FocusError::NameTaken { name, holder, .. } => {
            body.name = Some(name.clone());
            body.focus_id = holder.as_ref().map(ToString::to_string);
        }
        FocusError::Archived { focus } => body.focus_id = Some(focus.to_string()),
        FocusError::Limit { bound, max } => {
            body.bound = Some(*bound);
            body.max = Some(*max);
        }
        FocusError::UnresolvedReference { reference, .. } => {
            body.reference = Some(reference.clone());
        }
        FocusError::SourceUnavailable { reason, origin, .. } => {
            body.reason = Some(reason.as_str());
            body.source = Some(origin.clone());
        }
        FocusError::UnavailableDefinition { focus, reason } => {
            body.focus_id = Some(focus.to_string());
            body.reason = Some(reason.as_str());
        }
        FocusError::NotFound { focus } => body.focus_id = Some(focus.clone()),
        FocusError::InvalidArgument { field, .. } => body.field = Some(field.clone()),
        FocusError::Random(_) | FocusError::Store(_) => {
            body.reason = Some("failed");
            body.source = Some("relay store".to_owned());
        }
    }
    body
}

/// Prints a failure on `err`: one typed JSON object with `--json`, one line otherwise.
pub(crate) fn failure(err: &mut impl Write, json: bool, error: &FocusError) {
    let _ = if json {
        json::write_line(err, &error_json(error))
    } else {
        writeln!(err, "agent-msg: {}: {error}", error.code().as_str())
    };
}
