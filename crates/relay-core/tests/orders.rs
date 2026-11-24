//! `Acted` receipts (FR-067) and order ack deadlines (FR-071) against an isolated Redis.
#![expect(clippy::unwrap_used, reason = "test assertions")]

use std::{num::NonZeroUsize, time::Duration};

use ix_trace_rs::trace;
use relay_core::{
    ActivityEvent, ActivityKind, Address, AgentName, Claim, Harness, HerdrPlaceIds, Message, Now,
    Place, Popper, Registration,
    receipts::{
        DeliveryEvidence, EXPIRED_ORDERS_CAP, OrderEscalationState, OrderSpec, ReceiptEvent,
        ReceiptOutcome, ReceiptRequest,
    },
};
use relay_test_support::Bus;

const LANE: &str = "terra:SWM:code";

fn lead(bus: &mut Bus, session: &str) -> (AgentName, Address) {
    lead_on(bus, "terra", session)
}

fn lead_on(bus: &mut Bus, machine: &str, session: &str) -> (AgentName, Address) {
    let machine = machine.parse().unwrap();
    let session_id: relay_core::SessionId = session.parse().unwrap();
    let name = AgentName::for_session(&machine, &session_id);
    let reg = Registration {
        name: name.clone(),
        machine: machine.clone(),
        kind: "claude".into(),
        session_id: Some(session_id.clone()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: Some(Place {
            workspace: "SWM".into(),
            tab: "code".into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "w1".parse().unwrap(),
                tab: "w1:t1".parse().unwrap(),
                pane: "w1:p1".parse().unwrap(),
                session_id,
                verified_at_ms: 1,
            }),
        }),
        last_seen_ms: 1,
    };
    bus.relay.register(&reg).unwrap();
    let lane = reg.verified_place().unwrap().address(&machine);
    (name, lane)
}

fn message(to: &AgentName) -> Message {
    Message {
        from: "sender".parse().unwrap(),
        to: relay_core::Target::Agent(to.clone()),
        text: "SECRET order text".into(),
        sent_at_ms: 42,
        forwarded: false,
    }
}

fn server_ms(bus: &mut Bus) -> i64 {
    let (seconds, micros): (i64, i64) = redis::cmd("TIME").query(&mut bus.raw).unwrap();
    seconds * 1_000 + micros / 1_000
}

/// Claims up to `count` messages and records their delivery, returning the delivery time.
fn deliver(bus: &mut Bus, to: &AgentName, count: usize) -> i64 {
    let at = server_ms(bus);
    let batch = bus
        .relay
        .pop_inbox_budget(
            to,
            NonZeroUsize::new(count).unwrap(),
            1_000_000,
            Claim::new(Popper::Watcher)
                .with_lease(Duration::from_secs(600))
                .at(Now::AtMs(at)),
        )
        .unwrap()
        .batch
        .unwrap();
    bus.relay
        .observe_delivery(
            to,
            batch,
            Popper::Watcher,
            DeliveryEvidence::HerdrPrompt,
            Now::AtMs(at),
        )
        .unwrap();
    at
}

fn event(kind: ActivityKind, session: &str, ts: i64) -> ActivityEvent {
    ActivityEvent::new(kind, Harness::Claude, session, ts)
}

fn events(bus: &mut Bus) -> Vec<ReceiptEvent> {
    // Injected clocks put some receipts after the server's present.
    let now = server_ms(bus) + 3_600_000;
    let request = ReceiptRequest {
        since_ms: now - 7_200_000,
        until_ms: now,
        now_ms: now,
        page_size: 1_000,
        after: None,
    };
    bus.relay
        .receipt_export(&request)
        .unwrap()
        .events
        .into_iter()
        .map(|row| row.event)
        .collect()
}

fn count(bus: &mut Bus, outcome: ReceiptOutcome) -> usize {
    events(bus).iter().filter(|e| e.outcome == outcome).count()
}

fn order(ms: i64, ticket: Option<&str>) -> OrderSpec {
    OrderSpec::new(ms, ticket.map(str::to_owned)).unwrap()
}

