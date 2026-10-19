//! Wire stability (AGE-2027 phase B): every stored or emitted type that carries
//! a session, pane or machine id serialises to exactly the bytes `main` (8578b01)
//! produced when those ids were bare strings, and decodes them back.
//!
//! The literals were captured from `main`'s output for these same inputs, not
//! generated from this code: a change to the serialised form of any of them
//! fails here, and an older binary on another machine keeps reading what this
//! one writes.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

use relay_core::{
    Message, PaneId, Place, Registration,
    health::{
        HeartbeatError, HeartbeatErrorKind, LastError, OtlpStatus, WatcherFile, WatcherHeartbeat,
    },
};
use relay_test_support::Bus;

/// A value's exact bytes, its decode of those bytes, and the re-encode of that.
fn assert_wire<T>(value: &T, golden: &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    assert_eq!(serde_json::to_string(value).unwrap(), golden, "encode");
    let decoded: T = serde_json::from_str(golden).unwrap();
    assert_eq!(&decoded, value, "decode");
    assert_eq!(
        serde_json::to_string(&decoded).unwrap(),
        golden,
        "re-encode"
    );
}

const REGISTRATION_FULL: &str = r#"{"name":"qsl-lead","machine":"node-a","kind":"claude","session_id":"sid-1","cwd":"/wörk/日本","herdr_pane":"w1:p1","place":{"workspace":"QSL","tab":"review","herdr_ids":{"workspace":"w1","tab":"w1:t1","pane":"w1:p1","session_id":"sid-1","verified_at_ms":1700000000000}},"last_seen_ms":1700000000000}"#;
const REGISTRATION_MIN: &str = r#"{"name":"hub-lead","machine":"hub","kind":"codex","session_id":null,"cwd":null,"herdr_pane":null,"place":null,"last_seen_ms":0}"#;
const MESSAGE_BROADCAST: &str =
    r#"{"from":"qsl-lead","to":"all","text":"hi \"q\"\n é","sent_at_ms":5}"#;
const MESSAGE_DIRECT: &str = r#"{"from":"qsl-lead","to":"tl-lead","text":"x","sent_at_ms":-1}"#;
const HEARTBEAT_FULL: &str = r#"{"machine":"node-a","pid":4242,"version":"0.1.0","started_at_ms":1,"last_tick_ms":2,"last_sweep_ms":3,"sweep_secs":30,"herdr_ok":false,"herdr_problem":"bad","herdr_blocks_delivery":true,"redis_ok":true,"leads_watched":3,"otlp":{"state":"failing","target":"collector:4318","reason":"timeout"},"last_error":{"kind":"redis_unreachable","at_ms":9}}"#;
const HEARTBEAT_MIN: &str = r#"{"machine":"hub","pid":1,"version":"v","started_at_ms":0,"last_tick_ms":0,"last_sweep_ms":null,"sweep_secs":0,"herdr_ok":true,"herdr_problem":null,"herdr_blocks_delivery":false,"redis_ok":false,"leads_watched":0,"otlp":{"state":"disabled"},"last_error":null}"#;
const WATCHER_FILE_FULL: &str =
    r#"{"pid":7,"started_at_ms":1,"last_ok_tick_ms":2,"last_error":{"at_ms":3,"message":"boom"}}"#;
const WATCHER_FILE_EMPTY: &str =
    r#"{"pid":7,"started_at_ms":1,"last_ok_tick_ms":null,"last_error":null}"#;

fn registration_full() -> Registration {
    Registration {
        name: "qsl-lead".parse().unwrap(),
        machine: "node-a".parse().unwrap(),
        kind: "claude".into(),
        session_id: Some("sid-1".parse().unwrap()),
        cwd: Some("/w\u{f6}rk/\u{65e5}\u{672c}".into()),
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: Some(Place {
            workspace: "QSL".into(),
            tab: "review".into(),
            herdr_ids: Some(relay_core::HerdrPlaceIds {
                workspace: "w1".parse().unwrap(),
                tab: "w1:t1".parse().unwrap(),
                pane: "w1:p1".parse().unwrap(),
                session_id: "sid-1".parse().unwrap(),
                verified_at_ms: 1_700_000_000_000,
            }),
        }),
        last_seen_ms: 1_700_000_000_000,
    }
}

fn registration_min() -> Registration {
    Registration {
        name: "hub-lead".parse().unwrap(),
        machine: "hub".parse().unwrap(),
        kind: "codex".into(),
        session_id: None,
        cwd: None,
        herdr_pane: None,
        place: None,
        last_seen_ms: 0,
    }
}

/// Tracing: TC-008-07
/// ACs: FR-008-AC-7
#[test]
fn a_registration_is_byte_identical_to_main() {
    assert_wire(&registration_full(), REGISTRATION_FULL);
    assert_wire(&registration_min(), REGISTRATION_MIN);
}

