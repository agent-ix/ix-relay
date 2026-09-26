//! The relay's own health records against a real Redis: the watcher heartbeat,
//! the doctor's queue reads, the registration liveness refresh and the session
//! cache expiry.
//!
//! Needs `IX_RELAY_TEST_URL` (`make test-redis`); without it each test returns
//! early and says so on stderr.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

use std::time::{Duration, Instant};

use relay_core::{
    AgentName, Message, Registration,
    health::{HeartbeatRead, WatcherHeartbeat},
};
use relay_test_support::Bus;

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn heartbeat(machine: &str, tick: i64) -> WatcherHeartbeat {
    WatcherHeartbeat {
        machine: machine.parse().unwrap(),
        pid: 1,
        version: "test".into(),
        started_at_ms: 1,
        last_tick_ms: tick,
        last_sweep_ms: None,
        sweep_secs: 15,
        herdr_ok: true,
        herdr_problem: None,
        herdr_blocks_delivery: false,
        redis_ok: true,
        leads_watched: 0,
        otlp: Some(relay_core::health::OtlpStatus::Disabled),
        last_error: None,
    }
}

fn reg(n: &str, machine: &str, last_seen_ms: i64) -> Registration {
    Registration {
        name: name(n),
        machine: machine.parse().unwrap(),
        kind: "claude".into(),
        session_id: None,
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: None,
        last_seen_ms,
    }
}

fn msg(to: &str, at: i64) -> Message {
    Message {
        from: name("sender"),
        to: to.parse().unwrap(),
        text: "hi".into(),
        sent_at_ms: at,
    }
}

