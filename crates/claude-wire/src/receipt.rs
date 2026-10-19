//! The control lines Claude Code writes back to the sender's reply socket.
//!
//! One JSON line per connection. Parsing is **total** ([`parse_control_line`]):
//! every input maps to a [`ControlLine`], and the ones that are not a shape this
//! crate knows say so ([`ControlLine::Unrecognised`], [`ControlLine::NotJson`])
//! so the caller can count them as drift instead of dropping them silently.
//!
//! **Provenance.** Only `held` (a default-mode receiver with
//! `crossSessionInbound: "hold"`) and `refused` (`expired` with detail
//! `refused`) receipts were captured live from Claude Code 2.1.283.
//! `delivered`, `denied`, `dropped` (with `drop_reason` and `dropped_msg_ids`),
//! `expired` with `hold_timeout` and the duplicate detail were read from the
//! installed program's text; they are exercised by fixtures marked
//! `derived-from-program-text` only. Blank connections (liveness checks) were
//! observed live.

use serde::Deserialize;
use serde_json::Value;

/// What a receipt says happened to our message. Read from the exact `status`
/// and `status_detail` strings, never by substring: an unknown pair is
/// [`Verdict::Unknown`], which callers count as drift.
///
/// `held` and `refused` were captured live on 2.1.283; the rest are derived from
/// the program text (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Held behind an approval prompt (a bypass-mode receiver). Captured.
    Held,
    /// The user approved a held message: it is being delivered. Derived.
    Delivered,
    /// The user denied a held message: it will never be delivered. Derived.
    Denied,
    /// The receiver refuses cross-session messages; not delivered
    /// (`expired` with detail `refused`, or `refused`). Captured.
    Refused,
    /// A hold timed out unapproved (`expired` with detail `hold_timeout`): the
    /// message was not delivered and is gone from the receiver's hold queue. Derived.
    HoldExpired,
    /// Discarded as a duplicate of a body it saw from us in the last 30 s. The
    /// earlier copy may have been delivered. Derived.
    Duplicate,
    /// Discarded for another reason; not delivered. Derived.
    Dropped,
    /// The receiver gave up on it for a reason this code does not know; whether
    /// it was seen is not known. Derived.
    Expired,
    /// A status this code does not know: drift.
    Unknown,
}

/// A `peer_message_status` control line, as far as this reads it.
#[derive(Debug, Deserialize)]
struct RawReceipt {
    status: String,
    #[serde(default)]
    status_detail: Option<String>,
    #[serde(default)]
    orig_msg_id: Option<String>,
    #[serde(default)]
    drop_reason: Option<String>,
    #[serde(default)]
    dropped_msg_ids: Vec<String>,
}

/// A receipt: which messages it is about and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// The `msg_id` of the message it answers, when it names one.
    pub orig_msg_id: Option<String>,
    /// The ids of every message a `dropped` receipt covers (`dropped_msg_ids`).
    pub dropped_msg_ids: Vec<String>,
    /// What happened to them.
    pub verdict: Verdict,
}

impl Receipt {
    /// Whether this receipt is about message `id`: it answers it, or a drop
    /// names it.
    #[must_use]
    pub fn concerns(&self, id: &str) -> bool {
        self.orig_msg_id.as_deref() == Some(id) || self.dropped_msg_ids.iter().any(|d| d == id)
    }
}

/// The most ids kept from one receipt.
const MAX_DROPPED_IDS: usize = 256;

/// The verdict for a receipt's `status`, `status_detail` and `drop_reason`.
/// Total: every triple maps to exactly one [`Verdict`].
#[must_use]
pub fn classify(status: &str, detail: Option<&str>, drop_reason: Option<&str>) -> Verdict {
    match (status, detail) {
        ("held", _) => Verdict::Held,
        ("delivered", _) => Verdict::Delivered,
        ("denied", _) => Verdict::Denied,
        ("refused", _) | ("expired", Some("refused")) => Verdict::Refused,
        ("expired", Some("hold_timeout")) => Verdict::HoldExpired,
        ("expired", _) => Verdict::Expired,
        ("dropped", detail) if detail == Some("duplicate") || drop_reason == Some("duplicate") => {
            Verdict::Duplicate
        }
        ("dropped", _) => Verdict::Dropped,
        _ => Verdict::Unknown,
    }
}

/// A line that is understood and needs no action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// A blank line or a connection that sent nothing: a liveness check
    /// (observed live on 2.1.283).
    Liveness,
    /// A `peer_message_status` that names no message (`orig_msg_id` and
    /// `dropped_msg_ids` both absent): nothing to attribute it to.
    UnaddressedStatus,
}

