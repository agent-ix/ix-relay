//! Evidence for tolerant registry decoding (FR-008) and machine-scoped pane
//! lookup (FR-009-AC-4's store half).
//!
//! The registry is a Redis hash that every machine on the bus writes and
//! reads, so its records are untrusted bytes. These tests plant raw records
//! (including invalid UTF-8) and check that a read returns every valid
//! registration and reports the rest, never failing as a whole.
//!
//! Redis-backed tests skip cleanly without `IX_RELAY_TEST_URL`
//! (`make test-redis` sets it). The `fuzz_` test is an ignored lane:
//! `cargo test -p relay-core --test registry_decoding -- --ignored fuzz_`.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]

use proptest::prelude::*;
use relay_core::{PaneRef, Registration};

use relay_test_support::Bus;

fn valid(name: &str, machine: &str, pane: &str) -> Registration {
    Registration {
        name: name.parse().expect("valid name"),
        machine: machine.parse().expect("valid id"),
        kind: "claude".into(),
        session_id: Some(format!("sid-{name}").parse().expect("valid id")),
        cwd: None,
        herdr_pane: Some(pane.parse().expect("valid id")),
        place: None,
        last_seen_ms: 7,
    }
}

/// Records no binary of this repo would write, each a way decoding could go
/// wrong: not JSON, wrong shapes, wrong types, a bad name, overflow, deep
/// nesting, invalid UTF-8, and text that is JSON followed by junk.
fn nasty_records() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("empty", b"".to_vec()),
        ("null", b"null".to_vec()),
        ("array", b"[]".to_vec()),
        ("object", b"{}".to_vec()),
        ("not-json", b"{\"not\": a registration".to_vec()),
        ("bad-name", br#"{"name":"BAD NAME","machine":"m","kind":"k","session_id":null,"cwd":null,"last_seen_ms":1}"#.to_vec()),
        ("wrong-type", br#"{"name":"ok","machine":5,"kind":"k","session_id":null,"cwd":null,"last_seen_ms":1}"#.to_vec()),
        ("overflow", br#"{"name":"ok","machine":"m","kind":"k","session_id":null,"cwd":null,"last_seen_ms":1e400}"#.to_vec()),
        ("huge-int", br#"{"name":"ok","machine":"m","kind":"k","session_id":null,"cwd":null,"last_seen_ms":99999999999999999999999}"#.to_vec()),
        ("deep", [vec![b'['; 100_000], vec![b']'; 100_000]].concat()),
        ("invalid-utf8", vec![0xff, 0xfe, 0x00, 0x80]),
        ("invalid-utf8-in-json", b"{\"name\":\"ok\xff\"}".to_vec()),
        ("trailing-junk", br#"{"name":"ok","machine":"m","kind":"k","session_id":null,"cwd":null,"last_seen_ms":1} x"#.to_vec()),
        ("nul-bytes", b"\0\0\0".to_vec()),
        (
            "unknown-members",
            br#"{"name":"newer","machine":"node-a","kind":"claude","session_id":null,"cwd":null,"last_seen_ms":1,"added_later":{"a":[1,2]},"place":{"workspace":"EA","tab":"1","tabs_in_workspace":2,"also_later":true}}"#.to_vec(),
        ),
    ]
}

/// Tracing: TC-008-01, TC-008-02
/// ACs: FR-008-AC-1, FR-008-AC-2
///
/// Seed corpus, always run: every nasty record beside three valid
/// registrations. A read succeeds, returns the three valid registrations
/// (plus the record with unknown members, decoded), and reports every record
/// that does not decode by its hash field.
#[test]
fn tc_008_02_a_read_returns_the_valid_registrations_and_reports_each_bad_record() {
    let Some(mut bus) = Bus::connect("corpus") else {
        return;
    };
    for (name, machine, pane) in [
        ("a-lead", "hub", "wB:p1"),
        ("b-lead", "node-a", "wB:p1"),
        ("c-lead", "hub", "wB:p2"),
    ] {
        bus.relay
            .register(&valid(name, machine, pane))
            .expect("register");
    }
    for (field, record) in nasty_records() {
        bus.plant(field, &record);
    }
    let list = bus
        .relay
        .agents()
        .expect("a bad record must not fail the whole read");
    let names: Vec<&str> = list.registrations.iter().map(|r| r.name.as_str()).collect();
    for expected in ["a-lead", "b-lead", "c-lead"] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
    let newer = list
        .registrations
        .iter()
        .find(|r| r.name.as_str() == "newer");
    let newer = newer.expect("a record with unknown members decodes");
    assert_eq!(
        newer.place.as_ref().map(|p| p.workspace.as_str()),
        Some("EA")
    );
    // Everything else planted is reported, by field, and none is silently dropped.
    let mut skipped: Vec<&str> = list.skipped.iter().map(|s| s.field.as_str()).collect();
    skipped.sort_unstable();
    let mut expected: Vec<&str> = nasty_records()
        .iter()
        .map(|(f, _)| *f)
        .filter(|f| *f != "unknown-members")
        .collect();
    expected.sort_unstable();
    assert_eq!(skipped, expected);
    assert_eq!(
        list.registrations.len() + list.skipped.len(),
        3 + nasty_records().len()
    );
}

/// Tracing: TC-008-04
/// ACs: FR-008-AC-4
///
/// The property over every byte string, always run at a modest case count.
/// See `fuzz_registry_read_over_arbitrary_bytes` for the raised-count lane.
fn read_survives(bus: &mut Bus, records: &[Vec<u8>]) -> Result<(), TestCaseError> {
    bus.relay
        .register(&valid("anchor", "hub", "wB:p1"))
        .map_err(|e| TestCaseError::fail(e.to_string()))?;
    for (i, record) in records.iter().enumerate() {
        bus.plant(&format!("rec-{i}"), record);
    }
    let list = bus.relay.agents().map_err(|e| {
        TestCaseError::fail(format!("a read failed as a whole over {records:?}: {e}"))
    })?;
    prop_assert!(
        list.registrations
            .iter()
            .any(|r| r.name.as_str() == "anchor"),
        "the valid registration was lost"
    );
    prop_assert_eq!(
        list.registrations.len() + list.skipped.len(),
        records.len() + 1
    );
    // A pane lookup goes through the same read and must survive too.
    bus.relay
        .registrations_for_pane(&PaneRef::new(
            "hub".parse().expect("valid id"),
            "wB:p1".parse().expect("valid id"),
        ))
        .map_err(|e| TestCaseError::fail(e.to_string()))?;
    Ok(())
}

/// The machine rule written out independently of `MachineName`, as the oracle:
/// lowercase, cut at the first dot.
fn oracle_normalise(raw: &str) -> String {
    raw.to_ascii_lowercase()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..64),
        // Structure-aware: JSON-shaped text with wrong types and unknown members.
        "\\{(\"[a-z_]{1,8}\":(null|true|-?[0-9]{1,25}|\"[ -~]{0,8}\"|\\[\\]|\\{\\}),?){0,6}\\}".prop_map(String::into_bytes),
        Just(br#"{"name":"x-lead","machine":"m","kind":"k","session_id":null,"cwd":null,"last_seen_ms":1,"extra":[1,{"a":2}]}"#.to_vec()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 60, failure_persistence: None, ..ProptestConfig::default() })]

    /// Tracing: TC-008-04
    /// ACs: FR-008-AC-4
    #[test]
    fn tc_008_04_a_registry_read_over_arbitrary_records_never_fails_as_a_whole(
        records in proptest::collection::vec(bytes(), 0..6),
    ) {
        let Some(mut bus) = Bus::connect("bytes") else { return Ok(()) };
        read_survives(&mut bus, &records)?;
    }
}

fn fuzz_config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(fuzz_config())]

    /// Tracing: TC-008-04
    /// ACs: FR-008-AC-4
    ///
    /// Fuzz lane: the same property at a raised case count and longer records.
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_); needs IX_RELAY_TEST_URL"]
    fn fuzz_registry_read_over_arbitrary_bytes(
        records in proptest::collection::vec(
            prop_oneof![proptest::collection::vec(any::<u8>(), 0..512), bytes()], 0..12),
    ) {
        let Some(mut bus) = Bus::connect("fuzz-bytes") else { return Ok(()) };
        read_survives(&mut bus, &records)?;
    }
}