/// Tracing: TC-028-04
/// ACs: FR-028-AC-4
///
/// A heartbeat written with a 60 s expiry is stored with that expiry, and one
/// written with a short expiry is gone once it passes: a missing key means no
/// watcher ticked recently.
#[test]
fn tc_028_04_a_heartbeat_carries_its_expiry_and_vanishes_with_it() {
    let Some(mut bus) = Bus::connect("hb_expiry") else {
        return;
    };
    let hb = heartbeat("hub", 5);
    bus.relay
        .write_heartbeat(&hb, Duration::from_secs(60))
        .unwrap();
    let ttl_ms: i64 = redis::cmd("PTTL")
        .arg(bus.key("health:watcher:hub"))
        .query(&mut bus.raw)
        .unwrap();
    assert!((59_000..=60_000).contains(&ttl_ms), "ttl {ttl_ms}");
    assert_eq!(
        bus.relay.heartbeat(&"hub".parse().unwrap()).unwrap(),
        Some(HeartbeatRead::Live(Box::new(hb.clone())))
    );

    bus.relay
        .write_heartbeat(&hb, Duration::from_millis(100))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while bus
        .relay
        .heartbeat(&"hub".parse().unwrap())
        .unwrap()
        .is_some()
    {
        assert!(Instant::now() < deadline, "the heartbeat never expired");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The key is gone; the non-expiring marker still says when it last ticked.
    let after = bus.relay.heartbeats(&["hub".parse().unwrap()]).unwrap();
    assert!(after.found.is_empty());
    assert_eq!(after.last_ticks.get(&"hub".parse().unwrap()), Some(&5));
}

#[test]
fn heartbeats_lists_every_machine_marks_an_unreadable_one_and_reads_named_ones_directly() {
    let Some(mut bus) = Bus::connect("hb_list") else {
        return;
    };
    for machine in ["node-a", "hub"] {
        bus.relay
            .write_heartbeat(&heartbeat(machine, 1), Duration::from_secs(60))
            .unwrap();
    }
    redis::cmd("SET")
        .arg(bus.key("health:watcher:node-b"))
        .arg("not json")
        .exec(&mut bus.raw)
        .unwrap();
    let all = bus.relay.heartbeats(&[]).unwrap();
    let names: Vec<_> = all.found.iter().map(|h| h.machine.as_str()).collect();
    // `machines` is a `BTreeSet<MachineName>`, so this is alphabetical.
    assert_eq!(names, ["hub", "node-a", "node-b"]);
    assert!(matches!(all.found[0].read, HeartbeatRead::Live(_)));
    assert!(matches!(all.found[1].read, HeartbeatRead::Live(_)));
    assert_eq!(all.found[2].read, HeartbeatRead::Undecodable);
    assert_eq!(all.truncated, None);
    // The marker outlives the key: it holds each machine's last tick.
    assert_eq!(all.last_ticks.get(&"hub".parse().unwrap()), Some(&1));

    // A named machine with no key is simply absent; a named one that exists is
    // read directly, whatever its case or domain.
    let named = bus
        .relay
        .heartbeats(&["Hub.local".parse().unwrap(), "nowhere".parse().unwrap()])
        .unwrap();
    assert_eq!(named.found.len(), 3);
}

/// Tracing: TC-029-08
/// ACs: FR-029-AC-8
///
/// A machine the caller names is found by a direct read even when the scan is
/// cut before it: a huge keyspace cannot produce a false "no heartbeat".
#[test]
fn a_named_machine_is_found_even_when_the_scan_is_full() {
    let Some(mut bus) = Bus::connect("hb_named_full") else {
        return;
    };
    let mut pipe = redis::pipe();
    for i in 0..relay_core::MAX_HEARTBEAT_KEYS + 20 {
        pipe.cmd("SET")
            .arg(bus.key(&format!("health:watcher:m{i:04}")))
            .arg("x")
            .ignore();
    }
    let () = pipe.query(&mut bus.raw).unwrap();
    bus.relay
        .write_heartbeat(&heartbeat("zzz-last", 9), Duration::from_secs(60))
        .unwrap();
    let found = bus
        .relay
        .heartbeats(&["zzz-last".parse().unwrap()])
        .unwrap();
    assert!(found.truncated.is_some());
    assert!(
        found
            .found
            .iter()
            .any(|h| h.machine.as_str() == "zzz-last" && matches!(h.read, HeartbeatRead::Live(_)))
    );
}

/// Tracing: TC-029-05
/// ACs: FR-029-AC-5
///
/// Reading the overviews, the backlog and the leases changes nothing: not a
/// length, not a value, not an expiry.
#[test]
fn tc_029_05_the_health_reads_change_no_inbox() {
    let Some(mut bus) = Bus::connect("overview") else {
        return;
    };
    for n in ["a", "b"] {
        bus.relay.register(&reg(n, "hub", 1)).unwrap();
    }
    bus.relay.send(&msg("a", 100)).unwrap();
    bus.relay.send(&msg("a", 200)).unwrap();
    // One in-flight row that is long overdue, and one that is not.
    redis::cmd("RPUSH")
        .arg(bus.key("inflight:b"))
        .arg("7 hook 1 {\"x\":1}")
        .arg("8 hook 99999999999999 {\"x\":2}")
        .exec(&mut bus.raw)
        .unwrap();
    redis::cmd("RPUSH")
        .arg(bus.key("deadletter:a"))
        .arg("x")
        .exec(&mut bus.raw)
        .unwrap();

    let (a, b) = (name("a"), name("b"));
    let overviews = bus.relay.inbox_overviews(&[&a, &b]).unwrap();
    let a_view = overviews[0].as_ref().unwrap();
    assert_eq!((a_view.len, a_view.oldest_sent_at_ms), (2, Some(100)));
    let b_view = overviews[1].as_ref().unwrap();
    assert_eq!((b_view.len, b_view.oldest_sent_at_ms), (0, None));

    let leases = bus.relay.overdue_leases(&["b".parse().unwrap()]).unwrap();
    assert_eq!(leases.len(), 1);
    let b_lease = &leases[0];
    assert_eq!(
        (
            b_lease.entries,
            b_lease.batches,
            b_lease.overdue,
            b_lease.rows_cut
        ),
        (2, 2, 1, false)
    );
    assert!(b_lease.most_overdue_ms > 1_000_000);
    assert!(bus.relay.overdue_leases(&[]).unwrap().is_empty());

    let backlog = bus.relay.backlog().unwrap();
    assert_eq!(backlog.leads.len(), 2);

    // Everything the reads could have touched, byte for byte: each key's dump
    // and whether it has an expiry.
    let snapshot = |raw: &mut redis::Connection| {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(format!("{}*", bus.prefix))
            .query(raw)
            .unwrap();
        let mut state: Vec<(String, Vec<u8>, i64)> = keys
            .into_iter()
            .map(|k| {
                let dump: Vec<u8> = redis::cmd("DUMP").arg(&k).query(raw).unwrap();
                let ttl: i64 = redis::cmd("PTTL").arg(&k).query(raw).unwrap();
                (k, dump, ttl)
            })
            .collect();
        state.sort();
        state
    };
    let before = snapshot(&mut bus.raw);
    bus.relay.inbox_overviews(&[&a, &b]).unwrap();
    bus.relay
        .overdue_leases(&["a".parse().unwrap(), "b".parse().unwrap()])
        .unwrap();
    bus.relay.backlog().unwrap();
    bus.relay.heartbeats(&["hub".parse().unwrap()]).unwrap();
    bus.relay.ping().unwrap();
    assert_eq!(snapshot(&mut bus.raw), before);
}

/// Tracing: TC-042-02
/// ACs: FR-042-AC-2
///
/// The liveness refresh is a compare-and-set. It moves `last_seen_ms` forward
/// on the registration that was read, keeps members this binary does not know,
/// and never brings back a pruned lead nor overwrites a newer registration.
#[test]
fn tc_042_02_the_last_seen_refresh_is_a_compare_and_set_that_never_resurrects() {
    let Some(mut bus) = Bus::connect("touch") else {
        return;
    };
    let lead = name("lead");
    bus.plant(
        "lead",
        br#"{"name":"lead","machine":"hub","kind":"claude","session_id":null,"cwd":null,
        "herdr_pane":"w1:p1","place":null,"last_seen_ms":1000,"added_later":7}"#,
    );
    assert!(bus.relay.touch_last_seen(&lead, 1000, 5000).unwrap());
    let stored: String = redis::cmd("HGET")
        .arg(bus.key("agents"))
        .arg("lead")
        .query(&mut bus.raw)
        .unwrap();
    let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored["last_seen_ms"], 5000);
    assert_eq!(stored["added_later"], 7, "unknown members survive");

    // Stale expectation: someone registered again meanwhile.
    assert!(!bus.relay.touch_last_seen(&lead, 1000, 9000).unwrap());
    // Not newer: nothing to do.
    assert!(!bus.relay.touch_last_seen(&lead, 5000, 5000).unwrap());
    // Pruned: the record is gone and stays gone.
    bus.relay.prune(&lead, 5000).unwrap().unwrap();
    assert!(!bus.relay.touch_last_seen(&lead, 5000, 9000).unwrap());
    assert!(bus.relay.agent(&lead).unwrap().is_none());
}

