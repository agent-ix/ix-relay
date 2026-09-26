//! The strong acknowledgement: our `msg_id` as the `origin.msg_id` of a line in
//! the receiver's transcript (`~/.claude/projects/<slug>/<sessionId>.jsonl`).
//!
//! **Provenance:** captured live from Claude Code 2.1.283: consuming a peer
//! message writes a JSON line with `origin.msg_id` equal to the sender's id, and
//! only when the id is a UUID. Everything else on the line is ignored. This is
//! pure scanning of text already read; the file access stays with the caller.
//!
//! The scan is drift-aware: a line that *contains* our id but has no
//! structured `origin.msg_id` is [`TranscriptMatch::ShapeUnrecognised`], not
//! "absent", because Claude Code may have renamed the field.

use serde::Deserialize;

/// The part of a transcript line this reads.
#[derive(Debug, Deserialize)]
struct Line {
    #[serde(default)]
    origin: Option<Origin>,
}

/// The `origin` object of a transcript line.
#[derive(Debug, Deserialize)]
struct Origin {
    #[serde(default)]
    msg_id: Option<String>,
}

/// What one scan of transcript text found about one message id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TranscriptMatch {
    /// No line mentions the id: not consumed yet.
    Absent,
    /// A line contains the id's text but has no structured `origin.msg_id`
    /// (or an `origin` of an unknown shape): the shape may have changed, so the
    /// ack is ambiguous. Counted as drift.
    ShapeUnrecognised,
    /// A line records the id as its `origin.msg_id`: consumed.
    Acked,
}

/// Scans `bytes` (whole lines of a transcript, read lossily as UTF-8) for
/// `msg_id`. `Acked` beats `ShapeUnrecognised` beats `Absent`. Total over any
/// input. An empty `msg_id` matches nothing.
#[must_use]
pub fn scan(bytes: &[u8], msg_id: &str) -> TranscriptMatch {
    if msg_id.is_empty() {
        return TranscriptMatch::Absent;
    }
    let text = String::from_utf8_lossy(bytes);
    let mut found = TranscriptMatch::Absent;
    for line in text.lines().filter(|l| l.contains(msg_id)) {
        match serde_json::from_str::<Line>(line) {
            Ok(Line {
                origin: Some(Origin {
                    msg_id: Some(id), ..
                }),
            }) => {
                if id == msg_id {
                    return TranscriptMatch::Acked;
                }
                // Structured, and another message's id: not ours, not drift.
            }
            _ => found = TranscriptMatch::ShapeUnrecognised,
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Tracing: TC-046-06
    /// ACs: FR-046-AC-6
    #[test]
    fn a_structured_origin_is_the_only_ack() {
        let acked = br#"{"type":"user","origin":{"kind":"peer","msg_id":"id-9"},"x":1}"#;
        assert_eq!(scan(acked, "id-9"), TranscriptMatch::Acked);
        // Another message's id that merely contains ours as text: structured, not ours.
        assert_eq!(
            scan(br#"{"origin":{"msg_id":"id-90"}}"#, "id-9"),
            TranscriptMatch::Absent
        );
        assert_eq!(scan(b"", "id-9"), TranscriptMatch::Absent);
        assert_eq!(scan(b"{\"a\":1}\n", "id-9"), TranscriptMatch::Absent);
    }

    /// Tracing: TC-046-06
    /// ACs: FR-046-AC-6
    #[test]
    fn a_line_with_our_id_and_no_structured_origin_is_shape_unrecognised() {
        for line in [
            r#"{"other":{"msg_id":"id-9"}}"#,
            r#"{"origin":{"message_id":"id-9"}}"#,
            r#"{"origin":"id-9"}"#,
            r#"{"origin":{"msg_id":9,"note":"id-9"}}"#,
            r#"{"origin":null,"t":"id-9"}"#,
            r#"{"type":"queue-operation","content":"has id-9 in it"}"#,
            "not json id-9",
            "[\"id-9\"]",
        ] {
            assert_eq!(
                scan(line.as_bytes(), "id-9"),
                TranscriptMatch::ShapeUnrecognised,
                "{line}"
            );
        }
    }

    /// Tracing: TC-046-06
    /// ACs: FR-046-AC-6
    #[test]
    fn an_ack_anywhere_beats_an_unrecognised_line() {
        let text = "{\"other\":\"id-9\"}\n{\"origin\":{\"msg_id\":\"id-9\"}}\n";
        assert_eq!(scan(text.as_bytes(), "id-9"), TranscriptMatch::Acked);
        let text = "{\"origin\":{\"msg_id\":\"id-9\"}}\n{\"other\":\"id-9\"}\n";
        assert_eq!(scan(text.as_bytes(), "id-9"), TranscriptMatch::Acked);
    }

    /// Tracing: TC-046-06
    /// ACs: FR-046-AC-6
    #[test]
    fn an_empty_id_matches_nothing() {
        assert_eq!(
            scan(b"{\"origin\":{\"msg_id\":\"\"}}", ""),
            TranscriptMatch::Absent
        );
    }

    proptest! {
        /// Tracing: TC-046-02
        /// ACs: FR-046-AC-2
        #[test]
        fn scan_is_total_over_bytes(b in proptest::collection::vec(any::<u8>(), 0..512), id in ".{0,8}") {
            let _ = scan(&b, &id);
        }

        /// Tracing: TC-046-06
        /// ACs: FR-046-AC-6
        ///
        /// A line with a structured origin for our id is always acked, whatever
        /// surrounds it.
        #[test]
        fn a_structured_ack_is_always_found(pre in "[^\\n]{0,40}", post in "[^\\n]{0,40}", id in "[a-f0-9-]{1,36}") {
            let line = format!("{{\"origin\":{{\"msg_id\":\"{id}\"}}}}");
            let text = format!("{pre}\n{line}\n{post}\n");
            prop_assert_eq!(scan(text.as_bytes(), &id), TranscriptMatch::Acked);
        }
    }
}
