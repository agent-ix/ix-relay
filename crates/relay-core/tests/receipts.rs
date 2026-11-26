//! Receipt lifecycle evidence against an explicitly configured isolated Redis.
#![expect(clippy::unwrap_used, reason = "test assertions")]

use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    num::NonZeroUsize,
    os::unix::net::UnixStream,
    thread,
    time::Duration,
};

use ix_trace_rs::trace;
use redis::{Commands, ConnectionAddr, IntoConnectionInfo};
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

fn bus() -> Option<Bus> {
    let mut bus = Bus::connect("receipts")?;
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
    Some(bus)
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
            let bytes: Vec<u8> = relay_test_support::canonical_dump(&mut bus.raw, &key);
            (key, bytes)
        })
        .collect()
}

fn resp_frame(reader: &mut impl BufRead) -> Vec<u8> {
    let mut frame = Vec::new();
    reader.read_until(b'\n', &mut frame).unwrap();
    assert!(
        !frame.is_empty(),
        "Redis frame must not end before its header"
    );
    let length = || {
        std::str::from_utf8(&frame[1..frame.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap()
    };
    match frame[0] {
        b'$' => {
            let length = length();
            if length >= 0 {
                let mut body = vec![0; usize::try_from(length).unwrap() + 2];
                reader.read_exact(&mut body).unwrap();
                frame.extend(body);
            }
        }
        b'*' => {
            for _ in 0..length().max(0) {
                frame.extend(resp_frame(reader));
            }
        }
        b'+' | b'-' | b':' => {}
        other => panic!("unsupported isolated fixture RESP frame {other}"),
    }
    frame
}

enum FixtureStream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl FixtureStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
            Self::Unix(stream) => stream.set_read_timeout(timeout),
        }
    }
}

impl Read for FixtureStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(output),
            Self::Unix(stream) => stream.read(output),
        }
    }
}

impl Write for FixtureStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(input),
            Self::Unix(stream) => stream.write(input),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            Self::Unix(stream) => stream.flush(),
        }
    }
}

/// Commit one EVAL at the isolated backend, then lose exactly its response.
fn lose_eval_reply(bus: &Bus) -> (Relay, thread::JoinHandle<()>) {
    let address = bus
        .url
        .as_str()
        .into_connection_info()
        .unwrap()
        .addr()
        .clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut client = BufReader::new(client);
        let upstream = match address {
            ConnectionAddr::Tcp(host, port) => {
                FixtureStream::Tcp(TcpStream::connect((host.as_str(), port)).unwrap())
            }
            ConnectionAddr::Unix(path) => FixtureStream::Unix(UnixStream::connect(path).unwrap()),
            other => panic!("unsupported isolated Redis transport: {other:?}"),
        };
        upstream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut upstream = BufReader::new(upstream);
        loop {
            let request = resp_frame(&mut client);
            upstream.get_mut().write_all(&request).unwrap();
            let response = resp_frame(&mut upstream);
            let marker = b"\r\n$4\r\nEVAL\r\n";
            if request.windows(marker.len()).any(|bytes| bytes == marker) {
                assert!(
                    !response.starts_with(b"-"),
                    "backend EVAL must commit before its reply is lost"
                );
                break;
            }
            client.get_mut().write_all(&response).unwrap();
        }
    });
    let relay = Relay::connect(&format!("redis://127.0.0.1:{port}"), Duration::from_secs(2))
        .unwrap()
        .with_prefix(bus.prefix.clone());
    (relay, worker)
}

