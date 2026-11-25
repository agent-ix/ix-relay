// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The ordered per-lane assignment list (FR-072): the plan an orchestrator writes,
//! kept apart from the work a lane reports about itself (FR-066).

use serde::{Deserialize, Serialize};

use crate::Address;

/// Most tickets one lane's list holds.
pub const MAX_ASSIGNED_TICKETS: usize = 50;
/// Longest ticket id, in bytes.
pub const MAX_TICKET_ID_BYTES: usize = 64;
/// Longest text reference, in bytes.
pub const MAX_TICKET_REF_BYTES: usize = 512;
/// Longest order id, in bytes.
pub const MAX_ORDER_ID_BYTES: usize = 64;
/// Longest writer name, in bytes.
pub const MAX_WRITER_BYTES: usize = 128;
/// Most lanes that hold a list at once.
pub const MAX_ASSIGNED_LANES: usize = 4_096;
/// Transition history count bound (FR-066).
pub const ASSIGN_EVENTS_MAX: usize = 4_096;
/// Transition history age bound, in milliseconds (FR-066: seven days).
pub const ASSIGN_EVENTS_MAX_AGE_MS: i64 = 604_800_000;

/// One assigned ticket: its id and the text that locates the ticket (a link or an
/// identifier; the relay never fetches it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignedTicket {
    /// The ticket id.
    pub id: String,
    /// The text reference.
    #[serde(rename = "ref")]
    pub reference: String,
}

/// One lane's list as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssignmentList {
    /// The lane address `server:workspace:tab`.
    pub lane: String,
    /// Count of writes the list has taken, moves included.
    pub version: u64,
    /// Relay server time of the last write, Unix milliseconds.
    pub written_at_ms: i64,
    /// The tickets, in order.
    pub tickets: Vec<AssignedTicket>,
}

/// One change to a lane's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignWrite {
    /// Replace the list.
    Set(Vec<AssignedTicket>),
    /// Add tickets after the last one.
    Append(Vec<AssignedTicket>),
    /// Remove the ticket with this id.
    Remove(String),
    /// Put the tickets in this order: a permutation of the ids held.
    Reorder(Vec<String>),
}

/// Who writes: the guard of FR-072 against accidental writes. It is a check, not
/// authentication; every participant that reaches the bus is trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignWriter {
    /// Recorded in each transition.
    pub name: String,
    /// Whether the caller holds the orchestrator role of its configuration.
    pub orchestrator: bool,
    /// The lane of the calling session, when it is one: a lane never writes its own list.
    pub caller_lane: Option<Address>,
}

/// One transition of a list, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignTransition {
    /// The stream cursor of the record; the history's order.
    #[serde(default)]
    pub id: String,
    /// `set`, `append`, `remove`, `reorder`, `move` or `move_failed`.
    pub op: String,
    /// The lane address the list has after the write.
    pub lane: String,
    /// The writer's name.
    pub writer: String,
    /// The list version after the write.
    pub version: u64,
    /// The ticket ids after the write, in order.
    pub tickets: Vec<String>,
    /// The order that carried the write, when it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    /// For a move, the address the list had before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// For a `move_failed`, why the list stayed under `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Relay server time, Unix milliseconds.
    pub at_ms: i64,
}

/// A bounded page of transitions with its coverage, read as FR-066 reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssignEventPage {
    /// Wire version.
    pub version: u32,
    /// Retained lower bound, Unix milliseconds.
    pub available_from_ms: Option<i64>,
    /// Relay server time of the read.
    pub observed_at_ms: i64,
    /// The cursor preceded retained data.
    pub truncated: bool,
    /// Exclusive cursor for the next page.
    pub next_cursor: String,
    /// More transitions follow.
    pub has_more: bool,
    /// The transitions, oldest first.
    pub events: Vec<AssignTransition>,
}

/// A host whose stale lanes were written to: the probe defect of FR-070 that FR-072
/// raises. It ends when the host publishes its lane list again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeDefect {
    /// The stale host.
    pub host: String,
    /// Refused writes since the defect began.
    pub refused: u64,
    /// Relay server time of the latest refusal.
    pub last_ms: i64,
}

/// A list that did not follow its lane to the lane's new address, and why. It stands until a
/// write to the lane at `to` or a successful move of that pair, and never expires while the
/// list is still under `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveDefect {
    /// The address the list is still under.
    pub from: String,
    /// The address the lane has now.
    pub to: String,
    /// `target_occupied`, `unreadable_storage` or the store error.
    pub reason: String,
    /// Relay server time, Unix milliseconds.
    pub at_ms: i64,
}

/// What [`crate::Relay::assignment_follow`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowOutcome {
    /// The list now sits under the new address.
    Moved,
    /// The old address held no list.
    NoList,
    /// The two addresses are the same text.
    Unchanged,
    /// The list stayed where it was, for this reason. A `move_failed` transition and a
    /// [`MoveDefect`] are recorded when the store could take them.
    Failed(String),
}

