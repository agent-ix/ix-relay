//! The ordered per-lane assignment list against real Redis (FR-072).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]
use ix_trace_rs::trace;
use relay_core::{
    Address, AgentName, Error, HerdrPlaceIds, Place, Registration,
    assignment::{AssignError, AssignWrite, AssignWriter, AssignedTicket, FollowOutcome},
    fleet::HostLaneList,
};
use relay_test_support::Bus;

const LANE: &str = "draco:SWM:code";

fn orchestrator() -> AssignWriter {
    AssignWriter {
        name: "draco:SWM:plan".into(),
        orchestrator: true,
        caller_lane: None,
    }
}

fn ticket(id: &str) -> AssignedTicket {
    AssignedTicket {
        id: id.into(),
        reference: format!("https://linear.app/t/{id}"),
    }
}

fn set(ids: &[&str]) -> AssignWrite {
    AssignWrite::Set(ids.iter().map(|id| ticket(id)).collect())
}

fn bus(test: &str) -> Option<Bus> {
    let mut bus = Bus::connect(test)?;
    let lanes = [LANE.to_owned(), "draco:SWM:review".to_owned()];
    bus.relay
        .publish_lane_list(&"draco".parse().unwrap(), &lanes)
        .unwrap();
    Some(bus)
}

fn ids(list: &relay_core::assignment::AssignmentList) -> Vec<&str> {
    list.tickets.iter().map(|t| t.id.as_str()).collect()
}

fn refused(result: Result<impl std::fmt::Debug, Error>) -> AssignError {
    match result {
        Err(Error::Assign(reason)) => reason,
        other => panic!("expected an assignment refusal, got {other:?}"),
    }
}

fn history(bus: &mut Bus) -> Vec<relay_core::assignment::AssignTransition> {
    bus.relay.assignment_events("0-0", 1000).unwrap().events
}

#[trace("FR-072-AC-1")]
#[test]
fn a_set_is_read_back_in_order_with_refs_version_time_and_one_transition() {
    let Some(mut bus) = bus("assign-set") else {
        return;
    };
    assert_eq!(bus.relay.assignment(LANE).unwrap(), None);
    let written = bus
        .relay
        .assign(&orchestrator(), LANE, &set(&["A-1", "B-2", "C-3"]), None)
        .unwrap();
    let read = bus.relay.assignment(LANE).unwrap().unwrap();
    assert_eq!(read, written);
    assert_eq!(ids(&read), ["A-1", "B-2", "C-3"]);
    assert_eq!(read.tickets[1].reference, "https://linear.app/t/B-2");
    assert_eq!(read.version, 1);
    assert!(read.written_at_ms > 0);
    let events = history(&mut bus);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].op, "set");
}

#[trace("FR-072-AC-2")]
#[test]
fn append_remove_and_reorder_follow_in_order_one_transition_each() {
    let Some(mut bus) = bus("assign-ops") else {
        return;
    };
    let w = orchestrator();
    bus.relay.assign(&w, LANE, &set(&["A-1"]), None).unwrap();
    let l = bus
        .relay
        .assign(
            &w,
            LANE,
            &AssignWrite::Append(vec![ticket("B-2"), ticket("C-3")]),
            None,
        )
        .unwrap();
    assert_eq!((ids(&l), l.version), (vec!["A-1", "B-2", "C-3"], 2));
    let l = bus
        .relay
        .assign(&w, LANE, &AssignWrite::Remove("B-2".into()), None)
        .unwrap();
    assert_eq!((ids(&l), l.version), (vec!["A-1", "C-3"], 3));
    let l = bus
        .relay
        .assign(
            &w,
            LANE,
            &AssignWrite::Reorder(vec!["C-3".into(), "A-1".into()]),
            None,
        )
        .unwrap();
    assert_eq!((ids(&l), l.version), (vec!["C-3", "A-1"], 4));
    let events = history(&mut bus);
    let seen: Vec<_> = events
        .iter()
        .map(|e| (e.op.as_str(), e.writer.as_str(), e.version))
        .collect();
    assert_eq!(
        seen,
        [
            ("set", "draco:SWM:plan", 1),
            ("append", "draco:SWM:plan", 2),
            ("remove", "draco:SWM:plan", 3),
            ("reorder", "draco:SWM:plan", 4)
        ]
    );
    assert_eq!(events[3].tickets, ["C-3", "A-1"]);
    // A remove of an absent ticket and a reorder that is not a permutation change nothing.
    assert_eq!(
        refused(
            bus.relay
                .assign(&w, LANE, &AssignWrite::Remove("Z-9".into()), None)
        ),
        AssignError::NoSuchTicket("Z-9".into())
    );
    assert_eq!(
        refused(
            bus.relay
                .assign(&w, LANE, &AssignWrite::Reorder(vec!["C-3".into()]), None)
        ),
        AssignError::ReorderMismatch
    );
    assert_eq!(history(&mut bus).len(), 4);
}