#[test]
#[trace("FR-067-AC-8")]
fn receipts_braced_recipient_preserves_payload_and_render_budget_on_claim() {
    let Some(mut bus) = bus() else { return };
    let recipient = name("session/valid{queue}");
    bus.relay
        .register(&Registration {
            name: recipient.clone(),
            machine: "fixture".parse().unwrap(),
            kind: "codex".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 0,
        })
        .unwrap();
    let message = Message {
        from: name("sender"),
        to: relay_core::Target::Agent(recipient.clone()),
        text: r#"literal braces { } and escaped "quotes"; _relay is recipient content"#.into(),
        sent_at_ms: 9_007_199_254_740_993,
        forwarded: false,
    };
    bus.relay.send(&message).unwrap();
    bus.relay.send(&message).unwrap();
    let slack = "[ix-relay] from ".len()
        + relay_core::MAX_LABEL_BYTES
        + " (to all)".len()
        + " (redelivered)".len()
        + ": ".len()
        + 1
        - 46;
    let weight = serde_json::to_string(&message).unwrap().len() + slack;
    let claimed = bus
        .relay
        .pop_inbox_budget(
            &recipient,
            NonZeroUsize::new(2).unwrap(),
            2 * weight,
            Claim::new(Popper::Watcher),
        )
        .unwrap();
    assert!(
        claimed.rejected.is_empty(),
        "valid braces in metadata must not corrupt recipient JSON"
    );
    assert_eq!(
        claimed.messages,
        [message.clone(), message.clone()],
        "metadata must neither consume the recipient render budget nor round its timestamp"
    );
    let rows: Vec<String> = bus
        .raw
        .lrange(bus.key(&format!("inflight:{recipient}")), 0, -1)
        .unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(
            row.matches(",\"_relay\":").count(),
            1,
            "exactly one private metadata member"
        );
        assert!(
            row.contains("9007199254740993"),
            "preserve the integer lexeme byte-for-byte"
        );
    }
}

