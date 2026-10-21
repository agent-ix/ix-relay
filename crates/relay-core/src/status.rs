//! Structured status requests and replies used by repeatable program reviews.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{AgentName, Error};

/// Status request retention window: seven days.
pub const STATUS_REQUEST_TTL_SECS: u64 = 7 * 24 * 60 * 60;
/// Maximum requested responder count.
pub const MAX_STATUS_AGENTS: usize = 128;
/// Maximum UTF-8 bytes in each reported item.
pub const MAX_STATUS_ITEM_BYTES: usize = 512;
/// Maximum number of reported items in each list.
pub const MAX_STATUS_ITEMS: usize = 64;

/// A safe identifier for one request, suitable for use as a Redis key segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StatusRequestId(String);
impl StatusRequestId {
    /// The identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for StatusRequestId {
    type Err = StatusError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = (1..=64).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(StatusError::InvalidRequestId(s.to_owned()))
        }
    }
}
impl TryFrom<String> for StatusRequestId {
    type Error = StatusError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<StatusRequestId> for String {
    fn from(value: StatusRequestId) -> Self {
        value.0
    }
}
impl fmt::Display for StatusRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A fixed-form snapshot request. `created_at_ms` is assigned by the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusRequest {
    /// Request identifier.
    pub id: StatusRequestId,
    /// Lead that opened the request.
    pub requester: AgentName,
    /// Exact set of leads permitted to submit a response.
    pub expected_agents: Vec<AgentName>,
    /// Store-assigned Unix timestamp in milliseconds.
    pub created_at_ms: u64,
}

/// Lane activity reported by a lead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneState {
    /// The lane is working on assigned tasks.
    Active,
    /// The lane has no current active task.
    Idle,
    /// The lane is waiting on an identified blocker.
    Blocked,
}

/// One sanitized item such as a ticket, PR, blocker, or decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StatusItem(String);
impl StatusItem {
    /// Validates and stores a bounded plain-text item.
    pub fn new(value: impl Into<String>) -> Result<Self, StatusError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.len() > MAX_STATUS_ITEM_BYTES
            || value.chars().any(crate::text::is_unsafe_char)
        {
            return Err(StatusError::InvalidItem);
        }
        Ok(Self(value))
    }
    /// Item text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for StatusItem {
    type Error = StatusError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<StatusItem> for String {
    fn from(v: StatusItem) -> Self {
        v.0
    }
}

/// A lead's fixed-form response to a status request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusReply {
    /// Request this response answers.
    pub request_id: StatusRequestId,
    /// Responding lead; must be one of the request's expected agents.
    pub agent: AgentName,
    /// Tickets in progress or recently completed.
    pub tickets: Vec<StatusItem>,
    /// Relevant pull requests.
    pub pull_requests: Vec<StatusItem>,
    /// Blocking Linear ticket IDs.
    pub blocker_ticket_ids: Vec<StatusItem>,
    /// Current lane activity.
    pub lane_state: LaneState,
    /// Number of additional coder slots available.
    pub spare_coder_slots: u16,
    /// Decisions required from the project owner.
    pub owner_decisions: Vec<StatusItem>,
    /// Assigned by the store on receipt, never trusted from input.
    #[serde(default)]
    pub submitted_at_ms: u64,
}
impl StatusReply {
    /// Validates bounds and ensures callers cannot choose a submission time.
    pub fn validate(&self) -> Result<(), StatusError> {
        for (field, values) in [
            ("tickets", &self.tickets),
            ("pull_requests", &self.pull_requests),
            ("blocker_ticket_ids", &self.blocker_ticket_ids),
            ("owner_decisions", &self.owner_decisions),
        ] {
            if values.len() > MAX_STATUS_ITEMS {
                return Err(StatusError::TooManyItems(field));
            }
        }
        for ticket in &self.blocker_ticket_ids {
            let id = ticket.as_str();
            if !valid_ticket_id(id) {
                return Err(StatusError::InvalidTicketId(id.to_owned()));
            }
        }
        Ok(())
    }
    /// Clear caller-supplied timestamp before persistence; the store fills it in.
    pub fn with_server_timestamp(mut self, now_ms: u64) -> Self {
        self.submitted_at_ms = now_ms;
        self
    }
}

