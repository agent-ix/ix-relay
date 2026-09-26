//! Escalations: a team lead asking the operator for a decision.
//!
//! A lead escalates with a ticket, some context, options and a recommendation.
//! The operator investigates (forking or starting a fresh session on the lead's
//! machine) and resolves it with a decision, which goes back to the lead over
//! the bus and is posted to the Linear ticket.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    error::Error,
    ids::{MachineName, SessionId},
    name::AgentName,
};

/// A Linear issue key, e.g. `AGE-2014`: an uppercase team prefix, a dash, and
/// a digit sequence. Validated on the way in so a typo fails at `escalate`
/// time rather than surfacing as a broken link when `resolve` tries to post
/// to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TicketId(String);

impl TicketId {
    /// The ticket id as a string slice, e.g. `"AGE-2014"`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TicketId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let mut parts = s.splitn(2, '-');
        let prefix = parts.next().unwrap_or("");
        let number = parts.next();
        let valid = !prefix.is_empty()
            && prefix.chars().all(|c| c.is_ascii_uppercase())
            && number.is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(Error::InvalidTicket {
                ticket: s.to_owned(),
            })
        }
    }
}

impl TryFrom<String> for TicketId {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        s.parse()
    }
}

impl From<TicketId> for String {
    fn from(ticket: TicketId) -> Self {
        ticket.0
    }
}

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An escalation's id: `e` followed by decimal digits (`e7`), exactly what
/// [`crate::Relay::escalate`] generates from its counter.
///
/// Validated on the way in because the id becomes part of a Redis key
/// (`esc:<id>`): an unchecked string like `seq` or `index` would alias the
/// counter and the index that live under the same prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EscalationId(String);

/// Longest digit run an [`EscalationId`] accepts: `u64::MAX` has 20 digits,
/// and the store's counter is a `u64`.
const MAX_ESCALATION_DIGITS: usize = 20;

impl EscalationId {
    /// The id the store generates for counter value `n`.
    pub(crate) fn from_counter(n: u64) -> Self {
        Self(format!("e{n}"))
    }

    /// The id as a string slice, e.g. `"e7"`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for EscalationId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let digits = s.strip_prefix('e').unwrap_or("");
        let valid = !digits.is_empty()
            && digits.len() <= MAX_ESCALATION_DIGITS
            && digits.bytes().all(|b| b.is_ascii_digit());
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(Error::InvalidEscalationId { id: s.to_owned() })
        }
    }
}

impl TryFrom<String> for EscalationId {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        s.parse()
    }
}

impl From<EscalationId> for String {
    fn from(id: EscalationId) -> Self {
        id.0
    }
}

impl PartialEq<&str> for EscalationId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl fmt::Display for EscalationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where an escalation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EscalationStatus {
    /// Filed; nobody has started looking at it yet.
    Waiting,
    /// Someone (normally the operator, via `investigate`) has a session open on it.
    Investigating,
    /// A decision was recorded. Terminal: neither `investigate` nor `resolve`
    /// may be called on it again.
    Resolved,
}

impl EscalationStatus {
    /// The lowercase name stored in Redis and printed on the CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Investigating => "investigating",
            Self::Resolved => "resolved",
        }
    }
}

impl FromStr for EscalationStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "waiting" => Ok(Self::Waiting),
            "investigating" => Ok(Self::Investigating),
            "resolved" => Ok(Self::Resolved),
            _ => Err(Error::InvalidEscalationStatus {
                status: s.to_owned(),
            }),
        }
    }
}

impl fmt::Display for EscalationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One escalation: a decision a lead is asking the operator to make.
///
/// The immutable half — everything the lead filed — is one
/// [`EscalationFiling`], so a field added there reaches the stored record,
/// this type and every caller without another hand-written mapping. Not
/// itself serialised: the stored shape is the private `EscalationMeta` plus the
/// `status` and `decision` hash fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escalation {
    /// Short id, e.g. `e7`. Generated by [`crate::Relay::escalate`].
    pub id: EscalationId,
    /// What the lead filed, snapshotted from its registration at escalate
    /// time.
    pub filing: EscalationFiling,
    /// Where it stands.
    pub status: EscalationStatus,
    /// The decision, once resolved.
    pub decision: Option<String>,
}