fn claim_with_queue_counter(
    bus: &mut Bus,
    recipient: &AgentName,
    counter: &str,
) -> relay_core::BatchId {
    let body = [
        include_str!("../src/store/lua/common.lua"),
        include_str!("../src/store/lua/receipt_core.lua"),
        include_str!("../src/store/lua/receipts.lua"),
        include_str!("../src/store/lua/pop.lua"),
    ]
    .join("\n");
    let reply: (i64, usize, u64, Vec<Vec<u8>>, Vec<u32>, usize, usize) = redis::cmd("EVAL")
        .arg(body)
        .arg(4)
        .arg(bus.key(&format!("inbox:{recipient}")))
        .arg(bus.key(&format!("inflight:{recipient}")))
        .arg(bus.key(&format!("deadletter:{recipient}")))
        .arg(bus.key(counter))
        .arg(1)
        .arg(10_000)
        .arg(1_024)
        .arg("w")
        .arg(60_000)
        .arg("")
        .arg(relay_core::MAX_REDELIVERIES)
        .arg(relay_core::DEAD_LETTER_CAP)
        .arg(relay_core::MAX_INFLIGHT_BATCHES)
        .arg(relay_core::MAX_INFLIGHT_BYTES)
        .arg(relay_core::INFLIGHT_ROW_OVERHEAD)
        .arg(1_024)
        .arg(recipient.as_str())
        .arg(bus.key("receipts"))
        .arg(bus.key("receipt-meta"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(reply.0, 0);
    assert_eq!(reply.3.len(), 1);
    relay_core::BatchId::new(reply.2)
}

#[test]
#[trace("FR-067-AC-2", "FR-067-AC-8")]
fn receipts_forwarding_to_another_queue_never_reuses_the_receipt_attempt() {
    let Some(mut bus) = bus() else { return };
    send(&mut bus);
    let old = bus.relay.agent(&name("receiver")).unwrap().unwrap();
    let replacement = Registration {
        name: name("replacement"),
        ..old.clone()
    };
    bus.relay.register(&replacement).unwrap();
    // Independently scoped queue counters legitimately have the same scalar batch ID.
    let first = claim_with_queue_counter(&mut bus, &old.name, "old-queue-counter");
    bus.relay
        .requeue_batch(&old.name, first, Popper::Watcher)
        .unwrap();
    assert_eq!(
        bus.relay.forward_stranded(&old, &replacement).unwrap(),
        Some(1)
    );
    let second = claim_with_queue_counter(&mut bus, &replacement.name, "replacement-queue-counter");
    assert_eq!(
        first.get(),
        second.get(),
        "exercise a scalar batch-ID collision across queues"
    );
    let report = export(&mut bus);
    let attempts: Vec<_> = report
        .events
        .iter()
        .filter(|row| row.event.outcome == ReceiptOutcome::AttemptStarted)
        .collect();
    assert_eq!(
        attempts.len(),
        2,
        "deduplication must retain both genuine attempts"
    );
    assert_eq!(attempts[0].event.original_id, attempts[1].event.original_id);
    assert_ne!(attempts[0].event.attempt_id, attempts[1].event.attempt_id);
    assert_ne!(attempts[0].event.event_id, attempts[1].event.event_id);
}

#[test]
#[trace("FR-067-AC-8")]
fn receipts_absent_configuration_uses_the_documented_skip_path() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env_remove("IX_RELAY_TEST_URL")
        .args([
            "--exact",
            "receipts_enqueue_and_ack_remain_atomic_when_committed_replies_are_lost",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("skipping receipts: IX_RELAY_TEST_URL unset"),
        "the missing configuration must take Bus::connect's explicit skip path"
    );
}

#[test]
#[trace("FR-067-AC-4")]
fn receipts_enqueue_and_ack_remain_atomic_when_committed_replies_are_lost() {
    let Some(mut bus) = bus() else { return };
    let (mut disconnected, worker) = lose_eval_reply(&bus);
    let message = Message {
        from: name("sender"),
        to: "receiver".parse().unwrap(),
        text: "lost reply".into(),
        sent_at_ms: 42,
        forwarded: false,
    };
    assert!(disconnected.send(&message).is_err());
    worker.join().unwrap();
    assert_eq!(bus.relay.inbox_len(&name("receiver")).unwrap(), 1);
    assert_eq!(
        export(&mut bus)
            .events
            .iter()
            .filter(|row| row.event.outcome == ReceiptOutcome::Queued)
            .count(),
        1
    );
    let at = now(&mut bus);
    let batch = claim(&mut bus, at).batch.unwrap();
    bus.relay
        .observe_delivery(
            &name("receiver"),
            batch,
            Popper::Watcher,
            DeliveryEvidence::HerdrPrompt,
            Now::AtMs(at),
        )
        .unwrap();
    let (mut disconnected, worker) = lose_eval_reply(&bus);
    assert!(
        disconnected
            .ack(&name("receiver"), batch, Popper::Watcher)
            .is_err()
    );
    worker.join().unwrap();
    assert_eq!(bus.relay.inflight_len(&name("receiver")).unwrap(), 0);
    assert!(
        !bus.relay
            .ack(&name("receiver"), batch, Popper::Watcher)
            .unwrap()
    );
    assert_eq!(
        export(&mut bus)
            .events
            .iter()
            .filter(|row| row.event.outcome == ReceiptOutcome::Acknowledged)
            .count(),
        1
    );
}

#[test]
#[trace("FR-067-AC-1", "FR-067-AC-3", "FR-067-AC-8")]
fn combined_hook_ack_records_ordered_delivery_and_settlement_with_stable_identities() {
    let Some(mut bus) = bus() else { return };
    send(&mut bus);
    send(&mut bus);
    let recipient = name("receiver");
    let batch = bus
        .relay
        .pop_inbox_budget(
            &recipient,
            NonZeroUsize::new(2).unwrap(),
            10_000,
            Claim::new(Popper::Hook),
        )
        .unwrap()
        .batch
        .unwrap();
    let before = snapshot(&mut bus);
    for (token, popper) in [
        (relay_core::BatchId::new(batch.get() + 1), Popper::Hook),
        (batch, Popper::Watcher),
    ] {
        assert!(
            !bus.relay
                .ack_delivered(&recipient, token, popper, DeliveryEvidence::HookStdout)
                .unwrap()
        );
        assert_eq!(
            snapshot(&mut bus),
            before,
            "an invalid token must neither observe nor settle"
        );
    }
    assert!(
        bus.relay
            .ack_delivered(
                &recipient,
                batch,
                Popper::Hook,
                DeliveryEvidence::HookStdout
            )
            .unwrap()
    );
    assert_eq!(bus.relay.inflight_len(&recipient).unwrap(), 0);
    assert_eq!(bus.relay.inbox_len(&recipient).unwrap(), 0);
    let report = export(&mut bus);
    assert_eq!(report.events.len(), 8);
    assert!(
        !report
            .reasons
            .contains(&ReceiptCoverageReason::RecordingGap)
    );
    let originals: Vec<_> = report
        .events
        .iter()
        .filter(|row| row.event.outcome == ReceiptOutcome::Queued)
        .map(|row| row.event.original_id.as_str())
        .collect();
    assert_ne!(originals[0], originals[1]);
    for original in originals {
        let events: Vec<_> = report
            .events
            .iter()
            .filter(|row| row.event.original_id == original)
            .map(|row| &row.event)
            .collect();
        assert_eq!(
            events.iter().map(|event| event.outcome).collect::<Vec<_>>(),
            [
                ReceiptOutcome::Queued,
                ReceiptOutcome::AttemptStarted,
                ReceiptOutcome::Delivered,
                ReceiptOutcome::Acknowledged
            ]
        );
        assert!(
            events
                .iter()
                .all(|event| event.sender == "sender" && event.target == "receiver")
        );
        assert!(events[1].attempt_id.is_some());
        assert_eq!(events[1].attempt_id, events[2].attempt_id);
        assert_eq!(events[2].attempt_id, events[3].attempt_id);
        assert_ne!(events[2].event_id, events[3].event_id);
        assert_eq!(events[2].evidence, Some(DeliveryEvidence::HookStdout));
        assert_eq!(events[2].event_time_ms, None);
        assert!(events[3].event_time_ms.is_some());
    }
    let settled = snapshot(&mut bus);
    assert!(
        !bus.relay
            .ack_delivered(
                &recipient,
                batch,
                Popper::Hook,
                DeliveryEvidence::HookStdout
            )
            .unwrap()
    );
    assert_eq!(snapshot(&mut bus), settled, "repeated settlement is inert");
}

#[test]
#[trace("FR-067-AC-3", "FR-067-AC-4", "FR-067-AC-8")]
fn combined_ack_rejects_wrong_storage_types_without_partial_observation_or_settlement() {
    for tail in ["receipts", "receipt-meta", "inflight:receiver"] {
        let Some(mut bus) = bus() else { return };
        send(&mut bus);
        let at = now(&mut bus);
        let batch = claim(&mut bus, at).batch.unwrap();
        let key = bus.key(tail);
        let saved = bus.key("saved");
        let _: () = bus.raw.rename(&key, &saved).unwrap();
        let _: () = bus.raw.set(&key, "wrong type").unwrap();
        let before = snapshot(&mut bus);
        assert!(matches!(
            bus.relay.ack_delivered(
                &name("receiver"),
                batch,
                Popper::Watcher,
                DeliveryEvidence::HerdrPrompt
            ),
            Err(relay_core::Error::InflightUnreadable { .. })
        ));
        assert_eq!(
            snapshot(&mut bus),
            before,
            "wrong-type {tail} must fail before any writes"
        );
        let _: () = bus.raw.del(&key).unwrap();
        let _: () = bus.raw.rename(&saved, &key).unwrap();
        assert_eq!(bus.relay.inflight_len(&name("receiver")).unwrap(), 1);
        assert!(
            bus.relay
                .ack_delivered(
                    &name("receiver"),
                    batch,
                    Popper::Watcher,
                    DeliveryEvidence::HerdrPrompt
                )
                .unwrap()
        );
        let report = export(&mut bus);
        assert_eq!(
            report
                .events
                .iter()
                .map(|row| row.event.outcome)
                .collect::<Vec<_>>(),
            [
                ReceiptOutcome::Queued,
                ReceiptOutcome::AttemptStarted,
                ReceiptOutcome::Delivered,
                ReceiptOutcome::Acknowledged
            ]
        );
        assert!(
            !report
                .reasons
                .contains(&ReceiptCoverageReason::RecordingGap)
        );
    }
}

#[test]
#[trace("FR-067-AC-1", "FR-067-AC-4")]
fn combined_ack_commits_both_receipts_once_when_the_response_is_lost() {
    let Some(mut bus) = bus() else { return };
    send(&mut bus);
    let at = now(&mut bus);
    let batch = claim(&mut bus, at).batch.unwrap();
    let (mut disconnected, worker) = lose_eval_reply(&bus);
    assert!(
        disconnected
            .ack_delivered(
                &name("receiver"),
                batch,
                Popper::Watcher,
                DeliveryEvidence::HerdrPrompt
            )
            .is_err()
    );
    worker.join().unwrap();
    assert_eq!(bus.relay.inflight_len(&name("receiver")).unwrap(), 0);
    let settled = snapshot(&mut bus);
    assert!(
        !bus.relay
            .ack_delivered(
                &name("receiver"),
                batch,
                Popper::Watcher,
                DeliveryEvidence::HerdrPrompt
            )
            .unwrap()
    );
    assert_eq!(snapshot(&mut bus), settled);
    let report = export(&mut bus);
    assert_eq!(
        report
            .events
            .iter()
            .map(|row| row.event.outcome)
            .collect::<Vec<_>>(),
        [
            ReceiptOutcome::Queued,
            ReceiptOutcome::AttemptStarted,
            ReceiptOutcome::Delivered,
            ReceiptOutcome::Acknowledged
        ]
    );
    assert_eq!(
        report.events[2].event.evidence,
        Some(DeliveryEvidence::HerdrPrompt)
    );
    assert!(
        !report
            .reasons
            .contains(&ReceiptCoverageReason::RecordingGap)
    );
}

#[test]
#[trace("FR-067-AC-2", "FR-067-AC-3", "FR-067-AC-8")]
fn combined_ack_does_not_settle_a_replacement_attempt_or_duplicate_existing_observation() {
    let Some(mut bus) = bus() else { return };
    send(&mut bus);
    let at = now(&mut bus) - 100;
    let expired = claim(&mut bus, at).batch.unwrap();
    assert_eq!(
        bus.relay
            .reap_expired(&name("receiver"), Now::AtMs(at + 20))
            .unwrap()
            .requeued,
        1
    );
    let current = claim(&mut bus, at + 21).batch.unwrap();
    let before = snapshot(&mut bus);
    assert!(
        !bus.relay
            .ack_delivered(
                &name("receiver"),
                expired,
                Popper::Watcher,
                DeliveryEvidence::HerdrPrompt
            )
            .unwrap()
    );
    assert_eq!(
        snapshot(&mut bus),
        before,
        "an old attempt cannot observe or settle a replacement"
    );
    bus.relay
        .observe_delivery(
            &name("receiver"),
            current,
            Popper::Watcher,
            DeliveryEvidence::SocketConfirmed,
            Now::Server,
        )
        .unwrap();
    let observed = export(&mut bus);
    assert!(
        bus.relay
            .ack_delivered(
                &name("receiver"),
                current,
                Popper::Watcher,
                DeliveryEvidence::HerdrPrompt
            )
            .unwrap()
    );
    let report = export(&mut bus);
    assert_eq!(report.events.len(), observed.events.len() + 1);
    assert_eq!(&report.events[..observed.events.len()], observed.events);
    assert_eq!(
        report.events.last().unwrap().event.outcome,
        ReceiptOutcome::Acknowledged
    );
    let delivered: Vec<_> = report
        .events
        .iter()
        .filter(|row| row.event.outcome == ReceiptOutcome::Delivered)
        .collect();
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].event.evidence,
        Some(DeliveryEvidence::SocketConfirmed)
    );
}

