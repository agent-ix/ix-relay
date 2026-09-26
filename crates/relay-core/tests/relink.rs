//! Relink, rename and replaced-pane records against a real Redis (FR-043,
//! FR-032): the compare-and-set guards, the event written with each change, and
//! a model check that no interleaving of registrations, prunes and stale
//! snapshots relinks the wrong lead.
//!
//! Needs `IX_RELAY_TEST_URL`; each test returns early and says so on stderr
//! without it (`make test-redis` sets it).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

use std::collections::BTreeMap;

use proptest::prelude::*;
use relay_core::{
    AgentName, Error, Place, PlaceRefresh, Registration, Relay,
    moves::{MoveEvent, MoveKind, PaneReplaced},
};
use relay_test_support::Bus;

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn place(workspace: &str, tab: &str, tabs: u32) -> Place {
    Place {
        workspace: workspace.into(),
        tab: tab.into(),
        tabs_in_workspace: tabs,
    }
}

fn reg(
    n: &str,
    machine: &str,
    session: Option<&str>,
    pane: &str,
    last_seen_ms: i64,
) -> Registration {
    Registration {
        name: name(n),
        machine: machine.parse().unwrap(),
        kind: "claude".into(),
        session_id: session.map(|s| s.parse().unwrap()),
        cwd: Some("/w".into()),
        herdr_pane: Some(pane.parse().unwrap()),
        place: Some(place("wsA", "1", 1)),
        last_seen_ms,
    }
}

/// Tracing: TC-043-03
/// ACs: FR-043-AC-3
#[test]
fn tc_043_03_a_relink_points_the_registration_at_the_new_pane_and_records_one_event() {
    let Some(mut bus) = Bus::connect("relink-basic") else {
        return;
    };
    let before = reg("lead-a", "hub", Some("sess-A3"), "w1:p3", 100);
    bus.relay.register(&before).unwrap();
    let marker = PaneReplaced {
        pane: "w1:p3".parse().unwrap(),
        seen_last_seen_ms: 100,
        since_ms: 150,
    };
    assert!(
        bus.relay
            .mark_pane_replaced(&name("lead-a"), &marker)
            .unwrap()
    );

    let target = place("wsB", "1", 1);
    let event = bus
        .relay
        .relink_pane(&before, &"w2:p2".parse().unwrap(), &target, 200)
        .unwrap()
        .expect("relinked");

    let after = bus.relay.agent(&name("lead-a")).unwrap().unwrap();
    assert_eq!(
        after,
        Registration {
            herdr_pane: Some("w2:p2".parse().unwrap()),
            place: Some(target.clone()),
            last_seen_ms: 200,
            ..before.clone()
        },
        "the pane and the place change and the stamp is the relink's: herdr has just \
         shown the session alive; the bus name and session stay"
    );
    // A prune made from the old stamp finds a registration it did not see: the
    // lead the relink rescued is not deleted by a stale snapshot (AGE-2033 review).
    assert_eq!(bus.relay.prune(&name("lead-a"), 100).unwrap(), None);
    assert!(bus.relay.agent(&name("lead-a")).unwrap().is_some());
    let moves = bus.relay.recent_moves(10).unwrap();
    assert_eq!(moves.entries.as_slice(), std::slice::from_ref(&event));
    assert_eq!(
        event,
        MoveEvent {
            lead: name("lead-a"),
            machine: "hub".parse().unwrap(),
            kind: MoveKind::Relink,
            session_id: Some("sess-A3".parse().unwrap()),
            previous_pane: Some("w1:p3".parse().unwrap()),
            pane: Some("w2:p2".parse().unwrap()),
            old_place: Some(place("wsA", "1", 1)),
            new_place: Some(target),
            at_ms: 200,
        }
    );
    assert!(
        bus.relay
            .panes_replaced(&[&name("lead-a")])
            .unwrap()
            .found
            .is_empty(),
        "a relinked lead is no longer replaced"
    );
    // The same snapshot again finds a different registration: nothing more happens.
    assert_eq!(
        bus.relay
            .relink_pane(
                &before,
                &"w2:p2".parse().unwrap(),
                &place("wsB", "1", 1),
                300
            )
            .unwrap(),
        None
    );
    assert_eq!(bus.relay.recent_moves(10).unwrap().entries.len(), 1);
}

