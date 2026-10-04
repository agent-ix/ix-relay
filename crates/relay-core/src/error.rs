//! The one error type `relay-core` returns.

use std::fmt;

use crate::{
    address::Address,
    ids::{TabId, WorkspaceId},
    name::AgentName,
};

/// Everything that can go wrong talking to the bus.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Work reporting validation failed.
    #[error(transparent)]
    Work(#[from] crate::work::WorkError),
    /// An opaque storage identity or actor value was malformed.
    #[error("invalid storage identity {name:?}")]
    InvalidName {
        /// The rejected input.
        name: String,
    },
    /// A message was addressed to a lead that has never registered.
    #[error("unknown agent {0}: it has not registered on the bus (see `agent-msg who`)")]
    UnknownAgent(AgentName),
    /// Text was not an exact public tab address.
    #[error("invalid address {input:?}: expected server:workspace:tab, e.g. draco:ix-relay:code")]
    InvalidAddress {
        /// The rejected input.
        input: String,
    },
    /// Atomic enqueue rejected a changed or expired directory; nothing written.
    #[error("recipient directory changed or expired before enqueue; nothing queued")]
    DirectoryChanged,
    /// Enrollment had no genuine session and verified placement.
    #[error("unverified enrollment: {reason}")]
    UnverifiedEnrollment {
        /// Why the session proof was rejected.
        reason: String,
    },
    /// Relative addressing needs the sender's verified Herdr location.
    #[error(
        "relative address {input:?} needs a verified Herdr location for this session; use a full server:workspace:tab address or enable the session hooks"
    )]
    RelativeAddressNeedsLocation {
        /// The relative target.
        input: String,
    },
    /// A target matched more than one lead. Never resolved by guessing: the
    /// operator places each lead in a separate tab.
    #[error("{input:?} matches {} leads, so nothing was sent: {}", .candidates.len(), list_candidates(.candidates))]
    AmbiguousTarget {
        /// What the sender typed.
        input: String,
        /// Every lead it matched, at least two.
        candidates: Vec<Candidate>,
    },
    /// A target matched no lead.
    #[error("{}", unknown_target_message(.input, .cause, .suggestions))]
    UnknownTarget {
        /// What the sender typed.
        input: String,
        /// The most likely reason nothing matched.
        cause: UnknownCause,
        /// Close canonical addresses, best first; may be empty.
        suggestions: Vec<String>,
    },
    /// A message's text was larger than [`crate::MAX_MESSAGE_BYTES`] allows.
    #[error("message too large: {len} bytes (max {max}); shorten it and send again")]
    MessageTooLarge {
        /// The rejected text's length, in bytes.
        len: usize,
        /// The limit it exceeded.
        max: usize,
    },
    /// A message's text contained a character that must never reach a
    /// terminal or a signed relay line (see [`crate::text::is_unsafe_char`]):
    /// ESC, CR, another control character, a line separator or a bidi control.
    /// Refused at send time so nothing is queued.
    #[error(
        "message text contains an unsafe character (U+{codepoint:04X}) at byte {byte_offset}; only plain text, newlines and tabs are allowed"
    )]
    UnsafeMessageText {
        /// The rejected character's Unicode codepoint.
        codepoint: u32,
        /// Its byte offset within the text.
        byte_offset: usize,
    },
    /// Redis refused a command or the connection failed.
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
    /// A Lua script returned something its caller does not understand: a
    /// bug in this crate or a Redis that runs a different script under the
    /// same hash.
    #[error("redis script {script} returned an unexpected reply: {reply}")]
    UnexpectedScriptReply {
        /// Which script.
        script: &'static str,
        /// What it returned, debug-formatted.
        reply: String,
    },
    /// One of a lead's inbox, in-flight, dead-letter or batch-counter keys holds
    /// a value of the wrong type. The operation changed nothing (every in-flight
    /// script checks its keys before it writes), so no message was lost.
    #[error("the in-flight keys of {lead} hold a value of the wrong type; nothing was changed")]
    InflightUnreadable {
        /// The lead whose keys could not be used.
        lead: AgentName,
    },
    /// A key a registration move writes (the registry, the moves stream or the
    /// lead's replaced-pane marker) holds a value of the wrong type. The move
    /// changed nothing: the script checks every key before its first write.
    #[error(
        "a key of the registration move of {lead} holds a value of the wrong type; nothing was changed"
    )]
    MoveUnreadable {
        /// The lead whose registration was to move.
        lead: AgentName,
    },
    /// A stored record did not decode; someone wrote to our keys by hand.
    #[error("corrupt record in {key}: {source}")]
    Decode {
        /// The Redis key holding the bad record.
        key: String,
        /// Why it failed to decode.
        source: serde_json::Error,
    },
    /// A ticket id didn't look like a Linear issue key, e.g. `AGE-2014`.
    #[error("invalid ticket id {ticket:?}: expected a Linear key like AGE-2014")]
    InvalidTicket {
        /// The rejected input.
        ticket: String,
    },
    /// A stored escalation's `status` field held something other than
    /// `waiting`/`investigating`/`resolved`.
    #[error("invalid escalation status {status:?}")]
    InvalidEscalationStatus {
        /// The unrecognized value.
        status: String,
    },
    /// Text was not an escalation id (`e` and digits, e.g. `e7`).
    #[error(
        "invalid escalation id {id:?}: expected e and digits, like e7 (see `agent-msg escalations`)"
    )]
    InvalidEscalationId {
        /// The rejected input.
        id: String,
    },
    /// No escalation exists with that id.
    #[error("no escalation {0}: check `agent-msg escalations`")]
    EscalationNotFound(String),
    /// The escalation was already resolved; `resolve` or `investigate` on it
    /// again is refused. Resolution is terminal.
    #[error("escalation {0} is already resolved")]
    EscalationAlreadyResolved(String),
    /// `resolve` was given an empty or whitespace-only decision. Refused because a resolved
    /// escalation always records a non-empty decision (FR-024-AC-2): an empty
    /// one is stored as no decision at all, which would leave an escalation
    /// resolved with nothing for `post-linear` to post.
    #[error("cannot resolve escalation {id} with an empty decision (empty or only whitespace)")]
    EmptyDecision {
        /// The escalation that was to be resolved.
        id: String,
    },
    /// A stored escalation's hash was missing a required field, or one held
    /// an unparsable value — someone wrote to `esc:<id>` by hand.
    #[error("corrupt escalation {id}: {reason}")]
    CorruptEscalation {
        /// The escalation id.
        id: String,
        /// What was wrong.
        reason: String,
    },
    /// A session id, pane id or machine name broke its shape rule (see
    /// [`crate::SessionId`]): empty, over-long, or carrying a control character.
    #[error(
        "invalid {kind} {value:?}: must be 1-256 bytes with no control or invisible characters"
    )]
    InvalidId {
        /// Which id: `"session id"`, `"pane id"` or `"machine name"`.
        kind: &'static str,
        /// The rejected input.
        value: String,
    },
    /// A lead's registered `session_id` doesn't look like a harness session
    /// id. Refused at `escalate` time rather than later, because it's
    /// eventually passed to `claude --resume <sid>` by `investigate`.
    #[error(
        "invalid session id {session_id:?}: expected 1-64 characters of ASCII letters, digits or '-'"
    )]
    InvalidSessionId {
        /// The rejected input.
        session_id: String,
    },
    /// `escalate`'s `context`, an `option`, or `recommend` text contained a
    /// character [`crate::text::is_unsafe_char`] rejects: a control character
    /// other than `\n`/`\t` (including ESC, which can trigger terminal escape
    /// sequences such as bracketed-paste), DEL, a C1 control, a line separator
    /// or a bidi control. Refused at store time so this text can never reach a
    /// pane or shell unescaped later.
    #[error(
        "{field} contains an unsafe character (U+{codepoint:04X}) at byte {byte_offset}; only plain text, newlines and tabs are allowed"
    )]
    UnsafeEscalationText {
        /// Which field was rejected: `context`, `option`, or `recommendation`.
        field: &'static str,
        /// The rejected character's Unicode codepoint.
        codepoint: u32,
        /// Its byte offset within the field.
        byte_offset: usize,
    },
}