/// What a lead files, and what [`crate::Relay::escalate`] needs to file a new
/// escalation. A struct rather than a long parameter list
/// (`too-many-arguments-threshold = 8` in `clippy.toml`), and because most of
/// it is one snapshot taken together. It is also the immutable half of a
/// stored [`Escalation`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscalationFiling {
    /// The Linear ticket this decision belongs to.
    pub ticket: TicketId,
    /// The lead filing it.
    pub lead: AgentName,
    /// The lead's machine, from its registration.
    pub machine: MachineName,
    /// The lead's harness session id, from its registration.
    pub session_id: SessionId,
    /// The lead's working directory, from its registration.
    pub cwd: Option<String>,
    /// The lead's harness kind, from its registration.
    pub kind: String,
    /// What the lead needs decided.
    pub context: String,
    /// The choices the lead sees.
    pub options: Vec<String>,
    /// The lead's own recommendation.
    pub recommendation: String,
    /// When it's being filed, Unix milliseconds. Taken from the caller (as
    /// [`crate::Message::sent_at_ms`] is) rather than read from the system
    /// clock inside the store, so tests can supply a fixed value.
    pub created_at_ms: i64,
}

/// The stored half of an [`Escalation`]: the id and what was filed, but not
/// `status` or `decision`, which [`crate::Relay`]'s transition script mutates
/// in place as separate hash fields. Stored as one JSON blob under the hash
/// field `meta` so it can be encoded/decoded in one shot, while
/// `status`/`decision` stay plain hash fields a Lua script can read and write
/// without touching JSON at all (see `store.rs`'s transition script).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EscalationMeta {
    pub id: EscalationId,
    #[serde(flatten)]
    pub filing: EscalationFiling,
}

impl EscalationMeta {
    pub(crate) fn into_escalation(
        self,
        status: EscalationStatus,
        decision: Option<String>,
    ) -> Escalation {
        Escalation {
            id: self.id,
            filing: self.filing,
            status,
            decision,
        }
    }
}

/// What [`crate::Relay::list`] found: the escalations that decoded cleanly,
/// and the ids of any hash entries that didn't (corrupt, or hand-edited).
#[derive(Debug, Default)]
pub struct EscalationList {
    /// Escalations ready for the caller, oldest first.
    pub escalations: Vec<Escalation>,
    /// `(id, why)` for every entry that failed to decode. Also already read;
    /// the caller decides how to surface them (`agent-msg escalations`
    /// prints them to stderr).
    pub rejected: Vec<(String, Error)>,
}

/// Longest a harness session id [`crate::Relay::escalate`] will accept.
/// A UUID (36 chars) comfortably fits; generous enough for a harness that
/// uses a longer opaque id, but still far short of anything that could carry
/// a meaningful payload.
const MAX_SESSION_ID_LEN: usize = 64;

/// Validates a harness session id before it's stored — and, eventually,
/// passed verbatim to `claude --resume <sid>` by `investigate`. Restricted to
/// what every harness's own session ids actually look like (a UUID, or a
/// short opaque token): ASCII letters, digits and `-`, 1–64 characters. This
/// is stricter than "no control characters"; it also can't contain shell
/// metacharacters, path separators, or spaces.
///
/// # Errors
///
/// [`Error::InvalidSessionId`] otherwise.
pub(crate) fn validate_session_id(session_id: &str) -> Result<(), Error> {
    let valid = !session_id.is_empty()
        && session_id.len() <= MAX_SESSION_ID_LEN
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidSessionId {
            session_id: session_id.to_owned(),
        })
    }
}