#[trace("FR-067-AC-9")]
#[test]
fn the_recipients_first_prompt_after_delivery_records_one_acted_without_changing_the_ack_state() {
    let Some(mut bus) = Bus::connect("acted-basic") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    bus.relay.send(&message(&name)).unwrap();
    deliver(&mut bus, &name, 1);
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 0);
    assert!(
        bus.relay
            .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
            .unwrap()
    );
    let all = events(&mut bus);
    let acted: Vec<_> = all
        .iter()
        .filter(|e| e.outcome == ReceiptOutcome::Acted)
        .collect();
    assert_eq!(acted.len(), 1);
    let queued = all
        .iter()
        .find(|e| e.outcome == ReceiptOutcome::Queued)
        .unwrap();
    assert_eq!(acted[0].original_id, queued.original_id);
    assert_eq!(acted[0].session.as_deref(), Some("s-1"));
    let position = |o: ReceiptOutcome| all.iter().position(|e| e.outcome == o).unwrap();
    assert!(position(ReceiptOutcome::Delivered) < position(ReceiptOutcome::Acted));
    assert_eq!(count(&mut bus, ReceiptOutcome::Acknowledged), 0);
    assert_eq!(bus.relay.inflight_len(&name).unwrap(), 1);
    assert!(!serde_json::to_string(&all).unwrap().contains("SECRET"));
}

#[trace("FR-067-AC-10")]
#[test]
fn only_a_prompt_or_tool_event_of_the_session_after_delivery_acts_and_it_acts_once() {
    let Some(mut bus) = Bus::connect("acted-negative") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    // Written before the Delivered receipt, and a message that was never delivered.
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    bus.relay.send(&message(&name)).unwrap();
    bus.relay
        .record_activity(&lane, &event(ActivityKind::PreTool, "s-1", 1))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 0, "never delivered");
    deliver(&mut bus, &name, 1);
    for (kind, session) in [
        (ActivityKind::Prompt, "other-session"),
        (ActivityKind::Stop, "s-1"),
        (ActivityKind::Idle, "s-1"),
        (ActivityKind::Blocked, "s-1"),
        (ActivityKind::SessionStart, "s-1"),
        (ActivityKind::SessionEnd, "s-1"),
    ] {
        bus.relay
            .record_activity(&lane, &event(kind, session, 1))
            .unwrap();
    }
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 0);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::PostTool, "s-1", 1))
        .unwrap();
    bus.relay
        .record_activity(&lane, &event(ActivityKind::PostTool, "s-1", 1))
        .unwrap();
    assert_eq!(
        count(&mut bus, ReceiptOutcome::Acted),
        1,
        "a retry adds none"
    );
}

#[trace("FR-067-AC-10")]
#[test]
fn one_event_marks_at_most_fifty_delivered_messages() {
    let Some(mut bus) = Bus::connect("acted-cap") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    for _ in 0..60 {
        bus.relay.send(&message(&name)).unwrap();
    }
    deliver(&mut bus, &name, 60);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 50);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 60);
}

#[trace("FR-067-AC-12")]
#[test]
fn acted_follows_the_relay_servers_order_not_the_hosts_clock() {
    let Some(mut bus) = Bus::connect("acted-skew") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    bus.relay.send(&message(&name)).unwrap();
    let now = server_ms(&mut bus);
    let ahead = now + 600_000;
    let behind = now - 600_000;
    // A host 10 minutes ahead writes before Delivered: no Acted, whatever its stamp.
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", ahead))
        .unwrap();
    deliver(&mut bus, &name, 1);
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 0);
    // A host 10 minutes behind writes after Delivered: Acted, whatever its stamp.
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", behind))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 1);
}

#[trace("FR-071-AC-2")]
#[test]
fn a_plain_message_has_no_deadline_expiry_or_escalation() {
    let Some(mut bus) = Bus::connect("order-plain") else {
        return;
    };
    let (name, _) = lead(&mut bus, "s-1");
    bus.relay.send(&message(&name)).unwrap();
    let at = deliver(&mut bus, &name, 1);
    let far = Now::AtMs(at + 48 * 3_600_000);
    assert!(bus.relay.expire_orders(far).unwrap().is_empty());
    assert_eq!(count(&mut bus, ReceiptOutcome::AckExpired), 0);
    assert_eq!(bus.relay.list(None).unwrap().escalations.len(), 0);
}