/// Why valid JSON is not a shape this crate knows. Each is a drift signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftReason {
    /// The JSON value is not an object.
    NotAnObject,
    /// No string `type` field (renamed or removed).
    TypeMissing,
    /// `type` is a string other than `control`.
    TypeUnknown,
    /// No string `action` field (renamed or removed).
    ActionMissing,
    /// `action` is a string other than `peer_message_status`: no other control
    /// action is known.
    ActionUnknown,
    /// `status` is absent or null on a `peer_message_status`.
    StatusMissing,
    /// A field this crate reads has another type (`status` not a string,
    /// `orig_msg_id` not a string, `dropped_msg_ids` not a list of strings...).
    FieldShape,
}

/// One line read from the reply socket, classified. Total: see
/// [`parse_control_line`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlLine {
    /// A `peer_message_status` naming at least one message.
    Receipt(Receipt),
    /// A known line that needs no action.
    KnownIgnored(Ignored),
    /// Valid JSON that is not a shape this crate knows: counted as drift.
    Unrecognised {
        /// Which expectation failed first.
        reason: DriftReason,
    },
    /// Not JSON at all: counted as drift.
    NotJson,
}

fn unrecognised(reason: DriftReason) -> ControlLine {
    ControlLine::Unrecognised { reason }
}

/// The most characters kept of a line's `type` or `action` ([`line_shape`]).
pub const MAX_SHAPE_CHARS: usize = 64;

/// The `type` of every receipt line.
pub const CONTROL_TYPE: &str = "control";

/// The `action` of every receipt line.
pub const RECEIPT_ACTION: &str = "peer_message_status";

/// The only text of an offending reply-socket line that is kept, for a drift signal: its
/// `type` and `action` strings, each cut to [`MAX_SHAPE_CHARS`] characters. Never the
/// line's body, a message id or a path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineShape {
    /// The `type` string, absent when the line has none (or is not JSON).
    pub line_type: Option<String>,
    /// The `action` string, absent when the line has none (or is not JSON).
    pub action: Option<String>,
}

impl LineShape {
    /// The shape of every receipt: what an unknown-status receipt looked like.
    #[must_use]
    pub fn receipt() -> Self {
        Self {
            line_type: Some(CONTROL_TYPE.to_owned()),
            action: Some(RECEIPT_ACTION.to_owned()),
        }
    }
}

/// The `type` and `action` strings of `line`, cut. Total: any input gives a shape, and one
/// that is not a JSON object has neither.
#[must_use]
pub fn line_shape(line: &str) -> LineShape {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(line.trim()) else {
        return LineShape::default();
    };
    let kept = |key: &str| match object.get(key) {
        Some(Value::String(s)) => Some(s.chars().take(MAX_SHAPE_CHARS).collect()),
        _ => None,
    };
    LineShape {
        line_type: kept("type"),
        action: kept("action"),
    }
}

/// The string field `key` of `object`: `Ok(None)` when absent or null, `Err`
/// when it is present with another type.
fn string_field<'v>(
    object: &'v serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<&'v str>, DriftReason> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(DriftReason::FieldShape),
    }
}

