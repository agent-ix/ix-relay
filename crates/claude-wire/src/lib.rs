//! Typed wire formats of Claude Code's cross-session messaging, and the drift
//! signals that say when Claude Code changed one.
//!
//! Claude Code's inbox socket, its receipts, its `~/.claude/sessions/<pid>.json`
//! record and the `origin.msg_id` of its transcript are **not documented**
//! upstream and can change in any release. Every one of them is typed here, in
//! one crate with no I/O, so a change on the other side is *detected*: parsing
//! is total and returns a typed drift result ([`ControlLine::Unrecognised`],
//! [`TranscriptMatch::ShapeUnrecognised`], [`RecordVerdict`]) instead of
//! silently absorbing what it does not know.
//!
//! **Provenance.** [`CAPTURED_ON`] is the Claude Code version the formats were
//! measured on. Only some shapes were captured live from a running Claude Code;
//! the rest were read from the installed program's text. Each public item says
//! which, and `fixtures/claude-2.1.283/MANIFEST` marks every fixture file as
//! `captured` or `derived-from-program-text`.
//!
//! - [`envelope`]: what is posted to a session's socket (outgoing);
//! - [`receipt`]: what Claude Code sends back to the reply socket (incoming);
//! - [`record`]: the session registry record and the version gate;
//! - [`transcript`]: the strong acknowledgement, `origin.msg_id`.

pub mod envelope;
pub mod receipt;
pub mod record;
pub mod transcript;

pub use envelope::{Body, Envelope, MAX_LINE_BYTES, wrap};
pub use receipt::{
    ControlLine, DriftReason, Ignored, Receipt, Verdict, classify, parse_control_line,
};
pub use record::{KNOWN_PEER_PROTOCOL, MIN_VERSION, RecordVerdict, SessionRecord, parse_version};
pub use transcript::{TranscriptMatch, scan};

/// The Claude Code version the wire formats in this crate were captured on.
///
/// Formats named "captured" in the docs were observed live on this version;
/// "derived" ones were read from its installed program text. A running Claude
/// Code with another version is *unverified*, not necessarily broken.
pub const CAPTURED_ON: &str = "2.1.283";
