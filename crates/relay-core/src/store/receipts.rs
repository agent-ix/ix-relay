//! Atomic receipt writes and side-effect-free bounded exports.

use std::{collections::BTreeMap, sync::LazyLock};

use redis::streams::StreamRangeReply;
use serde::Deserialize;

use super::{LOG_CAP, Relay, decode, encode, inflight::script};
use crate::{
    AgentName, BatchId, Error, Message, Now, Popper, Result, Target,
    receipts::{
        DeliveryEvidence, OrderedReceipt, RECEIPT_EVENT_CAP, RECEIPT_PAGE_CAP, RECEIPT_WINDOW_MS,
        ReceiptCoverageReason, ReceiptEvent, ReceiptExport, ReceiptRequest,
    },
};

static SEND_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    script(
        include_str!("lua/common.lua"),
        include_str!("lua/receipt_send.lua"),
    )
});

static OBSERVE_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    script(
        include_str!("lua/common.lua"),
        include_str!("lua/receipt_observe.lua"),
    )
});

/// The stored envelope is distinct from the public message/history shape.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMessage {
    from: AgentName,
    to: Target,
    text: String,
    sent_at_ms: i64,
    #[serde(default)]
    forwarded: bool,
    #[serde(default, rename = "_relay")]
    metadata: Option<StoredMetadata>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMetadata {
    original: String,
    #[serde(default)]
    attempt: Option<String>,
    #[serde(default)]
    delivered: Option<DeliveryEvidence>,
}

pub(crate) fn decode_message(bytes: &[u8]) -> std::result::Result<Message, serde_json::Error> {
    let stored: StoredMessage = serde_json::from_slice(bytes)?;
    // Metadata belongs to persistence, never to the recipient's public payload.
    if let Some(meta) = stored.metadata {
        let _ = (meta.original, meta.attempt, meta.delivered);
    }
    Ok(Message {
        from: stored.from,
        to: stored.to,
        text: stored.text,
        sent_at_ms: stored.sent_at_ms,
        forwarded: stored.forwarded,
    })
}

impl Relay {
    pub(super) fn enqueue_receipted(
        &mut self,
        message: &Message,
        recipients: &[AgentName],
        publications: &[(String, String)],
    ) -> Result<bool> {
        let mut command = redis::cmd("EVAL");
        command
            .arg(SEND_SCRIPT.as_str())
            .arg(publications.len() + recipients.len() + 1);
        for (key, _) in publications {
            command.arg(key);
        }
        for recipient in recipients {
            command.arg(self.keys.inbox_key(recipient));
        }
        command
            .arg(self.keys.log_key())
            .arg(publications.len())
            .arg(recipients.len());
        for (_, publication) in publications {
            command.arg(publication);
        }
        Ok(command
            .arg(encode(message))
            .arg(LOG_CAP)
            .arg("")
            .arg(self.keys.receipt_stream_key())
            .arg(self.keys.receipt_meta_key())
            .query(&mut self.conn)?)
    }

    /// Persists a transport's successful observation for an existing attempt.
    /// Repeating the observation or naming an expired attempt creates no duplicate.
    /// A failure leaves the pending-observation marker visible to receipt readers.
    pub fn observe_delivery(
        &mut self,
        agent: &AgentName,
        batch: BatchId,
        popper: Popper,
        evidence: DeliveryEvidence,
        now: Now,
    ) -> Result<usize> {
        let evidence = match evidence {
            DeliveryEvidence::HookStdout => "hook_stdout",
            DeliveryEvidence::HerdrPrompt => "herdr_prompt",
            DeliveryEvidence::SocketConfirmed => "socket_confirmed",
            DeliveryEvidence::SocketUnconfirmed => "socket_unconfirmed",
        };
        let count: i64 = redis::cmd("EVAL")
            .arg(OBSERVE_SCRIPT.as_str())
            .arg(1)
            .arg(self.keys.inflight_key(agent))
            .arg(batch.get())
            .arg(popper.letter().to_string())
            .arg(evidence)
            .arg(now.arg())
            .arg(self.keys.receipt_stream_key())
            .arg(self.keys.receipt_meta_key())
            .query(&mut self.conn)?;
        usize::try_from(count).map_err(|_| Error::InflightUnreadable {
            lead: agent.clone(),
        })
    }

    /// Reads receipt history without changing any inbox, lease or receipt state.
    /// Unreadable source evidence is returned as explicitly unavailable coverage.
    /// The request's interval is measured by observation time, including when the
    /// source event time is unknown or moves backwards.
    pub fn receipt_export(&mut self, request: &ReceiptRequest) -> Result<ReceiptExport> {
        validate_request(request)?;
        let mut export = empty_export(request);
        let result = self.read_receipts();
        let Ok((metadata, records)) = result else {
            export
                .reasons
                .push(ReceiptCoverageReason::SourceUnavailable);
            return Ok(export);
        };
        populate_export(request, &metadata, records, &mut export)?;
        Ok(export)
    }