/// Tracing: TC-008-07
/// ACs: FR-008-AC-7
#[test]
fn a_message_is_byte_identical_to_main() {
    let broadcast = Message {
        from: "qsl-lead".parse().unwrap(),
        to: "all".parse().unwrap(),
        text: "hi \"q\"\n \u{e9}".into(),
        sent_at_ms: 5,
        forwarded: false,
    };
    let direct = Message {
        from: "qsl-lead".parse().unwrap(),
        to: "tl-lead".parse().unwrap(),
        text: "x".into(),
        sent_at_ms: -1,
        forwarded: false,
    };
    assert_wire(&broadcast, MESSAGE_BROADCAST);
    assert_wire(&direct, MESSAGE_DIRECT);
}

fn heartbeat_full() -> WatcherHeartbeat {
    WatcherHeartbeat {
        machine: "node-a".parse().unwrap(),
        pid: 4242,
        version: "0.1.0".into(),
        started_at_ms: 1,
        last_tick_ms: 2,
        last_sweep_ms: Some(3),
        sweep_secs: 30,
        herdr_ok: false,
        herdr_problem: Some("bad".into()),
        herdr_blocks_delivery: true,
        redis_ok: true,
        leads_watched: 3,
        otlp: Some(OtlpStatus::Failing {
            target: "collector:4318".into(),
            reason: "timeout".into(),
            counters: Default::default(),
        }),
        last_error: Some(HeartbeatError {
            kind: HeartbeatErrorKind::RedisUnreachable,
            at_ms: 9,
        }),
    }
}

/// Tracing: TC-008-07
/// ACs: FR-008-AC-7
#[test]
fn a_heartbeat_and_the_watcher_file_are_byte_identical_to_main() {
    assert_wire(&heartbeat_full(), HEARTBEAT_FULL);
    let min = WatcherHeartbeat {
        machine: "hub".parse().unwrap(),
        pid: 1,
        version: "v".into(),
        started_at_ms: 0,
        last_tick_ms: 0,
        last_sweep_ms: None,
        sweep_secs: 0,
        herdr_ok: true,
        herdr_problem: None,
        herdr_blocks_delivery: false,
        redis_ok: false,
        leads_watched: 0,
        otlp: Some(OtlpStatus::Disabled),
        last_error: None,
    };
    assert_wire(&min, HEARTBEAT_MIN);
    let file = WatcherFile {
        pid: 7,
        started_at_ms: 1,
        last_ok_tick_ms: Some(2),
        last_error: Some(LastError {
            at_ms: 3,
            message: "boom".into(),
        }),
    };
    let empty = WatcherFile {
        pid: 7,
        started_at_ms: 1,
        last_ok_tick_ms: None,
        last_error: None,
    };
    assert_wire(&file, WATCHER_FILE_FULL);
    assert_wire(&empty, WATCHER_FILE_EMPTY);
}

