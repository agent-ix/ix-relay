//! `send --order` through the real binary against a real Redis (FR-071). Skips
//! without `IX_RELAY_TEST_URL` (`make test-redis`).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use std::{process::Command, time::Duration};

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

    fn agent_msg(&self, args: &[&str]) -> support::Run {
        let mut command = base_cmd(args, None);
        command
            .env("IX_RELAY_URL", &self.url)
            .env("IX_RELAY_PREFIX", &self.prefix);
        run(command, "")
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

#[trace("FR-071-AC-12")]
#[test]
fn all_order_statuses_are_listed_in_stable_read_only_pages() {
    let Some(mut bus) = Bus::new("all-orders") else {
        return;
    };
    for extra in [
        vec!["--order", "--expect-ticket", "SWM-309"],
        vec!["--order"],
        vec!["--order"],
        vec!["--order"],
        vec!["--order"],
    ] {
        let out = bus.send(&extra, None);
        assert_eq!(out.status, 0, "{}", out.stderr);
    }

    // Pin the stored rows to one millisecond so the test proves the order-id tie-breaker,
    // independently of how quickly the host clock advanced while sending.
    let index = format!("{}receipt-meta:orders-all", bus.prefix);
    let ids: Vec<String> = redis::cmd("ZRANGE")
        .arg(&index)
        .arg(0)
        .arg(-1)
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(ids.len(), 5);
    let at = now_ms();
    for (position, member) in ids.iter().enumerate() {
        let (_, id) = member.split_once('|').unwrap();
        redis::cmd("ZREM")
            .arg(&index)
            .arg(member)
            .exec(&mut bus.raw)
            .unwrap();
        let enqueued_at = at;
        redis::cmd("ZADD")
            .arg(&index)
            .arg(0)
            .arg(format!("{enqueued_at:013}|{id}"))
            .exec(&mut bus.raw)
            .unwrap();
        let status = match position {
            0 => "acted",
            1 => "expired",
            2 => "evicted",
            3 => "cancelled",
            _ => "open",
        };
        let order_key = format!("{}receipt-meta:order:{id}", bus.prefix);
        redis::cmd("HSET")
            .arg(&order_key)
            .arg("enqueued_at")
            .arg(enqueued_at)
            .arg("status")
            .arg(status)
            .exec(&mut bus.raw)
            .unwrap();
        if position == 0 {
            redis::cmd("HSET")
                .arg(&order_key)
                .arg("delivered_at")
                .arg(at - 250)
                .arg("acted_at")
                .arg(at - 100)
                .exec(&mut bus.raw)
                .unwrap();
        } else if position == 1 {
            redis::cmd("HSET")
                .arg(&order_key)
                .arg("acted_at")
                .arg(at + 50)
                .exec(&mut bus.raw)
                .unwrap();
            redis::cmd("HDEL")
                .arg(&order_key)
                .arg("delivered_at")
                .exec(&mut bus.raw)
                .unwrap();
        } else {
            redis::cmd("HDEL")
                .arg(&order_key)
                .arg("delivered_at")
                .arg("acted_at")
                .exec(&mut bus.raw)
                .unwrap();
        }
    }
    redis::cmd("HSET")
        .arg(format!("{}receipt-meta", bus.prefix))
        .arg("orders_all_since_ms")
        .arg(at)
        .arg("orders_all_complete_since_ms")
        .arg(at)
        .exec(&mut bus.raw)
        .unwrap();

    let url = bus.url.clone();
    let prefix = bus.prefix.clone();
    let at_text = at.to_string();
    let read = |cursor: Option<String>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-msg"));
        command
            .args(["--url", &url, "--prefix", &prefix, "orders"])
            .args(["--since", &at_text, "--until", &at_text])
            .args(["--limit", "1", "--json"]);
        if let Some(cursor) = cursor {
            command.args(["--cursor", &cursor]);
        }
        command.output().unwrap()
    };
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{}*", bus.prefix))
        .query(&mut bus.raw)
        .unwrap();
    let before: std::collections::BTreeMap<String, Vec<u8>> = keys
        .iter()
        .map(|key| {
            (
                key.clone(),
                relay_test_support::canonical_dump(&mut bus.raw, key),
            )
        })
        .collect();

    let first = read(None);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_json["orders"].as_array().unwrap().len(), 1);
    assert_eq!(first_json["truncated"], true);
    assert_eq!(first_json["window"]["filter"], "enqueued_at");
    assert_eq!(first_json["window"]["order"], "newest_first");
    assert_eq!(first_json["oldest_available_at"], at);
    assert_eq!(first_json["complete"], true);
    assert_eq!(first_json["incomplete_reason"], serde_json::Value::Null);
    let cursor = first_json["next_cursor"].clone();
    assert_eq!(cursor["enqueued_at"], at);
    let first_order = &first_json["orders"][0];
    assert_eq!(first_order["recipient"], "hub:EA:code");
    assert!(
        first_order["receipt_target"]
            .as_str()
            .unwrap()
            .starts_with("session/")
    );
    assert_eq!(first_order["state"], "open");
    assert_eq!(first_order["expired"], false);
    assert_eq!(
        first_order["deadline_at"],
        first_order["enqueued_at"].as_i64().unwrap() + first_order["ack_ms"].as_i64().unwrap()
    );
    assert!(first_order.get("text").is_none());
    assert!(!String::from_utf8_lossy(&first.stdout).contains("hi"));

    let second = read(Some(serde_json::to_string(&cursor).unwrap()));
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_json["truncated"], true);
    assert_eq!(second_json["orders"][0]["state"], "cancelled");
    assert_eq!(second_json["orders"][0]["expired"], false);
    let third = read(Some(
        serde_json::to_string(&second_json["next_cursor"].clone()).unwrap(),
    ));
    assert!(
        third.status.success(),
        "{}",
        String::from_utf8_lossy(&third.stderr)
    );
    let third_json: serde_json::Value = serde_json::from_slice(&third.stdout).unwrap();
    assert_eq!(third_json["truncated"], true);
    assert_eq!(third_json["orders"][0]["state"], "evicted");
    let fourth = read(Some(
        serde_json::to_string(&third_json["next_cursor"].clone()).unwrap(),
    ));
    assert!(
        fourth.status.success(),
        "{}",
        String::from_utf8_lossy(&fourth.stderr)
    );
    let fourth_json: serde_json::Value = serde_json::from_slice(&fourth.stdout).unwrap();
    assert_eq!(fourth_json["truncated"], true);
    assert_eq!(fourth_json["orders"][0]["state"], "expired");
    assert_eq!(fourth_json["orders"][0]["expired"], true);
    assert_eq!(fourth_json["orders"][0]["acted_at"], at + 50);
    let fifth = read(Some(
        serde_json::to_string(&fourth_json["next_cursor"].clone()).unwrap(),
    ));
    assert!(
        fifth.status.success(),
        "{}",
        String::from_utf8_lossy(&fifth.stderr)
    );
    let fifth_json: serde_json::Value = serde_json::from_slice(&fifth.stdout).unwrap();
    assert_eq!(fifth_json["truncated"], false);
    assert!(fifth_json["next_cursor"].is_null());
    assert_eq!(fifth_json["orders"][0]["state"], "acted");
    assert_eq!(fifth_json["orders"][0]["expired"], false);
    assert_eq!(fifth_json["orders"][0]["acted_at"], at - 100);
    assert_eq!(fifth_json["orders"][0]["expect_ticket"], "SWM-309");
    assert_eq!(
        fifth_json["orders"][0]["deadline_at"],
        fifth_json["orders"][0]["delivered_at"].as_i64().unwrap()
            + fifth_json["orders"][0]["ack_ms"].as_i64().unwrap()
    );

    let after_keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{}*", bus.prefix))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(after_keys, keys, "all-outcome reads create no Redis keys");
    let after: std::collections::BTreeMap<String, Vec<u8>> = keys
        .iter()
        .map(|key| {
            (
                key.clone(),
                relay_test_support::canonical_dump(&mut bus.raw, key),
            )
        })
        .collect();
    assert_eq!(after, before, "all-outcome reads leave Redis unchanged");
}