fn valid_ticket_id(value: &str) -> bool {
    let Some((team, number)) = value.split_once('-') else {
        return false;
    };
    !team.is_empty()
        && team.bytes().all(|b| b.is_ascii_uppercase())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

/// Validation and storage errors for status records.
#[derive(Debug, thiserror::Error)]
pub enum StatusError {
    /// Bad status request identifier.
    #[error("invalid status request id {0:?}: expected 1-64 ASCII letters, digits, '-' or '_'")]
    InvalidRequestId(String),
    /// Item was empty, too long, or contains unsafe control/bidi characters.
    #[error(
        "status item is empty, exceeds {MAX_STATUS_ITEM_BYTES} bytes, or contains unsafe characters"
    )]
    InvalidItem,
    /// A repeated field exceeded its item limit.
    #[error("too many items in {0}; maximum is {MAX_STATUS_ITEMS}")]
    TooManyItems(&'static str),
    /// A blocker was not a Linear ticket key.
    #[error("invalid blocker ticket id {0:?}")]
    InvalidTicketId(String),
    /// Request has too many agents or repeated agent names.
    #[error("invalid status request responder set")]
    InvalidResponders,
    /// Agent did not belong to the immutable expected responder set.
    #[error("agent {0} was not requested for this status request")]
    UnrequestedAgent(AgentName),
    /// The request id is already in use.
    #[error("status request {0} already exists")]
    RequestExists(StatusRequestId),
    /// The request does not exist or has expired.
    #[error("status request {0} was not found or has expired")]
    RequestNotFound(StatusRequestId),
    /// System clock could not produce a nonnegative Unix timestamp.
    #[error("system clock is before the Unix epoch or timestamp overflowed")]
    Clock,
    /// Redis returned a value outside the status persistence protocol.
    #[error("unexpected status storage result {0}")]
    UnexpectedStoreResult(i64),
    /// Record decoding failed.
    #[error("invalid stored status record: {0}")]
    Decode(#[from] serde_json::Error),
    /// Redis operation failed.
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
    /// Agent name parse failed.
    #[error(transparent)]
    AgentName(#[from] Error),
}

/// Checks a complete request before the store persists it.
pub(crate) fn validate_request(
    requester: &AgentName,
    agents: &[AgentName],
) -> Result<(), StatusError> {
    if agents.is_empty() || agents.len() > MAX_STATUS_AGENTS || agents.contains(requester) {
        return Err(StatusError::InvalidResponders);
    }
    let mut sorted = agents.iter().map(AgentName::as_str).collect::<Vec<_>>();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() != agents.len() {
        return Err(StatusError::InvalidResponders);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_ids_are_key_safe_and_bounded() {
        for id in ["r1", "2026_10-abc"] {
            assert!(id.parse::<StatusRequestId>().is_ok());
        }
        for id in ["", "bad:id", "../x", &"x".repeat(65)] {
            assert!(id.parse::<StatusRequestId>().is_err());
        }
    }
    /// Trace: FR-056-AC-3
    #[test]
    fn items_reject_unsafe_or_oversized_text() {
        assert!(StatusItem::new("SWM-102 in progress").is_ok());
        assert!(StatusItem::new("x".repeat(MAX_STATUS_ITEM_BYTES)).is_ok());
        assert!(StatusItem::new("\u{1b}[31mred").is_err());
        assert!(StatusItem::new("x".repeat(MAX_STATUS_ITEM_BYTES + 1)).is_err());
        assert!(StatusItem::new("  ").is_err());
    }
    /// Trace: FR-056-AC-3
    #[test]
    fn reply_enforces_bounded_lists_and_ticket_keys() {
        let agent = "lead".parse().unwrap();
        let mut reply = StatusReply {
            request_id: "r1".parse().unwrap(),
            agent,
            tickets: vec![],
            pull_requests: vec![],
            blocker_ticket_ids: vec![StatusItem::new("SWM-107").unwrap()],
            lane_state: LaneState::Active,
            spare_coder_slots: 0,
            owner_decisions: vec![],
            submitted_at_ms: 0,
        };
        assert!(reply.validate().is_ok());
        reply.blocker_ticket_ids[0] = StatusItem::new("not a ticket").unwrap();
        assert!(reply.validate().is_err());
        reply.blocker_ticket_ids[0] = StatusItem::new("SWM-107").unwrap();
        reply.tickets = vec![StatusItem::new("SWM-107").unwrap(); MAX_STATUS_ITEMS];
        assert!(reply.validate().is_ok());
        reply.tickets.push(StatusItem::new("SWM-107").unwrap());
        assert!(reply.validate().is_err());
    }
    #[test]
    fn responder_set_is_unique_and_excludes_requester() {
        let requester = "lead".parse().unwrap();
        assert!(validate_request(&requester, &["other".parse().unwrap()]).is_ok());
        assert!(validate_request(&requester, std::slice::from_ref(&requester)).is_err());
        assert!(
            validate_request(
                &requester,
                &["other".parse().unwrap(), "other".parse().unwrap()]
            )
            .is_err()
        );
    }

    /// Trace: FR-056-AC-1
    #[test]
    fn request_recipient_count_is_bounded() {
        let requester = "requester".parse().unwrap();
        let agents = (0..MAX_STATUS_AGENTS)
            .map(|index| format!("lane-{index}").parse().unwrap())
            .collect::<Vec<AgentName>>();
        assert!(validate_request(&requester, &agents).is_ok());
        let mut too_many = agents;
        too_many.push("lane-over-limit".parse().unwrap());
        assert!(validate_request(&requester, &too_many).is_err());
    }
}
