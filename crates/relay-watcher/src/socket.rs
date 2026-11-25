//! Delivery through Claude Code's per-session inbox socket (FR-044, AGE-2030).
//!
//! The default (`delivery = "socket"`): a Claude Code session that has an inbox
//! socket receives its mail only through that socket, whatever its pane is doing.
//! Mail the socket did not certainly take stays queued or in flight and is retried
//! or dead-lettered with a reason; it is never typed into the terminal. herdr types
//! only into a lead that has no socket at all: another harness whose session no record
//! names, or a session whose own registry record shows it has none. The lead's kind never
//! overrides a socket (FR-054). `delivery = "herdr"` is the operator's explicit way to
//! switch sockets off.
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

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use deliverer::SocketDeliverer;
use relay_core::{
    PaneId, SessionId,
    receipts::DeliveryEvidence,
    socket_status::{self, SocketStatus},
    text::sanitise_line,
};

use crate::{
    deliver::{BatchTarget, DeliverError, Deliverer, RegisteredLead, Route, Settlement},
    herdr::{AgentGet, CheckJob, Herdr, Layout, ProcessError},
};

/// Which socket transport the watcher runs with.
#[derive(Clone)]
enum Sockets {
    /// `delivery = "herdr"`: every lead is typed into.
    Disabled,
    /// Sockets are wanted but cannot be used on this host. Leads that have, or may
    /// have, a socket wait.
    Unavailable(Arc<Unavailable>),
    /// The socket deliverer.
    On(Arc<SocketDeliverer>),
}

/// Why sockets cannot be used here, and what can still be told about Claude Code.
struct Unavailable {
    why: String,
    /// Claude Code's session registry, when even its location is known.
    sessions_dir: Option<PathBuf>,
    /// The status file that tells `doctor` why, removed when the watcher exits.
    status: Option<PathBuf>,
}

impl Unavailable {
    /// Writes the status file saying `why` into `status_dir`.
    fn report(why: &str, sessions_dir: Option<PathBuf>, status_dir: Option<&Path>) -> Self {
        let pid = std::process::id();
        let status = status_dir.and_then(|dir| {
            let path = socket_status::path_for(dir, pid);
            let report = SocketStatus {
                pid,
                start_time: session::Procs::start_time(&session::ProcFs, pid),
                updated_at_ms: crate::now_ms(),
                unavailable: Some(sanitise_line(why)),
                ..SocketStatus::default()
            };
            match report.write(&path) {
                Ok(()) => Some(path),
                Err(e) => {
                    eprintln!("relay-watcher: cannot write {}: {e}", path.display());
                    None
                }
            }
        });
        Self {
            why: why.to_owned(),
            sessions_dir,
            status,
        }
    }

    /// Whether `session` may have an inbox socket, whatever its lead's kind says.
    fn may_have_socket(&self, session: &str) -> bool {
        self.sessions_dir
            .as_deref()
            .is_none_or(|dir| session::advertises(dir, session))
    }
}

impl Drop for Unavailable {
    fn drop(&mut self) {
        if let Some(path) = &self.status {
            // Best effort, as for the deliverer's own status file.
            let _ = std::fs::remove_file(path);
        }
    }
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

    /// Sockets cannot be used here (`why`): a Claude Code lead, and any lead whose
    /// session a record in `sessions_dir` names with a socket, is not typed into and
    /// its mail waits; the rest go through herdr. The reason is reported in a status
    /// file in `status_dir`.
    pub fn socket_unavailable(
        herdr: H,
        why: &str,
        sessions_dir: Option<PathBuf>,
        status_dir: Option<&Path>,
    ) -> Self {
        Self {
            herdr,
            sockets: Sockets::Unavailable(Arc::new(Unavailable::report(
                why,
                sessions_dir,
                status_dir,
            ))),
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
    fn session_alive(&self, session: &SessionId) -> bool {
        self.socket().is_some_and(|s| s.session_alive(session))
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
        match &self.sockets {
            Sockets::Disabled => Route::Terminal,
            // The socket belongs to the session, so the kind label alone never decides
            // that a lead may be typed into (FR-054).
            Sockets::Unavailable(u) => {
                if relay_core::tries_socket(kind)
                    || session.is_some_and(|id| u.may_have_socket(id.as_str()))
                {
                    Route::Wait(format!("inbox sockets unavailable: {}", u.why))
                } else {
                    Route::Terminal
                }
            }
            Sockets::On(socket) => socket.route(lead, kind, session),
        }
    }

    fn post(&self, target: &BatchTarget<'_>, text: &str) -> Result<DeliveryEvidence, DeliverError> {
        let Some(socket) = self.socket() else {
            return Err(DeliverError::NotSent("no socket transport".to_owned()));
        };
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

    fn open(&self, lead: &str) -> bool {
        self.socket().is_some_and(|s| s.open(lead))
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
