//! Delivery through Claude Code's per-session inbox socket (FR-044, AGE-2030).
//!
//! Opt-in (`[watcher] delivery = "socket"`): the watcher posts a batch to the
//! Unix socket Claude Code binds for the lead's session instead of typing it in
//! through herdr. herdr stays the fallback for every batch the socket certainly
//! did not take, so delivery is never worse than without it.
//!
//! **The wire format is not documented by Claude Code.** It was measured on
//! 2.1.283 and lives in `claude-wire`; a release may change it, which is why a session
//! is used only when its registry record reports a known `peerProtocol` and version
//! ([`session`]) and why what does not parse is counted, not dropped.
//!
//! - [`session`]: finds the live socket for a session id in Claude Code's
//!   read-only registry;
//! - `claude-wire` (its own crate): the line posted, the receipts read back, the
//!   session record and the transcript origin, typed, with drift signals;
//! - [`transcript`]: the strong acknowledgement, the message id in the
//!   lead's transcript;
//! - [`deliverer`]: the [`crate::deliver::Deliverer`] that ties them together.

pub mod deliverer;
pub mod session;
pub mod transcript;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use relay_core::PaneId;

use crate::{
    deliver::{BatchTarget, DeliverError, Deliverer, RegisteredLead, Settlement},
    herdr::{AgentGet, CheckJob, Herdr, Layout, ProcessError},
};
use deliverer::SocketDeliverer;

/// The watcher's delivery: `herdr` alone, or the socket first with herdr behind
/// it. With no socket (the default, `DeliveryMode::Herdr`) this is `herdr`
/// exactly: every call is passed straight through.
#[derive(Clone)]
pub struct Routed<H> {
    herdr: H,
    socket: Option<Arc<SocketDeliverer>>,
}

impl<H> Routed<H> {
    /// The reply socket's path, when the socket is in use and bound.
    #[cfg(test)]
    pub(crate) fn socket_reply_path(&self) -> Option<std::path::PathBuf> {
        self.socket.as_ref().and_then(|s| s.reply_path())
    }

    /// `herdr` alone.
    pub fn herdr_only(herdr: H) -> Self {
        Self {
            herdr,
            socket: None,
        }
    }

    /// `herdr` behind `socket`.
    pub fn socket_first(herdr: H, socket: SocketDeliverer) -> Self {
        Self {
            herdr,
            socket: Some(Arc::new(socket)),
        }
    }
}

impl<H: Herdr> Herdr for Routed<H> {
    fn directory_observations(&self) -> bool {
        self.herdr.directory_observations()
    }
    fn get(&self, pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
        self.herdr.get(pane)
    }
    fn notify(&self, title: &str, body: &str) -> Result<(), ProcessError> {
        self.herdr.notify(title, body)
    }
    fn layout(&self, machine: &relay_core::MachineName) -> Result<Layout, ProcessError> {
        self.herdr.layout(machine)
    }
    fn check(&self) -> relay_ops::herdr_check::HerdrCheck {
        self.herdr.check()
    }
    fn check_job(&self) -> CheckJob {
        self.herdr.check_job()
    }
    fn take_decode_failure(&self) -> bool {
        self.herdr.take_decode_failure()
    }
    fn dropped_entries(&self) -> usize {
        self.herdr.dropped_entries()
    }
}

impl<H: Deliverer> Deliverer for Routed<H> {
    fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), DeliverError> {
        self.herdr.deliver(pane, text)
    }

    fn deliver_to(
        &self,
        target: &BatchTarget<'_>,
        pane: &PaneId,
        text: &str,
    ) -> Result<(), DeliverError> {
        let Some(socket) = &self.socket else {
            return self.herdr.deliver_to(target, pane, text);
        };
        // The socket is Claude Code's: any other harness has no session record by design,
        // so it goes straight to herdr, leaves no note, and Claude Code's registry is not
        // read for it (FR-054).
        if !relay_core::tries_socket(target.kind) {
            socket.not_tried(target.lead);
            return self.herdr.deliver_to(target, pane, text);
        }
        match socket.deliver(target, text) {
            Ok(()) => Ok(()),
            Err(deliverer::SocketError::NotSent(why)) => {
                eprintln!(
                    "relay-watcher: {}: inbox socket not used ({why}); typing it in through herdr",
                    target.lead
                );
                self.herdr.deliver_to(target, pane, text)
            }
            Err(other) => Err(other.into()),
        }
    }

    fn registry_read(&self, registered: &[RegisteredLead<'_>]) {
        if let Some(socket) = &self.socket {
            socket.registry_read(registered);
        }
    }

    fn holds(&self, lead: &str, session: Option<&relay_core::SessionId>) -> bool {
        self.socket.as_ref().is_some_and(|s| s.holds(lead, session))
    }

    fn rotates(&self) -> bool {
        self.socket.is_some()
    }

    fn alerts(&self) -> Vec<(String, String)> {
        self.socket.as_ref().map(|s| s.alerts()).unwrap_or_default()
    }

    fn applied(&self, settlement: &Settlement, result: &Result<usize, String>) {
        if let Some(s) = &self.socket {
            s.applied(settlement, result);
        }
    }

    fn settle(&self) -> Vec<Settlement> {
        self.socket.as_ref().map(|s| s.settle()).unwrap_or_default()
    }
}
