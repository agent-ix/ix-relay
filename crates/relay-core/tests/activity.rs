//! Hook activity stream and last-activity record against real Redis (FR-068).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]
use relay_core::{
    ACTIVITY_MAX_EVENTS, ActivityEvent, ActivityKind, Address, AgentName, Harness, HerdrPlaceIds,
    Place, Registration,
};
use relay_test_support::Bus;

fn lead(bus: &mut Bus, place: bool) -> AgentName {
    let machine = "terra".parse().unwrap();
    let session = "s-1".parse().unwrap();
    let name = AgentName::for_session(&machine, &session);
    bus.relay
        .register(&Registration {
            name: name.clone(),
            machine,
            kind: "claude".into(),
            session_id: Some(session.clone()),
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: place.then(|| Place {
                workspace: "SWM".into(),
                tab: "code".into(),
                herdr_ids: Some(HerdrPlaceIds {
                    workspace: "w1".parse().unwrap(),
                    tab: "w1:t1".parse().unwrap(),
                    pane: "w1:p1".parse().unwrap(),
                    session_id: session,
                    verified_at_ms: 1,
                }),
            }),
            last_seen_ms: 1,
        })
        .unwrap();
    name
}

fn event(kind: ActivityKind) -> ActivityEvent {
    ActivityEvent::new(kind, Harness::Claude, "s-1", 1_000)
}

fn lane() -> Address {
    "terra:SWM:code".parse().unwrap()
}

/// Trace: FR-068-AC-1, FR-068-AC-2
#[test]
fn an_event_is_stored_with_the_lane_address_and_server_time() {
    let Some(mut bus) = Bus::connect("activity-shape") else {
        return;
    };
    let name = lead(&mut bus, true);
    let mut e = event(ActivityKind::PostTool);
    e.tool = Some("Edit".into());
    assert!(bus.relay.record_activity(&name, &e).unwrap());
    let stored = bus.relay.activity_events(10).unwrap();
    assert_eq!(stored.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&stored[0]).unwrap();
    assert_eq!(v["addr"], "terra:SWM:code");
    assert_eq!(v["kind"], "post_tool");
    assert_eq!(v["tool"], "Edit");
    assert_eq!(v["session"], "s-1");
    assert_eq!(v["ts"], 1_000);
    assert!(v["server_ms"].as_i64().unwrap() > 1_700_000_000_000);
    assert!(stored[0].len() <= relay_core::ACTIVITY_MAX_EVENT_BYTES);
}

/// Trace: FR-068-AC-9
#[test]
fn a_lead_with_no_lane_address_writes_nothing() {
    let Some(mut bus) = Bus::connect("activity-no-address") else {
        return;
    };
    let unplaced = lead(&mut bus, false);
    assert!(
        !bus.relay
            .record_activity(&unplaced, &event(ActivityKind::Prompt))
            .unwrap()
    );
    let unknown: AgentName = "nobody".parse().unwrap();
    assert!(
        !bus.relay
            .record_activity(&unknown, &event(ActivityKind::Prompt))
            .unwrap()
    );
    assert_eq!(bus.relay.activity_len().unwrap(), 0);
}

/// Trace: FR-068-AC-7
#[test]
fn the_stream_holds_at_most_20000_events_and_the_last_record_survives() {
    let Some(mut bus) = Bus::connect("activity-cap") else {
        return;
    };
    let name = lead(&mut bus, true);
    bus.relay
        .record_activity(&name, &event(ActivityKind::Stop))
        .unwrap();
    bus.relay
        .record_activity(&name, &event(ActivityKind::Idle))
        .unwrap();
    for _ in 0..ACTIVITY_MAX_EVENTS {
        bus.relay
            .record_activity(&name, &event(ActivityKind::PreTool))
            .unwrap();
    }
    assert_eq!(bus.relay.activity_len().unwrap(), ACTIVITY_MAX_EVENTS);
    // The stop and idle events were trimmed from the stream; the record keeps them.
    let last = bus.relay.activity_last(&lane()).unwrap();
    for kind in ["stop", "idle", "pre_tool"] {
        assert!(
            last.contains_key(&format!("kind:{kind}")),
            "{kind}: {last:?}"
        );
    }
    assert!(last["any"].contains("pre_tool"));
    let ttl: i64 = redis::cmd("TTL")
        .arg(format!("{}activity-last:terra:SWM:code", bus.prefix))
        .query(&mut bus.raw)
        .unwrap();
    assert!((1..=7 * 24 * 3600).contains(&ttl), "ttl {ttl}");
}

/// Trace: FR-068-AC-7
#[test]
fn events_older_than_24_hours_are_dropped_on_the_next_write() {
    let Some(mut bus) = Bus::connect("activity-age") else {
        return;
    };
    let name = lead(&mut bus, true);
    let now_ms: i64 = {
        let (s, us): (i64, i64) = redis::cmd("TIME").query(&mut bus.raw).unwrap();
        s * 1000 + us / 1000
    };
    let key = bus.key("activity");
    for age_h in [48_i64, 25] {
        let id = format!("{}-0", now_ms - age_h * 3_600_000);
        let _: String = redis::cmd("XADD")
            .arg(&key)
            .arg(&id)
            .arg("event")
            .arg("{}")
            .query(&mut bus.raw)
            .unwrap();
    }
    let _: String = redis::cmd("XADD")
        .arg(&key)
        .arg(format!("{}-0", now_ms - 3_600_000))
        .arg("event")
        .arg("{}")
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(bus.relay.activity_len().unwrap(), 3);
    bus.relay
        .record_activity(&name, &event(ActivityKind::Prompt))
        .unwrap();
    // The 25 h and 48 h events are gone; the 1 h one and the new one stay.
    assert_eq!(bus.relay.activity_len().unwrap(), 2);
}