#[trace("FR-071-AC-3", "FR-071-AC-7", "FR-067-AC-11")]
#[test]
fn a_delivered_order_never_acted_expires_once_and_escalates_once_across_restarts() {
    let Some(mut bus) = Bus::connect("order-expire") else {
        return;
    };
    let (name, _) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, Some("SWM-206"))))
        .unwrap();
    let at = deliver(&mut bus, &name, 1);
    assert!(
        bus.relay
            .expire_orders(Now::AtMs(at + 9_000))
            .unwrap()
            .is_empty(),
        "the deadline runs from Delivered"
    );
    let due = bus.relay.expire_orders(Now::AtMs(at + 10_001)).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].recipient, LANE);
    assert_eq!(due[0].sender, "sender");
    assert_eq!(due[0].last_state, "delivered");
    assert!(bus.relay.escalate_expired_order(&due[0], at).unwrap());
    // A restarted watcher scans again after the deadline and after AckExpired.
    for _ in 0..2 {
        let mut restarted = bus.another_connection();
        for order in restarted.expire_orders(Now::AtMs(at + 20_000)).unwrap() {
            assert!(!restarted.escalate_expired_order(&order, at).unwrap());
        }
    }
    assert_eq!(count(&mut bus, ReceiptOutcome::AckExpired), 1);
    let escalations = bus.relay.list(None).unwrap().escalations;
    assert_eq!(escalations.len(), 1);
    let context = &escalations[0].filing.context;
    assert!(context.contains(&due[0].original_id) && context.contains(LANE));
    assert!(context.contains("sender") && !context.contains("SECRET"));
    let filing = &escalations[0].filing;
    assert_eq!(
        (filing.ticket.is_none(), filing.kind.as_str()),
        (true, "order")
    );
    assert_eq!(filing.lead.as_str(), LANE, "the lane, not the hex queue");
    assert!(context.contains("SWM-206") && context.contains("delivered"));
    let all = events(&mut bus);
    let expired = all
        .iter()
        .find(|e| e.outcome == ReceiptOutcome::AckExpired)
        .unwrap();
    assert_eq!(expired.original_id, due[0].original_id);
    assert_eq!(expired.attempt_id, None, "distinct from an attempt timeout");
    assert_eq!(count(&mut bus, ReceiptOutcome::TimedOut), 0);
}

#[trace("FR-071-AC-4")]
#[test]
fn an_order_never_delivered_expires_measured_from_enqueue() {
    let Some(mut bus) = Bus::connect("order-dead-pane") else {
        return;
    };
    let (name, _) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    let queued = events(&mut bus)[0].observed_at_ms;
    assert!(
        bus.relay
            .expire_orders(Now::AtMs(queued + 9_000))
            .unwrap()
            .is_empty()
    );
    let due = bus.relay.expire_orders(Now::AtMs(queued + 10_001)).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].last_state, "queued");
    assert_eq!(count(&mut bus, ReceiptOutcome::AckExpired), 1);
    assert_eq!(count(&mut bus, ReceiptOutcome::Delivered), 0);
}

#[trace("FR-071-AC-5")]
#[test]
fn acted_before_the_deadline_cancels_it_and_acted_after_expiry_leaves_the_escalation() {
    let Some(mut bus) = Bus::connect("order-acted") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    let at = deliver(&mut bus, &name, 1);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    assert!(
        bus.relay
            .expire_orders(Now::AtMs(at + 20_000))
            .unwrap()
            .is_empty()
    );
    assert_eq!(count(&mut bus, ReceiptOutcome::AckExpired), 0);
    assert_eq!(bus.relay.list(None).unwrap().escalations.len(), 0);

    // Expired first, then acted.
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    let at = deliver(&mut bus, &name, 1);
    let due = bus.relay.expire_orders(Now::AtMs(at + 10_001)).unwrap();
    bus.relay.escalate_expired_order(&due[0], at).unwrap();
    bus.relay
        .record_activity(&lane, &event(ActivityKind::PostTool, "s-1", 1))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::AckExpired), 1);
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 2);
    assert_eq!(bus.relay.list(None).unwrap().escalations.len(), 1);
}