#[trace("FR-072-AC-3")]
#[test]
fn a_lane_or_a_caller_without_the_role_is_refused_and_nothing_changes() {
    let Some(mut bus) = bus("assign-role") else {
        return;
    };
    bus.relay
        .assign(&orchestrator(), LANE, &set(&["A-1"]), None)
        .unwrap();
    let before = bus.relay.assignment(LANE).unwrap();
    let plain = AssignWriter {
        orchestrator: false,
        ..orchestrator()
    };
    assert_eq!(
        refused(bus.relay.assign(&plain, LANE, &set(&["X-1"]), None)),
        AssignError::NotOrchestrator
    );
    let own = AssignWriter {
        caller_lane: Some(LANE.parse::<Address>().unwrap()),
        ..orchestrator()
    };
    assert_eq!(
        refused(bus.relay.assign(&own, LANE, &set(&["X-1"]), None)),
        AssignError::OwnLane
    );
    assert_eq!(bus.relay.assignment(LANE).unwrap(), before);
    assert_eq!(history(&mut bus).len(), 1);
}

#[trace("FR-072-AC-4")]
#[test]
fn bounds_and_an_unknown_lane_are_refused_and_the_list_is_unchanged() {
    let Some(mut bus) = bus("assign-bounds") else {
        return;
    };
    let w = orchestrator();
    bus.relay.assign(&w, LANE, &set(&["A-1"]), None).unwrap();
    let many: Vec<String> = (0..51).map(|n| format!("T-{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(
        refused(bus.relay.assign(&w, LANE, &set(&many), None)),
        AssignError::TooManyTickets
    );
    let fifty = &many[..50];
    bus.relay
        .assign(&w, LANE, &set(&fifty[..49]), None)
        .unwrap();
    bus.relay
        .assign(&w, LANE, &AssignWrite::Append(vec![ticket("T-49")]), None)
        .unwrap();
    assert_eq!(
        refused(
            bus.relay
                .assign(&w, LANE, &AssignWrite::Append(vec![ticket("T-50")]), None)
        ),
        AssignError::TooManyTickets
    );
    bus.relay.assign(&w, LANE, &set(&["A-1"]), None).unwrap();
    let long_id = "x".repeat(65);
    assert!(matches!(
        refused(bus.relay.assign(&w, LANE, &set(&[&long_id]), None)),
        AssignError::InvalidField("ticket id")
    ));
    let mut long_ref = ticket("A-2");
    long_ref.reference = "r".repeat(513);
    assert!(matches!(
        refused(
            bus.relay
                .assign(&w, LANE, &AssignWrite::Set(vec![long_ref]), None)
        ),
        AssignError::InvalidField("ticket ref")
    ));
    assert_eq!(
        refused(bus.relay.assign(&w, LANE, &set(&["A-1", "A-1"]), None)),
        AssignError::DuplicateTicket("A-1".into())
    );
    assert_eq!(
        refused(
            bus.relay
                .assign(&w, LANE, &AssignWrite::Append(vec![ticket("A-1")]), None)
        ),
        AssignError::DuplicateTicket("A-1".into())
    );
    assert_eq!(
        refused(
            bus.relay
                .assign(&w, "nowhere:SWM:code", &set(&["A-1"]), None)
        ),
        AssignError::UnknownLane("nowhere:SWM:code".into())
    );
    assert_eq!(
        refused(bus.relay.assign(&w, "draco:SWM", &set(&["A-1"]), None)),
        AssignError::InvalidLane
    );
    let read = bus.relay.assignment(LANE).unwrap().unwrap();
    assert_eq!(ids(&read), ["A-1"]);
    assert_eq!(bus.relay.assignment("nowhere:SWM:code").unwrap(), None);
}

#[trace("FR-072-AC-5")]
#[test]
fn history_keeps_its_count_and_age_bounds_and_reports_truncation() {
    let Some(mut bus) = bus("assign-history") else {
        return;
    };
    // An entry far past seven days is trimmed by the next write.
    redis::cmd("XADD")
        .arg(bus.key("assign-events"))
        .arg("1000-0")
        .arg("record")
        .arg(r#"{"op":"set","lane":"x","writer":"w","version":1,"tickets":[],"at_ms":1000}"#)
        .query::<String>(&mut bus.raw)
        .unwrap();
    let w = orchestrator();
    for n in 0..4_100 {
        let id = format!("T-{}", n % 3);
        bus.relay.assign(&w, LANE, &set(&[&id]), None).unwrap();
    }
    let count: usize = redis::cmd("XLEN")
        .arg(bus.key("assign-events"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(count, 4_096);
    let page = bus.relay.assignment_events("1-0", 1).unwrap();
    assert!(page.truncated);
    let lower = page.available_from_ms.unwrap();
    assert!(lower > 1_000, "the old entry is gone: {lower}");
    assert_eq!(page.events.len(), 1);
    assert!(page.events[0].at_ms >= page.observed_at_ms - 604_800_000);
    assert!(bus.relay.assignment_events("0-0", 0).is_err());
    assert!(bus.relay.assignment_events("nope", 10).is_err());
}

#[trace("FR-072-AC-6")]
#[test]
fn reads_change_nothing_and_pop_no_inbox() {
    let Some(mut bus) = bus("assign-reads") else {
        return;
    };
    let w = orchestrator();
    bus.relay
        .assign(&w, LANE, &set(&["A-1", "B-2"]), None)
        .unwrap();
    bus.relay
        .assign(&w, "draco:SWM:review", &set(&["C-3"]), None)
        .unwrap();
    bus.rpush_raw("inbox:some-lead", b"queued");
    let snapshot = |bus: &mut Bus| -> Vec<(String, Vec<u8>)> {
        let mut keys = bus.keys();
        keys.sort();
        keys.into_iter()
            .map(|key| {
                let dump: Vec<u8> = redis::cmd("DUMP").arg(&key).query(&mut bus.raw).unwrap();
                (key, dump)
            })
            .collect()
    };
    let before = snapshot(&mut bus);
    let mut reads = Vec::new();
    for _ in 0..2 {
        reads.push((
            bus.relay.assignment(LANE).unwrap(),
            bus.relay.assignments().unwrap(),
            bus.relay.assignment_events("0-0", 10).unwrap().events,
        ));
    }
    assert_eq!(reads[0], reads[1]);
    assert_eq!(reads[0].1.len(), 2);
    assert_eq!(snapshot(&mut bus), before);
}

#[trace("FR-072-AC-7")]
#[test]
fn a_list_follows_a_lane_to_its_new_address_with_one_move_transition() {
    let Some(mut bus) = bus("assign-move") else {
        return;
    };
    let w = orchestrator();
    bus.relay
        .assign(&w, LANE, &set(&["A-1", "B-2"]), None)
        .unwrap();
    assert_eq!(
        bus.relay
            .assignment_follow(LANE, "draco:SWM:review")
            .unwrap(),
        FollowOutcome::Moved
    );
    assert_eq!(bus.relay.assignment(LANE).unwrap(), None);
    let moved = bus.relay.assignment("draco:SWM:review").unwrap().unwrap();
    assert_eq!((ids(&moved), moved.version), (vec!["A-1", "B-2"], 2));
    let last = history(&mut bus).pop().unwrap();
    assert_eq!(last.op, "move");
    assert_eq!(last.lane, "draco:SWM:review");
    assert_eq!(last.from.as_deref(), Some("draco:SWM:code"));
    assert_eq!(bus.relay.assignments().unwrap().len(), 1);
    // Nothing follows from a lane with no list.
    assert_eq!(
        bus.relay.assignment_follow(LANE, "draco:SWM:x").unwrap(),
        FollowOutcome::NoList
    );
}

#[trace("FR-072-AC-7")]
#[test]
fn a_renamed_registration_carries_its_list_to_the_new_tab() {
    let Some(mut bus) = Bus::connect("assign-rename") else {
        return;
    };
    let machine: relay_core::MachineName = "terra".parse().unwrap();
    let session: relay_core::SessionId = "s-1".parse().unwrap();
    let place = |tab: &str| Place {
        workspace: "SWM".into(),
        tab: tab.into(),
        herdr_ids: Some(HerdrPlaceIds {
            workspace: "w1".parse().unwrap(),
            tab: "w1:t1".parse().unwrap(),
            pane: "w1:p1".parse().unwrap(),
            session_id: session.clone(),
            verified_at_ms: 1,
        }),
    };
    let reg = Registration {
        name: AgentName::for_session(&machine, &session),
        machine,
        kind: "claude".into(),
        session_id: Some(session.clone()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: Some(place("code")),
        last_seen_ms: 1,
    };
    bus.relay.register(&reg).unwrap();
    bus.relay
        .assign(&orchestrator(), "terra:SWM:code", &set(&["A-1"]), None)
        .unwrap();
    bus.relay
        .refresh_place(&reg.name, &place("review"), 5, true)
        .unwrap();
    assert_eq!(bus.relay.assignment("terra:SWM:code").unwrap(), None);
    let list = bus.relay.assignment("terra:SWM:review").unwrap().unwrap();
    assert_eq!(ids(&list), ["A-1"]);
    let last = history(&mut bus).pop().unwrap();
    assert_eq!(
        (last.op.as_str(), last.from.as_deref()),
        ("move", Some("terra:SWM:code"))
    );
}

#[trace("FR-072-AC-8")]
#[test]
fn an_assignment_alone_is_no_activity_focus_or_work() {
    let Some(mut bus) = bus("assign-not-work") else {
        return;
    };
    bus.relay
        .assign(&orchestrator(), LANE, &set(&["A-1"]), None)
        .unwrap();
    let lane: Address = LANE.parse().unwrap();
    assert_eq!(bus.relay.activity_len().unwrap(), 0);
    assert!(bus.relay.activity_last(&lane).unwrap().is_empty());
    assert!(bus.relay.activity_events(10).unwrap().is_empty());
    assert!(
        bus.keys().iter().all(|k| !k.contains(":work:")
            && !k.contains("work-events")
            && !k.contains("lane:focus"))
    );
}

#[trace("FR-072-AC-9")]
#[test]
fn a_stale_host_refuses_raises_the_defect_and_a_refresh_restores_the_write() {
    let Some(mut bus) = bus("assign-stale") else {
        return;
    };
    let old = HostLaneList {
        machine: "oldhost".parse().unwrap(),
        refreshed_ms: 1,
        lanes: vec!["oldhost:SWM:code".into()],
    };
    bus.hset_raw(
        "fleet-lanes",
        "oldhost",
        &serde_json::to_string(&old).unwrap(),
    );
    let w = orchestrator();
    assert_eq!(
        refused(
            bus.relay
                .assign(&w, "oldhost:SWM:code", &set(&["A-1"]), None)
        ),
        AssignError::HostRegistryStale("oldhost:SWM:code".into())
    );
    assert_eq!(bus.relay.assignment("oldhost:SWM:code").unwrap(), None);
    assert!(history(&mut bus).is_empty());
    let defects = bus.relay.assignment_probe_defects().unwrap();
    assert_eq!(defects.len(), 1);
    assert_eq!(
        (defects[0].host.as_str(), defects[0].refused),
        ("oldhost", 1)
    );
    assert!(defects[0].last_ms > 0);
    // A live host's lane is unaffected.
    bus.relay.assign(&w, LANE, &set(&["A-1"]), None).unwrap();
    bus.relay
        .publish_lane_list(
            &"oldhost".parse().unwrap(),
            &["oldhost:SWM:code".to_owned()],
        )
        .unwrap();
    assert!(bus.relay.assignment_probe_defects().unwrap().is_empty());
    let list = bus
        .relay
        .assign(&w, "oldhost:SWM:code", &set(&["A-1"]), None)
        .unwrap();
    assert_eq!(ids(&list), ["A-1"]);
}

#[trace("FR-072-AC-10")]
#[test]
fn each_transition_stores_the_order_id_its_write_carried() {
    let Some(mut bus) = bus("assign-order-id") else {
        return;
    };
    let w = orchestrator();
    bus.relay
        .assign(&w, LANE, &set(&["A-1"]), Some("ord-1"))
        .unwrap();
    bus.relay
        .assign(&w, LANE, &AssignWrite::Append(vec![ticket("B-2")]), None)
        .unwrap();
    bus.relay
        .assign(&w, LANE, &AssignWrite::Remove("A-1".into()), Some("ord-3"))
        .unwrap();
    let seen: Vec<_> = history(&mut bus).into_iter().map(|e| e.order_id).collect();
    assert_eq!(seen, [Some("ord-1".into()), None, Some("ord-3".into())]);
    for bad in ["x".repeat(65), "bad\nid".to_owned(), " ".to_owned()] {
        assert!(matches!(
            refused(bus.relay.assign(&w, LANE, &set(&["Z-1"]), Some(&bad))),
            AssignError::InvalidField("order id")
        ));
    }
    assert_eq!(history(&mut bus).len(), 3);
    assert_eq!(ids(&bus.relay.assignment(LANE).unwrap().unwrap()), ["B-2"]);
}

fn lead(bus: &mut Bus, session: &str, pane: &str, tab: &str) -> Registration {
    let machine: relay_core::MachineName = "terra".parse().unwrap();
    let session: relay_core::SessionId = session.parse().unwrap();
    let reg = Registration {
        name: AgentName::for_session(&machine, &session),
        machine,
        kind: "claude".into(),
        session_id: Some(session.clone()),
        cwd: None,
        herdr_pane: Some(pane.parse().unwrap()),
        place: Some(place_of(&session, pane, tab)),
        last_seen_ms: 1,
    };
    bus.relay.register(&reg).unwrap();
    reg
}

fn place_of(session: &relay_core::SessionId, pane: &str, tab: &str) -> Place {
    Place {
        workspace: "SWM".into(),
        tab: tab.into(),
        herdr_ids: Some(HerdrPlaceIds {
            workspace: "w1".parse().unwrap(),
            tab: format!("w1:t-{pane}").parse().unwrap(),
            pane: pane.parse().unwrap(),
            session_id: session.clone(),
            verified_at_ms: 1,
        }),
    }
}

#[trace("FR-072-AC-1")]
#[test]
fn any_spelling_of_an_address_reads_and_writes_the_one_list() {
    let Some(mut bus) = bus("assign-case") else {
        return;
    };
    bus.relay
        .assign(&orchestrator(), "DRACO:swm:CODE", &set(&["A-1"]), None)
        .unwrap();
    assert_eq!(bus.relay.assignments().unwrap()[0].lane, LANE);
    for spelling in [LANE, "DRACO:swm:CODE", "draco:SWM:Code"] {
        let list = bus.relay.assignment(spelling).unwrap().unwrap();
        assert_eq!((list.lane.as_str(), ids(&list)), (LANE, vec!["A-1"]));
    }
}

#[trace("FR-072-AC-1")]
#[test]
fn addresses_fold_case_beyond_ascii_as_address_equality_does() {
    let Some(mut bus) = Bus::connect("assign-unicode") else {
        return;
    };
    bus.relay
        .publish_lane_list(
            &"draco".parse().unwrap(),
            &["draco:SWM:Caf\u{e9}".to_owned()],
        )
        .unwrap();
    let list = bus
        .relay
        .assign(&orchestrator(), "draco:SWM:CAF\u{c9}", &set(&["A-1"]), None)
        .unwrap();
    assert_eq!(list.lane, "draco:SWM:Caf\u{e9}");
    assert!(
        bus.relay
            .assignment("draco:swm:caf\u{c9}")
            .unwrap()
            .is_some()
    );
}

#[trace("FR-072-AC-7")]
#[test]
fn a_case_only_rename_moves_the_list() {
    let Some(mut bus) = Bus::connect("assign-case-rename") else {
        return;
    };
    let reg = lead(&mut bus, "s-1", "w1:p1", "code");
    bus.relay
        .assign(&orchestrator(), "terra:SWM:code", &set(&["A-1"]), None)
        .unwrap();
    let session = reg.session_id.clone().unwrap();
    bus.relay
        .refresh_place(&reg.name, &place_of(&session, "w1:p1", "CODE"), 5, true)
        .unwrap();
    let all = bus.relay.assignments().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].lane, "terra:SWM:CODE");
    assert_eq!(ids(&all[0]), ["A-1"]);
    let last = history(&mut bus).pop().unwrap();
    assert_eq!(
        (last.op.as_str(), last.from.as_deref()),
        ("move", Some("terra:SWM:code"))
    );
}

#[trace("FR-072-AC-7")]
#[test]
fn a_move_onto_an_address_that_holds_a_list_is_recorded_and_changes_no_list() {
    let Some(mut bus) = bus("assign-move-occupied") else {
        return;
    };
    let w = orchestrator();
    bus.relay.assign(&w, LANE, &set(&["A-1"]), None).unwrap();
    bus.relay
        .assign(&w, "draco:SWM:review", &set(&["B-2"]), None)
        .unwrap();
    let outcome = bus
        .relay
        .assignment_follow(LANE, "draco:SWM:review")
        .unwrap();
    assert_eq!(outcome, FollowOutcome::Failed("target_occupied".into()));
    assert_eq!(ids(&bus.relay.assignment(LANE).unwrap().unwrap()), ["A-1"]);
    assert_eq!(
        ids(&bus.relay.assignment("draco:SWM:review").unwrap().unwrap()),
        ["B-2"]
    );
    let last = history(&mut bus).pop().unwrap();
    assert_eq!(
        (
            last.op.as_str(),
            last.lane.as_str(),
            last.from.as_deref(),
            last.reason.as_deref()
        ),
        (
            "move_failed",
            "draco:SWM:review",
            Some(LANE),
            Some("target_occupied")
        )
    );
    assert_eq!(last.tickets, ["A-1"]);
    let defects = bus.relay.assignment_move_defects().unwrap();
    assert_eq!(defects.len(), 1);
    assert_eq!(
        (
            defects[0].from.as_str(),
            defects[0].to.as_str(),
            defects[0].reason.as_str()
        ),
        (LANE, "draco:SWM:review", "target_occupied")
    );
    // A write to either lane settles it.
    bus.relay
        .assign(&w, "draco:SWM:review", &set(&["B-2"]), None)
        .unwrap();
    assert!(bus.relay.assignment_move_defects().unwrap().is_empty());
}

#[trace("FR-072-AC-7")]
#[test]
fn a_move_the_store_cannot_take_is_recorded_as_failed() {
    let Some(mut bus) = bus("assign-move-error") else {
        return;
    };
    bus.relay
        .assign(&orchestrator(), LANE, &set(&["A-1"]), None)
        .unwrap();
    // The new address's list key holds the wrong type: the move script writes nothing.
    redis::cmd("SET")
        .arg(bus.key("assign-list:draco:SWM:review"))
        .arg("x")
        .query::<()>(&mut bus.raw)
        .unwrap();
    let outcome = bus
        .relay
        .assignment_follow(LANE, "draco:SWM:review")
        .unwrap();
    assert_eq!(outcome, FollowOutcome::Failed("unreadable_storage".into()));
    assert_eq!(ids(&bus.relay.assignment(LANE).unwrap().unwrap()), ["A-1"]);
    let last = history(&mut bus).pop().unwrap();
    assert_eq!(
        (last.op.as_str(), last.reason.as_deref()),
        ("move_failed", Some("unreadable_storage"))
    );
    assert_eq!(bus.relay.assignment_move_defects().unwrap().len(), 1);
}

#[trace("FR-072-AC-7")]
#[test]
fn two_lanes_that_swap_tab_names_keep_their_lists_and_raise_the_defect() {
    let Some(mut bus) = Bus::connect("assign-swap") else {
        return;
    };
    let a = lead(&mut bus, "s-a", "w1:p1", "code");
    let b = lead(&mut bus, "s-b", "w1:p2", "review");
    let w = orchestrator();
    bus.relay
        .assign(&w, "terra:SWM:code", &set(&["A-1"]), None)
        .unwrap();
    bus.relay
        .assign(&w, "terra:SWM:review", &set(&["B-2"]), None)
        .unwrap();
    let (sa, sb) = (a.session_id.clone().unwrap(), b.session_id.clone().unwrap());
    bus.relay
        .refresh_place(&a.name, &place_of(&sa, "w1:p1", "review"), 5, true)
        .unwrap();
    bus.relay
        .refresh_place(&b.name, &place_of(&sb, "w1:p2", "code"), 6, true)
        .unwrap();
    // Neither list was overwritten or swapped.
    assert_eq!(
        ids(&bus.relay.assignment("terra:SWM:code").unwrap().unwrap()),
        ["A-1"]
    );
    assert_eq!(
        ids(&bus.relay.assignment("terra:SWM:review").unwrap().unwrap()),
        ["B-2"]
    );
    let failed = history(&mut bus)
        .into_iter()
        .filter(|e| e.op == "move_failed")
        .count();
    assert_eq!(failed, 2);
    assert_eq!(bus.relay.assignment_move_defects().unwrap().len(), 2);
    // Correcting lane A (now at `review`) settles only the defect that ends at `review`: lane
    // B still reads the wrong list at `code`, and its defect stands.
    bus.relay
        .assign(&w, "terra:SWM:review", &set(&["A-1"]), None)
        .unwrap();
    let left = bus.relay.assignment_move_defects().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(
        (left[0].from.as_str(), left[0].to.as_str()),
        ("terra:SWM:review", "terra:SWM:code")
    );
    // A successful move of another pair does not end it either.
    bus.relay
        .assignment_follow("terra:SWM:x", "terra:SWM:y")
        .unwrap();
    assert_eq!(bus.relay.assignment_move_defects().unwrap().len(), 1);
}

#[trace("FR-072-AC-2")]
#[test]
fn an_emptied_list_leaves_the_lane_set_and_the_lane_limit_drops_lists_of_lanes_long_gone() {
    let Some(mut bus) = bus("assign-prune") else {
        return;
    };
    let w = orchestrator();
    bus.relay.assign(&w, LANE, &set(&["A-1"]), None).unwrap();
    let cleared = bus.relay.assign(&w, LANE, &set(&[]), None).unwrap();
    assert_eq!((cleared.tickets.len(), cleared.version), (0, 2));
    assert!(bus.relay.assignments().unwrap().is_empty());
    let members: usize = redis::cmd("SCARD")
        .arg(bus.key("assign-lanes"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(members, 0);
    // Fill the set with lanes that no registry lists, last written long ago.
    for n in 0..4_096 {
        let lane = format!("gone:SWM:t{n}");
        redis::pipe()
            .sadd(bus.key("assign-lanes"), &lane)
            .hset(bus.key(&format!("assign-meta:{lane}")), "at_ms", 1)
            .hset(bus.key(&format!("assign-meta:{lane}")), "version", 1)
            .query::<()>(&mut bus.raw)
            .unwrap();
    }
    let list = bus.relay.assign(&w, LANE, &set(&["A-2"]), None).unwrap();
    assert_eq!(ids(&list), ["A-2"]);
    let members: usize = redis::cmd("SCARD")
        .arg(bus.key("assign-lanes"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(members, 1);
}

#[trace("FR-072-AC-3")]
#[test]
fn the_writer_is_built_from_the_config_role_and_the_guard_lives_in_assign() {
    let Some(mut bus) = bus("assign-from-config") else {
        return;
    };
    let machine: relay_core::MachineName = "draco".parse().unwrap();
    let on: relay_core::config::AssignFile = toml::from_str("orchestrator = true").unwrap();
    let off = relay_core::config::AssignFile::default();
    let own: Address = LANE.parse().unwrap();
    let writer = AssignWriter::from_config(&on, &machine, None).unwrap();
    assert_eq!(
        (writer.name.as_str(), writer.orchestrator),
        ("orchestrator@draco", true)
    );
    bus.relay
        .assign(&writer, LANE, &set(&["A-1"]), None)
        .unwrap();
    let plain = AssignWriter::from_config(&off, &machine, None).unwrap();
    assert_eq!(
        refused(bus.relay.assign(&plain, LANE, &set(&["X-1"]), None)),
        AssignError::NotOrchestrator
    );
    let lane_writer = AssignWriter::from_config(&on, &machine, Some(own)).unwrap();
    assert_eq!(lane_writer.name, "draco:SWM:code");
    assert_eq!(
        refused(bus.relay.assign(&lane_writer, LANE, &set(&["X-1"]), None)),
        AssignError::OwnLane
    );
    let bad: relay_core::config::AssignFile = toml::from_str("orchestrator = \"yes\"").unwrap();
    assert!(AssignWriter::from_config(&bad, &machine, None).is_err());
}

#[trace("FR-072-AC-7")]
#[test]
fn a_move_defect_never_expires_while_the_list_is_still_under_the_old_address() {
    let Some(mut bus) = bus("assign-defect-lifetime") else {
        return;
    };
    let w = orchestrator();
    bus.relay.assign(&w, LANE, &set(&["A-1"]), None).unwrap();
    bus.relay
        .assign(&w, "draco:SWM:review", &set(&["B-2"]), None)
        .unwrap();
    bus.relay
        .assignment_follow(LANE, "draco:SWM:review")
        .unwrap();
    let ttl: i64 = redis::cmd("TTL")
        .arg(bus.key("assign-move-defects"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(ttl, -1, "an unresolved defect carries no expiry");
    // A prune that finds the split still there keeps it.
    bus.relay.prune_assignments().unwrap();
    assert_eq!(bus.relay.assignment_move_defects().unwrap().len(), 1);
    // Once the list is no longer under the old address nothing is split.
    redis::cmd("DEL")
        .arg(bus.key("assign-list:draco:SWM:code"))
        .query::<()>(&mut bus.raw)
        .unwrap();
    bus.relay.prune_assignments().unwrap();
    assert!(bus.relay.assignment_move_defects().unwrap().is_empty());
}
