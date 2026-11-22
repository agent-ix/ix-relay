//! Receipt lifecycle evidence against an explicitly configured isolated Redis.
#![expect(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

use ix_trace_rs::trace;
use redis::Commands;
use relay_core::{
    AgentName, Claim, Message, Now, Popper, Registration, Relay,
    receipts::{
        DeliveryEvidence, RECEIPT_EVENT_CAP, RECEIPT_WINDOW_MS, ReceiptCoverageReason,
        ReceiptExport, ReceiptOutcome, ReceiptRequest,
    },
};
use relay_test_support::Bus;

fn name(value: &str) -> AgentName {
    value.parse().unwrap()
}

fn bus() -> Bus {
    let mut bus =
        Bus::connect("receipts").expect("receipt tests require isolated IX_RELAY_TEST_URL");
    bus.relay
        .register(&Registration {
            name: name("receiver"),
            machine: "fixture".parse().unwrap(),
            kind: "codex".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 0,
        })
        .unwrap();
    bus
}

fn send(bus: &mut Bus) {
    bus.relay
        .send(&Message {
            from: name("sender"),
            to: "receiver".parse().unwrap(),
            text: "secret body credential=private transcript".into(),
            sent_at_ms: 42,
            forwarded: false,
        })
        .unwrap();
}

fn now(bus: &mut Bus) -> i64 {
    let (seconds, micros): (i64, i64) = redis::cmd("TIME").query(&mut bus.raw).unwrap();
    seconds * 1_000 + micros / 1_000
}

fn request(bus: &mut Bus) -> ReceiptRequest {
    let now = now(bus);
    ReceiptRequest {
        since_ms: now - 1_000,
        until_ms: now,
        now_ms: now,
        page_size: 1_000,
        after: None,
    }
}

fn export(bus: &mut Bus) -> ReceiptExport {
    let request = request(bus);
    bus.relay.receipt_export(&request).unwrap()
}

fn claim(bus: &mut Bus, at: i64) -> relay_core::BudgetPopResult {
    bus.relay
        .pop_inbox_budget(
            &name("receiver"),
            NonZeroUsize::new(1).unwrap(),
            10_000,
            Claim::new(Popper::Watcher)
                .with_lease(Duration::from_millis(20))
                .at(Now::AtMs(at)),
        )
        .unwrap()
}

fn snapshot(bus: &mut Bus) -> BTreeMap<String, Vec<u8>> {
    bus.keys()
        .into_iter()
        .map(|key| {
            let bytes: Vec<u8> = redis::cmd("DUMP").arg(&key).query(&mut bus.raw).unwrap();
            (key, bytes)
        })
        .collect()
}

#[test]
#[trace("FR-067-AC-1", "FR-067-AC-7")]
fn receipts_lifecycle_separates_observation_from_settlement_without_content() {
    let mut bus = bus();
    send(&mut bus);
    let queued = export(&mut bus);
    assert_eq!(queued.events.len(), 1);
    assert_eq!(queued.events[0].event.outcome, ReceiptOutcome::Queued);
    let at = now(&mut bus);
    let batch = claim(&mut bus, at).batch.unwrap();
    assert_eq!(
        bus.relay
            .observe_delivery(
                &name("receiver"),
                batch,
                Popper::Watcher,
                DeliveryEvidence::SocketUnconfirmed,
                Now::AtMs(at)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        bus.relay
            .observe_delivery(
                &name("receiver"),
                batch,
                Popper::Watcher,
                DeliveryEvidence::SocketUnconfirmed,
                Now::AtMs(at + 1)
            )
            .unwrap(),
        0
    );
    assert!(
        bus.relay
            .ack(&name("receiver"), batch, Popper::Watcher)
            .unwrap()
    );
    assert!(
        !bus.relay
            .ack(&name("receiver"), batch, Popper::Watcher)
            .unwrap()
    );
    let report = export(&mut bus);
    let outcomes: Vec<_> = report.events.iter().map(|row| row.event.outcome).collect();
    assert_eq!(
        outcomes,
        [
            ReceiptOutcome::Queued,
            ReceiptOutcome::AttemptStarted,
            ReceiptOutcome::Delivered,
            ReceiptOutcome::Acknowledged
        ]
    );
    assert!(
        report
            .events
            .iter()
            .all(|row| row.event.original_id == queued.events[0].event.original_id)
    );
    assert_eq!(
        report.events[2].event.evidence,
        Some(DeliveryEvidence::SocketUnconfirmed)
    );
    assert_eq!(report.events[2].event.event_time_ms, None);
    assert_eq!(report.events[0].event.sender, "sender");
    assert_eq!(report.events[0].event.target, "receiver");
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("secret body"));
    assert!(!json.contains("private transcript"));
    assert!(!json.contains("credential="));
    let before = snapshot(&mut bus);
    assert_eq!(report.events, export(&mut bus).events);
    assert_eq!(
        before,
        snapshot(&mut bus),
        "export must change no Redis state"
    );
    assert_eq!(bus.relay.log_since(0).unwrap().len(), 1);
}