#[test]
#[trace("FR-067-AC-1", "FR-067-AC-7")]
fn receipts_lifecycle_separates_observation_from_settlement_without_content() {
    let Some(mut bus) = bus() else { return };
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
#[trace("FR-067-AC-1", "FR-067-AC-8")]
fn receipts_broadcast_retains_each_native_recipient_without_changing_message_history() {
    let Some(mut bus) = bus() else { return };
    bus.relay
        .register(&Registration {
            name: name("second"),
            machine: "fixture".parse().unwrap(),
            kind: "codex".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 0,
        })
        .unwrap();
    bus.relay
        .send(&Message {
            from: name("sender"),
            to: "all".parse().unwrap(),
            text: "broadcast".into(),
            sent_at_ms: 42,
            forwarded: false,
        })
        .unwrap();
    let report = export(&mut bus);
    assert_eq!(report.events.len(), 2);
    assert_eq!(report.events[0].event.target, "receiver");
    assert_eq!(report.events[1].event.target, "second");
    assert_ne!(
        report.events[0].event.original_id,
        report.events[1].event.original_id
    );
    let history = bus.relay.log_since(0).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].to, relay_core::Target::All);
}

#[test]
#[trace("FR-067-AC-2", "FR-067-AC-3", "FR-067-AC-8")]
fn receipts_identical_sends_recover_with_distinct_attempts_and_stale_ack_is_inert() {
    let Some(mut bus) = bus() else { return };
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
    let Some(mut bus) = bus() else { return };
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
    assert!(
        bus.relay
            .ack(&name("receiver"), batch, Popper::Watcher)
            .is_err(),
        "a receipt write failure must not remove the original in-flight message"
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
    let Some(mut bus) = bus() else { return };
    let since_ms = now(&mut bus);
    for _ in 0..1_003 {
        send(&mut bus);
    }
    let mut request = request(&mut bus);
    request.since_ms = since_ms;
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
    let Some(mut bus) = bus() else { return };
    let since_ms = now(&mut bus);
    send(&mut bus);
    let oldest = export(&mut bus).events[0].cursor.clone();
    for _ in 0..RECEIPT_EVENT_CAP {
        send(&mut bus);
    }
    let count: usize = bus.raw.xlen(bus.key("receipts")).unwrap();
    assert_eq!(count, RECEIPT_EVENT_CAP);
    let mut request = request(&mut bus);
    request.since_ms = since_ms;
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
#[trace("FR-044-AC-22")]
fn receiver_gone_redelivery_adds_the_redelivery_marker() {
    let Some(mut bus) = bus() else { return };
    send(&mut bus);
    let at = now(&mut bus);
    let batch = claim(&mut bus, at).batch.unwrap();
    assert_eq!(
        bus.relay
            .redeliver_batch(&name("receiver"), batch, Popper::Watcher)
            .unwrap(),
        1
    );
    let result = claim(&mut bus, at + 1);
    assert_eq!(result.redelivered, [true]);
    assert_eq!(
        result.messages[0].text,
        "secret body credential=private transcript"
    );
}

#[test]
#[trace("FR-067-AC-2", "FR-067-AC-8")]
fn receipts_plain_pop_requeue_preserves_original_without_inventing_delivery() {
    let Some(mut bus) = bus() else { return };
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
        .dead_letter_batch(
            &name("receiver"),
            batch,
            Popper::Watcher,
            relay_core::receipts::DeadLetterReason::ReceiverDenied,
        )
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
    // The plain read is the one delivery (the recipient took it); the requeue and the
    // dead letter invent none, and nothing is acknowledged.
    assert!(
        report
            .events
            .iter()
            .all(|row| row.event.outcome != ReceiptOutcome::Acknowledged)
    );
    let delivered: Vec<_> = report
        .events
        .iter()
        .filter(|row| row.event.outcome == ReceiptOutcome::Delivered)
        .collect();
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].event.evidence,
        Some(DeliveryEvidence::InboxRead)
    );
    assert_eq!(bus.relay.log_since(0).unwrap().len(), 1);
}

#[test]
#[trace("FR-067-AC-2", "FR-067-AC-8")]
fn receipts_expired_approval_retains_its_typed_reason_without_claiming_denial() {
    let Some(mut bus) = bus() else { return };
    send(&mut bus);
    let at = now(&mut bus);
    let batch = claim(&mut bus, at).batch.unwrap();
    bus.relay
        .dead_letter_batch(
            &name("receiver"),
            batch,
            Popper::Watcher,
            relay_core::receipts::DeadLetterReason::HoldExpired,
        )
        .unwrap();
    let report = export(&mut bus);
    let dead = report
        .events
        .iter()
        .find(|row| row.event.outcome == ReceiptOutcome::DeadLettered)
        .unwrap();
    assert_eq!(dead.event.reason.as_deref(), Some("hold_expired"));
    assert!(!report.events.iter().any(|row| matches!(
        row.event.outcome,
        ReceiptOutcome::Delivered | ReceiptOutcome::Acknowledged
    )));
}