/// Why a write or read of an assignment list was refused. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssignError {
    /// The caller does not hold the orchestrator role.
    #[error(
        "not_orchestrator: set `[assign] orchestrator = true` in config.toml to write assignment lists"
    )]
    NotOrchestrator,
    /// A lane tried to write its own list.
    #[error("own_lane: a lane does not write its own assignment list")]
    OwnLane,
    /// The address is not a lane address `server:workspace:tab`.
    #[error("invalid_lane: expected server:workspace:tab")]
    InvalidLane,
    /// The lane is in no registry the relay holds.
    #[error("unknown_lane: {0} is not in the fleet registry")]
    UnknownLane(String),
    /// The lane's host has an expired registry lease.
    #[error("host_registry_stale: the registry lease of the host of {0} has expired")]
    HostRegistryStale(String),
    /// A list holds at most 50 tickets.
    #[error("too_many_tickets: a list holds at most 50 tickets")]
    TooManyTickets,
    /// A field is empty, unsafe or over its bound.
    #[error("invalid_field: {0}")]
    InvalidField(&'static str),
    /// A ticket id appears twice.
    #[error("duplicate_ticket: {0}")]
    DuplicateTicket(String),
    /// `remove` named a ticket the list does not hold.
    #[error("no_such_ticket: {0}")]
    NoSuchTicket(String),
    /// `reorder` did not name each held ticket exactly once.
    #[error("reorder_mismatch: name each ticket of the list exactly once")]
    ReorderMismatch,
    /// The relay holds the most lists it keeps.
    #[error("lane_limit: the relay holds the most assignment lists it keeps")]
    LaneLimit,
    /// A key of the assignment lists holds the wrong kind of value.
    #[error("unreadable_storage: an assignment key holds the wrong type; nothing was written")]
    UnreadableStorage,
}

fn text(value: &str, field: &'static str, max: usize) -> Result<(), AssignError> {
    if value.trim().is_empty() || value.len() > max || value != crate::text::sanitise_line(value) {
        return Err(AssignError::InvalidField(field));
    }
    Ok(())
}

impl AssignedTicket {
    pub(crate) fn validate(&self) -> Result<(), AssignError> {
        text(&self.id, "ticket id", MAX_TICKET_ID_BYTES)?;
        text(&self.reference, "ticket ref", MAX_TICKET_REF_BYTES)
    }
}

pub(crate) fn validate_ids<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), AssignError> {
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        text(id, "ticket id", MAX_TICKET_ID_BYTES)?;
        if !seen.insert(id) {
            return Err(AssignError::DuplicateTicket(id.to_owned()));
        }
    }
    if seen.len() > MAX_ASSIGNED_TICKETS {
        return Err(AssignError::TooManyTickets);
    }
    Ok(())
}

impl AssignWrite {
    pub(crate) fn op(&self) -> &'static str {
        match self {
            Self::Set(_) => "set",
            Self::Append(_) => "append",
            Self::Remove(_) => "remove",
            Self::Reorder(_) => "reorder",
        }
    }

    pub(crate) fn validate(&self) -> Result<(), AssignError> {
        match self {
            Self::Set(tickets) | Self::Append(tickets) => {
                tickets.iter().try_for_each(AssignedTicket::validate)?;
                validate_ids(tickets.iter().map(|t| t.id.as_str()))
            }
            Self::Remove(id) => text(id, "ticket id", MAX_TICKET_ID_BYTES),
            Self::Reorder(ids) => validate_ids(ids.iter().map(String::as_str)),
        }
    }
}

impl AssignWriter {
    /// The writer for a caller: the role from the `[assign]` table of `config`, the
    /// caller's own lane when its session is one, and a name that says which.
    ///
    /// # Errors
    /// The `[assign] orchestrator` value is not a boolean.
    pub fn from_config(
        config: &crate::config::AssignFile,
        machine: &crate::MachineName,
        caller_lane: Option<Address>,
    ) -> Result<Self, String> {
        let orchestrator = config.is_orchestrator()?;
        Ok(Self {
            name: caller_lane
                .as_ref()
                .map_or_else(|| format!("orchestrator@{machine}"), ToString::to_string),
            orchestrator,
            caller_lane,
        })
    }

    pub(crate) fn validate(&self, lane: &Address) -> Result<(), AssignError> {
        if !self.orchestrator {
            return Err(AssignError::NotOrchestrator);
        }
        if self.caller_lane.as_ref().is_some_and(|own| own == lane) {
            return Err(AssignError::OwnLane);
        }
        text(&self.name, "writer", MAX_WRITER_BYTES)
    }
}

/// The lane as stored: a full `server:workspace:tab` address, nothing more.
///
/// # Errors
/// [`AssignError::InvalidLane`] when `lane` names no tab or names a pane.
pub fn lane_address(lane: &str) -> Result<Address, AssignError> {
    let address: Address = lane.parse().map_err(|_| AssignError::InvalidLane)?;
    if address.tab().is_none() || address.pane().is_some() {
        return Err(AssignError::InvalidLane);
    }
    Ok(address)
}

pub(crate) fn validate_order_id(order_id: Option<&str>) -> Result<(), AssignError> {
    order_id.map_or(Ok(()), |id| text(id, "order id", MAX_ORDER_ID_BYTES))
}
