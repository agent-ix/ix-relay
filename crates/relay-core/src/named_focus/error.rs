// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The typed refusals of the named-focus contract (FR-061 "Command contract").

use std::fmt;

use super::model::FocusId;

/// The stable error codes of the whole named-focus contract, including the two that
/// session activation (FR-062) returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusErrorCode {
    /// The expected or stated revision is not the latest.
    RevisionConflict,
    /// The expected owner epoch is not current.
    OwnerEpochConflict,
    /// The verified caller is not the stored owner.
    NotOwner,
    /// A non-archived definition of the team has the name.
    NameTaken,
    /// The definition is archived.
    Archived,
    /// A bound is reached.
    Limit,
    /// A reference matched no or several objects, or the wrong team.
    UnresolvedReference,
    /// The snapshot is unusable.
    SourceUnavailable,
    /// A stored definition cannot be used.
    UnavailableDefinition,
    /// No such definition.
    NotFound,
    /// The focus is already active at another revision (FR-062).
    AlreadyActive,
    /// The focus is not in the session's active set (FR-062).
    NotActive,
    /// A malformed input, a name where an ID is required, or a file member the relay
    /// does not define.
    InvalidArgument,
}

impl FocusErrorCode {
    /// The code as printed in a typed error object.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RevisionConflict => "revision_conflict",
            Self::OwnerEpochConflict => "owner_epoch_conflict",
            Self::NotOwner => "not_owner",
            Self::NameTaken => "name_taken",
            Self::Archived => "archived",
            Self::Limit => "limit",
            Self::UnresolvedReference => "unresolved_reference",
            Self::SourceUnavailable => "source_unavailable",
            Self::UnavailableDefinition => "unavailable_definition",
            Self::NotFound => "not_found",
            Self::AlreadyActive => "already_active",
            Self::NotActive => "not_active",
            Self::InvalidArgument => "invalid_argument",
        }
    }
}

/// Why a stored definition cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    /// The record, or a field of it, does not decode.
    Undecodable,
    /// A revision embeds a selection whose `schema_version` the pinned ix-board does not read.
    UnsupportedSchema,
}

impl UnavailableReason {
    /// The reason as printed.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Undecodable => "undecodable",
            Self::UnsupportedSchema => "unsupported_schema",
        }
    }
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why the workspace snapshot cannot validate a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceReason {
    /// No snapshot could be obtained: nothing is stored and no credential is ready.
    Missing,
    /// The snapshot is older than the five-minute bound.
    Stale,
    /// The snapshot is incomplete.
    Partial,
    /// The read failed.
    Failed,
    /// The overall deadline of the read expired.
    Timeout,
}

impl SourceReason {
    /// The reason as printed.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Stale => "stale",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
        }
    }
}

impl fmt::Display for SourceReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Everything a named-focus operation can refuse with.
#[derive(Debug, thiserror::Error)]
pub enum FocusError {
    /// The expected revision is not the latest; nothing was written.
    #[error(
        "revision conflict on {focus}: the latest revision is {latest}; reread before retrying"
    )]
    RevisionConflict {
        /// The definition.
        focus: FocusId,
        /// The latest revision at the atomic comparison.
        latest: u64,
    },
    /// The expected owner epoch (or, for a claim, the observed owner) is not current.
    #[error("owner epoch conflict on {focus}: the current owner is {owner_id} at epoch {epoch}")]
    OwnerEpochConflict {
        /// The definition.
        focus: FocusId,
        /// The stored owner at the atomic comparison.
        owner_id: String,
        /// The stored epoch at the atomic comparison.
        epoch: u64,
    },
    /// The verified caller is not the stored owner, or may not claim from a live or unknown
    /// owner without `--force`.
    #[error("{detail}")]
    NotOwner {
        /// The definition.
        focus: FocusId,
        /// The stored owner.
        owner_id: String,
        /// The stored epoch.
        epoch: u64,
        /// Why the caller was refused.
        detail: String,
    },
    /// A non-archived definition of the team already has the name.
    #[error("the name {name:?} is already used by a non-archived definition of team {team}")]
    NameTaken {
        /// The team.
        team: String,
        /// The name.
        name: String,
        /// The definition holding it, when known.
        holder: Option<FocusId>,
    },
    /// The definition is archived.
    #[error("definition {focus} is archived")]
    Archived {
        /// The definition.
        focus: FocusId,
    },
    /// A bound is reached.
    #[error("limit reached: {bound} (at most {max})")]
    Limit {
        /// The bound, named as in FR-061's table.
        bound: &'static str,
        /// Its value.
        max: u64,
    },
    /// A reference matched no or several objects, or the wrong team.
    #[error("unresolved reference {reference:?}: {message}")]
    UnresolvedReference {
        /// The reference as written.
        reference: String,
        /// What was wrong with it.
        message: String,
    },
    /// The snapshot is unusable.
    #[error("snapshot source {origin} is unusable ({reason}): {message}")]
    SourceUnavailable {
        /// Why.
        reason: SourceReason,
        /// What was read.
        origin: String,
        /// The underlying failure.
        message: String,
    },
    /// A stored definition cannot be used.
    #[error("definition {focus} is unavailable: {reason}")]
    UnavailableDefinition {
        /// The definition.
        focus: FocusId,
        /// Why.
        reason: UnavailableReason,
    },
    /// No definition has the ID or name.
    #[error("no definition {focus:?}")]
    NotFound {
        /// The ID or name as given.
        focus: String,
    },
    /// A malformed input.
    #[error("invalid {field}: {message}")]
    InvalidArgument {
        /// The field or flag.
        field: String,
        /// What was wrong with it.
        message: String,
    },
    /// The operating system gave no random bits for a focus ID.
    #[error("no source of randomness for a focus ID: {0}")]
    Random(String),
    /// Redis failed or answered something unexpected.
    #[error(transparent)]
    Store(#[from] crate::Error),
}

impl FocusError {
    /// The stable code of this refusal. A store or randomness failure is reported as an
    /// unusable source, since the contract has no other code for it.
    #[must_use]
    pub fn code(&self) -> FocusErrorCode {
        match self {
            Self::RevisionConflict { .. } => FocusErrorCode::RevisionConflict,
            Self::OwnerEpochConflict { .. } => FocusErrorCode::OwnerEpochConflict,
            Self::NotOwner { .. } => FocusErrorCode::NotOwner,
            Self::NameTaken { .. } => FocusErrorCode::NameTaken,
            Self::Archived { .. } => FocusErrorCode::Archived,
            Self::Limit { .. } => FocusErrorCode::Limit,
            Self::UnresolvedReference { .. } => FocusErrorCode::UnresolvedReference,
            Self::SourceUnavailable { .. } | Self::Random(_) | Self::Store(_) => {
                FocusErrorCode::SourceUnavailable
            }
            Self::UnavailableDefinition { .. } => FocusErrorCode::UnavailableDefinition,
            Self::NotFound { .. } => FocusErrorCode::NotFound,
            Self::InvalidArgument { .. } => FocusErrorCode::InvalidArgument,
        }
    }

    /// An [`FocusError::InvalidArgument`] for `field`.
    pub fn invalid(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            field: field.into(),
            message: message.into(),
        }
    }

    /// A [`FocusError::Limit`] for the bound `bound`.
    #[must_use]
    pub fn limit(bound: &'static str, max: usize) -> Self {
        Self::Limit {
            bound,
            max: u64::try_from(max).unwrap_or(u64::MAX),
        }
    }
}

/// `Result` with [`FocusError`].
pub type FocusResult<T> = std::result::Result<T, FocusError>;
