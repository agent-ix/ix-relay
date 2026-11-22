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
    /// Public target snapshot at enqueue.
    pub target: String,
    /// Source event time, unknown when it cannot be established.
    pub event_time_ms: Option<i64>,
    /// Time the relay recorded the evidence.
    pub observed_at_ms: i64,
    /// Evidence supporting a delivered outcome.
    pub evidence: Option<DeliveryEvidence>,
    /// Source-owned reason code, never message content or arbitrary error text.
    pub reason: Option<String>,
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
