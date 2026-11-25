//! Content-free, bounded transport lifecycle evidence (FR-067).

use serde::{Deserialize, Serialize};

/// One plain inbox entry, retaining its storage identity for an explicit requeue.
/// Serialization and dereferencing expose only the public message content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxEntry {
    /// Recipient content; receipt metadata never appears in this object.
    #[serde(flatten)]
    pub message: crate::Message,
    #[serde(skip)]
    pub(crate) stored: Vec<u8>,
}

impl std::ops::Deref for InboxEntry {
    type Target = crate::Message;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl PartialEq<crate::Message> for InboxEntry {
    fn eq(&self, other: &crate::Message) -> bool {
        self.message == *other
    }
}

impl TryFrom<&[u8]> for InboxEntry {
    type Error = serde_json::Error;

    fn try_from(stored: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self {
            message: crate::store::receipts::decode_message(stored)?,
            stored: stored.to_vec(),
        })
    }
}

/// Maximum retained receipt events for one relay namespace.
pub const RECEIPT_EVENT_CAP: usize = 10_000;
/// Maximum events returned in one export page.
pub const RECEIPT_PAGE_CAP: usize = 1_000;
/// Export age bound, measured using receipt observation time.
pub const RECEIPT_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// A transport observation, never evidence that a recipient read the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryEvidence {
    /// Hook output was written and flushed.
    HookStdout,
    /// Herdr reported successful input submission.
    HerdrPrompt,
    /// The socket receiver's transcript confirmed consumption.
    SocketConfirmed,
    /// The socket receiver reported delivery without transcript confirmation.
    SocketUnconfirmed,
    /// The recipient took the message with a plain inbox read.
    InboxRead,
}

impl DeliveryEvidence {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::HookStdout => "hook_stdout",
            Self::HerdrPrompt => "herdr_prompt",
            Self::SocketConfirmed => "socket_confirmed",
            Self::SocketUnconfirmed => "socket_unconfirmed",
            Self::InboxRead => "inbox_read",
        }
    }
}

/// Source-reported cause of an explicit batch dead-letter decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadLetterReason {
    /// The receiver explicitly denied the delivery.
    ReceiverDenied,
    /// The receiver's approval hold expired unanswered.
    HoldExpired,
    /// Persisted settlement evidence did not retain a more specific cause.
    Unknown,
}

impl DeadLetterReason {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::ReceiverDenied => "receiver_denied",
            Self::HoldExpired => "hold_expired",
            Self::Unknown => "unknown",
        }
    }
}

/// The actual transition recorded by the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    /// Original send committed to a recipient's inbox.
    Queued,
    /// A consumer claimed a batch for delivery.
    AttemptStarted,
    /// A transport reported successful submission.
    Delivered,
    /// The relay committed settlement of a valid in-flight attempt.
    Acknowledged,
    /// An attempt's lease expired.
    TimedOut,
    /// A definitely unsent attempt returned to its inbox.
    Requeued,
    /// A previously expired message was claimed again.
    Redelivered,
    /// A message moved to retained dead letters.
    DeadLettered,
    /// A plain at-most-once inbox read removed the entry.
    Removed,
    /// The recipient session recorded its first prompt or tool activity after delivery.
    Acted,
    /// An order reached its ack deadline without an `Acted` receipt.
    AckExpired,
}

/// One stable, content-free lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptEvent {
    /// Stable identity of this transition, unchanged by repeated settlement.
    pub event_id: String,
    /// Stable identity of the original enqueue, including across redeliveries.
    pub original_id: String,
    /// Claimed attempt, absent on enqueue and plain inbox removal.
    pub attempt_id: Option<String>,
    /// The observed transition.
    pub outcome: ReceiptOutcome,
    /// Source-native sender identity at enqueue.
    pub sender: String,
    /// Source-native immutable recipient queue identity at enqueue.
    pub target: String,
    /// Source event time, unknown when it cannot be established.
    pub event_time_ms: Option<i64>,
    /// Time the relay recorded the evidence.
    pub observed_at_ms: i64,
    /// Evidence supporting a delivered outcome.
    pub evidence: Option<DeliveryEvidence>,
    /// Source-owned reason code, never message content or arbitrary error text.
    pub reason: Option<String>,
    /// The recipient session, on `Acted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The ticket an order expects its recipient to take up, as sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_ticket: Option<String>,
    /// When the acted-on message was delivered, on `Acted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at_ms: Option<i64>,
}