impl Error {
    /// Whether this error means the Redis connection itself can no longer be
    /// trusted, so the holder must drop it and reconnect rather than reuse it.
    ///
    /// True for a Redis read or write that timed out (`EAGAIN` from a socket
    /// timeout included): the blip a read-only command may retry on a fresh
    /// connection. A refused or reset connection is not one.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Redis(e) if e.is_timeout())
    }

    /// Whether a system call was interrupted by a signal (`EINTR`, "Interrupted
    /// system call") while this connection read or wrote. redis-rs does not
    /// retry it. A process that spawns children on another thread gets it when a
    /// child's `SIGCHLD` wakes the thread waiting on the Redis socket; the call
    /// itself did nothing wrong, so a caller whose operation is a read may repeat
    /// it on a fresh connection.
    #[must_use]
    pub fn is_interrupted(&self) -> bool {
        self.io_kind() == Some(std::io::ErrorKind::Interrupted)
    }

    /// The `io::ErrorKind` under a Redis I/O failure, if this is one.
    fn io_kind(&self) -> Option<std::io::ErrorKind> {
        use std::{error::Error as _, io, sync::Arc};

        let Self::Redis(e) = self else {
            return None;
        };
        // redis-rs 1.7 keeps an I/O failure as an `Arc<dyn Error + Send + Sync>`
        // and hands that `Arc` out as its `source()`, so the `io::Error` is one
        // downcast further in. The unit tests below pin this against a real
        // `RedisError`: a redis-rs that changes the shape fails them.
        let cause = e.source()?;
        cause
            .downcast_ref::<Arc<dyn std::error::Error + Send + Sync>>()
            .and_then(|inner| inner.downcast_ref::<io::Error>())
            .or_else(|| cause.downcast_ref::<io::Error>())
            .map(io::Error::kind)
    }

    /// Why a connection to the bus failed, in the terms an operator fixes it
    /// in: the credentials, nothing listening, a backend that dropped the
    /// connection, a link that carries nothing, or no route to the bus at all.
    #[must_use]
    pub fn link_fault(&self) -> LinkFault {
        use std::io::ErrorKind as K;

        // RESP2's AUTH path gives `AuthenticationFailed`; a URL with no password
        // (`NOAUTH`) and RESP3's `HELLO` (`WRONGPASS`) come back as server codes.
        if let Self::Redis(e) = self
            && (e.kind() == redis::ErrorKind::AuthenticationFailed
                || matches!(e.code(), Some("NOAUTH" | "WRONGPASS")))
        {
            return LinkFault::Auth;
        }
        match self.io_kind() {
            Some(K::ConnectionRefused) => LinkFault::Refused,
            // Something accepted, then hung up: the hub's forwarder always accepts,
            // so this is relay-redis behind it failing.
            Some(K::ConnectionReset | K::ConnectionAborted | K::UnexpectedEof | K::BrokenPipe) => {
                LinkFault::Dropped
            }
            Some(K::TimedOut | K::WouldBlock) => LinkFault::TimedOut,
            Some(K::HostUnreachable | K::NetworkUnreachable | K::AddrNotAvailable) => {
                LinkFault::NoRoute
            }
            _ => LinkFault::Other,
        }
    }

    /// True for an I/O failure (refused, reset, broken pipe, and a read or
    /// write timeout) and for a reply the client could not parse or an
    /// authentication failure (redis-rs's own "connection must be replaced"
    /// verdict, `RedisError::is_unrecoverable_error`).
    ///
    /// Reconnecting after any I/O error is a conservative choice, not a
    /// necessity: redis-rs 1.7's blocking connection skips a late reply after a
    /// read timeout and shuts the socket on EOF, so reuse would often work. But
    /// after an I/O error the caller cannot tell whether a request was sent,
    /// half-sent or answered, and a fresh connection costs one round trip.
    ///
    /// False for everything a caller can survive on the same connection: a
    /// server-reported command error, a script reply this crate did not
    /// expect, a corrupt stored record, and every validation refusal.
    #[must_use]
    pub fn is_connection(&self) -> bool {
        match self {
            Self::Redis(e) => e.is_io_error() || e.is_unrecoverable_error(),
            Self::Work(_)
            | Self::InvalidName { .. }
            | Self::UnverifiedEnrollment { .. }
            | Self::DirectoryChanged
            | Self::UnknownAgent(_)
            | Self::InvalidAddress { .. }
            | Self::RelativeAddressNeedsLocation { .. }
            | Self::AmbiguousTarget { .. }
            | Self::UnknownTarget { .. }
            | Self::MessageTooLarge { .. }
            | Self::UnsafeMessageText { .. }
            | Self::UnexpectedScriptReply { .. }
            | Self::InflightUnreadable { .. }
            | Self::MoveUnreadable { .. }
            | Self::Decode { .. }
            | Self::InvalidTicket { .. }
            | Self::InvalidEscalationStatus { .. }
            | Self::InvalidEscalationId { .. }
            | Self::EscalationNotFound(_)
            | Self::EscalationAlreadyResolved(_)
            | Self::EmptyDecision { .. }
            | Self::CorruptEscalation { .. }
            | Self::InvalidSessionId { .. }
            | Self::InvalidId { .. }
            | Self::UnsafeEscalationText { .. } => false,
        }
    }
}

