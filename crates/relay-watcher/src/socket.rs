//! Delivery through Claude Code's per-session inbox socket (FR-044, AGE-2030).
//!
//! The default (`delivery = "socket"`): a Claude Code session that has an inbox
//! socket receives its mail only through that socket, whatever its pane is doing.
//! Mail the socket did not certainly take stays queued or in flight and is retried
//! or dead-lettered with a reason; it is never typed into the terminal. herdr types
//! only into a lead that has no socket at all: another harness, or a Claude Code
//! session whose own registry record names no socket. `delivery = "herdr"` is the
//! operator's explicit way to switch sockets off.
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
//! - [`deliverer`]: the [`deliverer::SocketDeliverer`] that ties them together.

pub mod deliverer;
pub mod session;
pub mod transcript;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use deliverer::SocketDeliverer;
use relay_core::{PaneId, SessionId, receipts::DeliveryEvidence};

use crate::{
    deliver::{BatchTarget, DeliverError, Deliverer, RegisteredLead, Route, Settlement},
    herdr::{AgentGet, CheckJob, Herdr, Layout, ProcessError},
};

/// Which socket transport the watcher runs with.
#[derive(Clone)]
enum Sockets {
    /// `delivery = "herdr"`: every lead is typed into.
    Disabled,
    /// Sockets are wanted but cannot be used on this host: why. Claude Code leads
    /// wait, since whether they have a socket cannot be told.
    Unavailable(Arc<str>),
    /// The socket deliverer.
    On(Arc<SocketDeliverer>),
}

/// The watcher's delivery: the socket for the leads that have one, herdr for the rest.
#[derive(Clone)]
pub struct Routed<H> {
    herdr: H,
    sockets: Sockets,
}

impl<H> Routed<H> {
    /// The reply socket's path, when the socket is in use and bound.
    #[cfg(test)]
    pub(crate) fn socket_reply_path(&self) -> Option<std::path::PathBuf> {
        match &self.sockets {
            Sockets::On(socket) => socket.reply_path(),
            Sockets::Disabled | Sockets::Unavailable(_) => None,
        }
    }

    /// `herdr` alone: the operator switched sockets off.
    pub fn herdr_only(herdr: H) -> Self {
        Self {
            herdr,
            sockets: Sockets::Disabled,
        }
    }

    /// Sockets through `socket`, herdr for the leads that have none.
    pub fn socket_first(herdr: H, socket: SocketDeliverer) -> Self {
        Self {
            herdr,
            sockets: Sockets::On(Arc::new(socket)),
        }
    }

    /// Sockets cannot be used here (`why`): Claude Code leads are not typed into and
    /// their mail waits; other harnesses go through herdr.
    pub fn socket_unavailable(herdr: H, why: &str) -> Self {
        Self {
            herdr,
            sockets: Sockets::Unavailable(why.into()),
        }
    }

    fn socket(&self) -> Option<&SocketDeliverer> {
        match &self.sockets {
            Sockets::On(socket) => Some(socket),
            Sockets::Disabled | Sockets::Unavailable(_) => None,
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
    fn route(&self, lead: &str, kind: &str, session: Option<&SessionId>) -> Route {
        // The socket is Claude Code's: any other harness has no session record by design,
        // so it goes straight to herdr, leaves no note, and Claude Code's registry is not
        // read for it (FR-054).
        if !relay_core::tries_socket(kind) {
            if let Some(socket) = self.socket() {
                socket.not_tried(lead);
            }
            return Route::Terminal;
        }
        match &self.sockets {
            Sockets::Disabled => Route::Terminal,
            Sockets::Unavailable(why) => Route::Wait(format!("inbox sockets unavailable: {why}")),
            Sockets::On(socket) => socket.route(lead, session),
        }
    }

    fn post(&self, target: &BatchTarget<'_>, text: &str) -> Result<DeliveryEvidence, DeliverError> {
        let Some(socket) = self.socket() else {
            return Err(DeliverError::NotSent("no socket transport".to_owned()));
        };
        if !relay_core::tries_socket(target.kind) {
            return Err(DeliverError::NotSent(
                "only Claude Code has an inbox socket".to_owned(),
            ));
        }
        socket.deliver(target, text).map_err(DeliverError::from)
    }

    /// A bare pane names no session, so it is typed into only with sockets switched off.
    fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), DeliverError> {
        match self.sockets {
            Sockets::Disabled => self.herdr.deliver(pane, text),
            Sockets::Unavailable(_) | Sockets::On(_) => Err(DeliverError::NotSent(
                "nothing is typed without knowing whose pane it is".to_owned(),
            )),
        }
    }

    /// Types a batch, asking the route again immediately before: a socket that appeared
    /// since the caller's checks stops the typing.
    fn deliver_to(
        &self,
        target: &BatchTarget<'_>,
        pane: &PaneId,
        text: &str,
    ) -> Result<DeliveryEvidence, DeliverError> {
        match self.route(target.lead, target.kind, target.session) {
            Route::Terminal => self.herdr.deliver_to(target, pane, text),
            Route::Socket => Err(DeliverError::Deferred(
                "the session has an inbox socket; nothing typed".to_owned(),
            )),
            Route::Wait(why) => Err(DeliverError::Deferred(why)),
        }
    }

    fn registry_read(&self, registered: &[RegisteredLead<'_>]) {
        if let Some(socket) = self.socket() {
            socket.registry_read(registered);
        }
    }

    fn holds(&self, lead: &str, session: Option<&SessionId>) -> bool {
        self.socket().is_some_and(|s| s.holds(lead, session))
    }

    fn rotates(&self) -> bool {
        self.socket().is_some()
    }

    fn alerts(&self) -> Vec<(String, String)> {
        self.socket()
            .map(SocketDeliverer::alerts)
            .unwrap_or_default()
    }

    fn applied(&self, settlement: &Settlement, result: &Result<usize, String>) {
        if let Some(s) = self.socket() {
            s.applied(settlement, result);
        }
    }

    fn settle(&self) -> Vec<Settlement> {
        self.socket()
            .map(SocketDeliverer::settle)
            .unwrap_or_default()
    }
}