#[trace("FR-071-AC-6")]
#[test]
fn a_lease_timeout_and_requeue_do_not_move_the_order_deadline() {
    let Some(mut bus) = Bus::connect("order-lease") else {
        return;
    };
    let (name, _) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(60_000, None)))
        .unwrap();
    let due_key = bus.key("receipt-meta:orders-due");
    let score = |bus: &mut Bus| -> Vec<(String, f64)> {
        redis::cmd("ZRANGE")
            .arg(&due_key)
            .arg(0)
            .arg(-1)
            .arg("WITHSCORES")
            .query(&mut bus.raw)
            .unwrap()
    };
    let before = score(&mut bus);
    let at = server_ms(&mut bus);
    bus.relay
        .pop_inbox_budget(
            &name,
            NonZeroUsize::new(1).unwrap(),
            1_000_000,
            Claim::new(Popper::Watcher)
                .with_lease(Duration::from_millis(20))
                .at(Now::AtMs(at)),
        )
        .unwrap();
    bus.relay
        .reap_expired(&name, Now::AtMs(at + 1_000))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::TimedOut), 1);
    assert_eq!(score(&mut bus), before);
    assert!(
        bus.relay
            .expire_orders(Now::AtMs(at + 30_000))
            .unwrap()
            .is_empty()
    );
    assert_eq!(count(&mut bus, ReceiptOutcome::AckExpired), 0);
}

#[trace("FR-071-AC-8")]
#[test]
fn order_stats_count_unacted_orders_by_recipient_and_record_act_latency() {
    let Some(mut bus) = Bus::connect("order-stats") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    for _ in 0..2 {
        bus.relay
            .send_as(&message(&name), Some(&order(10_000, None)))
            .unwrap();
    }
    bus.relay.send(&message(&name)).unwrap();
    let terra: relay_core::MachineName = "terra".parse().unwrap();
    let (_, stats) = bus.relay.hook_health_and_order_stats(10, &terra).unwrap();
    assert_eq!(stats.unacted.get(LANE), Some(&2));
    // Another host's watcher does not report the lanes of this one.
    let luna: relay_core::MachineName = "luna".parse().unwrap();
    let (_, elsewhere) = bus.relay.hook_health_and_order_stats(10, &luna).unwrap();
    assert!(elsewhere.unacted.is_empty() && elsewhere.latency.count == 0);
    deliver(&mut bus, &name, 1);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    let (_, stats) = bus.relay.hook_health_and_order_stats(10, &terra).unwrap();
    assert_eq!(stats.unacted.get(LANE), Some(&1), "falls when one is acted");
    let (_, elsewhere) = bus.relay.hook_health_and_order_stats(10, &luna).unwrap();
    assert_eq!(
        elsewhere.latency.count, 0,
        "the latency belongs to the host of the lane"
    );
    assert_eq!(stats.latency.count, 1);
    assert_eq!(stats.latency.buckets.iter().sum::<u64>(), 1);
    assert_eq!(stats.latency.buckets[0], 1, "within the first bucket");
}

// FR-071-AC-10 is met only with its other half: `focus_info` shows the ticket parsed from
// the lane's newest branch (SWM-205). This test covers the expected-ticket half.
#[trace("FR-071-AC-10")]
#[test]
fn the_receipt_export_returns_the_expected_ticket_of_an_order() {
    let Some(mut bus) = Bus::connect("order-expect") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, Some("SWM-206"))))
        .unwrap();
    deliver(&mut bus, &name, 1);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    let all = events(&mut bus);
    assert!(all.len() >= 3);
    assert!(
        all.iter()
            .all(|e| e.expect_ticket.as_deref() == Some("SWM-206"))
    );
}

#[trace("FR-071-AC-1")]
#[test]
fn an_order_spec_refuses_a_deadline_outside_ten_seconds_to_a_day_and_a_long_ticket() {
    assert!(OrderSpec::new(9_999, None).is_err());
    assert!(OrderSpec::new(86_400_001, None).is_err());
    assert!(OrderSpec::new(10_000, None).is_ok());
    assert!(OrderSpec::new(86_400_000, Some("A".repeat(64))).is_ok());
    assert!(OrderSpec::new(10_000, Some("A".repeat(65))).is_err());
}