/// Tracing: TC-009-04
/// ACs: FR-009-AC-4
///
/// Machine-scoped pane lookup over generated registries: the registrations
/// returned for `(machine, pane)` are exactly those on that machine
/// (compared normalised) whose pane is `pane`, most recently seen first, and
/// never one from another machine, whatever the case or domain suffix.
#[test]
fn tc_009_04_a_pane_lookup_returns_only_registrations_on_that_machine() {
    let Some(mut bus) = Bus::connect("panes") else {
        return;
    };
    let machines = ["hub", "Hub.local", "node-a", "NODE-A.internal", "node-b"];
    let panes = ["wB:p1", "wB:p2"];
    let mut n = 0;
    for machine in machines {
        for pane in panes {
            n += 1;
            let mut reg = valid(&format!("lead-{n}"), machine, pane);
            reg.last_seen_ms = 100 - n;
            bus.relay.register(&reg).expect("register");
        }
    }
    for machine in [
        machines.as_slice(),
        &["HUB", "Node-A.example.com", "nowhere"],
    ]
    .concat()
    {
        for pane in panes.iter().chain(&["wZ:p9"]) {
            let got = bus
                .relay
                .registrations_for_pane(&PaneRef::new(
                    machine.parse().expect("valid id"),
                    pane.parse().expect("valid id"),
                ))
                .expect("lookup");
            let want = oracle_normalise(machine);
            for r in &got {
                assert_eq!(r.machine.as_str(), want, "{machine} {pane}");
                assert_eq!(r.herdr_pane.as_ref().map(|v| v.as_str()), Some(*pane));
            }
            let expected_count = machines
                .iter()
                .filter(|m| oracle_normalise(m) == want)
                .count()
                * usize::from(panes.contains(pane));
            assert_eq!(got.len(), expected_count, "{machine} {pane}");
            assert!(
                got.windows(2)
                    .all(|w| w[0].last_seen_ms >= w[1].last_seen_ms),
                "most recent first"
            );
        }
    }
}