/// One lead an ambiguous target matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Its bus name.
    pub name: AgentName,
    /// Its full address (tab included), when it registered a place.
    pub address: Option<Address>,
    /// Stable Herdr workspace identity when the mapping is available.
    pub workspace_id: Option<WorkspaceId>,
    /// Stable Herdr tab identity when the mapping is available.
    pub tab_id: Option<TabId>,
}

impl fmt::Display for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.address, &self.workspace_id, &self.tab_id) {
            (Some(address), Some(workspace), Some(tab)) => {
                write!(
                    f,
                    "{address} ({}, workspace {workspace}, tab {tab})",
                    self.name
                )
            }
            (Some(address), _, _) => write!(f, "{address} ({})", self.name),
            (None, _, _) => write!(f, "{}", self.name),
        }
    }
}

fn list_candidates(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Why an [`Error::UnknownTarget`] matched nothing — the most specific true
/// statement the registry supports, so the sender knows what to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownCause {
    /// No verified lead at the requested address.
    NotRegistered,
    /// An address whose server has no registered lead.
    UnknownServer {
        /// The server as typed.
        server: String,
        /// Servers that do have leads.
        known: Vec<String>,
    },
    /// The server has leads, but none in that workspace.
    UnknownWorkspace {
        /// The server.
        server: String,
        /// Workspaces on it that have a lead.
        available: Vec<String>,
    },
    /// The workspace has leads, but none in that tab.
    UnknownTab {
        /// The workspace.
        workspace: String,
        /// Tabs of it that have a lead.
        available: Vec<String>,
    },
}