#[trace("FR-067-AC-10")]
#[test]
fn an_event_of_the_same_session_id_on_another_machine_does_not_act() {
    let Some(mut bus) = Bus::connect("acted-machine") else {
        return;
    };
    let (name, _) = lead_on(&mut bus, "terra", "s-1");
    let (_, other_lane) = lead_on(&mut bus, "luna", "s-1");
    bus.relay.send(&message(&name)).unwrap();
    deliver(&mut bus, &name, 1);
    bus.relay
        .record_activity(&other_lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 0);
}

#[trace("FR-067-AC-9")]
#[test]
fn the_hook_call_that_delivered_the_mail_never_acts_but_the_next_event_does() {
    let Some(mut bus) = Bus::connect("acted-same-call") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    deliver(&mut bus, &name, 1);
    let mut same_call = event(ActivityKind::PostTool, "s-1", 1);
    same_call.delivered_mail = true;
    bus.relay.record_activity(&lane, &same_call).unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 0);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 1);
}

#[trace("FR-071-AC-5")]
#[test]
fn a_plain_inbox_pop_of_an_order_records_delivered_so_it_can_be_acted() {
    let Some(mut bus) = Bus::connect("order-plain-pop") else {
        return;
    };
    let (name, lane) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    bus.relay
        .pop_inbox(&name, NonZeroUsize::new(5).unwrap())
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Delivered), 1);
    assert_eq!(count(&mut bus, ReceiptOutcome::Removed), 1);
    bus.relay
        .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
        .unwrap();
    assert_eq!(count(&mut bus, ReceiptOutcome::Acted), 1);
    let at = server_ms(&mut bus);
    assert!(
        bus.relay
            .expire_orders(Now::AtMs(at + 20_000))
            .unwrap()
            .is_empty()
    );
}

#[trace("FR-071-AC-7", "FR-045-AC-34")]
#[test]
fn an_escalation_that_keeps_failing_is_given_up_and_does_not_block_the_rest() {
    let Some(mut bus) = Bus::connect("order-poison") else {
        return;
    };
    // A queue whose session part cannot be a session id: its escalation never validates.
    let (poison, _) = lead_on(&mut bus, "terra", "bad_id");
    for _ in 0..25 {
        bus.relay
            .send_as(&message(&poison), Some(&order(10_000, None)))
            .unwrap();
    }
    let (good, _) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&good), Some(&order(10_000, None)))
        .unwrap();
    let at = server_ms(&mut bus);
    let mut filed = 0;
    for _ in 0..40 {
        let due = bus.relay.expire_orders(Now::AtMs(at + 20_000)).unwrap();
        if due.is_empty() {
            break;
        }
        for order in &due {
            match bus.relay.escalate_expired_order(order, at) {
                Ok(true) => filed += 1,
                Ok(false) => {}
                Err(_) => {
                    bus.relay.order_escalation_failed(order, 3).unwrap();
                }
            }
        }
    }
    assert_eq!(
        filed, 1,
        "the good order is escalated behind 25 poisoned ones"
    );
    let terra: relay_core::MachineName = "terra".parse().unwrap();
    let (_, stats) = bus.relay.hook_health_and_order_stats(10, &terra).unwrap();
    assert_eq!(stats.escalation_failed, 25);
    // The count belongs to the host of the lane: another host's watcher reports none.
    let luna: relay_core::MachineName = "luna".parse().unwrap();
    let (_, elsewhere) = bus.relay.hook_health_and_order_stats(10, &luna).unwrap();
    assert_eq!(elsewhere.escalation_failed, 0);
    assert!(
        bus.relay
            .expire_orders(Now::AtMs(at + 30_000))
            .unwrap()
            .is_empty()
    );
}

#[trace("FR-071-AC-1")]
#[test]
fn an_order_to_a_recipient_without_a_lane_address_is_refused_and_nothing_is_queued() {
    let Some(mut bus) = Bus::connect("order-no-lane") else {
        return;
    };
    let plain: AgentName = "plainlead".parse().unwrap();
    bus.relay
        .register(&Registration {
            name: plain.clone(),
            machine: "terra".parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 1,
        })
        .unwrap();
    assert!(
        bus.relay
            .send_as(&message(&plain), Some(&order(10_000, None)))
            .is_err()
    );
    assert_eq!(bus.relay.inbox_len(&plain).unwrap(), 0);
    assert!(events(&mut bus).is_empty());
    // The same lead takes a plain message.
    bus.relay.send(&message(&plain)).unwrap();
}