#[trace("FR-071-AC-12")]
#[test]
fn empty_pages_and_aged_out_cursors_are_json_arrays_with_coverage_metadata() {
    let Some(mut bus) = Bus::new("empty-pages") else {
        return;
    };
    let url = bus.url.clone();
    let prefix = bus.prefix.clone();
    let read = |since: i64, until: i64, cursor: Option<String>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-msg"));
        command
            .args(["--url", &url, "--prefix", &prefix, "orders"])
            .args([
                "--since",
                &since.to_string(),
                "--until",
                &until.to_string(),
                "--json",
            ]);
        if let Some(cursor) = cursor {
            command.args(["--cursor", &cursor]);
        }
        command.output().unwrap()
    };

    let now = now_ms();
    let never_indexed = read(0, now, None);
    assert!(
        never_indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&never_indexed.stderr)
    );
    let never_json: serde_json::Value = serde_json::from_slice(&never_indexed.stdout).unwrap();
    assert_eq!(never_json["orders"].as_array().unwrap().len(), 0);
    assert_eq!(never_json["oldest_available_at"], serde_json::Value::Null);
    assert_eq!(never_json["complete"], false);
    assert_eq!(never_json["incomplete_reason"], "index_not_started");

    assert_eq!(bus.send(&["--order"], None).status, 0);
    let at: i64 = redis::cmd("HGET")
        .arg(format!("{}receipt-meta", bus.prefix))
        .arg("orders_all_since_ms")
        .query(&mut bus.raw)
        .unwrap();
    let empty_window = read(at + 1, at + 100, None);
    assert!(
        empty_window.status.success(),
        "{}",
        String::from_utf8_lossy(&empty_window.stderr)
    );
    let empty_json: serde_json::Value = serde_json::from_slice(&empty_window.stdout).unwrap();
    assert_eq!(empty_json["orders"].as_array().unwrap().len(), 0);
    assert_eq!(empty_json["complete"], false);
    assert_eq!(empty_json["incomplete_reason"], "coverage_unconfirmed");
    assert!(empty_json["oldest_available_at"].is_null());
    let confirmed_at = bus.relay.confirm_orders_complete_since().unwrap();
    assert!(confirmed_at >= at);
    let before_coverage = read(0, confirmed_at, None);
    assert!(before_coverage.status.success());
    let before_json: serde_json::Value = serde_json::from_slice(&before_coverage.stdout).unwrap();
    assert_eq!(before_json["orders"].as_array().unwrap().len(), 1);
    assert_eq!(before_json["complete"], false);
    assert_eq!(before_json["incomplete_reason"], "before_complete_coverage");
    assert_eq!(before_json["oldest_available_at"], confirmed_at);
    let complete_window = read(confirmed_at, confirmed_at + 100, None);
    assert!(complete_window.status.success());
    let complete_json: serde_json::Value = serde_json::from_slice(&complete_window.stdout).unwrap();
    assert_eq!(complete_json["complete"], true);
    assert_eq!(complete_json["oldest_available_at"], confirmed_at);

    let aged_cursor = serde_json::json!({
        "enqueued_at": at - relay_core::receipts::ORDER_RETENTION_MS - 1,
        "order_id": "expired-cursor"
    });
    let aged = read(0, at, Some(aged_cursor.to_string()));
    assert!(
        aged.status.success(),
        "{}",
        String::from_utf8_lossy(&aged.stderr)
    );
    let aged_json: serde_json::Value = serde_json::from_slice(&aged.stdout).unwrap();
    assert_eq!(aged_json["orders"].as_array().unwrap().len(), 0);
    assert_eq!(aged_json["truncated"], false);
    assert_eq!(aged_json["next_cursor"], serde_json::Value::Null);
    assert_eq!(aged_json["complete"], false);

    let later = now_ms();
    let retention = relay_core::receipts::ORDER_RETENTION_MS;
    let old_since = later - retention - 1_000;
    let expired_at = later - retention - 500;
    let order_id = "aged-index-fixture";
    let order_key = format!("{}receipt-meta:order:{order_id}", bus.prefix);
    redis::cmd("HSET")
        .arg(&order_key)
        .arg("from")
        .arg("node-a:EA:sender")
        .arg("to")
        .arg("hub:EA:code")
        .arg("recipient")
        .arg("session/687562/7331")
        .arg("ack_ms")
        .arg(60_000)
        .arg("enqueued_at")
        .arg(expired_at)
        .arg("status")
        .arg("acted")
        .arg("state")
        .arg("acted")
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("ZADD")
        .arg(format!("{}receipt-meta:orders-all", bus.prefix))
        .arg(0)
        .arg(format!("{expired_at:013}|{order_id}"))
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("HSET")
        .arg(format!("{}receipt-meta", bus.prefix))
        .arg("orders_all_since_ms")
        .arg(old_since)
        .arg("orders_all_complete_since_ms")
        .arg(old_since)
        .exec(&mut bus.raw)
        .unwrap();
    let retention_page = bus
        .relay
        .orders(&relay_core::receipts::OrderListRequest::new(0, later, 10, None).unwrap())
        .unwrap();
    assert!(!retention_page.complete);
    assert_eq!(
        retention_page.incomplete_reason.as_deref(),
        Some("outside_retention")
    );
    assert!(
        !retention_page
            .orders
            .iter()
            .any(|order| order.order_id == order_id)
    );
    assert!(retention_page.oldest_available_at.unwrap() >= later - retention);
    assert!(retention_page.oldest_available_at.unwrap() <= later - retention + 2_000);
}

