//! The JSON shape `agent-msg inbox --json` and `inbox --peek --json` print: a
//! stable CLI output struct, not `relay_core::Message` itself, so a caller
//! that parses it (the opencode plugin) is insulated from the storage type's
//! own fields and only has to trust the one field it actually needs
//! (`rendered`) instead of re-deriving the sender-line format itself.

use ix_trace_rs::implements;
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
            from: book.label(&m.from),
            to: match &m.to {
                relay_core::Target::All => "all".into(),
                relay_core::Target::Agent(id) => book.label(id),
            },
            text: m.text.clone(),
            sent_at_ms: m.sent_at_ms,
            rendered: book.render(m),
        }
    }
}

/// What `inbox --json` and `inbox --peek --json` print: the messages that
/// decoded cleanly, and a human-readable line per entry that didn't (so it's
/// at least visible in the JSON a caller parses, not only on stderr).
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct InboxReport {
    /// Messages returned by the inbox read, oldest first.
    pub messages: Vec<InboxMessage>,
    /// One `Display` line per entry that failed to decode.
    pub rejected: Vec<String>,
}

impl InboxReport {
    /// The report for `popped`, with senders shown per `book`.
    #[implements("FR-049-AC-1")]
    pub fn new(popped: &PopResult, book: &AddressBook) -> Self {
        Self::from_parts(&popped.messages, &popped.rejected, book)
    }

    /// The report for `peeked`, with senders shown per `book`.
    #[implements("FR-012-AC-6")]
    pub fn from_peek(peeked: &PeekResult, book: &AddressBook) -> Self {
        Self::from_parts(&peeked.messages, &peeked.rejected, book)
    }

    #[implements("FR-012-AC-6", "FR-049-AC-1")]
    fn from_parts(
        messages: &[relay_core::receipts::InboxEntry],
        rejected: &[Error],
        book: &AddressBook,
    ) -> Self {
        Self {
            messages: messages
                .iter()
                .map(|m| InboxMessage::new(m, book))
                .collect(),
            rejected: rejected.iter().map(Error::to_string).collect(),
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
            forwarded: false,
        }
    }

    fn entries(message: Message) -> Vec<relay_core::receipts::InboxEntry> {
        let bytes = serde_json::to_vec(&message).unwrap();
        vec![relay_core::receipts::InboxEntry::try_from(bytes.as_slice()).unwrap()]
    }

    #[test]
    fn reports_a_direct_message_with_its_rendered_line() {
        let popped = PopResult {
            messages: entries(msg("draco:qsl:code", "terra:tl:code", "hi")),
            rejected: vec![],
        };
        let report = InboxReport::new(&popped, &AddressBook::default());
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "messages": [{
                    "from": "draco:qsl:code",
                    "to": "terra:tl:code",
                    "text": "hi",
                    "sent_at_ms": 42,
                    "rendered": "[ix-relay] from draco:qsl:code: hi",
                }],
                "rejected": [],
            })
        );
    }

    #[test]
    fn a_broadcast_renders_with_to_all_and_reports_to_as_all() {
        let popped = PopResult {
            messages: entries(msg("draco:qsl:code", "all", "merged AGE-2007")),
            rejected: vec![],
        };
        let report = InboxReport::new(&popped, &AddressBook::default());
        assert_eq!(report.messages[0].to, "all");
        assert_eq!(
            report.messages[0].rendered,
            "[ix-relay] from draco:qsl:code (to all): merged AGE-2007"
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
}