/// Tracing: TC-043-04
/// ACs: FR-043-AC-4
#[test]
fn tc_043_04_a_relink_never_resurrects_never_overwrites_and_never_crosses_machines() {
    let Some(mut bus) = Bus::connect("relink-guards") else {
        return;
    };
    let to = "w2:p2".parse().unwrap();
    let target = place("wsB", "1", 1);
    let seen = reg("lead-a", "hub", Some("s1"), "w1:p3", 100);

    // Pruned meanwhile: the lead stays gone.
    bus.relay.register(&seen).unwrap();
    assert!(bus.relay.prune(&name("lead-a"), 100).unwrap().is_some());
    assert_eq!(
        bus.relay.relink_pane(&seen, &to, &target, 200).unwrap(),
        None
    );
    assert_eq!(bus.relay.agent(&name("lead-a")).unwrap(), None);

    // Registered again meanwhile: the newer registration stays.
    let newer = reg("lead-a", "hub", Some("s1"), "w1:p4", 150);
    bus.relay.register(&newer).unwrap();
    assert_eq!(
        bus.relay.relink_pane(&seen, &to, &target, 200).unwrap(),
        None
    );
    assert_eq!(bus.relay.agent(&name("lead-a")).unwrap().unwrap(), newer);

    // Another machine registered under that name: its record is not hub's.
    let elsewhere = reg("lead-a", "node-a", Some("s1"), "w1:p3", 100);
    bus.relay.register(&elsewhere).unwrap();
    assert_eq!(
        bus.relay.relink_pane(&seen, &to, &target, 200).unwrap(),
        None
    );
    assert_eq!(
        bus.relay.agent(&name("lead-a")).unwrap().unwrap(),
        elsewhere
    );

    // No session to follow, or already there: nothing.
    let no_session = reg("lead-b", "hub", None, "w1:p3", 100);
    bus.relay.register(&no_session).unwrap();
    assert_eq!(
        bus.relay
            .relink_pane(&no_session, &to, &target, 200)
            .unwrap(),
        None
    );
    let here = reg("lead-c", "hub", Some("s3"), "w2:p2", 100);
    bus.relay.register(&here).unwrap();
    assert_eq!(
        bus.relay.relink_pane(&here, &to, &target, 200).unwrap(),
        None
    );

    assert!(bus.relay.recent_moves(10).unwrap().entries.is_empty());
}

/// Tracing: TC-043-03
/// ACs: FR-043-AC-3, FR-034-AC-3
#[test]
fn tc_043_03_a_rename_records_one_event_and_an_unchanged_place_none() {
    let Some(mut bus) = Bus::connect("relink-rename") else {
        return;
    };
    bus.relay
        .register(&reg("lead-a", "hub", Some("s1"), "w1:p1", 100))
        .unwrap();
    let renamed = place("wsA-renamed", "1", 1);

    let PlaceRefresh::Renamed(event) = bus
        .relay
        .refresh_place(&name("lead-a"), &renamed, 200, true)
        .unwrap()
    else {
        panic!("a label changed: a rename is recorded");
    };
    assert_eq!(event.kind, MoveKind::Rename);
    assert_eq!(event.previous_pane, None);
    assert_eq!(event.pane, Some("w1:p1".parse().unwrap()));
    assert_eq!(event.old_place, Some(place("wsA", "1", 1)));
    assert_eq!(event.new_place, Some(renamed.clone()));
    // Ten polls of the same place: no more events (FR-034-AC-3).
    for at in 0..10 {
        assert_eq!(
            bus.relay
                .refresh_place(&name("lead-a"), &renamed, 300 + at, true)
                .unwrap(),
            PlaceRefresh::Unchanged
        );
    }
    assert_eq!(bus.relay.recent_moves(20).unwrap().entries, [*event]);

    // Only the workspace's tab count changed (a tab was opened): the address is
    // kept true and no rename is recorded (AGE-2033 review, finding 7).
    assert_eq!(
        bus.relay
            .refresh_place(&name("lead-a"), &place("wsA-renamed", "1", 3), 400, true)
            .unwrap(),
        PlaceRefresh::Refreshed
    );
    assert_eq!(
        bus.relay.agent(&name("lead-a")).unwrap().unwrap().place,
        Some(place("wsA-renamed", "1", 3))
    );
    // The caller asked for no event (the lead's rename gap): the place still follows.
    assert_eq!(
        bus.relay
            .refresh_place(&name("lead-a"), &place("wsA-again", "1", 3), 500, false)
            .unwrap(),
        PlaceRefresh::Refreshed
    );
    assert_eq!(bus.relay.recent_moves(20).unwrap().entries.len(), 1);
}