#[trace("FR-071-AC-12")]
#[test]
fn order_writer_and_reader_share_retention_and_report_visible_legacy_gaps() {
    let Some(mut bus) = Bus::new("retention-coverage") else {
        return;
    };
    assert_eq!(bus.send(&["--order"], None).status, 0);
    let boundary: i64 = redis::cmd("HGET")
        .arg(format!("{}receipt-meta", bus.prefix))
        .arg("orders_all_since_ms")
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(bus.send(&["--order"], None).status, 0);
    let after_boundary: i64 = redis::cmd("HGET")
        .arg(format!("{}receipt-meta", bus.prefix))
        .arg("orders_all_since_ms")
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(
        after_boundary, boundary,
        "the coverage boundary is set once"
    );

    let initial_members: Vec<String> = redis::cmd("ZRANGE")
        .arg(format!("{}receipt-meta:orders-all", bus.prefix))
        .arg(0)
        .arg(0)
        .query(&mut bus.raw)
        .unwrap();
    let initial_member = &initial_members[0];
    let initial_id = initial_member.split_once('|').unwrap().1;
    let record_ttl: i64 = redis::cmd("TTL")
        .arg(format!("{}receipt-meta:order:{initial_id}", bus.prefix))
        .query(&mut bus.raw)
        .unwrap();
    let index_ttl: i64 = redis::cmd("TTL")
        .arg(format!("{}receipt-meta:orders-all", bus.prefix))
        .query(&mut bus.raw)
        .unwrap();
    assert!((1..=691_200).contains(&record_ttl));
    assert!((1..=691_200).contains(&index_ttl));

    let index = format!("{}receipt-meta:orders-all", bus.prefix);
    let stale_time = boundary - relay_core::receipts::ORDER_RETENTION_MS - 1;
    let stale_member = format!("{stale_time:013}|trim-marker");
    redis::cmd("ZADD")
        .arg(&index)
        .arg(0)
        .arg(&stale_member)
        .exec(&mut bus.raw)
        .unwrap();
    assert_eq!(bus.send(&["--order"], None).status, 0);
    let stale_score: Option<f64> = redis::cmd("ZSCORE")
        .arg(&index)
        .arg(&stale_member)
        .query(&mut bus.raw)
        .unwrap();
    assert!(
        stale_score.is_none(),
        "the shared retention removes old index rows"
    );

    let members: Vec<String> = redis::cmd("ZRANGE")
        .arg(&index)
        .arg(0)
        .arg(0)
        .query(&mut bus.raw)
        .unwrap();
    let first_id = members[0].split_once('|').unwrap().1;
    redis::cmd("ZREM")
        .arg(&index)
        .arg(&members[0])
        .exec(&mut bus.raw)
        .unwrap();
    let confirmed_at = bus.relay.confirm_orders_complete_since().unwrap();
    let id = first_id.to_owned();
    let request = relay_core::receipts::OrderListRequest::new(0, now_ms(), 10, None).unwrap();
    let page = bus.relay.orders(&request).unwrap();
    assert!(!page.complete);
    assert_eq!(
        page.incomplete_reason.as_deref(),
        Some("unindexed_open_orders")
    );
    assert_eq!(page.oldest_available_at, Some(confirmed_at));
    assert!(!page.orders.iter().any(|order| order.order_id == id));
}

