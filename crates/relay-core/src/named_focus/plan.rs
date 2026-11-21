// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The vocabulary of a named-focus write: who the caller is, what a prepared write holds, and
//! what an applied one returns.
//!
//! A restricted write has two steps so that its atomicity can be tested and relied on. The
//! relay *plans* the write from one read of the definition, refusing everything that read
//! already refutes, and then *commits* the plan in one server-side operation that compares
//! again before its first write. A change that lands between the two makes the commit refuse
//! (or ask for a re-plan); it is never overwritten.

use super::{
    error::FocusError,
    model::{FocusId, Meta, Owner, OwnerBinding, Revision},
};

/// A definition addressed by ID, or by the exact name it had when it was resolved.
///
/// A change by name resolves the name to an ID first and then changes by ID; the commit
/// refuses when the name no longer resolves to that ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The definition the reference resolved to.
    pub id: FocusId,
    pub(crate) by_name: Option<ResolvedName>,
}

/// The name a [`Resolved`] was found by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedName {
    pub(crate) team: String,
    pub(crate) name: String,
    pub(crate) key: String,
}

impl Resolved {
    /// A definition addressed by its ID.
    #[must_use]
    pub fn by_id(id: FocusId) -> Self {
        Self { id, by_name: None }
    }

    /// The name key the commit must still find mapped to the ID, or empty when addressed by ID.
    pub(crate) fn name_key(&self) -> String {
        self.by_name
            .as_ref()
            .map_or_else(String::new, |name| name.key.clone())
    }
}

/// The verified caller of a restricted write and what it verified.
#[derive(Clone, Copy, Debug)]
pub struct Caller<'a> {
    /// The caller's verified live session.
    pub binding: &'a OwnerBinding,
    /// The owner epoch the caller verified. For a claim it is the epoch of the owner the
    /// caller observed.
    pub expected_epoch: u64,
    /// The time of the write, Unix milliseconds.
    pub now_ms: i64,
}

/// What a claim adds to the [`Caller`].
#[derive(Clone, Copy, Debug)]
pub struct ClaimRequest<'a> {
    /// The stored owner ID the caller observed in the read it validated. A claim compares this
    /// with the stored owner, never the caller's own ID.
    pub observed_owner_id: &'a str,
    /// `--force --reason <text>`: take over a live or unknown owner, recording the reason.
    pub force_reason: Option<&'a str>,
}

/// What an applied write changed.
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    /// A new revision was written.
    Revision(Box<Revision>),
    /// The definition was archived.
    Archived(Box<Meta>),
    /// The owner record was replaced.
    Owner(Box<Owner>),
}

/// The kinds of restricted write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanKind {
    Update,
    Archive,
    Claim,
    Transfer,
}

/// A write prepared from one read and not yet applied; see [`crate::Relay::focus_commit`].
#[derive(Clone, Debug)]
pub struct WritePlan {
    pub(crate) kind: PlanKind,
    pub(crate) focus: FocusId,
    pub(crate) keys: Vec<String>,
    pub(crate) args: Vec<Vec<u8>>,
    /// The name this write gives the definition (an update), for a `name_taken` refusal.
    pub(crate) new_name: Option<(String, String)>,
    /// The name the target was resolved by, for a `name_moved` refusal.
    pub(crate) resolved_name: Option<(String, String)>,
    pub(crate) applied: Applied,
}

impl WritePlan {
    /// The definition the write is for.
    #[must_use]
    pub fn focus(&self) -> &FocusId {
        &self.focus
    }

    /// What the write changes once applied.
    #[must_use]
    pub fn applied(&self) -> &Applied {
        &self.applied
    }
}

/// The result of planning an update.
#[derive(Clone, Debug)]
pub enum Planned {
    /// A write to commit.
    Write(Box<WritePlan>),
    /// The normalized content equals the current revision: nothing to write.
    Unchanged(Box<Revision>),
}

/// The result of committing a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Committed {
    /// The write was applied in one atomic operation.
    Applied,
    /// A concurrent write changed a part of the record this plan was built from, without
    /// refuting it; nothing was written and the write must be planned again from a new read.
    Retry,
}

/// The result of an update.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateOutcome {
    /// The next revision was written.
    Written(Box<Revision>),
    /// The content equalled the current revision; nothing was written.
    Unchanged(Box<Revision>),
}

/// The result of a creation.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateOutcome {
    /// The new definition's meta.
    pub meta: Meta,
    /// Its owner record.
    pub owner: Owner,
    /// Revision 1.
    pub revision: Revision,
    /// The archived definition removed to make room, if the 256-definition limit required one.
    pub removed: Option<FocusId>,
}

/// A write that kept finding the record changed under it.
pub(crate) fn contended(focus: &FocusId) -> FocusError {
    FocusError::Store(crate::Error::UnexpectedScriptReply {
        script: "focus-write",
        reply: format!("{focus} changed on every one of the attempts"),
    })
}
