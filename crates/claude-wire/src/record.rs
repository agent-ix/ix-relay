//! Claude Code's `~/.claude/sessions/<pid>.json` record and the version gate.
//!
//! Pure parsing: reading the file, checking the process and the socket stay
//! with the caller. **Provenance:** the field names (`sessionId`, `procStart`,
//! `pidDomain`, `kind`, `version`, `peerProtocol`, `messagingSocketPath`) were
//! captured live from Claude Code 2.1.283; `MIN_VERSION` is the oldest
//! version the upstream documentation says has the inbox socket. The record has
//! many other fields, ignored on purpose.

use std::path::PathBuf;

use serde::Deserialize;

/// The one `peerProtocol` this crate knows the wire format of (captured on 2.1.283).
pub const KNOWN_PEER_PROTOCOL: u32 = 1;

/// The oldest Claude Code version that has the inbox socket (documented
/// upstream); the format was measured on [`crate::CAPTURED_ON`].
pub const MIN_VERSION: (u64, u64, u64) = (2, 1, 224);

/// `major.minor.patch` from a version string; a suffix after the patch number
/// (`-beta`) is ignored. Total: `None` for anything else.
#[must_use]
pub fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().splitn(3, '.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch_text = parts.next()?;
    let digits: String = patch_text
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor, digits.parse().ok()?))
}

/// One session record, as far as this reads it. Unknown fields are expected and
/// ignored; only `pid` and `sessionId` are required to decode.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    /// The Claude Code process.
    pub pid: u32,
    /// The session it holds now (`/clear` changes it in place).
    pub session_id: String,
    /// The process start time as Claude Code recorded it.
    pub proc_start: Option<String>,
    /// The pid namespace it recorded.
    pub pid_domain: Option<String>,
    /// `interactive` or another kind.
    pub kind: Option<String>,
    /// Claude Code's version.
    pub version: Option<String>,
    /// The messaging protocol number.
    pub peer_protocol: Option<u32>,
    /// The inbox socket it bound.
    pub messaging_socket_path: Option<PathBuf>,
}

/// Whether a record describes a Claude Code this crate knows how to talk to.
/// A drift signal, not a failure: anything but [`RecordVerdict::Supported`] means
/// "not verified", and the caller falls back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordVerdict {
    /// Known `peerProtocol`, version at least [`MIN_VERSION`], socket path present.
    Supported,
    /// `peerProtocol` is absent or not [`KNOWN_PEER_PROTOCOL`].
    UnsupportedProtocol {
        /// What the record said.
        peer_protocol: Option<u32>,
        /// The record's version, for the message.
        version: Option<String>,
    },
    /// The version is absent, unparseable or older than [`MIN_VERSION`].
    UnsupportedVersion {
        /// What the record said.
        version: Option<String>,
        /// The record's `peerProtocol`, for the message.
        peer_protocol: Option<u32>,
    },
    /// Supported, but a field needed to deliver is absent.
    Malformed {
        /// The missing field's name.
        missing: &'static str,
    },
}

impl SessionRecord {
    /// Decodes a record; `None` when it is not JSON or lacks `pid` or
    /// `sessionId`. Total over any bytes.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    /// Whether this record is one the crate knows how to talk to. The protocol
    /// is checked before the version; both before the socket path.
    #[must_use]
    pub fn verdict(&self) -> RecordVerdict {
        if self.peer_protocol != Some(KNOWN_PEER_PROTOCOL) {
            return RecordVerdict::UnsupportedProtocol {
                peer_protocol: self.peer_protocol,
                version: self.version.clone(),
            };
        }
        let known = self
            .version
            .as_deref()
            .and_then(parse_version)
            .is_some_and(|v| v >= MIN_VERSION);
        if !known {
            return RecordVerdict::UnsupportedVersion {
                version: self.version.clone(),
                peer_protocol: self.peer_protocol,
            };
        }
        if self.messaging_socket_path.is_none() {
            return RecordVerdict::Malformed {
                missing: "messagingSocketPath",
            };
        }
        RecordVerdict::Supported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn record(version: Option<&str>, protocol: Option<u32>, socket: bool) -> SessionRecord {
        SessionRecord {
            pid: 1,
            session_id: "s".into(),
            proc_start: None,
            pid_domain: None,
            kind: None,
            version: version.map(str::to_owned),
            peer_protocol: protocol,
            messaging_socket_path: socket.then(|| PathBuf::from("/r/1.sock")),
        }
    }

    /// Tracing: TC-046-07
    /// ACs: FR-046-AC-7
    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("2.1.283"), Some((2, 1, 283)));
        assert_eq!(parse_version(" 3.0.0-beta.1 "), Some((3, 0, 0)));
        for bad in ["", "2", "2.1", "2.1.x", "a.b.c", "2..3", "-1.0.0"] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
    }

    /// Tracing: TC-046-07
    /// ACs: FR-046-AC-7
    #[test]
    fn the_verdict_is_typed_for_each_way_to_be_unsupported() {
        assert_eq!(
            record(Some("2.1.283"), Some(1), true).verdict(),
            RecordVerdict::Supported
        );
        for v in ["2.1.224", "2.2.0", "3.0.0-beta.1"] {
            assert_eq!(
                record(Some(v), Some(1), true).verdict(),
                RecordVerdict::Supported,
                "{v}"
            );
        }
        assert_eq!(
            record(Some("2.1.283"), Some(2), true).verdict(),
            RecordVerdict::UnsupportedProtocol {
                peer_protocol: Some(2),
                version: Some("2.1.283".into())
            }
        );
        assert!(matches!(
            record(Some("2.1.283"), None, true).verdict(),
            RecordVerdict::UnsupportedProtocol {
                peer_protocol: None,
                ..
            }
        ));
        for v in [Some("2.1.223"), Some("1.9.9"), Some("garbage"), None] {
            assert_eq!(
                record(v, Some(1), true).verdict(),
                RecordVerdict::UnsupportedVersion {
                    version: v.map(str::to_owned),
                    peer_protocol: Some(1)
                },
                "{v:?}"
            );
        }
        assert_eq!(
            record(Some("2.1.283"), Some(1), false).verdict(),
            RecordVerdict::Malformed {
                missing: "messagingSocketPath"
            }
        );
    }

    /// Tracing: TC-046-07
    /// ACs: FR-046-AC-7
    #[test]
    fn a_record_decodes_camel_case_and_ignores_unknown_fields() {
        let r = SessionRecord::parse(
            br#"{"pid":7,"sessionId":"s","procStart":"9","pidDomain":"d","kind":"interactive","version":"2.1.283","peerProtocol":1,"messagingSocketPath":"/r/7.sock","status":"idle","new":{"x":1}}"#,
        )
        .unwrap();
        assert_eq!(r.pid, 7);
        assert_eq!(r.session_id, "s");
        assert_eq!(r.kind.as_deref(), Some("interactive"));
        assert_eq!(r.verdict(), RecordVerdict::Supported);
        assert!(SessionRecord::parse(br#"{"sessionId":"s"}"#).is_none());
        assert!(SessionRecord::parse(br#"{"pid":"7","sessionId":"s"}"#).is_none());
        assert!(SessionRecord::parse(b"\xff\xfe").is_none());
    }

    proptest! {
        /// Tracing: TC-046-02
        /// ACs: FR-046-AC-2
        #[test]
        fn parsing_is_total(b in proptest::collection::vec(any::<u8>(), 0..256), v in ".*") {
            if let Some(r) = SessionRecord::parse(&b) {
                let _ = r.verdict();
            }
            let _ = parse_version(&v);
        }
    }
}