/// Validates that `text` (one of `escalate`'s `context`/`option`/
/// `recommendation` fields) carries no control character that could do
/// something other than display as text later — in particular ESC, which can
/// trigger a terminal escape sequence (e.g. bracketed paste) if this text is
/// ever typed into a pane. The rule is [`crate::text::is_unsafe_char`], shared
/// with message rendering; `\n` and `\t` are the only control characters it
/// allows. This is defense in depth: nothing in this crate or
/// `agent-msg` types escalation text into a shell or terminal (`investigate`
/// passes only a fixed, field-built string as the fresh session's initial
/// prompt — see `agent-msg`'s `escalate.rs`), but a store-level refusal means
/// that stays true even if a future caller forgets.
///
/// # Errors
///
/// [`Error::UnsafeEscalationText`] naming `field`, the offending codepoint,
/// and its byte offset.
pub(crate) fn validate_safe_text(field: &'static str, text: &str) -> Result<(), Error> {
    match crate::text::first_unsafe(text) {
        Some((byte_offset, c)) => Err(Error::UnsafeEscalationText {
            field,
            codepoint: u32::from(c),
            byte_offset,
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {

    /// The stored `meta` blob, byte for byte as `main` wrote it before
    /// `EscalationFiling` existed. Node-A and node-b run different binaries:
    /// renaming or reordering a field here breaks every other machine.
    const META_SOME: &str = r#"{"id":"e7","ticket":"AGE-2027","lead":"qsl-lead","machine":"node-a","session_id":"sid-1","cwd":"/wörk/日本","kind":"claude","context":"ctx \"q\"\n é","options":["a","b"],"recommendation":"a","created_at_ms":9223372036854775807}"#;
    const META_NULL_CWD: &str = r#"{"id":"e7","ticket":"AGE-2027","lead":"qsl-lead","machine":"node-a","session_id":"sid-1","cwd":null,"kind":"claude","context":"c","options":[],"recommendation":"r","created_at_ms":0}"#;

    #[test]
    fn the_stored_meta_blob_is_byte_exact_in_both_directions() {
        for raw in [META_SOME, META_NULL_CWD] {
            let meta: EscalationMeta = serde_json::from_str(raw).unwrap();
            assert_eq!(serde_json::to_string(&meta).unwrap(), raw);
        }
        let meta: EscalationMeta = serde_json::from_str(META_SOME).unwrap();
        assert_eq!(meta.filing.created_at_ms, i64::MAX);
        assert_eq!(meta.filing.cwd.as_deref(), Some("/wörk/日本"));
        let meta: EscalationMeta = serde_json::from_str(META_NULL_CWD).unwrap();
        assert_eq!(meta.filing.cwd, None);
    }
    use super::*;

    #[test]
    fn accepts_a_well_formed_linear_key() {
        assert_eq!("AGE-2014".parse::<TicketId>().unwrap().as_str(), "AGE-2014");
        assert_eq!("QSL-1".parse::<TicketId>().unwrap().as_str(), "QSL-1");
    }

    #[test]
    fn rejects_malformed_ticket_ids() {
        for bad in [
            "",
            "age-2014",
            "AGE",
            "AGE-",
            "-2014",
            "AGE-2014x",
            "AGE 2014",
            "2014-AGE",
        ] {
            assert!(
                matches!(bad.parse::<TicketId>(), Err(Error::InvalidTicket { ticket }) if ticket == bad),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn escalation_ids_are_e_and_digits_only() {
        for good in ["e1", "e7", "e18446744073709551615"] {
            assert_eq!(good.parse::<EscalationId>().unwrap().as_str(), good);
        }
        for bad in [
            "",
            "e",
            "seq",
            "index",
            "e1:attempts",
            "E1",
            "e-1",
            "1",
            "e1 ",
            &format!("e{}", "9".repeat(21)),
        ] {
            assert!(
                matches!(bad.parse::<EscalationId>(), Err(Error::InvalidEscalationId { id }) if id == bad),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn ticket_id_round_trips_through_json() {
        let t: TicketId = serde_json::from_str("\"AGE-2014\"").unwrap();
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"AGE-2014\"");
        assert!(serde_json::from_str::<TicketId>("\"bad\"").is_err());
    }

    #[test]
    fn status_round_trips_through_str_and_display() {
        for s in [
            EscalationStatus::Waiting,
            EscalationStatus::Investigating,
            EscalationStatus::Resolved,
        ] {
            assert_eq!(s.as_str().parse::<EscalationStatus>().unwrap(), s);
            assert_eq!(s.to_string(), s.as_str());
        }
        assert!(matches!(
            "bogus".parse::<EscalationStatus>(),
            Err(Error::InvalidEscalationStatus { status }) if status == "bogus"
        ));
    }

    /// The shared rule also refuses what the old control-character check let
    /// through: C1 controls, line separators and bidi controls.
    #[test]
    fn safe_text_rejects_c1_line_separators_and_bidi_controls() {
        for (bad, codepoint) in [
            ("a\u{85}b", 0x85_u32),
            ("a\u{2028}b", 0x2028),
            ("a\u{202e}b", 0x202e),
            ("a\u{2066}b", 0x2066),
        ] {
            let err = validate_safe_text("context", bad).unwrap_err();
            assert!(
                matches!(err, Error::UnsafeEscalationText { codepoint: c, byte_offset: 1, .. } if c == codepoint),
                "{bad:?}: {err:?}"
            );
        }
    }

    #[test]
    fn accepts_typical_session_ids() {
        for ok in [
            "a",
            "sid-1",
            "32eb5177-ab07-44ea-8363-7a93ad06b7a2",
            &"a".repeat(64),
        ] {
            assert!(validate_session_id(ok).is_ok(), "{ok:?} was rejected");
        }
    }

    #[test]
    fn rejects_empty_overlong_or_shell_meaningful_session_ids() {
        let too_long = "a".repeat(65);
        for bad in [
            "",
            &too_long,
            "sid with spaces",
            "sid;rm -rf /",
            "$(whoami)",
            "sid`x`",
            "sid'",
            "sid\n",
            "../../etc/passwd",
        ] {
            assert!(
                matches!(validate_session_id(bad), Err(Error::InvalidSessionId { session_id }) if session_id == bad),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn safe_text_accepts_plain_text_newlines_and_tabs() {
        assert!(validate_safe_text("context", "plain text").is_ok());
        assert!(validate_safe_text("context", "line one\nline two").is_ok());
        assert!(validate_safe_text("context", "a\ttabbed value").is_ok());
    }

    #[test]
    fn safe_text_rejects_control_characters_including_esc_and_etx() {
        for (name, bad) in [
            ("ESC (bracketed-paste trigger)", "hi\x1b[201~evil"),
            ("ETX / ctrl-C", "hi\x03"),
            ("DEL", "hi\x7f"),
            ("NUL", "hi\0"),
            ("BEL", "hi\x07"),
        ] {
            let err = validate_safe_text("context", bad).unwrap_err();
            assert!(
                matches!(&err, Error::UnsafeEscalationText { field, .. } if *field == "context"),
                "{name}: {err}"
            );
        }
    }

    proptest::proptest! {
        /// Any string built only from the characters `TicketId::from_str`
        /// documents as valid (uppercase ASCII prefix, `-`, ASCII digits,
        /// each part nonempty) is accepted and round-trips through `as_str`.
        #[test]
        fn ticket_id_accepts_every_well_formed_key(
            prefix in "[A-Z]{1,8}",
            number in "[0-9]{1,8}",
        ) {
            let raw = format!("{prefix}-{number}");
            let parsed = raw.parse::<TicketId>().expect("well-formed key must parse");
            assert_eq!(parsed.as_str(), raw);
        }

        /// Any string containing a byte outside the accepted alphabet (a
        /// lowercase letter, or anything that isn't `[A-Z0-9-]`) is refused,
        /// whatever else surrounds it.
        #[test]
        fn ticket_id_rejects_any_key_containing_a_lowercase_letter(
            prefix in "[A-Z]{1,8}",
            lower in "[a-z]",
            number in "[0-9]{1,8}",
        ) {
            let raw = format!("{prefix}{lower}-{number}");
            assert!(raw.parse::<TicketId>().is_err());
        }
    }
}
