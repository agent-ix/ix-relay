//! `send --order` through the real binary against a real Redis (FR-071). Skips
//! without `IX_RELAY_TEST_URL` (`make test-redis`).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use std::time::Duration;

use ix_trace_rs::trace;
use relay_core::{Registration, Relay};
use support::{base_cmd, run};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap())
        .unwrap()
}

struct Bus {
    relay: Relay,
    url: String,
    prefix: String,
    raw: redis::Connection,
}

impl Bus {
    fn new(test: &str) -> Option<Self> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        let prefix = format!("test:{}:send-order-{test}:", std::process::id());
        let mut relay = Relay::connect(&url, Duration::from_secs(2))
            .unwrap()
            .with_prefix(prefix.clone());
        for (machine, session, pane, tab) in [
            ("hub", "s1", "w1:p1", "code"),
            ("node-a", "sender-session", "w1:p2", "sender"),
        ] {
            let machine = machine.parse().unwrap();
            let session = session.parse().unwrap();
            let pane: relay_core::PaneId = pane.parse().unwrap();
            let registration = Registration {
                name: relay_core::AgentName::for_session(&machine, &session),
                machine: machine.clone(),
                kind: "claude".into(),
                session_id: Some(session.clone()),
                cwd: None,
                herdr_pane: Some(pane.clone()),
                last_seen_ms: now_ms(),
                place: Some(relay_core::Place {
                    workspace: "EA".into(),
                    tab: tab.into(),
                    herdr_ids: Some(relay_core::HerdrPlaceIds {
                        workspace: "w1".parse().unwrap(),
                        tab: if tab == "code" { "w1:t1" } else { "w1:t2" }
                            .parse()
                            .unwrap(),
                        pane,
                        session_id: session,
                        verified_at_ms: now_ms(),
                    }),
                }),
            };
            relay_ops::enrollment::enroll(&mut relay, &registration).unwrap();
            let revision = relay.begin_directory_observation(&machine).unwrap();
            relay
                .publish_directory(&machine, revision, &[registration])
                .unwrap();
        }
        let raw = redis::Client::open(url.as_str())
            .unwrap()
            .get_connection()
            .unwrap();
        Some(Self {
            relay,
            url,
            prefix,
            raw,
        })
    }

    fn send(&self, args: &[&str], config: Option<&std::path::Path>) -> support::Run {
        self.send_to("hub:EA:code", args, config)
    }

    fn send_to(&self, to: &str, args: &[&str], config: Option<&std::path::Path>) -> support::Run {
        let mut all = vec!["send", to, "hi"];
        all.extend_from_slice(args);
        let mut cmd = base_cmd(&all, Some("hub:EA:sender"));
        cmd.env("IX_RELAY_URL", &self.url)
            .env("IX_RELAY_PREFIX", &self.prefix)
            .env("IX_RELAY_MACHINE", "node-a");
        if let Some(path) = config {
            cmd.env("IX_RELAY_CONFIG", path);
        }
        run(cmd, "")
    }

    /// Each order's deadline length: its due time less its queued time.
    fn deadlines(&mut self) -> Vec<i64> {
        let rows: Vec<(String, f64)> = redis::cmd("ZRANGE")
            .arg(format!("{}receipt-meta:orders-due", self.prefix))
            .arg(0)
            .arg(-1)
            .arg("WITHSCORES")
            .query(&mut self.raw)
            .unwrap();
        let now = now_ms() + 60_000;
        let request = relay_core::receipts::ReceiptRequest {
            since_ms: now - 600_000,
            until_ms: now,
            now_ms: now,
            page_size: 1_000,
            after: None,
        };
        let queued: Vec<i64> = self
            .relay
            .receipt_export(&request)
            .unwrap()
            .events
            .iter()
            .filter(|e| e.event.outcome == relay_core::receipts::ReceiptOutcome::Queued)
            .map(|e| e.event.observed_at_ms)
            .collect();
        rows.iter()
            .zip(queued)
            .map(|((_, due), queued)| {
                #[expect(clippy::cast_possible_truncation, reason = "millisecond score")]
                let due = *due as i64;
                due - queued
            })
            .collect()
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(format!("{}*", self.prefix))
            .query(&mut self.raw)
            .unwrap();
        if !keys.is_empty() {
            redis::cmd("DEL").arg(keys).exec(&mut self.raw).unwrap();
        }
    }
}