fn dump_all(bus: &mut Bus) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut keys = bus.keys();
    keys.sort();
    keys.into_iter()
        .map(|key| {
            let bytes: Vec<u8> = redis::cmd("DUMP").arg(&key).query(&mut bus.raw).unwrap();
            (key, bytes)
        })
        .collect()
}

#[trace("FR-071-AC-11")]
#[test]
fn expired_orders_lists_every_unacted_expired_order_and_changes_nothing() {
    let Some(mut bus) = Bus::connect("order-expired-read") else {
        return;
    };
    assert!(bus.relay.expired_orders().unwrap().orders.is_empty());
    let (name, _) = lead(&mut bus, "s-1");
    // A plain message and an open order never appear.
    bus.relay.send_as(&message(&name), None).unwrap();
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    let at = deliver(&mut bus, &name, 2);
    let at_first = at;
    // Delivered, ticketed, expired, escalation filed.
    let due = bus.relay.expire_orders(Now::AtMs(at + 10_001)).unwrap();
    assert_eq!(due.len(), 1);
    bus.relay.escalate_expired_order(&due[0], at).unwrap();
    let delivered_id = due[0].original_id.clone();
    // Never delivered, left pending.
    bus.relay
        .send_as(&message(&name), Some(&order(20_000, Some("SWM-9"))))
        .unwrap();
    let queued_at = server_ms(&mut bus);
    let pending = bus
        .relay
        .expire_orders(Now::AtMs(queued_at + 30_000))
        .unwrap();
    assert_eq!(pending.len(), 1);
    let pending_id = pending[0].original_id.clone();
    // Acted after expiry: excluded.
    let (other, other_lane) = lead(&mut bus, "s-2");
    bus.relay
        .send_as(&message(&other), Some(&order(10_000, None)))
        .unwrap();
    let at = deliver(&mut bus, &other, 1);
    let acted = bus.relay.expire_orders(Now::AtMs(at + 10_001)).unwrap();
    let acted = acted.iter().find(|o| o.original_id != pending_id).unwrap();
    let acted_id = acted.original_id.clone();
    bus.relay.escalate_expired_order(acted, at).unwrap();
    bus.relay
        .record_activity(&other_lane, &event(ActivityKind::PostTool, "s-2", 1))
        .unwrap();
    // A poisoned lane whose escalation fails.
    let (poison, _) = lead_on(&mut bus, "terra", "bad_id");
    bus.relay
        .send_as(&message(&poison), Some(&order(10_000, None)))
        .unwrap();
    let at = server_ms(&mut bus);
    let poisoned = bus.relay.expire_orders(Now::AtMs(at + 20_000)).unwrap();
    for o in poisoned.iter().filter(|o| o.original_id != pending_id) {
        for _ in 0..3 {
            bus.relay.order_escalation_failed(o, 3).unwrap();
        }
    }

    let before = dump_all(&mut bus);
    let listed = bus.relay.expired_orders().unwrap();
    assert_eq!(dump_all(&mut bus), before, "the read changes no key");
    assert!(!listed.truncated);
    assert!(listed.orders.iter().all(|o| o.original_id != acted_id));
    assert_eq!(listed.orders.len(), 3);
    let by = |id: &str| listed.orders.iter().find(|o| o.original_id == id).unwrap();
    let first = by(&delivered_id);
    assert_eq!(first.recipient, LANE);
    assert!(first.delivered_before_expiry);
    assert_eq!(first.last_state, "delivered");
    assert_eq!(first.escalation, OrderEscalationState::Filed);
    assert_eq!(first.deadline_ms, at_first + 10_000);
    assert_eq!(first.expired_at_ms, first.deadline_ms + 1);
    let second = by(&pending_id);
    assert!(!second.delivered_before_expiry);
    assert_eq!(second.last_state, "queued");
    assert_eq!(second.expect_ticket.as_deref(), Some("SWM-9"));
    assert_eq!(second.escalation, OrderEscalationState::Pending);
    assert_eq!(second.deadline_ms, second.enqueued_at_ms + 20_000);
    assert!(
        listed
            .orders
            .iter()
            .any(|o| o.escalation == OrderEscalationState::Failed)
    );
    assert!(
        listed
            .orders
            .windows(2)
            .all(|w| w[0].expired_at_ms >= w[1].expired_at_ms)
    );
}

