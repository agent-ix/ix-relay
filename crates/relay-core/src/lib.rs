//! Store and message types for the ix-relay lead-to-lead bus.
//!
//! Each team lead has an inbox list in Redis. Senders push onto it; hooks and the
//! watcher pop from it. A pop reads and removes in one command, so whichever
//! consumer gets there first owns the message. A pop moves what it takes into an
//! in-flight record that an acknowledgement deletes and a lease expiry returns to
//! the inbox, so delivery is at-least-once (AGE-2032).

pub mod activity;
mod address;
pub mod assignment;
pub mod config;
pub mod directory;
mod env;
mod error;
mod escalation;
pub mod fleet;
pub mod health;
pub mod hook_skips;
mod ids;
pub mod lane_registry;
mod message;
pub mod moves;
mod name;
pub mod named_focus;
pub mod receipts;
mod resolve;
pub mod socket_status;
pub mod status;
mod store;
#[cfg(test)]
mod test_env;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod text;
pub mod work;
pub mod work_history;

pub use activity::{
    ACTIVITY_LAST_TTL_SECS, ACTIVITY_MAX_AGE_MS, ACTIVITY_MAX_CWD_BYTES, ACTIVITY_MAX_EVENT_BYTES,
    ACTIVITY_MAX_EVENTS, ACTIVITY_MAX_FIELD_BYTES, ACTIVITY_MAX_SKILL_NAME_BYTES,
    ACTIVITY_MAX_SKILL_PAIRS, ACTIVITY_MAX_SKILL_VERSION_BYTES, ActivityEvent, ActivityKind,
    BlockedReason, Harness, LastClass,
};
pub use address::{Address, AddressBook, HerdrPlaceIds, MAX_LABEL_BYTES, Place, duplicated};
pub use env::machine_name;
pub use error::{Candidate, Error, LinkFault, Result, UnknownCause};
pub use escalation::{
    Escalation, EscalationFiling, EscalationId, EscalationList, EscalationStatus, TicketId,
};
pub use ids::{MAX_ID_BYTES, MachineName, PaneId, PaneRef, SessionId, TabId, WorkspaceId};
pub use lane_registry::{FocusRecord, ServerCapacity, ServerCapacityError, focus_is_stale};
pub use message::{CLAUDE_KIND, Message, Registration, tries_socket};
pub use name::{AgentName, Target};
pub use resolve::{resolve_target, resolve_target_scoped};
pub use status::{
    LaneState, MAX_STATUS_AGENTS, MAX_STATUS_ITEM_BYTES, MAX_STATUS_ITEMS, STATUS_REQUEST_TTL_SECS,
    StatusError, StatusItem, StatusReply, StatusRequest, StatusRequestId,
};
pub use store::{
    AgentList, AgentSlot, BACKLOG_SCAN_BOUND, Backlog, BatchId, BudgetPopResult, Buried, Claim,
    CommitOutcome, DEAD_LETTER_CAP, DEFAULT_PREFIX, DIRECTORY_LEASE, DirectoryRead,
    DirectoryRecipient, HERDR_FAILED_TTL_SECS, HOOK_LEASE, Heartbeats, HookHealth, HookHealthEntry,
    INFLIGHT_ROW_OVERHEAD, IdentityCommit, InboxOverview, LaneLists, LeadBacklog, LeadOverview,
    LeaseState, MAX_HEARTBEAT_KEYS, MAX_INFLIGHT_BATCHES, MAX_INFLIGHT_BYTES, MAX_LEASE,
    MAX_MESSAGE_BYTES, MAX_REDELIVERIES, Moves, Now, PanesReplaced, PeekResult, PlaceRefresh,
    PopResult, Popper, Reaped, RecipientProbe, Relay, ReplyOutcome, Salvage, Salvaged, ScanCut,
    SessionProbe, SkippedRegistration, Sweep, WATCHER_LEASE,
};