#[trace("FR-071-AC-1")]
#[test]
fn an_order_has_a_five_minute_deadline_unless_the_option_or_the_config_sets_one() {
    let Some(mut bus) = Bus::new("default") else {
        return;
    };
    let out = bus.send(&["--order"], None);
    assert_eq!(out.status, 0, "{}", out.stderr);
    let out = bus.send(&["--order", "--ack-deadline", "90s"], None);
    assert_eq!(out.status, 0, "{}", out.stderr);
    let dir = support::Scratch::new("send-order-config");
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "[send]\nack_deadline = \"2m\"\n").unwrap();
    let out = bus.send(&["--order"], Some(&file));
    assert_eq!(out.status, 0, "{}", out.stderr);
    let mut deadlines = bus.deadlines();
    deadlines.sort_unstable();
    // The queued time is read back at its own millisecond; the three differ by it only.
    assert_eq!(deadlines.len(), 3);
    assert!(
        deadlines
            .iter()
            .zip([90_000, 120_000, 300_000])
            .all(|(got, want)| (got - want).abs() < 1_000),
        "{deadlines:?}"
    );
}

#[trace("FR-071-AC-1")]
#[test]
fn a_deadline_outside_the_range_or_without_order_is_refused_and_nothing_is_queued() {
    let Some(mut bus) = Bus::new("refused") else {
        return;
    };
    for args in [
        &["--order", "--ack-deadline", "2s"][..],
        &["--order", "--ack-deadline", "25h"],
        &["--ack-deadline", "90s"],
        &["--expect-ticket", "SWM-1"],
        &["--order", "--expect-ticket", &"T".repeat(65)],
    ] {
        let out = bus.send(args, None);
        assert_ne!(out.status, 0, "{args:?}");
    }
    let inbox = relay_core::AgentName::for_session(&"hub".parse().unwrap(), &"s1".parse().unwrap());
    assert_eq!(bus.relay.inbox_len(&inbox).unwrap(), 0);
    assert!(bus.deadlines().is_empty());
}

#[trace("FR-071-AC-10")]
#[test]
fn an_expected_ticket_is_returned_in_the_receipt_export() {
    let Some(bus) = Bus::new("ticket") else {
        return;
    };
    let out = bus.send(&["--order", "--expect-ticket", "SWM-206"], None);
    assert_eq!(out.status, 0, "{}", out.stderr);
    let mut cmd = base_cmd(&["log", "--json", "--receipts"], Some("hub:EA:sender"));
    cmd.env("IX_RELAY_URL", &bus.url)
        .env("IX_RELAY_PREFIX", &bus.prefix)
        .env("IX_RELAY_MACHINE", "node-a");
    let log = run(cmd, "");
    assert_eq!(log.status, 0, "{}", log.stderr);
    let json: serde_json::Value = serde_json::from_str(&log.stdout).unwrap();
    assert_eq!(json["events"][0]["event"]["expect_ticket"], "SWM-206");
    assert!(!log.stdout.contains("hi\""));
}

#[trace("FR-071-AC-1")]
#[test]
fn an_order_to_a_recipient_that_is_not_a_lane_address_is_refused_and_nothing_is_queued() {
    let Some(mut bus) = Bus::new("not-a-lane") else {
        return;
    };
    let plain: relay_core::AgentName = "plainlead".parse().unwrap();
    bus.relay
        .register(&Registration {
            name: plain.clone(),
            machine: "hub".parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: now_ms(),
        })
        .unwrap();
    let out = bus.send_to("plainlead", &["--order"], None);
    assert_ne!(out.status, 0, "{}", out.stdout);
    assert!(
        out.stderr.contains("server:workspace:tab"),
        "{}",
        out.stderr
    );
    assert_eq!(bus.relay.inbox_len(&plain).unwrap(), 0);
    assert!(bus.deadlines().is_empty());
}

#[trace("FR-071-AC-11")]
#[test]
fn orders_expired_json_lists_the_expired_orders_and_changes_nothing() {
    let Some(mut bus) = Bus::new("expired-cli") else {
        return;
    };
    let out = bus.send(
        &[
            "--order",
            "--ack-deadline",
            "10s",
            "--expect-ticket",
            "SWM-5",
        ],
        None,
    );
    assert_eq!(out.status, 0, "{}", out.stderr);
    let list = |bus: &Bus| {
        let mut cmd = base_cmd(&["orders", "expired", "--json"], Some("hub:EA:sender"));
        cmd.env("IX_RELAY_URL", &bus.url)
            .env("IX_RELAY_PREFIX", &bus.prefix);
        run(cmd, "")
    };
    let out = list(&bus);
    assert_eq!(out.status, 0, "{}", out.stderr);
    let empty: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(empty["orders"].as_array().unwrap().len(), 0);
    assert_eq!(empty["truncated"], false);
    let expired = bus
        .relay
        .expire_orders(relay_core::Now::AtMs(now_ms() + 20_000))
        .unwrap();
    assert_eq!(expired.len(), 1);
    let out = list(&bus);
    assert_eq!(out.status, 0, "{}", out.stderr);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let orders = json["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0]["recipient"], "hub:EA:code");
    assert_eq!(orders[0]["expect_ticket"], "SWM-5");
    assert_eq!(orders[0]["delivered_before_expiry"], false);
    assert_eq!(orders[0]["escalation"], "pending");
    assert_eq!(orders[0]["last_state"], "queued");
    assert!(!out.stdout.contains("\"hi\""));
}