/// Tracing: TC-043-05
/// ACs: FR-043-AC-5
#[test]
fn tc_043_05_the_moves_stream_is_capped_and_reads_newest_first() {
    let Some(mut bus) = Bus::connect("relink-cap") else {
        return;
    };
    bus.relay
        .register(&reg("lead-a", "hub", Some("s1"), "w1:p1", 100))
        .unwrap();
    for i in 0..1_250_i64 {
        let tab = if i % 2 == 0 { "x" } else { "y" };
        bus.relay
            .refresh_place(&name("lead-a"), &place("wsA", tab, 2), i, true)
            .unwrap();
    }
    let key = bus.key("health:moves");
    let len: usize = redis::cmd("XLEN").arg(&key).query(&mut bus.raw).unwrap();
    assert!(
        (1_000..1_250).contains(&len),
        "capped near 1000, not unbounded: {len}"
    );
    let newest = bus.relay.recent_moves(3).unwrap().entries;
    assert_eq!(
        newest.iter().map(|e| e.at_ms).collect::<Vec<_>>(),
        [1_249, 1_248, 1_247]
    );
    let unreadable = {
        let () = redis::cmd("XADD")
            .arg(&key)
            .arg("*")
            .arg("rec")
            .arg("not json")
            .exec(&mut bus.raw)
            .unwrap();
        bus.relay.recent_moves(1).unwrap()
    };
    assert_eq!(
        (unreadable.entries.len(), unreadable.unreadable),
        (0, 1),
        "an entry that does not decode is counted, never dropped silently"
    );
}

/// Tracing: TC-032-02
/// ACs: FR-032-AC-2, FR-032-AC-4
#[test]
fn tc_032_02_a_replaced_marker_keeps_its_first_time_and_clears_by_name() {
    let Some(mut bus) = Bus::connect("relink-replaced") else {
        return;
    };
    let lead = name("lead-a");
    let first = PaneReplaced {
        pane: "w1:p1".parse().unwrap(),
        seen_last_seen_ms: 100,
        since_ms: 500,
    };
    assert!(bus.relay.mark_pane_replaced(&lead, &first).unwrap());
    let later = PaneReplaced {
        since_ms: 900,
        ..first.clone()
    };
    assert!(
        !bus.relay.mark_pane_replaced(&lead, &later).unwrap(),
        "the same finding keeps its first time"
    );
    assert_eq!(
        bus.relay.panes_replaced(&[&lead]).unwrap().found[&lead],
        first
    );
    // One key per lead with an expiry, so the markers are bounded (review finding 6).
    let key = bus.key("health:replaced:lead-a");
    let ttl: i64 = redis::cmd("TTL").arg(&key).query(&mut bus.raw).unwrap();
    assert!(ttl > 0 && ttl <= 7 * 24 * 60 * 60, "expiring: {ttl}");
    let again = PaneReplaced {
        seen_last_seen_ms: 200,
        ..later
    };
    assert!(bus.relay.mark_pane_replaced(&lead, &again).unwrap());
    assert!(bus.relay.clear_pane_replaced(&lead).unwrap());
    assert!(!bus.relay.clear_pane_replaced(&lead).unwrap());
    // An undecodable field is counted.
    let () = redis::cmd("SET")
        .arg(bus.key("health:replaced:lead-b"))
        .arg("garbage")
        .exec(&mut bus.raw)
        .unwrap();
    let other = name("lead-b");
    let read = bus.relay.panes_replaced(&[&lead, &other]).unwrap();
    assert_eq!((read.found.len(), read.unreadable), (0, 1));
}