#[test]
#[trace("FR-067-AC-2", "FR-067-AC-3", "FR-067-AC-8")]
fn receipts_identical_sends_recover_with_distinct_attempts_and_stale_ack_is_inert() {
    let mut bus = bus();
    send(&mut bus);
    send(&mut bus);
    let queued = export(&mut bus);
    assert_ne!(
        queued.events[0].event.original_id,
        queued.events[1].event.original_id
    );
    let at = now(&mut bus);
    let old = claim(&mut bus, at).batch.unwrap();
    assert_eq!(
        bus.relay
            .reap_expired(&name("receiver"), Now::AtMs(at + 20))
            .unwrap()
            .requeued,
        1
    );
    bus.relay = Relay::connect(&bus.url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(bus.prefix.clone());
    let recovered = claim(&mut bus, at + 21);
    assert_eq!(recovered.redelivered, [true]);
    let current = recovered.batch.unwrap();
    assert_ne!(old, current);
    assert!(
        !bus.relay
            .ack(&name("receiver"), old, Popper::Watcher)
            .unwrap()
    );
    assert_eq!(bus.relay.inflight_len(&name("receiver")).unwrap(), 1);
    bus.relay
        .observe_delivery(
            &name("receiver"),
            current,
            Popper::Watcher,
            DeliveryEvidence::HerdrPrompt,
            Now::AtMs(at - 10),
        )
        .unwrap();
    bus.relay
        .ack(&name("receiver"), current, Popper::Watcher)
        .unwrap();
    let mut request = request(&mut bus);
    request.until_ms = at + 100;
    request.now_ms = at + 100;
    let report = bus.relay.receipt_export(&request).unwrap();
    let original = &queued.events[0].event.original_id;
    let events: Vec<_> = report
        .events
        .iter()
        .filter(|row| &row.event.original_id == original)
        .collect();
    assert!(
        events
            .iter()
            .any(|row| row.event.outcome == ReceiptOutcome::TimedOut)
    );
    assert!(
        events
            .iter()
            .any(|row| row.event.outcome == ReceiptOutcome::Redelivered)
    );
    let attempts: Vec<_> = events
        .iter()
        .filter(|row| row.event.outcome == ReceiptOutcome::AttemptStarted)
        .map(|row| row.event.attempt_id.clone().unwrap())
        .collect();
    assert_eq!(attempts.len(), 2);
    assert_ne!(attempts[0], attempts[1]);
    let position = |cursor: &str| {
        let (time, sequence) = cursor.split_once('-').unwrap();
        (
            time.parse::<u64>().unwrap(),
            sequence.parse::<u64>().unwrap(),
        )
    };
    assert!(
        events
            .windows(2)
            .all(|pair| position(&pair[0].cursor) < position(&pair[1].cursor))
    );
    assert_eq!(
        bus.relay.log_since(0).unwrap().len(),
        2,
        "recovery is not another original send"
    );
}

#[test]
#[trace("FR-067-AC-4", "FR-067-AC-6")]
fn receipts_failed_observation_survives_recovery_as_an_explicit_gap() {
    let mut bus = bus();
    send(&mut bus);
    let at = now(&mut bus);
    let batch = claim(&mut bus, at).batch.unwrap();
    let stream = bus.key("receipts");
    let saved = bus.key("saved-receipts");
    let _: () = bus.raw.rename(&stream, &saved).unwrap();
    let _: () = bus.raw.set(&stream, "wrong type").unwrap();
    assert!(
        bus.relay
            .observe_delivery(
                &name("receiver"),
                batch,
                Popper::Watcher,
                DeliveryEvidence::HookStdout,
                Now::AtMs(at + 1)
            )
            .is_err()
    );
    assert_eq!(
        export(&mut bus).reasons,
        [ReceiptCoverageReason::SourceUnavailable]
    );
    assert_eq!(bus.relay.inflight_len(&name("receiver")).unwrap(), 1);
    let _: () = bus.raw.del(&stream).unwrap();
    let _: () = bus.raw.rename(&saved, &stream).unwrap();
    bus.relay
        .reap_expired(&name("receiver"), Now::AtMs(at + 20))
        .unwrap();
    bus.relay = Relay::connect(&bus.url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(bus.prefix.clone());
    let mut request = request(&mut bus);
    request.until_ms = at + 100;
    request.now_ms = at + 100;
    let report = bus.relay.receipt_export(&request).unwrap();
    assert!(
        report
            .reasons
            .contains(&ReceiptCoverageReason::RecordingGap)
    );
    assert!(!report.events.iter().any(|row| matches!(
        row.event.outcome,
        ReceiptOutcome::Delivered | ReceiptOutcome::Acknowledged
    )));
    assert_eq!(bus.relay.inbox_len(&name("receiver")).unwrap(), 1);
}

#[test]
#[trace("FR-067-AC-5", "FR-067-AC-6", "FR-067-AC-7")]
fn receipts_pages_retention_and_empty_coverage_remain_truthful_and_read_only() {
    let mut bus = bus();
    for _ in 0..1_003 {
        send(&mut bus);
    }
    let mut request = request(&mut bus);
    request.page_size = 1_000;
    let first = bus.relay.receipt_export(&request).unwrap();
    assert_eq!(first.events.len(), 1_000);
    assert!(first.has_more);
    request.after = first.next_cursor.clone();
    let second = bus.relay.receipt_export(&request).unwrap();
    assert_eq!(second.events.len(), 3);
    assert!(!second.has_more);
    assert_ne!(
        first.events.last().unwrap().event.event_id,
        second.events[0].event.event_id
    );
    let before = snapshot(&mut bus);
    request.now_ms += RECEIPT_WINDOW_MS + 1_000;
    let expired = bus.relay.receipt_export(&request).unwrap();
    assert!(expired.events.is_empty());
    assert!(expired.reasons.contains(&ReceiptCoverageReason::Retention));
    assert_eq!(
        snapshot(&mut bus),
        before,
        "age expiry is a read filter, not a trim"
    );
    let first_id = first.events[0].event.original_id.clone();
    let at = request.now_ms;
    let batch = claim(&mut bus, at).batch.unwrap();
    let mut recent = ReceiptRequest {
        since_ms: at,
        until_ms: at + 1,
        now_ms: at + 1,
        page_size: 1_000,
        after: None,
    };
    let delivery = bus.relay.receipt_export(&recent).unwrap();
    assert_eq!(
        delivery.events[0].event.original_id, first_id,
        "receipt age expiry must not erase queued identity"
    );
    bus.relay
        .requeue_batch(&name("receiver"), batch, Popper::Watcher)
        .unwrap();
    recent.since_ms = at + 1;
    recent.until_ms = at + 1;
    let empty = bus.relay.receipt_export(&recent).unwrap();
    assert!(empty.events.is_empty());
    assert!(
        empty.complete,
        "a covered empty interval must be distinguishable from unavailable history"
    );
}

#[test]
#[trace("FR-067-AC-5", "FR-067-AC-6")]
fn receipts_count_bound_marks_an_evicted_continuation_incomplete() {
    let mut bus = bus();
    send(&mut bus);
    let oldest = export(&mut bus).events[0].cursor.clone();
    for _ in 0..RECEIPT_EVENT_CAP {
        send(&mut bus);
    }
    let count: usize = bus.raw.xlen(bus.key("receipts")).unwrap();
    assert_eq!(count, RECEIPT_EVENT_CAP);
    let mut request = request(&mut bus);
    request.after = Some(oldest);
    let report = bus.relay.receipt_export(&request).unwrap();
    assert!(
        report
            .reasons
            .contains(&ReceiptCoverageReason::CursorEvicted)
    );
    assert!(report.reasons.contains(&ReceiptCoverageReason::Retention));
    assert!(!report.complete);
}

#[test]
#[trace("FR-067-AC-2", "FR-067-AC-8")]
fn receipts_plain_pop_requeue_preserves_original_without_inventing_delivery() {
    let mut bus = bus();
    send(&mut bus);
    let original = export(&mut bus).events[0].event.original_id.clone();
    let popped = bus
        .relay
        .pop_inbox(&name("receiver"), NonZeroUsize::new(1).unwrap())
        .unwrap();
    assert_eq!(bus.relay.inflight_len(&name("receiver")).unwrap(), 0);
    bus.relay
        .requeue_front(&name("receiver"), &popped.messages)
        .unwrap();
    let at = now(&mut bus);
    let batch = claim(&mut bus, at).batch.unwrap();
    bus.relay
        .dead_letter_batch(&name("receiver"), batch, Popper::Watcher)
        .unwrap();
    let report = export(&mut bus);
    assert!(
        report
            .events
            .iter()
            .all(|row| row.event.original_id == original)
    );
    assert!(
        report
            .events
            .iter()
            .any(|row| row.event.outcome == ReceiptOutcome::Removed)
    );
    assert!(
        report
            .events
            .iter()
            .any(|row| row.event.outcome == ReceiptOutcome::DeadLettered)
    );
    assert!(!report.events.iter().any(|row| matches!(
        row.event.outcome,
        ReceiptOutcome::Delivered | ReceiptOutcome::Acknowledged
    )));
    assert_eq!(bus.relay.log_since(0).unwrap().len(), 1);
}