    fn read_receipts(&mut self) -> Result<(BTreeMap<String, String>, Vec<OrderedReceipt>)> {
        let mut pipeline = redis::pipe();
        pipeline
            .atomic()
            .cmd("HGETALL")
            .arg(self.keys.receipt_meta_key())
            .cmd("XRANGE")
            .arg(self.keys.receipt_stream_key())
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(RECEIPT_EVENT_CAP + 1);
        let (metadata, stream): (BTreeMap<String, String>, StreamRangeReply) =
            pipeline.query(&mut self.conn)?;
        if stream.ids.len() > RECEIPT_EVENT_CAP {
            return Err(Error::InvalidName {
                name: "receipt event bound exceeded".into(),
            });
        }
        let key = self.keys.receipt_stream_key();
        let mut events = Vec::with_capacity(stream.ids.len());
        for row in stream.ids {
            let json = row
                .get::<String>("event")
                .ok_or_else(|| Error::InvalidName {
                    name: "receipt event is missing".into(),
                })?;
            let event: ReceiptEvent = decode(&key, &json)?;
            events.push(OrderedReceipt {
                cursor: row.id,
                event,
            });
        }
        Ok((metadata, events))
    }
}

fn validate_request(request: &ReceiptRequest) -> Result<()> {
    if request.since_ms > request.until_ms
        || request.until_ms > request.now_ms
        || !(1..=RECEIPT_PAGE_CAP).contains(&request.page_size)
        || request
            .after
            .as_ref()
            .is_some_and(|cursor| cursor_parts(cursor).is_none())
    {
        return Err(Error::InvalidName {
            name: "invalid receipt interval, page size or cursor".into(),
        });
    }
    Ok(())
}

fn cursor_parts(cursor: &str) -> Option<(u64, u64)> {
    let (time, sequence) = cursor.split_once('-')?;
    Some((time.parse().ok()?, sequence.parse().ok()?))
}

fn empty_export(request: &ReceiptRequest) -> ReceiptExport {
    ReceiptExport {
        requested_since_ms: request.since_ms,
        requested_until_ms: request.until_ms,
        covered_since_ms: None,
        covered_until_ms: None,
        instrumentation_start_ms: None,
        retention_boundary_ms: request.now_ms.saturating_sub(RECEIPT_WINDOW_MS),
        complete: false,
        reasons: Vec::new(),
        events: Vec::new(),
        next_cursor: None,
        has_more: false,
    }
}

fn populate_export(
    request: &ReceiptRequest,
    metadata: &BTreeMap<String, String>,
    records: Vec<OrderedReceipt>,
    export: &mut ReceiptExport,
) -> Result<()> {
    let number = |key: &str| metadata.get(key).and_then(|v| v.parse::<i64>().ok());
    let Some(start) = number("instrumentation_start_ms") else {
        export
            .reasons
            .push(ReceiptCoverageReason::BeforeInstrumentation);
        return Ok(());
    };
    export.instrumentation_start_ms = Some(start);
    if request.since_ms < start {
        export
            .reasons
            .push(ReceiptCoverageReason::BeforeInstrumentation);
    }
    if let Some(evicted) = number("evicted_through_ms") {
        export.retention_boundary_ms = export.retention_boundary_ms.max(evicted.saturating_add(1));
    }
    if request.since_ms < export.retention_boundary_ms {
        export.reasons.push(ReceiptCoverageReason::Retention);
    }
    if let (Some(first), Some(last)) = (number("gap_start"), number("gap_end"))
        && first <= request.until_ms
        && last >= request.since_ms
    {
        export.reasons.push(ReceiptCoverageReason::RecordingGap);
    }
    if metadata.iter().any(|(key, value)| {
        key.starts_with("pending:")
            && value
                .parse::<i64>()
                .is_ok_and(|start| start <= request.until_ms)
    }) {
        export
            .reasons
            .push(ReceiptCoverageReason::ObservationPending);
    }
    let after = request.after.as_deref().and_then(cursor_parts);
    let first_cursor = records.first().and_then(|row| cursor_parts(&row.cursor));
    if after
        .zip(first_cursor)
        .is_some_and(|(after, first)| after < first)
    {
        export.reasons.push(ReceiptCoverageReason::CursorEvicted);
    }
    export.covered_since_ms = Some(
        request
            .since_ms
            .max(start)
            .max(export.retention_boundary_ms),
    );
    export.covered_until_ms = Some(request.until_ms);
    for record in records {
        let Some(position) = cursor_parts(&record.cursor) else {
            return Err(Error::InvalidName {
                name: "invalid stored receipt cursor".into(),
            });
        };
        let time = record.event.observed_at_ms;
        if time < request.since_ms
            || time > request.until_ms
            || time < request.now_ms.saturating_sub(RECEIPT_WINDOW_MS)
            || after.is_some_and(|after| position <= after)
        {
            continue;
        }
        if export.events.len() == request.page_size {
            export.has_more = true;
            export.next_cursor = export.events.last().map(|row| row.cursor.clone());
            break;
        }
        export.events.push(record);
    }
    export.complete = export.reasons.is_empty();
    Ok(())
}