/// A receipt together with its ordering position in the retained stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderedReceipt {
    /// Redis stream position, monotonic even if source time moves backwards.
    pub cursor: String,
    /// Transport lifecycle evidence.
    pub event: ReceiptEvent,
}

/// Why the requested interval cannot be reported as complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptCoverageReason {
    /// The request predates receipt instrumentation.
    BeforeInstrumentation,
    /// A seven-day or event-count boundary excluded requested evidence.
    Retention,
    /// A continuation's position has already been evicted.
    CursorEvicted,
    /// The supplied position is not part of the available source history.
    CursorUnavailable,
    /// A recorded observation may be missing after an interrupted attempt.
    RecordingGap,
    /// A current attempt has not yet persisted its transport observation.
    ObservationPending,
    /// The Redis source or its stored receipt representation cannot be read.
    SourceUnavailable,
}

/// A read-only, bounded receipt history request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptRequest {
    /// Inclusive observation-time lower bound.
    pub since_ms: i64,
    /// Inclusive observation-time upper bound.
    pub until_ms: i64,
    /// Read time used for the age bound; production passes its current clock.
    pub now_ms: i64,
    /// Maximum number of events; must be in `1..=RECEIPT_PAGE_CAP`.
    pub page_size: usize,
    /// Last ordering position from the preceding page.
    pub after: Option<String>,
}

/// A page of read-only history and explicit coverage limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptExport {
    /// Requested inclusive interval.
    pub requested_since_ms: i64,
    /// Requested inclusive interval.
    pub requested_until_ms: i64,
    /// Earliest interval boundary for which retained evidence may be complete.
    pub covered_since_ms: Option<i64>,
    /// Upper bound actually considered, excluding future time.
    pub covered_until_ms: Option<i64>,
    /// First instrumented transition, absent if no instrumentation is available.
    pub instrumentation_start_ms: Option<i64>,
    /// Age/count retention boundary.
    pub retention_boundary_ms: i64,
    /// True only when the entire requested interval has available evidence.
    pub complete: bool,
    /// The limitations that prevent complete coverage.
    pub reasons: Vec<ReceiptCoverageReason>,
    /// Ordered content-free events in this page.
    pub events: Vec<OrderedReceipt>,
    /// Resume after this position when another page remains.
    pub next_cursor: Option<String>,
    /// Whether additional retained matching events remain.
    pub has_more: bool,
}

impl ReceiptExport {
    /// Reports an unavailable source for a validated interval without claiming
    /// that the missing history is empty or disclosing connection credentials.
    pub fn source_unavailable(request: &ReceiptRequest) -> crate::Result<Self> {
        crate::store::receipts::validate_request(request)?;
        let mut report = crate::store::receipts::empty_export(request);
        report
            .reasons
            .push(ReceiptCoverageReason::SourceUnavailable);
        Ok(report)
    }
}

/// Shortest order ack deadline, in milliseconds.
pub const ORDER_ACK_MIN_MS: i64 = 10_000;
/// Longest order ack deadline, in milliseconds.
pub const ORDER_ACK_MAX_MS: i64 = 24 * 60 * 60 * 1_000;
/// Default order ack deadline, in milliseconds.
pub const ORDER_ACK_DEFAULT_MS: i64 = 5 * 60 * 1_000;
/// Longest `--expect-ticket`, in bytes.
pub const ORDER_EXPECT_MAX_BYTES: usize = 64;
/// Bucket upper bounds of the delivered-to-acted latency, in seconds.
pub const ORDER_LATENCY_BOUNDS_SECS: [u64; 7] = [1, 5, 15, 60, 300, 900, 3_600];

/// A message sent as an order (FR-071): it has an ack deadline and, optionally, the
/// ticket its recipient is expected to take up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderSpec {
    ack_deadline_ms: i64,
    expect_ticket: Option<String>,
}

impl OrderSpec {
    /// An order with `ack_deadline_ms` and an optional expected ticket.
    ///
    /// # Errors
    /// Fails when the deadline is outside 10 seconds to 24 hours, or the ticket is
    /// empty, longer than 64 bytes or holds a control character.
    pub fn new(ack_deadline_ms: i64, expect_ticket: Option<String>) -> Result<Self, String> {
        if !(ORDER_ACK_MIN_MS..=ORDER_ACK_MAX_MS).contains(&ack_deadline_ms) {
            return Err("an ack deadline is between 10 seconds and 24 hours".into());
        }
        if let Some(ticket) = &expect_ticket
            && (ticket.is_empty()
                || ticket.len() > ORDER_EXPECT_MAX_BYTES
                || ticket.chars().any(char::is_control))
        {
            return Err("an expected ticket is 1 to 64 bytes without control characters".into());
        }
        Ok(Self {
            ack_deadline_ms,
            expect_ticket,
        })
    }