/// A key the move script writes holds the wrong type: the script refuses before
/// its first write with a typed error, and nothing moved and nothing was
/// recorded, at each key position (AGE-2033 review, finding 2).
///
/// Tracing: TC-043-06
/// ACs: FR-043-AC-6
#[test]
fn tc_043_06_a_key_of_the_wrong_type_refuses_the_move_before_any_write() {
    // Which key gets the wrong type: the moves stream, or the lead's marker.
    for wrong in ["health:moves", "health:replaced:lead-a"] {
        let Some(mut bus) = Bus::connect("relink-wrong-type") else {
            return;
        };
        let seen = reg("lead-a", "hub", Some("s1"), "w1:p3", 100);
        bus.relay.register(&seen).unwrap();
        let () = redis::cmd("LPUSH")
            .arg(bus.key(wrong))
            .arg("x")
            .exec(&mut bus.raw)
            .unwrap();
        let to = "w2:p2".parse().unwrap();

        let relink = bus
            .relay
            .relink_pane(&seen, &to, &place("wsB", "1", 1), 200);
        assert!(
            matches!(relink, Err(Error::MoveUnreadable { .. })),
            "{wrong}: {relink:?}"
        );
        let rename = bus
            .relay
            .refresh_place(&name("lead-a"), &place("wsZ", "9", 1), 300, true);
        assert!(
            matches!(rename, Err(Error::MoveUnreadable { .. })),
            "{wrong}: {rename:?}"
        );
        assert_eq!(
            bus.relay.agent(&name("lead-a")).unwrap().unwrap(),
            seen,
            "{wrong}: nothing moved"
        );
        let stream_len: Result<usize, _> = redis::cmd("XLEN")
            .arg(bus.key("health:moves"))
            .query(&mut bus.raw);
        assert!(
            stream_len.is_err() || stream_len == Ok(0),
            "{wrong}: no event"
        );
    }
    // The registry itself of the wrong type fails before the script.
    let Some(mut bus) = Bus::connect("relink-wrong-agents") else {
        return;
    };
    let () = redis::cmd("SET")
        .arg(bus.key("agents"))
        .arg("x")
        .exec(&mut bus.raw)
        .unwrap();
    let seen = reg("lead-a", "hub", Some("s1"), "w1:p3", 100);
    assert!(
        bus.relay
            .relink_pane(&seen, &"w2:p2".parse().unwrap(), &place("wsB", "1", 1), 2)
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// Model: no interleaving relinks the wrong lead
// ---------------------------------------------------------------------------

const LEADS: usize = 2;
const MACHINES: [&str; 2] = ["hub", "node-a"];
const SESSIONS: [&str; 2] = ["s1", "s2"];
const PANES: [&str; 3] = ["w1:p1", "w1:p2", "w2:p1"];

#[derive(Debug, Clone)]
enum Op {
    /// A hook registers a lead (a whole-record write).
    Register {
        lead: usize,
        machine: usize,
        session: Option<usize>,
        pane: usize,
    },
    /// A watcher (or any consumer) reads a lead's registration.
    Snapshot { lead: usize },
    /// A watcher prunes a lead by its current stamp.
    Prune { lead: usize },
    /// A consumer relinks from an earlier snapshot: it may be stale.
    Relink { snap: usize, to: usize },
    /// A consumer renames the lead's place from what it reads now.
    Rename { lead: usize, tab: usize },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..LEADS, 0..MACHINES.len(), proptest::option::of(0..SESSIONS.len()), 0..PANES.len())
            .prop_map(|(lead, machine, session, pane)| Op::Register { lead, machine, session, pane }),
        3 => (0..LEADS).prop_map(|lead| Op::Snapshot { lead }),
        1 => (0..LEADS).prop_map(|lead| Op::Prune { lead }),
        4 => (0..8_usize, 0..PANES.len()).prop_map(|(snap, to)| Op::Relink { snap, to }),
        2 => (0..LEADS, 0..2_usize).prop_map(|(lead, tab)| Op::Rename { lead, tab }),
    ]
}

fn lead_name(i: usize) -> AgentName {
    name(&format!("lead-{i}"))
}

/// The protocol, written from FR-043 and not from the store: a relink applies to
/// exactly the registration that was snapshotted, whole, and changes the pane
/// and the place; nothing else does.
#[derive(Default)]
struct Model {
    registry: BTreeMap<AgentName, Registration>,
    snapshots: Vec<Registration>,
    events: Vec<MoveEvent>,
    clock: i64,
}

impl Model {
    fn tick(&mut self) -> i64 {
        self.clock += 1;
        self.clock
    }
}

