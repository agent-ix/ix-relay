// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Durable, revisioned named focus definitions with one owner (FR-061).
//!
//! A definition is a library entry: a named, shareable selection of Linear work for one team.
//! It has one stable focus ID, one editable name, immutable numbered revisions and exactly one
//! owner, and it outlives the session that created it. This module owns the stored shapes, the
//! bounds, the session liveness rule and the typed refusals; the Redis operations are methods
//! of [`crate::Relay`] (`focus_*`).
//!
//! Reference resolution, the workspace snapshot and membership belong to the shared ix-board
//! library. Nothing here reads Linear: a revision stores canonical IDs and display labels, and
//! the selection it embeds is the shared ix-board selection, which only the caller's
//! [`SelectionCodec`] encodes and decodes.

mod error;
pub mod limits;
mod liveness;
pub(crate) mod model;
mod plan;

pub use error::{FocusError, FocusErrorCode, FocusResult, SourceReason, UnavailableReason};
pub use liveness::{LivenessEvidence, SessionState};
pub use model::{
    ChangeKind, DecodedSelection, Definition, DefinitionState, FOCUS_ID_PREFIX, FocusContent,
    FocusId, Incarnation, Labels, ListEntry, Member, MemberKind, Meta, NewPin, Owner, OwnerBinding,
    OwnerChange, PinEntry, PinValue, Revision, SelectionCodec, Summary,
};
pub use plan::{
    Applied, Caller, ClaimRequest, Committed, CreateOutcome, Planned, Resolved, UpdateOutcome,
    WritePlan,
};
pub(crate) use plan::{PlanKind, ResolvedName, contended};