    /// The ack deadline in milliseconds.
    #[must_use]
    pub fn ack_deadline_ms(&self) -> i64 {
        self.ack_deadline_ms
    }

    /// The expected ticket.
    #[must_use]
    pub fn expect_ticket(&self) -> Option<&str> {
        self.expect_ticket.as_deref()
    }
}

/// An order whose ack deadline passed, as the watcher escalates it. Holds no message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredOrder {
    /// The order's original message identity.
    pub original_id: String,
    /// Sender address.
    pub sender: String,
    /// Recipient address as sent.
    pub recipient: String,
    /// Immutable recipient queue identity.
    pub queue: String,
    /// The last receipt outcome before expiry.
    pub last_state: String,
    /// The expected ticket, if the order carried one.
    pub expect_ticket: Option<String>,
}

/// Most orders [`crate::Relay::expired_orders`] returns.
pub const EXPIRED_ORDERS_CAP: usize = 500;

/// Most index rows one [`crate::Relay::expired_orders`] call examines.
pub const EXPIRED_ORDERS_SCAN_CAP: usize = 1_000;

/// Where the escalation of an expired order stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderEscalationState {
    /// Not filed yet; the watcher still retries it.
    Pending,
    /// Filed once.
    Filed,
    /// Given up after repeated failures.
    Failed,
    /// The relay stored a value this version does not know.
    Unknown,
}

impl OrderEscalationState {
    /// The name used in JSON and in plain output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Filed => "filed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

/// An order whose `AckExpired` the relay recorded and that was not `Acted`. Holds no
/// message text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiredOrderState {
    /// The order's original message identity.
    pub original_id: String,
    /// Recipient lane address, in the spelling of the host lane lists (FR-069) when the
    /// lane is listed, else as stored at send time; compare it as an [`crate::Address`].
    pub recipient: String,
    /// The ticket the order expected its recipient to take up.
    pub expect_ticket: Option<String>,
    /// Relay time the order was enqueued, in Unix milliseconds.
    pub enqueued_at_ms: i64,
    /// Relay time the ack deadline passed, in Unix milliseconds: from `Delivered`,
    /// else from enqueue.
    pub deadline_ms: i64,
    /// Relay time `AckExpired` was recorded, in Unix milliseconds.
    pub expired_at_ms: i64,
    /// Whether a `Delivered` receipt was recorded before expiry; a delivery after expiry
    /// does not set it.
    pub delivered_before_expiry: bool,
    /// The last receipt outcome before expiry.
    pub last_state: String,
    /// The escalation's state.
    pub escalation: OrderEscalationState,
}

/// The expired, un-acted orders, newest expiry first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiredOrders {
    /// At most [`EXPIRED_ORDERS_CAP`] orders.
    pub orders: Vec<ExpiredOrderState>,
    /// Rows the relay could not decode; they are not in `orders`.
    pub skipped: u64,
    /// When the relay began indexing expiries, in Unix milliseconds. Orders that expired
    /// earlier, or while a relay-watcher that predates the index recorded them, are not
    /// listed. `None` until the first expiry scan has run.
    pub indexed_since_ms: Option<i64>,
    /// Qualifying orders may be missing: more exist than were returned, the scan stopped at
    /// [`EXPIRED_ORDERS_SCAN_CAP`] index rows, or the index dropped live rows at its size bound.
    pub truncated: bool,
}

/// Delivered-to-acted latency of orders, as per-bucket counts over
/// [`ORDER_LATENCY_BOUNDS_SECS`] plus a last bucket for larger values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderLatency {
    /// One count per bound and one more for larger values.
    pub buckets: [u64; 8],
    /// Observations.
    pub count: u64,
    /// Sum of the latencies, in milliseconds.
    pub sum_ms: u64,
}

/// What the order metrics read for one machine: its lanes' orders awaiting `Acted` by
/// recipient address, and the latency of orders delivered there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderStats {
    /// Orders with neither `Acted` nor `AckExpired`, by recipient address.
    pub unacted: std::collections::BTreeMap<String, u64>,
    /// Delivered-to-acted latency.
    pub latency: OrderLatency,
    /// Orders of the machine whose escalation kept failing and was given up.
    pub escalation_failed: u64,
}
