//! Typed ids: the harness session, the herdr pane, tab and workspace, and the machine.
//!
//! Each is a string on the wire and in Redis (`#[serde(try_from = "String",
//! into = "String")]`, so stored and emitted JSON is byte-identical to the bare
//! strings they replaced), but a different type in code, so two ids cannot be
//! swapped at a call site and a raw `&str` cannot reach a Redis key builder
//! (AGE-2027 phase B).
//!
//! Validation is one rule per type, applied on construction from user input
//! (`FromStr`) and on decode. A stored value that breaks the rule fails to
//! decode; every reader of the registry then skips and reports that record
//! (FR-008) rather than failing the read. The rules are deliberately no
//! stricter than what the writers already enforced (see spec section 15).

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{error::Error, text};

/// Longest id or machine name accepted, in bytes. Far above any real session
/// id (a UUID), pane id (`w1:p1`) or hostname; it only bounds what one Redis key
/// or registry field can carry.
pub const MAX_ID_BYTES: usize = 256;

/// The shared shape rule: non-empty, at most [`MAX_ID_BYTES`], and nothing
/// [`text::sanitise_line`] would replace (so no control, bidi or invisible
/// character, and no line break).
fn is_well_formed(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_BYTES
        && !s
            .chars()
            .any(|c| text::is_unsafe_char(c) || c == '\n' || c == '\t')
}

/// Declares a validated string newtype with the shared trait set.
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// The id as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self, Error> {
                if is_well_formed(s) {
                    Ok(Self(s.to_owned()))
                } else {
                    Err(Error::InvalidId { kind: $kind, value: s.to_owned() })
                }
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;

            fn try_from(s: String) -> Result<Self, Error> {
                if is_well_formed(&s) {
                    Ok(Self(s))
                } else {
                    Err(Error::InvalidId { kind: $kind, value: s })
                }
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id! {
    /// A harness session id (Claude Code's or Codex's `session_id`): the key of
    /// the session cache and the value `claude --resume` takes. Opaque; only
    /// its shape is checked (the shape rule in the module docs).
    SessionId, "session id"
}

string_id! {
    /// A herdr pane id such as `w1:p1` (`HERDR_PANE_ID`). Only unique within one
    /// machine's herdr: compare it only through a [`PaneRef`].
    PaneId, "pane id"
}

string_id! {
    /// A herdr workspace id such as `w1`. Only unique within one machine's herdr.
    WorkspaceId, "workspace id"
}

string_id! {
    /// A herdr tab id such as `w1:t2`. Only unique within one machine's herdr.
    TabId, "tab id"
}

/// A machine's name on the bus, normalised: lowercase, cut at the first `.`, so
/// `Node-B.local` and `node-b` are one machine. A hostname's case and domain
/// suffix are noise, not identity, and letting them differ between two calls on
/// the same box (or between an old registration and a newer binary) silently
/// hides a lead from `relay-watcher`.
///
/// Normalising happens in [`MachineName::new`] and nowhere else, so two
/// `MachineName`s are equal exactly when the machines are the same, and `==` is
/// the comparison. Decoding a stored name normalises it (idempotent for what
/// every writer already stored). An empty result is refused: the machine `""`
/// hides a lead from every watcher.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MachineName(String);

impl MachineName {
    /// Normalises `raw` (lowercase, cut at the first `.`).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidId`] when the result is empty, too long, or carries an
    /// unsafe character.
    pub fn new(raw: &str) -> Result<Self, Error> {
        let short = Self::normalise(raw);
        if is_well_formed(&short) {
            Ok(Self(short))
        } else {
            Err(Error::InvalidId {
                kind: "machine name",
                value: raw.to_owned(),
            })
        }
    }

    /// The normalisation rule, and the only place it is written: lowercase,
    /// cut at the first `.`. Idempotent. [`crate::Address`] applies it to the
    /// server segment of a typed address, which may be any text.
    pub(crate) fn normalise(raw: &str) -> String {
        let lower = raw.to_ascii_lowercase();
        lower.split('.').next().unwrap_or(&lower).to_owned()
    }

    /// The normalised name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for MachineName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        Self::new(s)
    }
}