/// Tracing: TC-008-07
/// ACs: FR-008-AC-7
///
/// The bytes Redis holds, not just the serde form: a registration, a heartbeat,
/// a health-stream entry and the session cache, read back raw.
#[test]
fn what_redis_holds_is_byte_identical_to_main() {
    let Some(mut bus) = Bus::connect("wire-golden") else {
        return;
    };
    bus.relay.register(&registration_full()).unwrap();
    let stored: String = redis::cmd("HGET")
        .arg(bus.key("agents"))
        .arg("qsl-lead")
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(stored, REGISTRATION_FULL);

    bus.relay
        .write_heartbeat(&heartbeat_full(), std::time::Duration::from_secs(60))
        .unwrap();
    let beat: String = redis::cmd("GET")
        .arg(bus.key("health:watcher:node-a"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(beat, HEARTBEAT_FULL);
    let seen: i64 = redis::cmd("HGET")
        .arg(bus.key("health:watcher-seen"))
        .arg("node-a")
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(seen, 2);

    bus.relay
        .push_hook_health(&[relay_core::HookHealthEntry {
            machine: "node-a".parse().unwrap(),
            lead: Some("qsl-lead".into()),
            record: r#"{"at_ms":1}"#.into(),
        }])
        .unwrap();
    let fields: Vec<(String, String)> = redis::cmd("XRANGE")
        .arg(bus.key("health:hooks"))
        .arg("-")
        .arg("+")
        .query::<Vec<(String, Vec<String>)>>(&mut bus.raw)
        .unwrap()
        .into_iter()
        .flat_map(|(_, kv)| {
            kv.chunks(2)
                .map(|p| (p[0].clone(), p[1].clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        fields,
        [
            ("machine".to_owned(), "node-a".to_owned()),
            ("lead".to_owned(), "qsl-lead".to_owned()),
            ("rec".to_owned(), r#"{"at_ms":1}"#.to_owned()),
        ]
    );

    let commit = relay_core::IdentityCommit {
        session_name: Some(("sid-1".parse().unwrap(), "qsl-lead".parse().unwrap())),
        suppress_session: Some("sid-2".parse().unwrap()),
        ..relay_core::IdentityCommit::default()
    };
    bus.relay.commit_identity(&commit).unwrap();
    let name: String = redis::cmd("GET")
        .arg(bus.key("session:sid-1"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(name, "qsl-lead");
    let suppressed: String = redis::cmd("GET")
        .arg(bus.key("suppressed:sid-2"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(suppressed, "1");
}

// ---------------------------------------------------------------------------
// Tolerance: what an older binary may have stored that the new types refuse.
// ---------------------------------------------------------------------------

fn planted(machine: &str, session: &str, pane: &str) -> String {
    format!(
        r#"{{"name":"odd-lead","machine":{machine},"kind":"claude","session_id":{session},"cwd":null,"herdr_pane":{pane},"place":null,"last_seen_ms":1}}"#
    )
}

/// Tracing: TC-008-06
/// ACs: FR-008-AC-6
///
/// A stored machine name is normalised on read (idempotent for what every writer
/// stored), so an old record written with the case or domain a hostname had still
/// decodes and still matches the machine.
#[test]
fn an_unnormalised_stored_machine_decodes_normalised() {
    let json = planted(r#""Node-B.local""#, "null", "null");
    let reg: Registration = serde_json::from_str(&json).unwrap();
    assert_eq!(reg.machine.as_str(), "node-b");
}

/// Tracing: TC-008-06
/// ACs: FR-008-AC-6
///
/// A record whose machine, session or pane breaks its shape rule fails to
/// decode, so the registry read skips it and reports it (FR-008) and returns
/// every other registration; a single-record read is a decode error.
#[test]
fn a_registration_with_a_malformed_id_is_skipped_and_reported_never_fatal() {
    let Some(mut bus) = Bus::connect("wire-tolerance") else {
        return;
    };
    bus.relay.register(&registration_min()).unwrap();
    let too_long = format!("\"{}\"", "x".repeat(300));
    let cases = [
        ("empty-machine", planted(r#""""#, "null", "null")),
        ("dot-machine", planted(r#"".local""#, "null", "null")),
        ("empty-session", planted(r#""m""#, r#""""#, "null")),
        ("long-session", planted(r#""m""#, &too_long, "null")),
        ("empty-pane", planted(r#""m""#, "null", r#""""#)),
        (
            "control-pane",
            planted(r#""m""#, "null", r#""w1\u001b[2J""#),
        ),
        ("number-pane", planted(r#""m""#, "null", "7")),
    ];
    for (field, json) in &cases {
        bus.plant(field, json.as_bytes());
    }
    let list = bus.relay.agents().unwrap();
    assert_eq!(
        list.registrations.len(),
        1,
        "the valid registration survives"
    );
    assert_eq!(list.skipped.len(), cases.len());
    for (field, _) in &cases {
        assert!(
            list.skipped.iter().any(|s| s.field == *field),
            "{field} was not reported"
        );
    }
    let err = bus
        .relay
        .agent(&"empty-machine".parse().unwrap())
        .unwrap_err();
    assert!(matches!(err, relay_core::Error::Decode { .. }), "{err}");
}

/// Tracing: TC-008-06
/// ACs: FR-008-AC-6
///
/// A heartbeat whose machine is not a machine name is a key that is there but
/// not a heartbeat this binary reads; the read says so rather than failing.
#[test]
fn a_heartbeat_with_a_malformed_machine_reads_as_undecodable() {
    let Some(mut bus) = Bus::connect("wire-tolerance-hb") else {
        return;
    };
    let json = HEARTBEAT_FULL.replace(r#""machine":"node-a""#, r#""machine":"""#);
    redis::cmd("SET")
        .arg(bus.key("health:watcher:node-a"))
        .arg(json)
        .exec(&mut bus.raw)
        .unwrap();
    let read = bus.relay.heartbeat(&"node-a".parse().unwrap()).unwrap();
    assert!(
        matches!(read, Some(relay_core::health::HeartbeatRead::Undecodable)),
        "{read:?}"
    );
}

/// Tracing: TC-008-06
/// ACs: FR-008-AC-6
///
/// A health-stream entry with a malformed machine field is left out of the read
/// (as an entry with a missing field always was); the others come back.
#[test]
fn a_hook_health_entry_with_a_malformed_machine_is_skipped() {
    let Some(mut bus) = Bus::connect("wire-tolerance-stream") else {
        return;
    };
    for machine in ["", "node-a"] {
        redis::cmd("XADD")
            .arg(bus.key("health:hooks"))
            .arg("*")
            .arg("machine")
            .arg(machine)
            .arg("lead")
            .arg("")
            .arg("rec")
            .arg("{}")
            .exec(&mut bus.raw)
            .unwrap();
    }
    let read = bus.relay.hook_health(10).unwrap();
    assert_eq!(read.unreadable, 1, "the bad entry is counted, not silent");
    let entries = read.entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].machine.as_str(), "node-a");
}

#[test]
fn a_pane_id_is_a_bare_json_string() {
    let pane: PaneId = "w1:p1".parse().unwrap();
    assert_eq!(serde_json::to_string(&pane).unwrap(), r#""w1:p1""#);
}
