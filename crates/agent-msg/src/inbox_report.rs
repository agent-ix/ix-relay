//! The JSON shape `agent-msg inbox --json` prints: a stable CLI output struct,
//! not `relay_core::Message` itself, so a caller that parses it (the opencode
//! plugin) is insulated from the storage type's own fields and only has to
//! trust the one field it actually needs (`rendered`) instead of re-deriving
//! the sender-line format itself.

use relay_core::{AddressBook, Error, Message, PeekResult, PopResult};
use serde::Serialize;

/// One message as the CLI reports it.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct InboxMessage {
    /// The sending lead.
    pub from: String,
    /// Who it was addressed to, as the sender wrote it (`"all"` for a broadcast).
    pub to: String,
    /// The raw body, unrendered.
    pub text: String,
    /// Sender's clock, Unix milliseconds.
    pub sent_at_ms: i64,
    /// The signed line a lead's context sees, from [`AddressBook::render`] —
    /// the one renderer, so callers never re-derive the `[ix-relay] from ...`
    /// format themselves. It signs with the sender's `server:workspace[:tab[:pane]]`
    /// address when known; `from` stays the bus name.
    pub rendered: String,
}

impl InboxMessage {
    fn new(m: &Message, book: &AddressBook) -> Self {
        Self {
            from: m.from.as_str().to_owned(),
            to: m.to.to_string(),
            text: m.text.clone(),
            sent_at_ms: m.sent_at_ms,
            rendered: book.render(m),
        }
    }
}

/// What `inbox --json` prints: the messages that decoded cleanly, and a
/// human-readable line per entry that didn't (so it's at least visible in the
/// JSON a caller parses, not only on stderr).
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct InboxReport {
    /// Delivered messages, oldest first.
    pub messages: Vec<InboxMessage>,
    /// One `Display` line per entry that was popped but failed to decode.
    pub rejected: Vec<String>,
}

impl InboxReport {
    /// The report for `popped`, with senders shown per `book`.
    pub fn new(popped: &PopResult, book: &AddressBook) -> Self {
        Self {
            messages: popped
                .messages
                .iter()
                .map(|m| InboxMessage::new(m, book))
                .collect(),
            rejected: popped.rejected.iter().map(Error::to_string).collect(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PeekMessage {
    pub from: String,
    pub to: String,
    pub text: String,
    pub sent_at_ms: i64,
    pub redelivered: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PeekReport {
    pub pending: usize,
    pub truncated: bool,
    pub messages: Vec<PeekMessage>,
    pub rejected: Vec<String>,
}

impl PeekReport {
    pub fn new(peek: &PeekResult) -> Self {
        Self {
            pending: peek.pending,
            truncated: peek.truncated,
            messages: peek
                .messages
                .iter()
                .map(|row| PeekMessage {
                    from: row.message.from.to_string(),
                    to: row.message.to.to_string(),
                    text: row.message.text.clone(),
                    sent_at_ms: row.message.sent_at_ms,
                    redelivered: row.redelivered,
                })
                .collect(),
            rejected: peek.rejected.iter().map(ToString::to_string).collect(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HistoryMessage {
    pub from: String,
    pub to: String,
    pub text: String,
    pub sent_at_ms: i64,
    pub redelivery_state: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HistoryReport {
    pub truncated: bool,
    pub messages: Vec<HistoryMessage>,
}

impl HistoryReport {
    pub fn new(messages: &[Message], truncated: bool) -> Self {
        Self {
            truncated,
            messages: messages
                .iter()
                .map(|m| HistoryMessage {
                    from: m.from.to_string(),
                    to: m.to.to_string(),
                    text: m.text.clone(),
                    sent_at_ms: m.sent_at_ms,
                    redelivery_state: "not_recorded",
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use relay_core::AgentName;
    use serde_json::json;

    use super::*;

    fn msg(from: &str, to: &str, text: &str) -> Message {
        Message {
            from: from.parse().unwrap(),
            to: to.parse().unwrap(),
            text: text.into(),
            sent_at_ms: 42,
        }
    }

    #[test]
    fn reports_a_direct_message_with_its_rendered_line() {
        let popped = PopResult {
            messages: vec![msg("qsl-lead", "tl-lead", "hi")],
            rejected: vec![],
        };
        let report = InboxReport::new(&popped, &AddressBook::default());
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "messages": [{
                    "from": "qsl-lead",
                    "to": "tl-lead",
                    "text": "hi",
                    "sent_at_ms": 42,
                    "rendered": "[ix-relay] from qsl-lead: hi",
                }],
                "rejected": [],
            })
        );
    }

    #[test]
    fn a_broadcast_renders_with_to_all_and_reports_to_as_all() {
        let popped = PopResult {
            messages: vec![msg("qsl-lead", "all", "merged AGE-2007")],
            rejected: vec![],
        };
        let report = InboxReport::new(&popped, &AddressBook::default());
        assert_eq!(report.messages[0].to, "all");
        assert_eq!(
            report.messages[0].rendered,
            "[ix-relay] from qsl-lead (to all): merged AGE-2007"
        );
    }

    #[test]
    fn a_rejected_entry_is_reported_as_a_display_string() {
        let popped = PopResult {
            messages: vec![],
            rejected: vec![Error::UnknownAgent(
                "nope-lead".parse::<AgentName>().unwrap(),
            )],
        };
        let report = InboxReport::new(&popped, &AddressBook::default());
        assert!(report.messages.is_empty());
        assert_eq!(report.rejected.len(), 1);
        assert!(
            report.rejected[0].contains("nope-lead"),
            "rejected[0]: {}",
            report.rejected[0]
        );
    }

    /// Trace: FR-051-AC-1
    #[test]
    fn peek_and_history_reports_pin_fields_and_do_not_invent_redelivery_state() {
        let message = msg("qsl-lead", "all", "first line\nsecond\u{1b}[31m");
        let peek = PeekReport::new(&PeekResult {
            messages: vec![relay_core::PeekedMessage {
                message: message.clone(),
                redelivered: true,
            }],
            rejected: vec![],
            pending: 7,
            truncated: true,
        });
        assert_eq!(
            serde_json::to_value(peek).unwrap(),
            json!({"pending":7,"truncated":true,"messages":[{
                "from":"qsl-lead","to":"all","text":"first line\nsecond\u{1b}[31m",
                "sent_at_ms":42,"redelivered":true
            }],"rejected":[]})
        );
        let history = HistoryReport::new(&[message], true);
        assert!(history.truncated);
        assert_eq!(history.messages[0].redelivery_state, "not_recorded");
        assert_eq!(history.messages[0].to, "all");
    }
}