#[trace("FR-071-AC-11")]
#[test]
fn expired_orders_returns_at_most_the_cap_and_says_when_more_exist() {
    let Some(mut bus) = Bus::connect("order-expired-cap") else {
        return;
    };
    let (name, _) = lead(&mut bus, "s-1");
    let total = EXPIRED_ORDERS_CAP + 1;
    for _ in 0..total {
        bus.relay
            .send_as(&message(&name), Some(&order(10_000, None)))
            .unwrap();
    }
    let at = server_ms(&mut bus);
    let mut expired = 0;
    while expired < total {
        let batch = bus.relay.expire_orders(Now::AtMs(at + 20_000)).unwrap();
        for o in &batch {
            bus.relay.escalate_expired_order(o, at).unwrap();
        }
        expired = bus
            .relay
            .hook_health_and_order_stats(1, &"terra".parse().unwrap())
            .map(|(_, s)| total - usize::try_from(s.unacted.values().sum::<u64>()).unwrap())
            .unwrap();
    }
    let listed = bus.relay.expired_orders().unwrap();
    assert_eq!(listed.orders.len(), EXPIRED_ORDERS_CAP);
    assert!(listed.truncated);
}

#[derive(Clone, Copy, Debug)]
enum Late {
    Ack,
    Requeue,
    DeadLetter,
    LeaseTimeout,
}

#[trace("FR-071-AC-11")]
#[test]
fn an_acted_order_stays_out_of_the_list_whatever_receipt_follows_its_acted() {
    for late in [
        Late::Ack,
        Late::Requeue,
        Late::DeadLetter,
        Late::LeaseTimeout,
    ] {
        let Some(mut bus) = Bus::connect(&format!("order-late-{late:?}").to_lowercase()) else {
            return;
        };
        let (name, lane) = lead(&mut bus, "s-1");
        bus.relay
            .send_as(&message(&name), Some(&order(10_000, None)))
            .unwrap();
        let at = server_ms(&mut bus);
        let claim = || {
            Claim::new(Popper::Watcher)
                .with_lease(Duration::from_secs(600))
                .at(Now::AtMs(at))
        };
        let size = NonZeroUsize::new(1).unwrap();
        let batch = bus
            .relay
            .pop_inbox_budget(&name, size, 1_000_000, claim())
            .unwrap()
            .batch
            .unwrap();
        bus.relay
            .observe_delivery(
                &name,
                batch,
                Popper::Watcher,
                DeliveryEvidence::HerdrPrompt,
                Now::AtMs(at),
            )
            .unwrap();
        let due = bus.relay.expire_orders(Now::AtMs(at + 10_001)).unwrap();
        bus.relay.escalate_expired_order(&due[0], at).unwrap();
        assert_eq!(bus.relay.expired_orders().unwrap().orders.len(), 1);
        bus.relay
            .record_activity(&lane, &event(ActivityKind::Prompt, "s-1", 1))
            .unwrap();
        assert!(bus.relay.expired_orders().unwrap().orders.is_empty());
        match late {
            Late::Ack => {
                bus.relay.ack(&name, batch, Popper::Watcher).unwrap();
            }
            Late::Requeue => {
                bus.relay
                    .requeue_batch(&name, batch, Popper::Watcher)
                    .unwrap();
                let again = bus
                    .relay
                    .pop_inbox_budget(&name, size, 1_000_000, claim())
                    .unwrap()
                    .batch
                    .unwrap();
                bus.relay
                    .observe_delivery(
                        &name,
                        again,
                        Popper::Watcher,
                        DeliveryEvidence::HerdrPrompt,
                        Now::AtMs(at + 1),
                    )
                    .unwrap();
            }
            Late::DeadLetter => {
                bus.relay
                    .dead_letter_batch(
                        &name,
                        batch,
                        Popper::Watcher,
                        relay_core::receipts::DeadLetterReason::ReceiverDenied,
                    )
                    .unwrap();
            }
            Late::LeaseTimeout => {
                bus.relay
                    .reap_expired(&name, Now::AtMs(at + 700_000))
                    .unwrap();
            }
        }
        assert!(
            bus.relay.expired_orders().unwrap().orders.is_empty(),
            "{late:?} after Acted must not list the order again"
        );
    }
}