fn unknown_target_message(input: &str, cause: &UnknownCause, suggestions: &[String]) -> String {
    let why = match cause {
        UnknownCause::NotRegistered => "no verified lead is registered at that address".to_owned(),
        UnknownCause::UnknownServer { server, known } => {
            format!(
                "no lead is registered on server {server:?} (servers with leads: {})",
                known.join(", ")
            )
        }
        UnknownCause::UnknownWorkspace { server, available } if available.is_empty() => {
            format!("no lead on {server} has recorded its workspace yet; address it by bus name")
        }
        UnknownCause::UnknownWorkspace { server, available } => {
            format!(
                "no workspace with a lead on {server} matches (workspaces: {})",
                available.join(", ")
            )
        }
        UnknownCause::UnknownTab {
            workspace,
            available,
        } => {
            format!(
                "workspace {workspace} has no matching tab with a lead (tabs: {})",
                available.join(", ")
            )
        }
    };
    let mut message = format!(
        "no agent matches {input:?}: {why}. A pane that never started a lead session is not on the bus (`agent-msg find` lists live panes)"
    );
    if !suggestions.is_empty() {
        message.push_str(&format!("; did you mean {}?", suggestions.join(", ")));
    }
    message
}

/// `Result` with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// What kept a connection from reaching the bus ([`Error::link_fault`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFault {
    /// The bus refused this machine's user or password.
    Auth,
    /// The address answered, but nothing listens there.
    Refused,
    /// Something accepted the connection, then dropped it before Redis
    /// answered: a forwarder whose backend is down.
    Dropped,
    /// Nothing came back in time: the link carries nothing, or Redis hangs.
    TimedOut,
    /// This machine has no route to the bus address.
    NoRoute,
    /// Anything else: a malformed URL, a protocol error, an unexpected reply.
    Other,
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    fn io_error(kind: io::ErrorKind) -> Error {
        Error::Redis(redis::RedisError::from(io::Error::new(kind, "test")))
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: unit test; the row's method is fault-injection.
    #[test]
    fn io_failures_including_timeouts_are_connection_errors() {
        for kind in [
            io::ErrorKind::TimedOut,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::UnexpectedEof,
        ] {
            assert!(io_error(kind).is_connection(), "{kind:?}");
        }
    }

    /// Only a signal interruption is an interruption: a timeout, a reset and a
    /// server-reported error are not.
    #[test]
    fn only_an_interrupted_call_is_interrupted() {
        assert!(io_error(io::ErrorKind::Interrupted).is_interrupted());
        for kind in [
            io::ErrorKind::TimedOut,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::ConnectionReset,
        ] {
            assert!(!io_error(kind).is_interrupted(), "{kind:?}");
        }
        assert!(!Error::UnknownAgent("x".parse().unwrap()).is_interrupted());
    }

    /// Tracing: TC-029-13
    /// ACs: FR-029-AC-13
    #[test]
    fn tc_029_13_a_link_fault_is_told_apart_by_what_the_connect_hit() {
        let auth = Error::Redis(redis::RedisError::from((
            redis::ErrorKind::AuthenticationFailed,
            "Password authentication failed",
        )));
        assert_eq!(auth.link_fault(), LinkFault::Auth);
        for (kind, fault) in [
            (io::ErrorKind::ConnectionRefused, LinkFault::Refused),
            (io::ErrorKind::TimedOut, LinkFault::TimedOut),
            (io::ErrorKind::WouldBlock, LinkFault::TimedOut),
            (io::ErrorKind::HostUnreachable, LinkFault::NoRoute),
            (io::ErrorKind::NetworkUnreachable, LinkFault::NoRoute),
            (io::ErrorKind::AddrNotAvailable, LinkFault::NoRoute),
            (io::ErrorKind::ConnectionReset, LinkFault::Dropped),
            (io::ErrorKind::UnexpectedEof, LinkFault::Dropped),
            (io::ErrorKind::BrokenPipe, LinkFault::Dropped),
            (io::ErrorKind::PermissionDenied, LinkFault::Other),
        ] {
            assert_eq!(io_error(kind).link_fault(), fault, "{kind:?}");
        }
        assert_eq!(
            Error::UnknownAgent("x".parse().unwrap()).link_fault(),
            LinkFault::Other
        );
    }

    /// Connects to a one-shot server that answers the first command with
    /// `reply` (or hangs up, for `None`) and returns what connect-and-PING hit.
    fn against(reply: Option<&'static [u8]>) -> Error {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            if let Some(reply) = reply {
                let mut buf = [0u8; 256];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(reply);
            }
        });
        let result = crate::Relay::connect(
            &format!("redis://127.0.0.1:{port}"),
            std::time::Duration::from_secs(2),
        )
        .and_then(|mut relay| relay.ping());
        server.join().unwrap();
        result.expect_err("the fake server never answers PONG")
    }

    /// Tracing: TC-029-13
    /// ACs: FR-029-AC-13
    ///
    /// Real replies, not constructed errors: a URL with no password gets
    /// `NOAUTH`, RESP3's `HELLO` gets `WRONGPASS`, and the hub's forwarder,
    /// which always accepts, hangs up when relay-redis behind it is down.
    #[test]
    fn tc_029_13_real_auth_replies_and_a_dropped_connection_are_told_apart() {
        let noauth = against(Some(b"-NOAUTH Authentication required.\r\n"));
        assert_eq!(noauth.link_fault(), LinkFault::Auth, "{noauth}");
        let wrongpass = against(Some(
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n",
        ));
        assert_eq!(wrongpass.link_fault(), LinkFault::Auth, "{wrongpass}");
        let dropped = against(None);
        assert_eq!(dropped.link_fault(), LinkFault::Dropped, "{dropped}");
    }

    /// Tracing: TC-029-13
    /// ACs: FR-029-AC-13
    ///
    /// A real refused connect, not a constructed one: nothing listens on a
    /// port this test just bound and released.
    #[test]
    fn a_real_refused_connect_is_refused() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = crate::Relay::connect(
            &format!("redis://127.0.0.1:{port}"),
            std::time::Duration::from_secs(1),
        )
        .map(|_| ())
        .expect_err("nothing listens there");
        assert_eq!(err.link_fault(), LinkFault::Refused, "{err}");
    }

    /// Only a timeout (or `EAGAIN`) is a timeout: a refused connection, a
    /// reset and a server-reported error are not.
    #[test]
    fn only_a_timeout_or_eagain_is_a_timeout() {
        for kind in [io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock] {
            assert!(io_error(kind).is_timeout(), "{kind:?}");
        }
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::BrokenPipe,
        ] {
            assert!(!io_error(kind).is_timeout(), "{kind:?}");
        }
        assert!(!Error::UnknownAgent("x".parse().unwrap()).is_timeout());
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: unit test; the row's method is fault-injection.
    #[test]
    fn an_unparsable_reply_is_a_connection_error() {
        let parse = Error::Redis(redis::RedisError::from((
            redis::ErrorKind::Parse,
            "garbled reply",
        )));
        assert!(parse.is_connection());
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: unit test; the row's method is fault-injection.
    #[test]
    fn data_errors_leave_the_connection_usable() {
        let type_mismatch = Error::Redis(redis::RedisError::from((
            redis::ErrorKind::UnexpectedReturnType,
            "wrong type",
        )));
        assert!(!type_mismatch.is_connection());
        assert!(
            !Error::UnexpectedScriptReply {
                script: "s",
                reply: "r".into()
            }
            .is_connection()
        );
        assert!(!Error::InvalidTicket { ticket: "x".into() }.is_connection());
        assert!(!Error::EscalationNotFound("e1".into()).is_connection());
    }

    #[test]
    fn a_malformed_url_is_refused_without_connecting() {
        assert!(crate::Relay::validate_url("redis://127.0.0.1:6379").is_ok());
        let err = crate::Relay::validate_url("definitely not a url").unwrap_err();
        assert!(
            !err.is_connection(),
            "a config error is not an outage: {err}"
        );
    }
}