impl TryFrom<String> for MachineName {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        Self::new(&s)
    }
}

impl From<MachineName> for String {
    fn from(machine: MachineName) -> Self {
        machine.0
    }
}

impl AsRef<str> for MachineName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MachineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A herdr pane and the machine whose herdr issued its id: the only way to
/// compare panes. Pane ids repeat across machines (FR-009), so a bare
/// [`PaneId`] is never an identity; this pair is. Not stored or emitted as a
/// unit — a registration keeps its two fields, unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneRef {
    /// The machine the pane is on.
    pub machine: MachineName,
    /// The pane's id in that machine's herdr.
    pub pane: PaneId,
}

impl PaneRef {
    /// The pane `pane` on `machine`.
    #[must_use]
    pub fn new(machine: MachineName, pane: PaneId) -> Self {
        Self { machine, pane }
    }
}

impl fmt::Display for PaneRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.machine, self.pane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_names_normalise_once_and_compare_by_equality() {
        let a: MachineName = "Node-B.local".parse().unwrap();
        let b: MachineName = "node-b".parse().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "node-b");
        assert_eq!(MachineName::new(a.as_str()).unwrap(), a, "idempotent");
    }

    #[test]
    fn empty_and_unsafe_ids_are_refused() {
        assert!("".parse::<MachineName>().is_err());
        assert!(".local".parse::<MachineName>().is_err());
        assert!("".parse::<PaneId>().is_err());
        assert!("w1\n:p1".parse::<PaneId>().is_err());
        assert!("a\u{1b}b".parse::<SessionId>().is_err());
        assert!("x".repeat(MAX_ID_BYTES + 1).parse::<SessionId>().is_err());
        assert!("x".repeat(MAX_ID_BYTES).parse::<SessionId>().is_ok());
    }

    proptest::proptest! {
        /// Construction is total: any text is either accepted or refused with
        /// `InvalidId`, and an accepted machine name is already normalised.
        #[test]
        fn construction_is_total_over_arbitrary_text(raw in ".{0,300}") {
            for parsed in [
                raw.parse::<PaneId>().map(|_| ()),
                raw.parse::<SessionId>().map(|_| ()),
                raw.parse::<TabId>().map(|_| ()),
                raw.parse::<WorkspaceId>().map(|_| ()),
                raw.parse::<MachineName>().map(|_| ()),
            ] {
                let refused_properly = matches!(parsed, Ok(()) | Err(Error::InvalidId { .. }));
                proptest::prop_assert!(refused_properly, "{parsed:?}");
            }
            if let Ok(machine) = raw.parse::<MachineName>() {
                proptest::prop_assert_eq!(MachineName::new(machine.as_str()).unwrap(), machine);
            }
        }
    }

    #[test]
    fn ids_serialise_as_bare_strings() {
        let pane: PaneId = "w1:p1".parse().unwrap();
        assert_eq!(serde_json::to_string(&pane).unwrap(), "\"w1:p1\"");
        let sid: SessionId = serde_json::from_str("\"abc-123\"").unwrap();
        assert_eq!(serde_json::to_string(&sid).unwrap(), "\"abc-123\"");
        assert!(serde_json::from_str::<SessionId>("\"\"").is_err());
        assert!(serde_json::from_str::<PaneId>("7").is_err());
    }

    #[test]
    fn a_pane_ref_on_another_machine_is_a_different_pane() {
        let pane: PaneId = "wB:p1".parse().unwrap();
        let hub = PaneRef::new("hub".parse().unwrap(), pane.clone());
        let node_a = PaneRef::new("node-a".parse().unwrap(), pane);
        assert_ne!(hub, node_a);
    }
}