fn run(bus: &mut Bus, ops: &[Op]) -> Result<(), TestCaseError> {
    let mut model = Model::default();
    for step in ops {
        match step {
            Op::Register {
                lead,
                machine,
                session,
                pane,
            } => {
                let stamp = model.tick();
                let r = reg(
                    &format!("lead-{lead}"),
                    MACHINES[*machine],
                    session.map(|s| SESSIONS[s]),
                    PANES[*pane],
                    stamp,
                );
                bus.relay.register(&r).unwrap();
                model.registry.insert(r.name.clone(), r);
            }
            Op::Snapshot { lead } => {
                let read = bus.relay.agent(&lead_name(*lead)).unwrap();
                prop_assert_eq!(&read, &model.registry.get(&lead_name(*lead)).cloned());
                model.snapshots.extend(read);
            }
            Op::Prune { lead } => {
                let n = lead_name(*lead);
                let stamp = model.registry.get(&n).map_or(0, |r| r.last_seen_ms);
                let pruned = bus.relay.prune(&n, stamp).unwrap();
                prop_assert_eq!(pruned.is_some(), model.registry.remove(&n).is_some());
            }
            Op::Relink { snap, to } => {
                let Some(seen) = model.snapshots.get(*snap).cloned() else {
                    continue;
                };
                let at = model.tick();
                let target = place("wsB", &to.to_string(), 1);
                let to_pane = PANES[*to].parse().unwrap();
                let got = bus.relay.relink_pane(&seen, &to_pane, &target, at).unwrap();
                let applies = seen.session_id.is_some()
                    && seen.herdr_pane.as_ref() != Some(&to_pane)
                    && model.registry.get(&seen.name) == Some(&seen);
                prop_assert_eq!(got.is_some(), applies, "relink of {:?}", seen);
                if applies {
                    let moved = Registration {
                        herdr_pane: Some(to_pane),
                        place: Some(target),
                        last_seen_ms: at.max(seen.last_seen_ms),
                        ..seen.clone()
                    };
                    // The wrong-lead guard, stated directly: the moved record is
                    // the snapshotted lead's own (name, machine, session) with
                    // only the pane and place replaced.
                    prop_assert_eq!(&moved.name, &seen.name);
                    prop_assert_eq!(&moved.machine, &seen.machine);
                    prop_assert_eq!(&moved.session_id, &seen.session_id);
                    model.registry.insert(seen.name.clone(), moved);
                    model.events.extend(got);
                }
            }
            Op::Rename { lead, tab } => {
                let n = lead_name(*lead);
                let at = model.tick();
                let new_place = place("wsA", &tab.to_string(), 2);
                let got = bus.relay.refresh_place(&n, &new_place, at, true).unwrap();
                let old = model.registry.get(&n).and_then(|r| r.place.clone());
                let changes = model.registry.contains_key(&n) && old.as_ref() != Some(&new_place);
                // A rename is a change of the lead's own workspace or tab label; a
                // tab count alone is only refreshed.
                let relabelled = old
                    .as_ref()
                    .is_some_and(|o| o.workspace != new_place.workspace || o.tab != new_place.tab);
                match &got {
                    PlaceRefresh::Unchanged => prop_assert!(!changes),
                    PlaceRefresh::Refreshed => prop_assert!(changes && !relabelled),
                    PlaceRefresh::Renamed(_) => prop_assert!(changes && relabelled),
                }
                if let Some(r) = model.registry.get_mut(&n).filter(|_| changes) {
                    r.place = Some(new_place);
                    if let PlaceRefresh::Renamed(event) = got {
                        model.events.push(*event);
                    }
                }
            }
        }
        // After every step the store is the model, and the event stream is the
        // model's events, newest first.
        let stored: BTreeMap<AgentName, Registration> = bus
            .relay
            .agents()
            .unwrap()
            .registrations
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect();
        prop_assert_eq!(&stored, &model.registry);
        let moves = bus.relay.recent_moves(1_000).unwrap();
        let newest_first: Vec<MoveEvent> = model.events.iter().rev().cloned().collect();
        prop_assert_eq!(moves.entries, newest_first);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Tracing: TC-043-04
    /// ACs: FR-043-AC-4
    #[test]
    fn tc_043_04_no_interleaving_of_registers_prunes_and_stale_snapshots_relinks_the_wrong_lead(
        ops in proptest::collection::vec(op(), 1..24),
    ) {
        let Some(mut bus) = Bus::connect("relink-model") else { return Ok(()) };
        run(&mut bus, &ops)?;
    }
}

/// The store is the only writer the model checks; a `Relay` value is also all a
/// second consumer needs: two consumers holding the same stale snapshot cannot
/// both relink it.
///
/// Tracing: TC-043-04
/// ACs: FR-043-AC-4
#[test]
fn tc_043_04_two_consumers_with_the_same_snapshot_relink_it_once() {
    let Some(mut bus) = Bus::connect("relink-two-consumers") else {
        return;
    };
    let seen = reg("lead-a", "hub", Some("s1"), "w1:p1", 10);
    bus.relay.register(&seen).unwrap();
    let mut second: Relay = bus.another_connection();
    let target = place("wsB", "1", 1);
    let first = bus
        .relay
        .relink_pane(&seen, &"w2:p1".parse().unwrap(), &target, 20)
        .unwrap();
    let other = second
        .relink_pane(&seen, &"w1:p2".parse().unwrap(), &place("wsA", "2", 2), 21)
        .unwrap();
    assert!(first.is_some() && other.is_none());
    assert_eq!(
        bus.relay
            .agent(&name("lead-a"))
            .unwrap()
            .unwrap()
            .herdr_pane,
        Some("w2:p1".parse().unwrap()),
        "the first relink stands"
    );
    assert_eq!(bus.relay.recent_moves(10).unwrap().entries.len(), 1);
}