#[trace("FR-071-AC-11")]
#[test]
fn the_list_reports_its_coverage_unreadable_rows_and_the_spelling_of_the_lane_lists() {
    let Some(mut bus) = Bus::connect("order-expired-coverage") else {
        return;
    };
    let empty = bus.relay.expired_orders().unwrap();
    assert_eq!(empty.indexed_since_ms, None);
    let (name, _) = lead(&mut bus, "s-1");
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    let at = server_ms(&mut bus);
    let due = bus.relay.expire_orders(Now::AtMs(at + 20_000)).unwrap();
    let id = due[0].original_id.clone();
    let listed = bus.relay.expired_orders().unwrap();
    assert_eq!(listed.indexed_since_ms, Some(at + 20_000));
    assert_eq!((listed.skipped, listed.truncated), (0, false));

    // The lane lists spell the lane; the stored spelling is only a fallback.
    let order_key = bus.key(&format!("receipt-meta:order:{id}"));
    redis::cmd("HSET")
        .arg(&order_key)
        .arg("to")
        .arg("TERRA:swm:CODE")
        .exec(&mut bus.raw)
        .unwrap();
    assert_eq!(
        bus.relay.expired_orders().unwrap().orders[0].recipient,
        "TERRA:swm:CODE"
    );
    bus.relay
        .publish_lane_list(&"terra".parse().unwrap(), &[LANE.to_owned()])
        .unwrap();
    assert_eq!(
        bus.relay.expired_orders().unwrap().orders[0].recipient,
        LANE
    );

    // A row that cannot be decoded is counted, never dropped silently.
    let index = bus.key("receipt-meta:orders-expired");
    redis::cmd("HSET")
        .arg(bus.key("receipt-meta:order:corrupt"))
        .arg("to")
        .arg(LANE)
        .arg("status")
        .arg("expired")
        .arg("enqueued_at")
        .arg("not-a-number")
        .arg("ack_ms")
        .arg("1")
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("ZADD")
        .arg(&index)
        .arg(1)
        .arg("corrupt")
        .exec(&mut bus.raw)
        .unwrap();
    let with_corrupt = bus.relay.expired_orders().unwrap();
    assert_eq!(with_corrupt.orders.len(), 1);
    assert_eq!(with_corrupt.skipped, 1);

    let floor = server_ms(&mut bus);
    // More index rows than one call examines: the call says so.
    let mut pipeline = redis::pipe();
    for n in 0..1_500 {
        pipeline
            .cmd("ZADD")
            .arg(&index)
            .arg(floor)
            .arg(format!("gone-{n}"));
    }
    pipeline.exec(&mut bus.raw).unwrap();
    let wide = bus.relay.expired_orders().unwrap();
    assert!(wide.truncated);
    assert_eq!(wide.orders.len(), 1, "the newest rows are examined first");

    // Past 10,000 index rows the oldest go and the list says it may be short.
    let mut pipeline = redis::pipe();
    for n in 0..10_000 {
        pipeline
            .cmd("ZADD")
            .arg(&index)
            .arg(floor)
            .arg(format!("filler-{n}"));
    }
    pipeline.exec(&mut bus.raw).unwrap();
    bus.relay
        .send_as(&message(&name), Some(&order(10_000, None)))
        .unwrap();
    let at = server_ms(&mut bus);
    bus.relay.expire_orders(Now::AtMs(at + 20_000)).unwrap();
    let trimmed: i64 = redis::cmd("ZCARD").arg(&index).query(&mut bus.raw).unwrap();
    assert_eq!(trimmed, 10_000);
    assert!(bus.relay.expired_orders().unwrap().truncated);
}
