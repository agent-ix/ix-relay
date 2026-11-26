//! How a popped batch reaches a lead's session.
//!
//! [`Deliverer`] is the seam between the watcher's decision (is this lead
//! ready, has it changed under us, what do we do when delivery fails) and the
//! transports. There are two: the session's inbox socket, and `herdr agent prompt`
//! ([`crate::herdr::RealHerdr`]), which types into the terminal. A lead is on exactly
//! one of them at a time ([`Route`]): a Claude Code session with an inbox socket is
//! never typed into, whatever happens to a post. A transport reports only what the
//! decision needs: whether it is certain nothing was sent ([`DeliverError::NotSent`],
//! the batch goes back to the front of the inbox) or not
//! ([`DeliverError::Ambiguous`], it does not, or the batch could be delivered twice).

use std::fmt;

use herdr_cli::ProcessError;
use relay_core::{BatchId, PaneId, SessionId};

/// How a lead's mail may reach it now (FR-044).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The session has no inbox socket at all (its own record names none, another harness
    /// whose session no record names, or `delivery = "herdr"`): typed into its pane,
    /// after the session and readiness checks.
    Terminal,
    /// The session has an inbox socket: posted to it, whatever the pane is doing.
    Socket,
    /// A Claude Code session whose socket cannot take mail now, or whose lack of one
    /// cannot be shown: nothing is popped and nothing is typed. The text says why.
    Wait(String),
}

/// Why a delivery failed, in the only terms the caller acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliverError {
    /// The transport is certain no input reached the session, so the batch can
    /// be put back and tried again.
    NotSent(String),
    /// Input may have reached the session (a timeout, an unrecognised failure).
    /// Retrying could type the batch twice, so it is not retried.
    Ambiguous(String),
    /// Posted through a socket; ownership is retained until consumption is known.
    Pending(String),
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
        | Self::Pending(why)
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
    /// The harness kind the lead registered (`claude`, `codex`, ...).
    pub kind: &'a str,
    /// The in-flight batch being delivered.
    pub batch: BatchId,
    /// The harness session id the lead registered under (for Claude, the
    /// session id its hooks report), `None` when it registered without one.
    pub session: Option<&'a SessionId>,
    /// Who sent it, as the sender addresses read in the rendered text.
    pub senders: &'a str,
}

/// A lead's registration as a tick read it, for a transport that remembers leads between
/// ticks (FR-055).
#[derive(Debug, Clone, Copy)]
pub struct RegisteredLead<'a> {
    /// The lead's bus name.
    pub name: &'a str,
    /// Whether the registration is on this watcher's machine.
    pub here: bool,
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
    /// Typed source cause for a dead-letter decision, when it is known.
    pub dead_letter_reason: Option<relay_core::receipts::DeadLetterReason>,
}

/// What a [`Settlement`] does to its batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleAction {
    /// It was delivered: acknowledge it.
    Ack,
    /// Socket reported delivery without structured transcript confirmation.
    AckUnconfirmed,
    /// It was certainly not delivered: put it back, unmarked.
    Requeue,
    /// The receiver process disappeared; transcript evidence may have been unavailable,
    /// so the message is put back with its redelivery marker.
    Redeliver,
    /// It will never be delivered (the receiver's user denied it, or nobody answered
    /// its approval prompt in time): dead-letter it.
    DeadLetter,
    /// The message is being held at the receiver: extend the batch's lease by this
    /// much, so it is neither returned to the inbox nor popped by another consumer
    /// while it is held.
    Renew(std::time::Duration),
}

/// Gets a batch into a lead's session: typed into its pane, or posted to its socket.
pub trait Deliverer {
    /// Which transport may carry this lead's mail now. Asked before anything is popped.
    /// The default transport only types.
    fn route(&self, _lead: &str, _kind: &str, _session: Option<&SessionId>) -> Route {
        Route::Terminal
    }

    /// Posts a batch to the lead's inbox socket; only for a lead [`Deliverer::route`]
    /// put on [`Route::Socket`]. Never types.
    ///
    /// # Errors
    ///
    /// [`DeliverError`]. The default has no socket, so nothing is sent.
    fn post(
        &self,
        _target: &BatchTarget<'_>,
        _text: &str,
    ) -> Result<relay_core::receipts::DeliveryEvidence, DeliverError> {
        Err(DeliverError::NotSent("no socket transport".to_owned()))
    }

    /// Types one rendered batch into `pane` and submits it.
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
    /// As [`Deliverer::deliver`], and [`DeliverError::Deferred`] when the lead may no
    /// longer be typed into (it is off [`Route::Terminal`]).
    fn deliver_to(
        &self,
        _target: &BatchTarget<'_>,
        pane: &PaneId,
        text: &str,
    ) -> Result<relay_core::receipts::DeliveryEvidence, DeliverError> {
        self.deliver(pane, text)
            .map(|()| relay_core::receipts::DeliveryEvidence::HerdrPrompt)
    }

    /// Told at the start of every tick which leads the registry the tick read holds, so a
    /// transport that remembers leads can forget the ones that are gone (FR-055). The
    /// default remembers none.
    fn registry_read(&self, _registered: &[RegisteredLead<'_>]) {}

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

    /// Whether the transport still has a send to the lead that it has not settled. The
    /// default has none.
    fn open(&self, _lead: &str) -> bool {
        false
    }

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
