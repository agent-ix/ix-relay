//! How a popped batch reaches a lead's session.
//!
//! [`Deliverer`] is the seam between the watcher's decision (is this lead
//! ready, has it changed under us, what do we do when delivery fails) and the
//! transport that types the text in. `herdr agent prompt` ([`crate::herdr::RealHerdr`])
//! is the only implementation today; a Claude Code inbox-socket transport is
//! meant to be a second one, so a transport reports only what the decision
//! needs: whether it is certain nothing was sent ([`DeliverError::NotSent`],
//! the batch goes back to the front of the inbox) or not
//! ([`DeliverError::Ambiguous`], it does not, or the batch could be typed twice).

use std::fmt;

use herdr_cli::ProcessError;
use relay_core::{BatchId, PaneId, SessionId};

/// Why a delivery failed, in the only terms the caller acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliverError {
    /// The transport is certain no input reached the session, so the batch can
    /// be put back and tried again.
    NotSent(String),
    /// Input may have reached the session (a timeout, an unrecognised failure).
    /// Retrying could type the batch twice, so it is not retried.
    Ambiguous(String),
    /// The session's Claude Code holds the batch behind an approval prompt (a
    /// bypass-mode receiver without `crossSessionInbound: "accept"`). It is in
    /// the session's hold queue, so it is neither requeued nor sent again by
    /// another transport (which would type it twice and leave the prompt up):
    /// the batch stays in flight, and the operator is told once per episode (FR-044).
    Held(String),
    /// The transport chose not to try this batch now (mail for a held lead): it
    /// was not sent, and no other transport may send it either. It goes back to
    /// the front of the inbox unmarked, so it is not a redelivery and is never
    /// dead-lettered for waiting (FR-044).
    Deferred(String),
    /// The transport did not post the batch this tick because its time on sockets is
    /// spent. It is put back unmarked, and nothing is said about the lead: no notification,
    /// no episode.
    Postponed(String),
    /// The receiver's user denied the message (they clicked Deny): it will never be
    /// delivered. The batch is dead-lettered, and neither requeued nor typed by
    /// another transport.
    Denied(String),
}

impl fmt::Display for DeliverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Self::NotSent(why)
        | Self::Ambiguous(why)
        | Self::Held(why)
        | Self::Deferred(why)
        | Self::Postponed(why)
        | Self::Denied(why)) = self;
        f.write_str(why)
    }
}

impl From<ProcessError> for DeliverError {
    fn from(error: ProcessError) -> Self {
        if error.definitely_not_sent() {
            Self::NotSent(error.to_string())
        } else {
            Self::Ambiguous(error.to_string())
        }
    }
}

/// Who a batch is for and where it came from, for a transport that addresses the
/// session rather than a pane.
#[derive(Debug, Clone, Copy)]
pub struct BatchTarget<'a> {
    /// The lead's bus name.
    pub lead: &'a str,
    /// The harness kind the lead registered (`claude`, `codex`, ...): the socket is tried
    /// only for a Claude Code lead (FR-054).
    pub kind: &'a str,
    /// The in-flight batch being delivered.
    pub batch: BatchId,
    /// The harness session id the lead registered under (for Claude, the
    /// session id its hooks report), `None` when it registered without one.
    pub session: Option<&'a SessionId>,
    /// Who sent it, as the sender addresses read in the rendered text.
    pub senders: &'a str,
}

/// What a transport learned, after a delivery returned, about a batch it could
/// not settle then (a receipt that arrived minutes late).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    /// The lead whose batch it is.
    pub lead: String,
    /// The batch.
    pub batch: BatchId,
    /// What to do with it.
    pub action: SettleAction,
    /// Why, for the log.
    pub why: String,
    /// A desktop notification to show (title, body), when the operator must be told.
    pub notice: Option<(String, String)>,
}

/// What a [`Settlement`] does to its batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleAction {
    /// It was delivered: acknowledge it.
    Ack,
    /// It was certainly not delivered: put it back, unmarked.
    Requeue,
    /// It will never be delivered (the receiver's user denied it, or nobody answered
    /// its approval prompt in time): dead-letter it.
    DeadLetter,
    /// The message is being held at the receiver: extend the batch's lease by this
    /// much, so it is neither returned to the inbox nor popped by another consumer
    /// while it is held.
    Renew(std::time::Duration),
}

/// Types `text` into the session in `pane` and submits it.
pub trait Deliverer {
    /// Delivers one rendered batch.
    ///
    /// # Errors
    ///
    /// [`DeliverError::NotSent`] only when the transport is certain nothing was
    /// delivered; anything else is [`DeliverError::Ambiguous`].
    fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), DeliverError>;

    /// [`Deliverer::deliver`] for a transport that also needs to know whom the
    /// batch is for. The default is the pane transport, which does not.
    ///
    /// # Errors
    ///
    /// As [`Deliverer::deliver`], and [`DeliverError::Held`] for a transport that
    /// can tell a batch is being held for approval.
    fn deliver_to(
        &self,
        _target: &BatchTarget<'_>,
        pane: &PaneId,
        text: &str,
    ) -> Result<(), DeliverError> {
        self.deliver(pane, text)
    }

    /// Whether this transport is holding mail for the lead and wants none popped
    /// for it now. Asked before a batch is popped, so mail that will not be
    /// tried stays in the inbox instead of in flight. The default never holds.
    fn holds(&self, _lead: &str, _session: Option<&SessionId>) -> bool {
        false
    }

    /// The transport is told what became of a [`Settlement`] it returned: the write
    /// succeeded (with how many entries it moved), or failed and why. It forgets a
    /// decision only once the write is confirmed, and returns it again until then.
    /// The default ignores it.
    fn applied(&self, _settlement: &Settlement, _result: &Result<usize, String>) {}

    /// Whether the transport's time per tick is limited, so the watcher looks at the
    /// leads in a rotating order. The default is no.
    fn rotates(&self) -> bool {
        false
    }

    /// Notifications the transport wants shown (title, body), taken once: a settlement it
    /// cannot write, say. The default has none.
    fn alerts(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// What the transport has settled since it was last asked (late receipts).
    /// The default has nothing to settle.
    fn settle(&self) -> Vec<Settlement> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tracing: TC-013-06
    /// ACs: FR-013-AC-6
    ///
    /// A herdr failure is `NotSent` exactly when herdr documents that no input
    /// was sent, and the text survives for the log line.
    #[test]
    fn a_herdr_error_is_not_sent_only_when_herdr_documents_no_input_was_sent() {
        let rejected = ProcessError::Reported {
            code: "agent_blocked".into(),
            message: "blocked".into(),
        };
        let text = rejected.to_string();
        assert_eq!(
            DeliverError::from(rejected),
            DeliverError::NotSent(text.clone())
        );
        let stalled = ProcessError::Timeout {
            program: "herdr".into(),
            secs: 10,
        };
        assert!(matches!(
            DeliverError::from(stalled),
            DeliverError::Ambiguous(_)
        ));
        assert_eq!(DeliverError::NotSent(text.clone()).to_string(), text);
    }
}
