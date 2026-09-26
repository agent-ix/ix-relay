//! Store and message types for the ix-relay lead-to-lead bus.
//!
//! Each team lead has an inbox list in Redis. Senders push onto it; hooks and the
//! watcher pop from it. A pop reads and removes in one command, so whichever
//! consumer gets there first owns the message. A pop moves what it takes into an
//! in-flight record that an acknowledgement deletes and a lease expiry returns to
//! the inbox, so delivery is at-least-once (AGE-2032).

mod address;
pub mod config;
mod env;
mod error;
mod escalation;
pub mod health;
mod ids;
mod message;
pub mod moves;
mod name;
pub mod naming;
mod resolve;
pub mod socket_status;
mod store;
#[cfg(test)]
mod test_env;
pub mod text;

pub use address::{Address, AddressBook, MAX_LABEL_BYTES, Place, duplicated};
pub use env::machine_name;
pub use error::{Candidate, Error, LinkFault, Result, UnknownCause};
pub use escalation::{
    Escalation, EscalationFiling, EscalationId, EscalationList, EscalationStatus, TicketId,
};
pub use ids::{MAX_ID_BYTES, MachineName, PaneId, PaneRef, SessionId, TabId, WorkspaceId};
pub use message::{Message, Registration};
pub use name::{AgentName, Target};
pub use resolve::resolve_target;
pub use store::{
    AgentList, AgentSlot, BACKLOG_SCAN_BOUND, Backlog, BatchId, BudgetPopResult, Claim,
    CommitOutcome, DEAD_LETTER_CAP, DEFAULT_PREFIX, Demotion, HERDR_FAILED_TTL_SECS, HOOK_LEASE,
    Heartbeats, HookHealth, HookHealthEntry, INFLIGHT_ROW_OVERHEAD, IdentityCommit, InboxOverview,
    LeadBacklog, LeadOverview, LeaseState, MAX_HEARTBEAT_KEYS, MAX_INFLIGHT_BATCHES,
    MAX_INFLIGHT_BYTES, MAX_LEASE, MAX_MESSAGE_BYTES, MAX_REDELIVERIES, Moves, Now, PanesReplaced,
    PlaceRefresh, PopResult, Popper, Reaped, RecipientProbe, RefusedDemotion, Relay, Salvage,
    Salvaged, ScanCut, SessionProbe, SkippedRegistration, Sweep, UNRESOLVED_TTL_SECS,
    WATCHER_LEASE,
};