/// Tracing: TC-042-04
/// ACs: FR-042-AC-4
///
/// Looking a session up renews the expiry of its remembered name and its
/// suppression mark, and does not create either.
#[test]
fn tc_042_04_reading_a_session_renews_its_cache_expiry_and_creates_nothing() {
    let Some(mut bus) = Bus::connect("session_ttl") else {
        return;
    };
    for key in ["session:s1", "suppressed:s2"] {
        redis::cmd("SET")
            .arg(bus.key(key))
            .arg("lead")
            .arg("EX")
            .arg(100)
            .exec(&mut bus.raw)
            .unwrap();
    }
    let probe = bus.relay.probe_session(&"s1".parse().unwrap()).unwrap();
    assert_eq!(probe.name, Some(name("lead")));
    assert!(
        bus.relay
            .probe_session(&"s2".parse().unwrap())
            .unwrap()
            .suppressed
    );
    for key in ["session:s1", "suppressed:s2"] {
        let ttl: i64 = redis::cmd("TTL")
            .arg(bus.key(key))
            .query(&mut bus.raw)
            .unwrap();
        assert!(ttl > 29 * 24 * 3600, "{key} ttl {ttl}");
    }
    let unknown = bus.relay.probe_session(&"nobody".parse().unwrap()).unwrap();
    assert_eq!(unknown, relay_core::SessionProbe::default());
    let exists: bool = redis::cmd("EXISTS")
        .arg(bus.key("session:nobody"))
        .query(&mut bus.raw)
        .unwrap();
    assert!(!exists);
}

/// Tracing: TC-008-06
/// ACs: FR-008-AC-6
///
/// The last-tick marker is per machine, and the machine is the normalised name:
/// fields that normalise to the same machine (`Hub` and `hub`, which only
/// something other than a watcher writes) are one machine and the newest tick
/// wins; a field that names no machine, and a heartbeat key with such a suffix,
/// are counted, never silently dropped.
#[test]
fn marker_fields_of_one_machine_merge_to_the_newest_and_unreadable_ones_are_counted() {
    let Some(mut bus) = Bus::connect("hb_marker_merge") else {
        return;
    };
    for (field, ms) in [
        ("Hub", "9"),
        ("hub", "5"),
        ("node-a", "2"),
        ("", "1"),
        (".x", "1"),
    ] {
        bus.hset_raw("health:watcher-seen", field, ms);
    }
    redis::cmd("SET")
        .arg(bus.key("health:watcher:.bad"))
        .arg("{}")
        .exec(&mut bus.raw)
        .unwrap();
    // A key written under an un-normalised name is never read back as itself.
    redis::cmd("SET")
        .arg(bus.key("health:watcher:Hub"))
        .arg("{}")
        .exec(&mut bus.raw)
        .unwrap();
    let read = bus.relay.heartbeats(&[]).unwrap();
    assert_eq!(read.last_ticks.len(), 2, "hub (merged) and node-a");
    assert_eq!(read.last_ticks.get(&"hub".parse().unwrap()), Some(&9));
    assert_eq!(read.last_ticks.get(&"node-a".parse().unwrap()), Some(&2));
    assert_eq!(
        read.unreadable, 4,
        "two marker fields and two heartbeat keys"
    );
}

#[test]
fn a_recipient_probe_is_one_read_of_the_queue_and_the_machines_heartbeat() {
    let Some(mut bus) = Bus::connect("probe") else {
        return;
    };
    bus.relay.register(&reg("lead", "Hub.local", 1)).unwrap();
    bus.relay.send(&msg("lead", 10)).unwrap();
    let down = bus
        .relay
        .recipient_probe(&name("lead"), &"hub".parse().unwrap())
        .unwrap();
    assert_eq!((down.inbox_len, down.heartbeat), (1, None));

    bus.relay
        .write_heartbeat(&heartbeat("hub", 1), Duration::from_secs(60))
        .unwrap();
    let up = bus
        .relay
        .recipient_probe(&name("lead"), &"Hub.local".parse().unwrap())
        .unwrap();
    assert!(matches!(up.heartbeat, Some(HeartbeatRead::Live(_))));
}