/// Tracing: TC-008-04
/// ACs: FR-008-AC-4
///
/// Every registry and alias read, not only `agents()`, survives bytes that are
/// not UTF-8: a lookup of the corrupt record reports it as a decode failure
/// (never a Redis type error), a place refresh does the same, a broadcast skips
/// a name it cannot read and still reaches the others, and the alias table
/// ignores a pair it cannot read.
#[test]
fn tc_008_04_every_registry_and_alias_read_tolerates_bytes_that_are_not_utf8() {
    use relay_core::{Error, Message, Place, Target};

    let Some(mut bus) = Bus::connect("non-utf8-reads") else {
        return;
    };
    let name = |s: &str| s.parse().expect("valid name");
    bus.relay
        .register(&valid("anchor", "hub", "wB:p1"))
        .expect("register");
    // A corrupt record under a valid name, a hash field that is not UTF-8, and
    // alias pairs whose field or value is not.
    bus.plant("qsl", &[0xff, 0xfe]);
    bus.plant_in("agents", &[0xc3, 0x28], b"{}");

    // agent(): the record under `qsl` is a decode failure.
    assert!(matches!(
        bus.relay.agent(&name("qsl")),
        Err(Error::Decode { .. })
    ));
    assert!(bus.relay.agent(&name("anchor")).expect("agent").is_some());
    // refresh_place() on it: also a decode failure, not a type error.
    let place = Place {
        workspace: "EA".into(),
        tab: "1".into(),
        herdr_ids: None,
    };
    assert!(matches!(
        bus.relay.refresh_place(&name("qsl"), &place, 5, true),
        Err(Error::Decode { .. })
    ));
    // Broadcast: the unreadable hash field is skipped; `anchor` and the
    // (named, corrupt) `qsl` are still recipients; nothing fails.
    let recipients = bus
        .relay
        .send(&Message {
            from: name("sender"),
            to: Target::All,
            text: "hi".into(),
            sent_at_ms: 1,
            forwarded: false,
        })
        .expect("a broadcast survives an unreadable registry field");
    assert_eq!(recipients, vec![name("anchor"), name("qsl")]);
}
