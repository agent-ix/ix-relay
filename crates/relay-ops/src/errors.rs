//! `relay-ops`'s one error type. Every condition a caller (`agent-msg`,
//! `relay-tui`) might need to tell apart from another is its own variant —
//! never a boxed `dyn Error`, so a caller can match on it instead of
//! parsing its `Display` text. See `lib.rs`'s module doc comment for why
//! this crate exists at all.

use relay_core::{AgentName, EscalationId, EscalationStatus, TabId};

use crate::{herdr::ProcessError, linear::LinearError};

/// Everything a `relay-ops` command function can fail with.
#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    /// The bus itself refused the operation: an unknown agent, a
    /// not-found or already-resolved escalation, a malformed ticket or
    /// session id, unsafe escalation text, or a Redis-level failure.
    #[error(transparent)]
    Relay(#[from] relay_core::Error),
    /// A herdr call failed (`tab create`, `agent start`, `tab close`) and,
    /// unlike [`OpsError::InvestigateCleanupFailed`], no cleanup was
    /// attempted or needed.
    #[error(transparent)]
    Herdr(#[from] ProcessError),
    /// Posting to Linear failed. Only returned by
    /// [`crate::escalate::run_escalation_post_linear`] — that command's
    /// whole job is the post, so its failure is this crate's error.
    /// `run_resolve`'s own Linear attempt is different: it is not this
    /// crate's error at all, see
    /// [`crate::escalate::ResolveOutcome::linear`].
    #[error(transparent)]
    Linear(#[from] LinearError),
    /// [`crate::escalate::run_investigate`]'s `herdr agent start` failed,
    /// and the tab `tab_create` had just opened also failed to close.
    /// Both failures are named here so the cleanup failure is never
    /// silently dropped in favor of only the original one. Both errors are
    /// boxed: this variant, unboxed, would roughly triple `OpsError`'s
    /// size (two [`ProcessError`]s plus a `String`) for every caller's
    /// `Result<T, OpsError>`, for what is deliberately a rare, doubled
    /// failure.
    #[error("{start}; also failed to clean up tab {tab_id}: {cleanup}")]
    InvestigateCleanupFailed {
        /// Why starting the agent failed.
        start: Box<ProcessError>,
        /// The dangling tab that could not be closed either.
        tab_id: TabId,
        /// Why closing it failed.
        cleanup: Box<ProcessError>,
    },
    /// The escalating lead's own record exists but this binary cannot read it (an
    /// id an older binary stored is outside its type's rule).
    #[error(
        "{agent}'s registration is unreadable here (an id in it is outside the rules of this version): \
         run `agent-msg register` to write it again, then escalate"
    )]
    RegistrationUnreadable {
        /// The lead whose record cannot be read.
        agent: AgentName,
    },
    /// The escalating lead has never registered on the bus.
    #[error("{agent} is not registered on the bus (see `agent-msg who`)")]
    NotRegistered {
        /// The unregistered lead.
        agent: AgentName,
    },
    /// The lead's registration carries no session id, so `investigate`
    /// would have nothing to fork later.
    #[error("{agent} has no recorded session id; investigate needs one to fork a session later")]
    NoSessionId {
        /// The lead whose registration lacks a session id.
        agent: AgentName,
    },
    /// `post-linear` on an escalation that has no decision yet.
    #[error("escalation {id} has no decision yet (status {status}); nothing to post")]
    NoDecisionYet {
        /// The escalation asked about.
        id: EscalationId,
        /// Its current status.
        status: EscalationStatus,
    },
    /// `investigate` on an escalation that already has its decision.
    #[error("escalation {id} is already resolved; nothing to investigate")]
    NothingToInvestigate {
        /// The escalation asked about.
        id: EscalationId,
    },
    /// `investigate` on an escalation whose registration recorded no cwd.
    #[error("escalation {id} has no recorded cwd; investigate can't open a session")]
    NoRecordedCwd {
        /// The escalation asked about.
        id: EscalationId,
    },
    /// A fork `investigate` on a lead whose harness is not `claude`.
    #[error(
        "investigate can only fork a claude session; escalation {id}'s lead is a {kind:?} \
         session — use --fresh instead"
    )]
    ForkNeedsClaude {
        /// The escalation asked about.
        id: EscalationId,
        /// The lead's harness kind.
        kind: String,
    },
    /// `--human` did not parse as an agent name.
    #[error("--human {human:?}: {reason}")]
    InvalidHuman {
        /// The text supplied.
        human: String,
        /// Why it is not a valid name. Rendered in the message, so not also
        /// a `source()` (chain printers would show it twice).
        reason: Box<relay_core::Error>,
    },
    /// The lead notification for a decision would exceed the message cap.
    #[error(
        "decision too long: the notification to {lead} would be {bytes} bytes (max {}); \
         shorten it and try again",
        relay_core::MAX_MESSAGE_BYTES
    )]
    DecisionTooLong {
        /// The lead the notification is for.
        lead: AgentName,
        /// The rendered notification's size.
        bytes: usize,
    },
}

/// `Result` with [`OpsError`].
pub type Result<T> = std::result::Result<T, OpsError>;