/// Classifies one line from the reply socket. Never panics, for any input.
#[must_use]
pub fn parse_control_line(line: &str) -> ControlLine {
    let line = line.trim();
    if line.is_empty() {
        return ControlLine::KnownIgnored(Ignored::Liveness);
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return ControlLine::NotJson;
    };
    let Value::Object(object) = &value else {
        return unrecognised(DriftReason::NotAnObject);
    };
    match string_field(object, "type") {
        Ok(Some(CONTROL_TYPE)) => {}
        Ok(Some(_)) => return unrecognised(DriftReason::TypeUnknown),
        Ok(None) => return unrecognised(DriftReason::TypeMissing),
        Err(reason) => return unrecognised(reason),
    }
    match string_field(object, "action") {
        Ok(Some(RECEIPT_ACTION)) => {}
        Ok(Some(_)) => return unrecognised(DriftReason::ActionUnknown),
        Ok(None) => return unrecognised(DriftReason::ActionMissing),
        Err(reason) => return unrecognised(reason),
    }
    if matches!(object.get("status"), None | Some(Value::Null)) {
        return unrecognised(DriftReason::StatusMissing);
    }
    let Ok(raw) = serde_json::from_value::<RawReceipt>(value) else {
        return unrecognised(DriftReason::FieldShape);
    };
    if raw.orig_msg_id.is_none() && raw.dropped_msg_ids.is_empty() {
        return ControlLine::KnownIgnored(Ignored::UnaddressedStatus);
    }
    let verdict = classify(
        &raw.status,
        raw.status_detail.as_deref(),
        raw.drop_reason.as_deref(),
    );
    ControlLine::Receipt(Receipt {
        orig_msg_id: raw.orig_msg_id,
        dropped_msg_ids: raw
            .dropped_msg_ids
            .into_iter()
            .take(MAX_DROPPED_IDS)
            .collect(),
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Tracing: TC-046-17
    /// ACs: FR-046-AC-17
    #[test]
    fn a_line_shape_keeps_only_a_cut_type_and_action() {
        let long = "x".repeat(10_000);
        let line = format!(
            r#"{{"type":"{long}","action":"a\u001b[31m","body":"secret","path":"/etc/passwd","orig_msg_id":"m"}}"#
        );
        let shape = line_shape(&line);
        assert_eq!(
            shape.line_type.as_deref().map(str::len),
            Some(MAX_SHAPE_CHARS)
        );
        assert_eq!(shape.action.as_deref(), Some("a\u{1b}[31m"));
        assert_eq!(line_shape("not json"), LineShape::default());
        assert_eq!(line_shape("[1]"), LineShape::default());
        assert_eq!(line_shape(r#"{"type":3}"#), LineShape::default());
        assert_eq!(line_shape(""), LineShape::default());
    }

    fn of(status: &str, detail: &str) -> Verdict {
        match parse_control_line(&format!(
            r#"{{"type":"control","action":"peer_message_status","status":"{status}","status_detail":"{detail}","orig_msg_id":"m"}}"#
        )) {
            ControlLine::Receipt(r) => r.verdict,
            other => panic!("not a receipt: {other:?}"),
        }
    }

    /// Tracing: TC-046-02
    /// ACs: FR-046-AC-2
    #[test]
    fn every_verdict_is_reached_by_its_exact_strings() {
        assert_eq!(of("held", ""), Verdict::Held);
        assert_eq!(of("delivered", ""), Verdict::Delivered);
        assert_eq!(of("denied", ""), Verdict::Denied);
        assert_eq!(of("refused", ""), Verdict::Refused);
        assert_eq!(of("expired", "refused"), Verdict::Refused);
        assert_eq!(of("expired", "hold_timeout"), Verdict::HoldExpired);
        assert_eq!(of("dropped", "duplicate"), Verdict::Duplicate);
        assert_eq!(of("dropped", "queue_full"), Verdict::Dropped);
        assert_eq!(of("expired", "timeout"), Verdict::Expired);
        assert_eq!(of("expired", ""), Verdict::Expired);
        assert_eq!(of("new-thing", ""), Verdict::Unknown);
        // Exact strings, never substrings.
        assert_eq!(of("onholdish", ""), Verdict::Unknown);
        assert_eq!(of("held-over", ""), Verdict::Unknown);
        assert_eq!(of("expired", "not_refused_really"), Verdict::Expired);
        assert_eq!(of("", ""), Verdict::Unknown);
    }

    /// Tracing: TC-046-02
    /// ACs: FR-046-AC-2
    #[test]
    fn a_drop_reason_of_duplicate_is_a_duplicate() {
        assert_eq!(
            classify("dropped", None, Some("duplicate")),
            Verdict::Duplicate
        );
        assert_eq!(classify("dropped", None, Some("x")), Verdict::Dropped);
        assert_eq!(classify("dropped", None, None), Verdict::Dropped);
    }

    /// Tracing: TC-046-02
    /// ACs: FR-046-AC-2
    #[test]
    fn a_drop_names_its_messages() {
        let ControlLine::Receipt(r) = parse_control_line(
            r#"{"type":"control","action":"peer_message_status","status":"dropped","drop_reason":"queue-full","dropped_msg_ids":["a","b"]}"#,
        ) else {
            panic!("not a receipt");
        };
        assert_eq!(r.verdict, Verdict::Dropped);
        assert!(r.concerns("a") && r.concerns("b") && !r.concerns("c"));
    }

    /// Tracing: TC-046-02
    /// ACs: FR-046-AC-2
    #[test]
    fn dropped_ids_are_bounded() {
        let ids: Vec<String> = (0..MAX_DROPPED_IDS + 50)
            .map(|i| format!("\"{i}\""))
            .collect();
        let line = format!(
            r#"{{"type":"control","action":"peer_message_status","status":"dropped","dropped_msg_ids":[{}]}}"#,
            ids.join(",")
        );
        let ControlLine::Receipt(r) = parse_control_line(&line) else {
            panic!("not a receipt");
        };
        assert_eq!(r.dropped_msg_ids.len(), MAX_DROPPED_IDS);
    }

    /// Tracing: TC-046-02
    /// ACs: FR-046-AC-2
    ///
    /// Every drift reason, and every known-ignored kind, is reachable and exact.
    #[test]
    fn each_line_class_is_exact() {
        let cases: [(&str, ControlLine); 14] = [
            ("", ControlLine::KnownIgnored(Ignored::Liveness)),
            ("  \n", ControlLine::KnownIgnored(Ignored::Liveness)),
            ("not json", ControlLine::NotJson),
            ("{\"type\":", ControlLine::NotJson),
            ("[1]", unrecognised(DriftReason::NotAnObject)),
            ("null", unrecognised(DriftReason::NotAnObject)),
            (
                r#"{"kind":"control"}"#,
                unrecognised(DriftReason::TypeMissing),
            ),
            (r#"{"type":"user"}"#, unrecognised(DriftReason::TypeUnknown)),
            (
                r#"{"type":"control"}"#,
                unrecognised(DriftReason::ActionMissing),
            ),
            (
                r#"{"type":"control","action":"other","orig_msg_id":"m"}"#,
                unrecognised(DriftReason::ActionUnknown),
            ),
            (
                r#"{"type":"control","action":"peer_message_status","orig_msg_id":"m"}"#,
                unrecognised(DriftReason::StatusMissing),
            ),
            (
                r#"{"type":"control","action":"peer_message_status","status":7,"orig_msg_id":"m"}"#,
                unrecognised(DriftReason::FieldShape),
            ),
            (
                r#"{"type":"control","action":"peer_message_status","status":"held","orig_msg_id":5}"#,
                unrecognised(DriftReason::FieldShape),
            ),
            (
                r#"{"type":"control","action":"peer_message_status","status":"held"}"#,
                ControlLine::KnownIgnored(Ignored::UnaddressedStatus),
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_control_line(line), expected, "{line:?}");
        }
    }

    proptest! {
        /// Tracing: TC-046-02
        /// ACs: FR-046-AC-2
        ///
        /// Total over arbitrary strings.
        #[test]
        fn parse_control_line_never_panics(s in ".*") {
            let _ = parse_control_line(&s);
        }

        /// Tracing: TC-046-02
        /// ACs: FR-046-AC-2
        ///
        /// Total over arbitrary bytes read lossily, as the socket reader does.
        #[test]
        fn parse_control_line_is_total_over_bytes(b in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse_control_line(&String::from_utf8_lossy(&b));
        }

        /// Tracing: TC-046-02
        /// ACs: FR-046-AC-2
        ///
        /// Total over JSON-shaped input with the fields swapped for random values.
        #[test]
        fn parse_control_line_is_total_over_shaped_json(
            t in "[a-z_]{0,8}", a in "[a-z_]{0,20}", s in "[a-z_]{0,8}", d in "[a-z_]{0,12}",
        ) {
            let line = format!(
                r#"{{"type":"{t}","action":"{a}","status":"{s}","status_detail":"{d}","orig_msg_id":"m"}}"#
            );
            let parsed = parse_control_line(&line);
            if t == "control" && a == "peer_message_status" {
                prop_assert_eq!(
                    parsed,
                    ControlLine::Receipt(Receipt {
                        orig_msg_id: Some("m".into()),
                        dropped_msg_ids: vec![],
                        verdict: classify(&s, Some(&d), None),
                    })
                );
            } else {
                let is_unrecognised = matches!(parsed, ControlLine::Unrecognised { .. });
                prop_assert!(is_unrecognised);
            }
        }

        /// Tracing: TC-046-02
        /// ACs: FR-046-AC-2
        ///
        /// classify is exhaustive: any triple yields a verdict, and only the
        /// listed status words yield anything but Unknown.
        #[test]
        fn classify_is_exhaustive(status in ".{0,12}", detail in proptest::option::of(".{0,12}"),
                                  reason in proptest::option::of(".{0,12}")) {
            let v = classify(&status, detail.as_deref(), reason.as_deref());
            let known = ["held", "delivered", "denied", "refused", "expired", "dropped"];
            prop_assert_eq!(v == Verdict::Unknown, !known.contains(&status.as_str()));
        }
    }
}