#[trace("FR-071-AC-12")]
#[test]
fn coverage_commands_confirm_clear_and_show_a_relay_time_boundary() {
    let Some(mut bus) = Bus::new("coverage-commands") else {
        return;
    };
    let shown = bus.agent_msg(&["orders", "coverage", "show"]);
    assert_eq!(shown.status, 0, "{}", shown.stderr);
    assert!(shown.stdout.contains("orders_all_complete_since_ms: null"));

    let before = now_ms();
    let confirmed = bus.agent_msg(&["orders", "coverage", "confirm"]);
    assert_eq!(confirmed.status, 0, "{}", confirmed.stderr);
    let confirmed_at = bus.relay.orders_complete_since().unwrap().unwrap();
    assert!(confirmed_at >= before);
    assert!(confirmed.stdout.contains(&confirmed_at.to_string()));
    let shown = bus.agent_msg(&["orders", "coverage", "show"]);
    assert_eq!(shown.status, 0, "{}", shown.stderr);
    assert!(shown.stdout.contains(&confirmed_at.to_string()));

    assert_eq!(bus.send(&["--order"], None).status, 0);
    let indexed_at: i64 = redis::cmd("HGET")
        .arg(format!("{}receipt-meta", bus.prefix))
        .arg("orders_all_since_ms")
        .query(&mut bus.raw)
        .unwrap();
    let before_index = bus
        .relay
        .orders(
            &relay_core::receipts::OrderListRequest::new(
                confirmed_at.saturating_sub(1),
                now_ms(),
                10,
                None,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(!before_index.complete);
    assert_eq!(
        before_index.incomplete_reason.as_deref(),
        Some("before_index_start")
    );
    assert_eq!(before_index.oldest_available_at, Some(indexed_at));
    assert_eq!(
        before_index.orders.len(),
        1,
        "indexed rows remain listable before confirmation coverage"
    );
    let covered = bus
        .relay
        .orders(
            &relay_core::receipts::OrderListRequest::new(indexed_at, now_ms(), 10, None).unwrap(),
        )
        .unwrap();
    assert!(covered.complete);
    assert_eq!(
        covered.oldest_available_at,
        Some(indexed_at.max(confirmed_at))
    );

    let cleared = bus.agent_msg(&["orders", "coverage", "clear"]);
    assert_eq!(cleared.status, 0, "{}", cleared.stderr);
    assert!(bus.relay.orders_complete_since().unwrap().is_none());
    let unconfirmed = bus
        .relay
        .orders(
            &relay_core::receipts::OrderListRequest::new(indexed_at, now_ms(), 10, None).unwrap(),
        )
        .unwrap();
    assert!(!unconfirmed.complete);
    assert!(unconfirmed.oldest_available_at.is_none());
    assert_eq!(
        unconfirmed.incomplete_reason.as_deref(),
        Some("coverage_unconfirmed")
    );
}

#[trace("FR-071-AC-12")]
#[test]
fn closing_an_unindexed_order_clears_the_confirmed_coverage_boundary() {
    let Some(mut bus) = Bus::new("coverage-close-gap") else {
        return;
    };
    assert_eq!(bus.send(&["--order"], None).status, 0);
    let mut members: Vec<String> = redis::cmd("ZRANGE")
        .arg(format!("{}receipt-meta:orders-all", bus.prefix))
        .arg(0)
        .arg(-1)
        .query(&mut bus.raw)
        .unwrap();
    let indexed_member = members.pop().unwrap();
    let indexed_id = indexed_member.split_once('|').unwrap().1.to_owned();
    let confirmed_at = bus.relay.confirm_orders_complete_since().unwrap();
    assert_eq!(
        Some(confirmed_at),
        bus.relay.orders_complete_since().unwrap()
    );

    redis::cmd("ZADD")
        .arg(format!("{}receipt-meta:orders-due", bus.prefix))
        .arg(0)
        .arg(&indexed_id)
        .exec(&mut bus.raw)
        .unwrap();
    let expired = bus
        .relay
        .expire_orders(relay_core::Now::AtMs(now_ms() + 10_000))
        .unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(
        Some(confirmed_at),
        bus.relay.orders_complete_since().unwrap()
    );

    assert_eq!(bus.send(&["--order"], None).status, 0);
    let member: String = redis::cmd("ZREVRANGE")
        .arg(format!("{}receipt-meta:orders-all", bus.prefix))
        .arg(0)
        .arg(0)
        .query(&mut bus.raw)
        .unwrap();
    let order_id = member.split_once('|').unwrap().1.to_owned();
    let confirmed_at = bus.relay.confirm_orders_complete_since().unwrap();

    redis::cmd("ZREM")
        .arg(format!("{}receipt-meta:orders-all", bus.prefix))
        .arg(&member)
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("ZADD")
        .arg(format!("{}receipt-meta:orders-due", bus.prefix))
        .arg(0)
        .arg(&order_id)
        .exec(&mut bus.raw)
        .unwrap();
    let expired = bus
        .relay
        .expire_orders(relay_core::Now::AtMs(now_ms() + 10_000))
        .unwrap();
    assert_eq!(expired.len(), 1);
    assert!(bus.relay.orders_complete_since().unwrap().is_none());
}

#[trace("FR-071-AC-12")]
#[test]
fn unreadable_index_rows_are_skipped_and_make_coverage_incomplete() {
    let Some(mut bus) = Bus::new("unreadable-row") else {
        return;
    };
    assert_eq!(bus.send(&["--order"], None).status, 0);
    let at = now_ms();
    let index = format!("{}receipt-meta:orders-all", bus.prefix);
    let corrupt = format!("{at:013}|missing-record");
    redis::cmd("ZADD")
        .arg(&index)
        .arg(0)
        .arg(&corrupt)
        .exec(&mut bus.raw)
        .unwrap();
    let wrong_type = format!("{at:013}|wrong-type-record");
    redis::cmd("ZADD")
        .arg(&index)
        .arg(0)
        .arg(&wrong_type)
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("SET")
        .arg(format!(
            "{}receipt-meta:order:wrong-type-record",
            bus.prefix
        ))
        .arg("not-a-hash")
        .exec(&mut bus.raw)
        .unwrap();
    let bad_ack = format!("{at:013}|bad-ack-record");
    redis::cmd("ZADD")
        .arg(&index)
        .arg(0)
        .arg(&bad_ack)
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("HSET")
        .arg(format!("{}receipt-meta:order:bad-ack-record", bus.prefix))
        .arg("from")
        .arg("node-a:EA:sender")
        .arg("to")
        .arg("hub:EA:code")
        .arg("recipient")
        .arg("session/687562/7331")
        .arg("ack_ms")
        .arg("not-numeric")
        .arg("enqueued_at")
        .arg(at)
        .arg("status")
        .arg("open")
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("ZADD")
        .arg(format!("{}receipt-meta:orders-due", bus.prefix))
        .arg(at + 60_000)
        .arg("wrong-due-record")
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("SET")
        .arg(format!("{}receipt-meta:order:wrong-due-record", bus.prefix))
        .arg("not-a-hash")
        .exec(&mut bus.raw)
        .unwrap();
    let request = relay_core::receipts::OrderListRequest::new(0, at, 10, None).unwrap();
    let page = bus.relay.orders(&request).unwrap();
    assert_eq!(page.skipped, 3);
    assert!(!page.complete);
    assert_eq!(
        page.incomplete_reason.as_deref(),
        Some("unreadable_order_records")
    );
}

#[trace("FR-071-AC-12")]
#[test]
fn order_send_refuses_wrong_typed_history_index_before_writing() {
    let Some(mut bus) = Bus::new("wrong-index-type-send") else {
        return;
    };
    let index = format!("{}receipt-meta:orders-all", bus.prefix);
    redis::cmd("SET")
        .arg(&index)
        .arg("wrong-type")
        .exec(&mut bus.raw)
        .unwrap();
    let log = format!("{}log", bus.prefix);
    let before: i64 = redis::cmd("XLEN").arg(&log).query(&mut bus.raw).unwrap();
    let out = bus.send(&["--order"], None);
    assert_ne!(out.status, 0);
    let after: i64 = redis::cmd("XLEN").arg(&log).query(&mut bus.raw).unwrap();
    assert_eq!(
        after, before,
        "refused sends leave the message log unchanged"
    );
}

#[trace("FR-071-AC-12")]
#[test]
fn order_listing_reports_each_wrong_type_error_without_writing() {
    for (test, key_kind) in [
        ("wrong-stream-type", "stream"),
        ("wrong-index-type-list", "index"),
        ("wrong-due-type", "due"),
    ] {
        let Some(mut bus) = Bus::new(test) else {
            return;
        };
        let key = match key_kind {
            "stream" => format!("{}receipts", bus.prefix),
            "index" => format!("{}receipt-meta:orders-all", bus.prefix),
            _ => format!("{}receipt-meta:orders-due", bus.prefix),
        };
        redis::cmd("SET")
            .arg(&key)
            .arg("wrong-type")
            .exec(&mut bus.raw)
            .unwrap();
        let request = relay_core::receipts::OrderListRequest::new(0, now_ms(), 10, None).unwrap();
        assert!(bus.relay.orders(&request).is_err(), "{key_kind}");
    }
}

#[trace("FR-071-AC-12")]
#[test]
fn order_listing_rejects_bounds_that_break_the_padded_lexical_index() {
    assert!(relay_core::receipts::OrderListRequest::new(0, 9_999_999_999_999, 10, None).is_err());
    assert!(
        relay_core::receipts::OrderListRequest::new(
            10_000_000_000_000,
            10_000_000_000_000,
            10,
            None
        )
        .is_err()
    );
    assert!(
        relay_core::receipts::OrderListRequest::new(9_999_999_999_998, 9_999_999_999_998, 10, None)
            .is_ok()
    );
    let error =
        relay_core::receipts::OrderListRequest::new(0, 9_999_999_999_999, 10, None).unwrap_err();
    assert!(error.contains("9999999999998"));
}

#[trace("FR-071-AC-12")]
#[test]
fn order_listing_options_cannot_be_silently_ignored_by_expired_subcommand() {
    let Some(bus) = Bus::new("expired-listing-options") else {
        return;
    };
    for args in [
        vec!["orders", "--since", "0", "--until", "5", "expired"],
        vec!["orders", "--limit", "500", "expired"],
        vec!["orders", "--cursor", "{}", "expired"],
        vec!["orders", "--json", "expired"],
    ] {
        let output = bus.agent_msg(&args);
        assert_ne!(output.status, 0, "{args:?}");
    }
    let output = bus.agent_msg(&["orders", "--url", &bus.url, "expired"]);
    assert_eq!(output.status, 0, "{}", output.stderr);
    let output = bus.agent_msg(&["orders", "expired", "--prefix", &bus.prefix]);
    assert_eq!(output.status, 0, "{}", output.stderr);
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
