// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Named focus definitions against a real, isolated Redis (FR-061): revisions, ownership,
//! restricted atomic writes, pins, retention, bounds and unreadable stored data.
//!
//! Every case runs on a `relay_test_support::Bus` under its own key prefix and is skipped
//! (with a note on standard error) when `IX_RELAY_TEST_URL` is unset. Nothing here touches a
//! shared Redis, a credential or Linear.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]

use std::{collections::BTreeMap, sync::Barrier, thread};

use ix_trace_rs::trace;
use relay_core::{
    AgentName, HerdrPlaceIds, MachineName, Place, Registration, SessionId,
    named_focus::{
        Applied, Caller, ChangeKind, ClaimRequest, Committed, CreateOutcome, DefinitionState,
        FocusContent, FocusError, FocusId, Incarnation, ListEntry, LivenessEvidence, Member,
        MemberKind, NewPin, OwnerBinding, Planned, Resolved, Revision, SessionState,
        UnavailableReason, UpdateOutcome, WritePlan,
        limits::{
            KEPT_REVISIONS, MAX_ACTIVE_DEFINITIONS, MAX_DEFINITIONS, MAX_DESCRIPTION_BYTES,
            MAX_FIELD_BYTES, MAX_NAME_BYTES, MAX_PINS, MAX_RECORD_FIELDS, MAX_REVISION_BYTES,
            MAX_SELECTOR_BYTES, MAX_SELECTORS, REMOVAL_EXAMINED,
        },
    },
};
use relay_test_support::Bus;

// The codec the command runs, as it is: the same ix-board encoder and decoder.
#[path = "../../agent-msg/src/focuses/codec.rs"]
mod codec;

use codec::IxBoardCodec;

const TEAM: &str = "SWM";
const T0: i64 = 1_800_000_000_000;
const DAY: i64 = 86_400_000;

fn check() -> IxBoardCodec {
    IxBoardCodec
}

fn machine(name: &str) -> MachineName {
    name.parse().unwrap()
}

fn session(id: &str) -> SessionId {
    id.parse().unwrap()
}

fn binding(session_id: &str, pid: u32, verified_at: i64) -> OwnerBinding {
    OwnerBinding::new(
        &machine("terra"),
        &session(session_id),
        Some(pid),
        &format!("terra:QSL:{session_id}"),
        verified_at,
    )
}

fn project(id: &str) -> Member {
    Member::new(MemberKind::Project, id, format!("project {id}"))
}

fn content(name: &str, projects: &[&str]) -> FocusContent {
    FocusContent {
        team: TEAM.to_owned(),
        name: name.to_owned(),
        description: String::new(),
        include: projects.iter().map(|id| project(id)).collect(),
        exclude: Vec::new(),
    }
}

fn caller(binding: &OwnerBinding, epoch: u64, now_ms: i64) -> Caller<'_> {
    Caller {
        binding,
        expected_epoch: epoch,
        now_ms,
    }
}

/// A registration whose placement is `Verified`: the session is live.
fn live(machine_name: &str, session_id: &str) -> Registration {
    let machine = machine(machine_name);
    let session = session(session_id);
    Registration {
        name: AgentName::for_session(&machine, &session),
        machine,
        kind: "codex".into(),
        session_id: Some(session.clone()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: Some(Place {
            workspace: "QSL".into(),
            tab: session_id.into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "w1".parse().unwrap(),
                tab: "w1:t1".parse().unwrap(),
                pane: "w1:p1".parse().unwrap(),
                session_id: session,
                verified_at_ms: 1,
            }),
        }),
        last_seen_ms: 1,
    }
}

/// A registration that claims a pane but whose placement nobody verified: state unknown.
fn unverified(machine_name: &str, session_id: &str) -> Registration {
    let mut registration = live(machine_name, session_id);
    registration.place = None;
    registration
}

fn nobody() -> LivenessEvidence {
    LivenessEvidence::complete(Vec::new())
}

fn create(bus: &mut Bus, name: &str, owner: &OwnerBinding, now_ms: i64) -> CreateOutcome {
    bus.relay
        .focus_create(&check(), content(name, &["p1"]), owner, now_ms, &nobody())
        .unwrap()
}

fn update(
    bus: &mut Bus,
    id: &FocusId,
    expected: u64,
    next: &FocusContent,
    owner: &OwnerBinding,
    epoch: u64,
) -> Result<UpdateOutcome, FocusError> {
    bus.relay.focus_update(
        &check(),
        &Resolved::by_id(id.clone()),
        expected,
        next,
        &caller(owner, epoch, T0 + 1),
        &nobody(),
    )
}

type Dump = BTreeMap<String, BTreeMap<Vec<u8>, Vec<u8>>>;

/// Every named-focus hash under the case's prefix, byte for byte.
fn dump(bus: &mut Bus) -> Dump {
    let mut all = Dump::new();
    let own = bus.key("focus:");
    for key in bus.keys() {
        if !key.starts_with(&own) {
            continue;
        }
        let fields: BTreeMap<Vec<u8>, Vec<u8>> =
            redis::cmd("HGETALL").arg(&key).query(&mut bus.raw).unwrap();
        all.insert(key, fields);
    }
    all
}

fn def_key(bus: &Bus, id: &FocusId) -> String {
    bus.key(&format!("focus:def:{id}"))
}

fn raw_field(bus: &mut Bus, id: &FocusId, field: &str) -> Vec<u8> {
    let key = def_key(bus, id);
    redis::cmd("HGET")
        .arg(key)
        .arg(field)
        .query(&mut bus.raw)
        .unwrap()
}

fn set_raw_field(bus: &mut Bus, id: &FocusId, field: &str, value: &[u8]) {
    let key = def_key(bus, id);
    redis::cmd("HSET")
        .arg(key)
        .arg(field)
        .arg(value)
        .exec(&mut bus.raw)
        .unwrap();
}

fn archive(bus: &mut Bus, id: &FocusId, owner: &OwnerBinding, now_ms: i64) {
    bus.relay
        .focus_archive(
            &check(),
            &Resolved::by_id(id.clone()),
            1,
            &caller(owner, 1, now_ms),
        )
        .unwrap();
}

fn pin(session_id: &str, revision: u64) -> NewPin {
    pin_on("terra", session_id, revision)
}

fn pin_on(machine_name: &str, session_id: &str, revision: u64) -> NewPin {
    NewPin {
        incarnation: Incarnation {
            machine: machine(machine_name),
            session: session(session_id),
            process_id: 7,
        },
        adopted_revision: revision,
        pinned_at: T0,
    }
}

fn code(error: &FocusError) -> &'static str {
    error.code().as_str()
}

// --- AC-1, AC-5: creation ------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-1")]
fn create_stores_revision_one_with_canonical_ids_and_the_caller_as_owner() {
    let Some(mut bus) = Bus::connect("focus-create") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let milestone = Member::milestone("m1", "p1", "Milestone one");
    let wanted = FocusContent {
        team: TEAM.into(),
        name: "Release".into(),
        description: "the release".into(),
        include: vec![
            project("p1"),
            Member::new(MemberKind::Initiative, "i1", "Initiative"),
            milestone,
            Member::new(MemberKind::Issue, "t1", "SWM-1"),
        ],
        exclude: vec![Member::new(MemberKind::Issue, "t2", "SWM-2")],
    };
    let created = bus
        .relay
        .focus_create(&check(), wanted, &owner, T0, &nobody())
        .unwrap();
    assert!(created.meta.focus_id.as_str().starts_with("foc-"));
    assert!(FocusId::is_well_formed(created.meta.focus_id.as_str()));
    assert_eq!(created.meta.latest_revision, 1);
    assert_eq!(created.owner.epoch, 1);
    assert_eq!(created.owner.owner_id, "terra/sess-a");
    assert_eq!(created.owner.change.kind, ChangeKind::Create);
    // The stored document is the SDK's own encoding of the same selection, byte for byte, and
    // the version it carries is the SDK's, not a number spelled in the relay.
    let sdk = ix_board::NamedSelection::new(
        ix_board::TeamKey(TEAM.into()),
        vec![
            ix_board::NamedMember::Initiative { id: "i1".into() },
            ix_board::NamedMember::Project { id: "p1".into() },
            ix_board::NamedMember::Milestone {
                id: "m1".into(),
                project_id: "p1".into(),
            },
            ix_board::NamedMember::Issue { id: "t1".into() },
        ],
        vec![ix_board::NamedMember::Issue { id: "t2".into() }],
    );
    assert_eq!(created.revision.selection.get(), sdk.encode().unwrap());
    let stored = raw_field(&mut bus, &created.meta.focus_id, "rev:1");
    let stored: serde_json::Value = serde_json::from_slice(&stored).unwrap();
    assert_eq!(
        stored["selection"]["schema_version"],
        ix_board::NAMED_SELECTION_SCHEMA_VERSION
    );

    let by_name = bus.relay.focus_resolve("Release").unwrap();
    assert_eq!(
        by_name.id, created.meta.focus_id,
        "a name resolves to the stored ID"
    );
    let read = bus.relay.focus_get(&check(), &by_name.id).unwrap();
    assert_eq!(read.summary.latest, created.revision);
    assert_eq!(read.kept_revisions, [1]);
    let (include, exclude) = read.summary.latest.members(&check()).unwrap();
    assert_eq!(include.len(), 4);
    assert_eq!(exclude.len(), 1);
    assert!(
        include
            .iter()
            .any(|m| m.kind == MemberKind::Milestone && m.project_id.as_deref() == Some("p1"))
    );
}

#[test]
#[trace("FR-061-AC-5")]
fn an_empty_inclusion_list_creates_and_is_an_empty_selection() {
    let Some(mut bus) = Bus::connect("focus-empty") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let created = bus
        .relay
        .focus_create(&check(), content("Empty", &[]), &owner, T0, &nobody())
        .unwrap();
    let read = bus
        .relay
        .focus_get(&check(), &created.meta.focus_id)
        .unwrap();
    let (include, exclude) = read.summary.latest.members(&check()).unwrap();
    assert!(include.is_empty() && exclude.is_empty());
}

#[test]
#[trace("FR-061-AC-5")]
fn malformed_members_are_invalid_arguments_and_write_nothing() {
    let Some(mut bus) = Bus::connect("focus-invalid") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let mut milestone_without_project = content("A", &[]);
    milestone_without_project.include.push(Member {
        kind: MemberKind::Milestone,
        id: "m1".into(),
        project_id: None,
        label: String::new(),
    });
    let mut project_with_project = content("B", &[]);
    project_with_project.include.push(Member {
        kind: MemberKind::Project,
        id: "p1".into(),
        project_id: Some("p2".into()),
        label: String::new(),
    });
    let mut empty_team = content("D", &[]);
    empty_team.team = " ".into();
    // FR-061: a name is 1 to 128 bytes and sanitized as a single line. Sanitizing replaces a
    // control character with a visible one (it does not remove it), so only an empty name or
    // one of spaces alone is blank. A name of control characters is stored, sanitized, and
    // is covered by `a_name_is_stored_as_one_plain_line_...`.
    let mut empty_name = content("", &[]);
    empty_name.description = "d".into();
    let spaces_only_name = content("   ", &[]);
    for bad in [
        milestone_without_project,
        project_with_project,
        empty_team,
        empty_name,
        spaces_only_name,
    ] {
        let error = bus
            .relay
            .focus_create(&check(), bad, &owner, T0, &nobody())
            .unwrap_err();
        assert_eq!(code(&error), "invalid_argument", "{error}");
    }
    assert!(dump(&mut bus).is_empty(), "a refusal writes nothing");
}

#[test]
#[trace("FR-061-AC-5")]
fn a_name_is_stored_as_one_plain_line_and_a_description_keeps_its_line_breaks() {
    let Some(mut bus) = Bus::connect("focus-text-rules") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let mut wanted = content("  Two\nlines\there\u{1b}[0m ", &["p1"]);
    wanted.description = "one\ntwo\u{7}".into();
    let created = bus
        .relay
        .focus_create(&check(), wanted, &owner, T0, &nobody())
        .unwrap();
    let name = &created.meta.name;
    assert!(
        !name.chars().any(char::is_control),
        "no line break, tab or control character is stored: {name:?}"
    );
    assert!(name.starts_with("  Two") && name.contains("lines") && name.contains("here"));
    assert_eq!(
        bus.relay.focus_resolve(name).unwrap().id,
        created.meta.focus_id,
        "the stored name is the one that resolves"
    );
    let read = bus
        .relay
        .focus_get(&check(), &created.meta.focus_id)
        .unwrap();
    assert_eq!(read.summary.latest.name, *name);
    let description = &read.summary.latest.description;
    assert!(description.starts_with("one\ntwo"), "{description:?}");
    assert!(
        !description.chars().any(|c| c.is_control() && c != '\n'),
        "{description:?}"
    );

    // The same input again holds the content already stored: nothing is written.
    let again = FocusContent {
        description: "one\ntwo\u{7}".into(),
        ..content("  Two\nlines\there\u{1b}[0m ", &["p1"])
    };
    let before = dump(&mut bus);
    let outcome = update(&mut bus, &created.meta.focus_id, 1, &again, &owner, 1).unwrap();
    assert!(
        matches!(outcome, UpdateOutcome::Unchanged(_)),
        "{outcome:?}"
    );
    assert_eq!(dump(&mut bus), before);
}

// --- AC-3, AC-4: revisions and names --------------------------------------------------------

#[test]
#[trace("FR-061-AC-3")]
fn of_two_concurrent_updates_exactly_one_writes_revision_two() {
    let Some(mut bus) = Bus::connect("focus-race") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Race", &owner, T0).meta.focus_id;
    let barrier = Barrier::new(2);
    let results: Vec<Result<UpdateOutcome, FocusError>> = thread::scope(|scope| {
        let handles: Vec<_> = ["p2", "p3"]
            .into_iter()
            .map(|project_id| {
                let mut relay = bus.another_connection();
                let (id, owner, barrier) = (id.clone(), owner.clone(), &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    relay.focus_update(
                        &check(),
                        &Resolved::by_id(id),
                        1,
                        &content("Race", &[project_id]),
                        &caller(&owner, 1, T0 + 1),
                        &nobody(),
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let written = results
        .iter()
        .filter(|r| matches!(r, Ok(UpdateOutcome::Written(_))))
        .count();
    assert_eq!(written, 1, "{results:?}");
    let loser = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert!(
        matches!(loser, FocusError::RevisionConflict { latest: 2, .. }),
        "{loser:?}"
    );
    let stale = update(&mut bus, &id, 1, &content("Race", &["p4"]), &owner, 1).unwrap_err();
    assert!(
        matches!(stale, FocusError::RevisionConflict { latest: 2, .. }),
        "{stale:?}"
    );
}

#[test]
#[trace("FR-061-AC-3")]
fn identical_content_is_returned_and_writes_nothing() {
    let Some(mut bus) = Bus::connect("focus-unchanged") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Same", &owner, T0).meta.focus_id;
    let before = dump(&mut bus);
    let outcome = update(&mut bus, &id, 1, &content("Same", &["p1"]), &owner, 1).unwrap();
    let UpdateOutcome::Unchanged(revision) = outcome else {
        panic!("identical content must not write: {outcome:?}");
    };
    assert_eq!(revision.revision, 1);
    assert_eq!(dump(&mut bus), before, "byte-identical");
}

#[test]
#[trace("FR-061-AC-3")]
fn revision_overflow_is_refused() {
    let Some(mut bus) = Bus::connect("focus-overflow") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Max", &owner, T0).meta.focus_id;
    // Plant a definition whose latest revision is the largest number.
    let mut revision: serde_json::Value =
        serde_json::from_slice(&raw_field(&mut bus, &id, "rev:1")).unwrap();
    revision["revision"] = serde_json::json!(u64::MAX);
    set_raw_field(
        &mut bus,
        &id,
        &format!("rev:{}", u64::MAX),
        &serde_json::to_vec(&revision).unwrap(),
    );
    let mut meta: serde_json::Value =
        serde_json::from_slice(&raw_field(&mut bus, &id, "meta")).unwrap();
    meta["latest_revision"] = serde_json::json!(u64::MAX);
    set_raw_field(&mut bus, &id, "meta", &serde_json::to_vec(&meta).unwrap());
    let before = dump(&mut bus);
    let error = update(&mut bus, &id, u64::MAX, &content("Max", &["p9"]), &owner, 1).unwrap_err();
    assert_eq!(code(&error), "limit", "{error}");
    assert_eq!(dump(&mut bus), before);
}

#[test]
#[trace("FR-061-AC-4")]
fn a_rename_keeps_the_id_frees_the_old_name_and_names_are_unique_per_team() {
    let Some(mut bus) = Bus::connect("focus-names") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let first = create(&mut bus, "Alpha", &owner, T0).meta.focus_id;
    let second = create(&mut bus, "Beta", &owner, T0).meta.focus_id;

    let taken = update(&mut bus, &first, 1, &content("beta", &["p1"]), &owner, 1).unwrap_err();
    assert!(matches!(taken, FocusError::NameTaken { .. }), "{taken:?}");

    update(&mut bus, &first, 1, &content("Gamma", &["p1"]), &owner, 1).unwrap();
    assert_eq!(
        bus.relay.focus_resolve("Gamma").unwrap().id,
        first,
        "the ID is kept"
    );
    assert!(
        matches!(
            bus.relay.focus_resolve("Alpha"),
            Err(FocusError::NotFound { .. })
        ),
        "the old name is free"
    );
    create(&mut bus, "Alpha", &owner, T0);
    assert_ne!(bus.relay.focus_resolve("Alpha").unwrap().id, first);

    // The same name in another team is accepted.
    let mut other_team = content("Beta", &["p1"]);
    other_team.team = "OTHER".into();
    bus.relay
        .focus_create(&check(), other_team, &owner, T0, &nobody())
        .unwrap();
    let ambiguous = bus.relay.focus_resolve("Beta").unwrap_err();
    assert_eq!(
        code(&ambiguous),
        "invalid_argument",
        "one name in two teams needs the ID"
    );
    assert!(bus.relay.focus_get(&check(), &second).is_ok());
}

#[test]
#[trace("FR-061-AC-4")]
fn a_name_that_moved_between_resolution_and_change_is_refused() {
    let Some(mut bus) = Bus::connect("focus-name-moved") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let first = create(&mut bus, "Alpha", &owner, T0).meta.focus_id;
    let by_name = bus.relay.focus_resolve("Alpha").unwrap();
    assert_eq!(by_name.id, first);
    // Another change renames the definition after the name was resolved.
    update(&mut bus, &first, 1, &content("Renamed", &["p1"]), &owner, 1).unwrap();

    // Renamed only: no definition holds the name, and the message does not say one does.
    let before = dump(&mut bus);
    let error = bus
        .relay
        .focus_update(
            &check(),
            &by_name,
            2,
            &content("Renamed", &["p2"]),
            &caller(&owner, 1, T0 + 2),
            &nobody(),
        )
        .unwrap_err();
    assert!(
        matches!(&error, FocusError::NameMoved { focus, holder: None, .. } if *focus == first),
        "{error:?}"
    );
    assert_eq!(code(&error), "name_taken");
    assert!(
        error.to_string().contains("no longer resolves")
            && !error.to_string().contains("already used"),
        "{error}"
    );
    assert_eq!(dump(&mut bus), before);

    // Another definition took the name meanwhile: it is named as the holder.
    let holder = create(&mut bus, "Alpha", &owner, T0).meta.focus_id;
    let error = bus
        .relay
        .focus_update(
            &check(),
            &by_name,
            2,
            &content("Renamed", &["p2"]),
            &caller(&owner, 1, T0 + 2),
            &nobody(),
        )
        .unwrap_err();
    assert!(
        matches!(&error, FocusError::NameMoved { holder: Some(found), .. } if *found == holder),
        "{error:?}"
    );
    assert!(error.to_string().contains(holder.as_str()), "{error}");
}

/// One plan of each restricted write against the definition `by_name` resolved, all made
/// before anything changes it.
fn plans_by_name(bus: &mut Bus, by_name: &Resolved, owner: &OwnerBinding) -> Vec<WritePlan> {
    let claimant = binding("sess-b", 12, T0);
    let Planned::Write(update) = bus
        .relay
        .focus_plan_update(
            &check(),
            by_name,
            1,
            content("Plan", &["p2"]),
            &caller(owner, 1, T0 + 2),
        )
        .unwrap()
    else {
        panic!("different content plans a write");
    };
    vec![
        *update,
        bus.relay
            .focus_plan_archive(&check(), by_name, 1, &caller(owner, 1, T0 + 2))
            .unwrap(),
        bus.relay
            .focus_plan_claim(
                &check(),
                by_name,
                &caller(&claimant, 1, T0 + 2),
                &ClaimRequest {
                    observed_owner_id: "terra/sess-a",
                    force_reason: None,
                },
                &nobody(),
            )
            .unwrap(),
        bus.relay
            .focus_plan_transfer(&check(), by_name, &caller(owner, 1, T0 + 2), &claimant)
            .unwrap(),
    ]
}

#[test]
#[trace("FR-061-AC-4", "FR-061-AC-3")]
fn a_plan_made_for_a_name_that_then_left_its_definition_is_refused_by_every_commit() {
    let Some(mut bus) = Bus::connect("focus-stale-name-plans") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    for (what, other_holder) in [
        ("archived", None),
        ("renamed", None),
        ("renamed and the name taken", Some(())),
    ] {
        let name = format!("Plan {what}");
        let id = create(&mut bus, &name, &owner, T0).meta.focus_id;
        let by_name = bus.relay.focus_resolve(&name).unwrap();
        let plans = plans_by_name(&mut bus, &by_name, &owner);
        match what {
            "archived" => archive(&mut bus, &id, &owner, T0 + 1),
            _ => {
                let moved_to = content(&format!("Elsewhere {what}"), &["p1"]);
                update(&mut bus, &id, 1, &moved_to, &owner, 1).unwrap();
            }
        }
        let holder = other_holder.map(|()| create(&mut bus, &name, &owner, T0).meta.focus_id);
        let before = dump(&mut bus);
        for plan in &plans {
            let error = bus.relay.focus_commit(plan).unwrap_err();
            let FocusError::NameMoved {
                focus,
                holder: found,
                name: moved,
                ..
            } = &error
            else {
                panic!("{what}: expected the name to have moved, got {error:?}");
            };
            assert_eq!(*focus, id, "{what}");
            assert_eq!(*found, holder, "{what}");
            assert_eq!(moved, &name, "{what}");
            assert_eq!(code(&error), "name_taken", "{what}");
            assert!(
                !error.to_string().contains("already used"),
                "{what}: {error}"
            );
        }
        assert_eq!(dump(&mut bus), before, "{what}: no commit wrote anything");
    }
}

// --- AC-6: ownership -----------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-6")]
fn only_the_owner_session_writes_and_a_resumed_process_refreshes_its_binding() {
    let Some(mut bus) = Bus::connect("focus-owner") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Owned", &owner, T0).meta.focus_id;

    // The owner's session exits: everything stays readable and the owner is absent.
    let read = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(
        nobody().owner_state(
            &read.summary.owner.owner_id,
            &read.summary.owner.binding.address
        ),
        SessionState::Absent
    );

    // Another session, even at the same address, is not the owner.
    let other = binding("sess-b", 12, T0);
    let same_address = OwnerBinding::new(
        &machine("terra"),
        &session("sess-b"),
        Some(12),
        &owner.address,
        T0,
    );
    for intruder in [&other, &same_address] {
        let error = update(&mut bus, &id, 1, &content("Owned", &["p2"]), intruder, 1).unwrap_err();
        assert_eq!(code(&error), "not_owner", "{error}");
        let archive_error = bus
            .relay
            .focus_archive(
                &check(),
                &Resolved::by_id(id.clone()),
                1,
                &caller(intruder, 1, T0),
            )
            .unwrap_err();
        assert_eq!(code(&archive_error), "not_owner");
        let transfer_error = bus
            .relay
            .focus_transfer(
                &check(),
                &Resolved::by_id(id.clone()),
                &caller(intruder, 1, T0),
                &other,
            )
            .unwrap_err();
        assert_eq!(code(&transfer_error), "not_owner");
    }

    // The same native session, resumed in a new process at a new address, is the same owner.
    let resumed = OwnerBinding::new(
        &machine("terra"),
        &session("sess-a"),
        Some(99),
        "terra:QSL:elsewhere",
        T0 + 5_000,
    );
    update(&mut bus, &id, 1, &content("Owned", &["p2"]), &resumed, 1).unwrap();
    let after = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(after.summary.owner.owner_id, "terra/sess-a");
    assert_eq!(after.summary.owner.epoch, 1);
    assert_eq!(after.summary.owner.binding.process_id, Some(99));
    assert_eq!(after.summary.owner.binding.address, "terra:QSL:elsewhere");
    assert_eq!(after.summary.owner.binding.verified_at, T0 + 5_000);
    assert_eq!(after.summary.meta.latest_revision, 2);
}

// --- AC-7, AC-8: claim ---------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-7")]
fn an_absent_or_replaced_owner_can_be_claimed_and_a_live_or_unknown_one_cannot() {
    let Some(mut bus) = Bus::connect("focus-claim") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let claimant = binding("sess-b", 12, T0 + 1);
    let id = create(&mut bus, "Claimed", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    let request = ClaimRequest {
        observed_owner_id: "terra/sess-a",
        force_reason: None,
    };

    let live_owner = LivenessEvidence::complete(vec![live("terra", "sess-a")]);
    let unknown_owner = LivenessEvidence::unreadable();
    let unverified_owner = LivenessEvidence::complete(vec![unverified("terra", "sess-a")]);
    for evidence in [&live_owner, &unknown_owner, &unverified_owner] {
        let error = bus
            .relay
            .focus_claim(
                &check(),
                &target,
                &caller(&claimant, 1, T0 + 2),
                &request,
                evidence,
            )
            .unwrap_err();
        assert_eq!(code(&error), "not_owner", "{error}");
    }
    assert_eq!(
        bus.relay
            .focus_get(&check(), &id)
            .unwrap()
            .summary
            .owner
            .epoch,
        1
    );

    // A different session now occupies the owner's recorded address: replaced.
    let replaced = LivenessEvidence::complete(vec![live("terra", "sess-c")]);
    assert_eq!(
        replaced.owner_state("terra/sess-a", "terra:QSL:sess-c"),
        SessionState::Replaced
    );

    // A stale epoch is a conflict, not a claim.
    let stale = bus
        .relay
        .focus_claim(
            &check(),
            &target,
            &caller(&claimant, 7, T0 + 2),
            &request,
            &nobody(),
        )
        .unwrap_err();
    assert_eq!(code(&stale), "owner_epoch_conflict");

    // The claim compares the observed old owner, never the caller's own ID.
    let wrong_observation = ClaimRequest {
        observed_owner_id: "terra/sess-b",
        force_reason: None,
    };
    let confused = bus
        .relay
        .focus_claim(
            &check(),
            &target,
            &caller(&claimant, 1, T0 + 2),
            &wrong_observation,
            &nobody(),
        )
        .unwrap_err();
    assert_eq!(code(&confused), "owner_epoch_conflict");

    let claimed = bus
        .relay
        .focus_claim(
            &check(),
            &target,
            &caller(&claimant, 1, T0 + 2),
            &request,
            &nobody(),
        )
        .unwrap();
    assert_eq!(claimed.owner_id, "terra/sess-b");
    assert_eq!(claimed.epoch, 2);
    assert_eq!(claimed.change.kind, ChangeKind::Claim);
    assert_eq!(
        claimed.change.previous_owner_id.as_deref(),
        Some("terra/sess-a")
    );

    // The former owner is refused after the claim.
    let former = update(&mut bus, &id, 1, &content("Claimed", &["p2"]), &owner, 1).unwrap_err();
    assert!(
        matches!(code(&former), "not_owner" | "owner_epoch_conflict"),
        "{former}"
    );
    update(&mut bus, &id, 1, &content("Claimed", &["p2"]), &claimant, 2).unwrap();
}

#[test]
#[trace("FR-061-AC-8")]
fn a_forced_claim_takes_a_live_owner_with_a_reason_and_records_it() {
    let Some(mut bus) = Bus::connect("focus-force") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let claimant = binding("sess-b", 12, T0 + 1);
    let id = create(&mut bus, "Forced", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    let evidence = LivenessEvidence::complete(vec![live("terra", "sess-a")]);
    for reason in ["", "  ", "two\nlines"] {
        let error = bus
            .relay
            .focus_claim(
                &check(),
                &target,
                &caller(&claimant, 1, T0 + 2),
                &ClaimRequest {
                    observed_owner_id: "terra/sess-a",
                    force_reason: Some(reason),
                },
                &evidence,
            )
            .unwrap_err();
        assert_eq!(code(&error), "invalid_argument", "{reason:?}: {error}");
    }
    let too_long = "x".repeat(257);
    let error = bus
        .relay
        .focus_claim(
            &check(),
            &target,
            &caller(&claimant, 1, T0 + 2),
            &ClaimRequest {
                observed_owner_id: "terra/sess-a",
                force_reason: Some(&too_long),
            },
            &evidence,
        )
        .unwrap_err();
    assert_eq!(code(&error), "limit");

    let owner_after = bus
        .relay
        .focus_claim(
            &check(),
            &target,
            &caller(&claimant, 1, T0 + 2),
            &ClaimRequest {
                observed_owner_id: "terra/sess-a",
                force_reason: Some("the owner is wedged"),
            },
            &evidence,
        )
        .unwrap();
    assert_eq!(owner_after.epoch, 2);
    assert_eq!(owner_after.change.kind, ChangeKind::ForcedClaim);
    assert_eq!(
        owner_after.change.reason.as_deref(),
        Some("the owner is wedged")
    );
    let stored = bus.relay.focus_get(&check(), &id).unwrap().summary.owner;
    assert_eq!(stored.change.reason.as_deref(), Some("the owner is wedged"));
}

// --- AC-9, AC-10: races and transfer -------------------------------------------------------

#[test]
#[trace("FR-061-AC-9")]
fn a_write_by_the_former_owner_after_a_transfer_is_refused_inside_the_atomic_operation() {
    let Some(mut bus) = Bus::connect("focus-stale-owner") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let successor = binding("sess-b", 12, T0);
    let id = create(&mut bus, "Racing", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());

    // The three writes are planned while A is still the owner.
    let Planned::Write(update_plan) = bus
        .relay
        .focus_plan_update(
            &check(),
            &target,
            1,
            content("Racing", &["p2"]),
            &caller(&owner, 1, T0 + 1),
        )
        .unwrap()
    else {
        panic!("the content differs");
    };
    let archive_plan = bus
        .relay
        .focus_plan_archive(&check(), &target, 1, &caller(&owner, 1, T0 + 1))
        .unwrap();
    let transfer_plan = bus
        .relay
        .focus_plan_transfer(&check(), &target, &caller(&owner, 1, T0 + 1), &successor)
        .unwrap();
    assert_eq!(transfer_plan.focus(), &id);
    assert!(matches!(transfer_plan.applied(), Applied::Owner(_)));

    // The transfer is applied first.
    assert_eq!(
        bus.relay.focus_commit(&transfer_plan).unwrap(),
        Committed::Applied
    );
    // A target whose process was measured keeps that fact.
    let stored = bus.relay.focus_get(&check(), &id).unwrap().summary.owner;
    assert_eq!(stored.binding.process_id, Some(12));
    let after = dump(&mut bus);

    for stale in [&*update_plan, &archive_plan] {
        let error = bus.relay.focus_commit(stale).unwrap_err();
        assert!(
            matches!(code(&error), "not_owner" | "owner_epoch_conflict"),
            "{error}"
        );
    }
    let again = bus.relay.focus_commit(&transfer_plan).unwrap_err();
    assert!(
        matches!(code(&again), "not_owner" | "owner_epoch_conflict"),
        "{again}"
    );
    assert_eq!(dump(&mut bus), after, "the refused writes changed nothing");
}

#[test]
#[trace("FR-061-AC-9")]
fn a_transfer_racing_a_claim_of_the_same_epoch_yields_exactly_one_new_owner() {
    let Some(mut bus) = Bus::connect("focus-transfer-claim") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let target_session = binding("sess-b", 12, T0);
    let claimant = binding("sess-c", 13, T0);
    let id = create(&mut bus, "Contested", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());

    let transfer = bus
        .relay
        .focus_plan_transfer(
            &check(),
            &target,
            &caller(&owner, 1, T0 + 1),
            &target_session,
        )
        .unwrap();
    let claim = bus
        .relay
        .focus_plan_claim(
            &check(),
            &target,
            &caller(&claimant, 1, T0 + 1),
            &ClaimRequest {
                observed_owner_id: "terra/sess-a",
                force_reason: None,
            },
            &nobody(),
        )
        .unwrap();
    let outcomes = [
        bus.relay.focus_commit(&transfer),
        bus.relay.focus_commit(&claim),
    ];
    let applied = outcomes
        .iter()
        .filter(|o| matches!(o, Ok(Committed::Applied)))
        .count();
    assert_eq!(applied, 1, "{outcomes:?}");
    let stored = bus.relay.focus_get(&check(), &id).unwrap().summary.owner;
    assert_eq!(stored.epoch, 2);
}

#[test]
#[trace("FR-061-AC-10")]
fn a_transfer_moves_ownership_and_a_stale_epoch_or_a_missing_target_changes_nothing() {
    let Some(mut bus) = Bus::connect("focus-transfer") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    // A transfer target from the registry has no measured process yet (FR-061: it is known
    // when the target next writes), so the binding says so.
    let successor = OwnerBinding::new(
        &machine("terra"),
        &session("sess-b"),
        None,
        "terra:QSL:sess-b",
        T0,
    );
    let id = create(&mut bus, "Handed", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());

    let before = dump(&mut bus);
    let stale = bus
        .relay
        .focus_transfer(&check(), &target, &caller(&owner, 9, T0 + 1), &successor)
        .unwrap_err();
    assert_eq!(code(&stale), "owner_epoch_conflict");
    let to_self = bus
        .relay
        .focus_transfer(&check(), &target, &caller(&owner, 1, T0 + 1), &owner)
        .unwrap_err();
    assert_eq!(code(&to_self), "invalid_argument");
    assert_eq!(dump(&mut bus), before);

    let moved = bus
        .relay
        .focus_transfer(&check(), &target, &caller(&owner, 1, T0 + 1), &successor)
        .unwrap();
    assert_eq!(moved.epoch, 2);
    assert_eq!(moved.owner_id, "terra/sess-b");
    assert_eq!(moved.change.kind, ChangeKind::Transfer);
    assert_eq!(
        moved.binding.process_id, None,
        "the target's process is not yet measured"
    );
    update(&mut bus, &id, 1, &content("Handed", &["p2"]), &successor, 2).unwrap();
    let old = update(&mut bus, &id, 2, &content("Handed", &["p3"]), &owner, 1).unwrap_err();
    assert!(
        matches!(code(&old), "not_owner" | "owner_epoch_conflict"),
        "{old}"
    );
}

// --- AC-11: archive ------------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-11")]
fn an_archived_definition_is_final_readable_and_releases_its_name() {
    let Some(mut bus) = Bus::connect("focus-archived") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Done", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    let meta = bus
        .relay
        .focus_archive(&check(), &target, 1, &caller(&owner, 1, T0 + 9))
        .unwrap();
    assert_eq!(meta.state, DefinitionState::Archived);
    assert_eq!(meta.latest_revision, 1, "archiving writes no revision");
    assert_eq!(meta.archived_at_ms, Some(T0 + 9));

    let read = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(read.kept_revisions, [1]);
    assert_eq!(read.summary.meta.state, DefinitionState::Archived);

    let updated = update(&mut bus, &id, 1, &content("Done", &["p2"]), &owner, 1).unwrap_err();
    assert_eq!(code(&updated), "archived");
    let transferred = bus
        .relay
        .focus_transfer(
            &check(),
            &target,
            &caller(&owner, 1, T0 + 10),
            &binding("sess-b", 12, T0),
        )
        .unwrap_err();
    assert_eq!(code(&transferred), "archived");
    let pinned = bus
        .relay
        .focus_pin_put(&check(), &id, &pin("sess-b", 1), &nobody())
        .unwrap_err();
    assert_eq!(code(&pinned), "archived");
    let again = bus
        .relay
        .focus_archive(&check(), &target, 1, &caller(&owner, 1, T0 + 11))
        .unwrap_err();
    assert_eq!(code(&again), "archived");

    assert!(matches!(
        bus.relay.focus_resolve("Done"),
        Err(FocusError::NotFound { .. })
    ));
    create(&mut bus, "Done", &owner, T0 + 12);
    let listed = bus.relay.focus_list(&check()).unwrap();
    assert_eq!(listed.len(), 2, "the archived definition is still listed");
}

// --- AC-12: bounds -------------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-12")]
fn boundary_sized_input_is_accepted_and_one_more_is_refused_naming_the_bound() {
    let Some(mut bus) = Bus::connect("focus-bounds") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id128 = "a".repeat(MAX_SELECTOR_BYTES);
    let label64 = "é".repeat(32);
    let milestones = |count: usize| -> Vec<Member> {
        (0..count)
            .map(|n| {
                let mut id = format!("{n:03}");
                id.push_str(&id128[3..]);
                Member {
                    kind: MemberKind::Milestone,
                    id,
                    project_id: Some(id128.clone()),
                    label: label64.clone(),
                }
            })
            .collect()
    };
    let full = FocusContent {
        team: TEAM.into(),
        name: "n".repeat(MAX_NAME_BYTES),
        description: "d".repeat(MAX_DESCRIPTION_BYTES),
        include: milestones(MAX_SELECTORS),
        exclude: Vec::new(),
    };
    let created = bus
        .relay
        .focus_create(&check(), full.clone(), &owner, T0, &nobody())
        .unwrap();
    for (_, fields) in dump(&mut bus) {
        for value in fields.values() {
            assert!(value.len() <= 64 * 1024, "no stored field exceeds 64 KiB");
        }
    }
    let read = bus
        .relay
        .focus_get(&check(), &created.meta.focus_id)
        .unwrap();
    let (include, _) = read.summary.latest.members(&check()).unwrap();
    assert_eq!(include.len(), MAX_SELECTORS);
    assert!(include.iter().all(|m| m.label.len() == 64));

    let before = dump(&mut bus);
    let mut one_more = full.clone();
    one_more.name = "other".into();
    one_more.include = milestones(MAX_SELECTORS + 1);
    let mut long_name = content(&"n".repeat(MAX_NAME_BYTES + 1), &[]);
    long_name.description.clear();
    let mut long_description = content("ok", &[]);
    long_description.description = "d".repeat(MAX_DESCRIPTION_BYTES + 1);
    let mut long_selector = content("ok2", &[]);
    long_selector
        .include
        .push(project(&"s".repeat(MAX_SELECTOR_BYTES + 1)));
    let mut long_project = content("ok3", &[]);
    long_project.include.push(Member::milestone(
        "m",
        "p".repeat(MAX_SELECTOR_BYTES + 1),
        "l",
    ));
    for (what, bad) in [
        ("selectors per revision", one_more),
        ("name bytes", long_name),
        ("description bytes", long_description),
        ("bytes per stable ID or selector", long_selector),
        ("bytes per stable ID or selector", long_project),
    ] {
        let error = bus
            .relay
            .focus_create(&check(), bad, &owner, T0, &nobody())
            .unwrap_err();
        let FocusError::Limit { bound, .. } = &error else {
            panic!("{what}: expected a limit, got {error:?}");
        };
        assert_eq!(*bound, what);
    }
    assert_eq!(dump(&mut bus), before, "a refused input changes nothing");
}

#[test]
#[trace("FR-061-AC-12")]
fn an_encoded_revision_of_exactly_56_kib_is_accepted_and_one_byte_more_is_refused() {
    let id = FocusId::generate().unwrap();
    let mut revision = Revision::build(&check(), &id, 1, &content("Big", &["p1"]), 1, T0).unwrap();
    let base = revision.encode().unwrap().len();
    revision.description = "d".repeat(MAX_REVISION_BYTES - base);
    assert_eq!(revision.encode().unwrap().len(), MAX_REVISION_BYTES);
    revision.description.push('d');
    let error = revision.encode().unwrap_err();
    assert_eq!(code(&error), "limit");
    assert!(matches!(
        error,
        FocusError::Limit { max, .. } if max == MAX_REVISION_BYTES as u64
    ));
}

// --- AC-13, AC-17: definition limits and reclamation ---------------------------------------

/// `ARCHIVED` definitions archived at `archived_at(n)` and the rest active: 256 in all, and
/// fewer than 128 of them active.
const ARCHIVED: usize = 160;

fn fill_to_the_limit(bus: &mut Bus, archived_at: impl Fn(usize) -> i64) -> Vec<FocusId> {
    let owner = binding("sess-a", 11, T0);
    let mut archived = Vec::new();
    for n in 0..ARCHIVED {
        let id = create(bus, &format!("old-{n:03}"), &owner, T0)
            .meta
            .focus_id;
        archive(bus, &id, &owner, archived_at(n));
        archived.push(id);
    }
    for n in 0..MAX_DEFINITIONS - ARCHIVED {
        create(bus, &format!("new-{n:03}"), &owner, T0);
    }
    archived
}

#[test]
#[trace("FR-061-AC-13")]
fn the_129th_active_and_the_257th_definition_are_refused_and_nothing_is_evicted() {
    let Some(mut bus) = Bus::connect("focus-limits") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    for n in 0..MAX_ACTIVE_DEFINITIONS {
        create(&mut bus, &format!("d-{n:03}"), &owner, T0);
    }
    let before = dump(&mut bus);
    let error = bus
        .relay
        .focus_create(
            &check(),
            content("one-too-many", &[]),
            &owner,
            T0,
            &nobody(),
        )
        .unwrap_err();
    assert!(
        matches!(error, FocusError::Limit { bound, max } if bound.contains("non-archived") && max == MAX_ACTIVE_DEFINITIONS as u64),
        "{error:?}"
    );
    assert_eq!(dump(&mut bus), before);
}

#[test]
#[trace("FR-061-AC-13")]
fn the_257th_definition_including_archived_is_refused_and_nothing_is_evicted() {
    let Some(mut bus) = Bus::connect("focus-limits-total") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    // Every archived definition is younger than 90 days, so none is removable.
    let ids = fill_to_the_limit(&mut bus, |_| T0 + 1);
    assert_eq!(ids.len(), ARCHIVED);
    let before = dump(&mut bus);
    assert_eq!(
        bus.relay.focus_list(&check()).unwrap().len(),
        MAX_DEFINITIONS
    );
    let error = bus
        .relay
        .focus_create(&check(), content("extra", &[]), &owner, T0 + DAY, &nobody())
        .unwrap_err();
    assert!(
        matches!(error, FocusError::Limit { bound, max } if bound.contains("including archived") && max == MAX_DEFINITIONS as u64),
        "{error:?}"
    );
    assert_eq!(dump(&mut bus), before, "no definition was evicted");
}

#[test]
#[trace("FR-061-AC-17")]
fn create_at_the_limit_removes_only_an_old_unpinned_archived_definition() {
    let Some(mut bus) = Bus::connect("focus-removal") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    // The first three are 100 days old when `now` arrives; the rest are one day old.
    let now = T0 + 100 * DAY;
    let ids = fill_to_the_limit(&mut bus, |n| if n < 3 { T0 + n as i64 } else { now - DAY });
    // The oldest carries a pin (planted: a pin cannot be put on an archived definition).
    set_raw_field(
        &mut bus,
        &ids[0],
        "pin:terra/sess-p/7",
        br#"{"incarnation":"terra/sess-p/7","adopted_revision":1,"pinned_at":1}"#,
    );
    // Its session is live: it is kept; the next oldest is removed.
    let evidence = LivenessEvidence::complete(vec![live("terra", "sess-p")]);
    let first = bus
        .relay
        .focus_create(&check(), content("c-1", &[]), &owner, now, &evidence)
        .unwrap();
    assert_eq!(first.removed.as_ref(), Some(&ids[1]));
    assert!(matches!(
        bus.relay.focus_get(&check(), &ids[1]),
        Err(FocusError::NotFound { .. })
    ));
    assert!(
        bus.relay.focus_get(&check(), &ids[0]).is_ok(),
        "a live pin keeps its definition"
    );
    assert!(
        bus.relay.focus_get(&check(), &ids[10]).is_ok(),
        "a young archived one is kept"
    );

    // Once the pin's session is gone the pin is reclaimed and the definition may go.
    let second = bus
        .relay
        .focus_create(&check(), content("c-2", &[]), &owner, now, &nobody())
        .unwrap();
    assert_eq!(second.removed.as_ref(), Some(&ids[0]));
    let third = bus
        .relay
        .focus_create(&check(), content("c-3", &[]), &owner, now, &nobody())
        .unwrap();
    assert_eq!(third.removed.as_ref(), Some(&ids[2]));

    // Nothing removable is left: the next create is refused with `limit`.
    let before = dump(&mut bus);
    let error = bus
        .relay
        .focus_create(&check(), content("c-4", &[]), &owner, now, &nobody())
        .unwrap_err();
    assert_eq!(code(&error), "limit", "{error}");
    assert_eq!(dump(&mut bus), before);
}

// --- AC-14: retention ----------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-14")]
fn only_the_eight_newest_revisions_are_kept_and_nothing_expires() {
    let Some(mut bus) = Bus::connect("focus-retention") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Long", &owner, T0).meta.focus_id;
    for n in 2..=21_u64 {
        let project_id = format!("p{n}");
        update(
            &mut bus,
            &id,
            n - 1,
            &content("Long", &[&project_id]),
            &owner,
            1,
        )
        .unwrap();
    }
    let read = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(read.summary.meta.latest_revision, 21);
    let expected: Vec<u64> = (21 - KEPT_REVISIONS + 1..=21).collect();
    assert_eq!(read.kept_revisions, expected);
    assert!(matches!(
        bus.relay.focus_revision(&check(), &id, 1),
        Err(FocusError::NotFound { .. })
    ));
    assert_eq!(
        bus.relay
            .focus_revision(&check(), &id, 14)
            .unwrap()
            .revision,
        14
    );

    for key in bus.keys() {
        let ttl: i64 = redis::cmd("PTTL").arg(&key).query(&mut bus.raw).unwrap();
        assert_eq!(ttl, -1, "{key} has no time-to-live");
    }
    // Reading is independent of the clock: 31 idle days later nothing has gone.
    let later = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(later.summary.owner.epoch, 1);
    assert_eq!(later.kept_revisions, read.kept_revisions);
}

// --- AC-15, AC-16: pins --------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-15")]
fn a_pin_is_one_per_incarnation_bounded_and_removed_only_when_unchanged() {
    let Some(mut bus) = Bus::connect("focus-pins") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Pinned", &owner, T0).meta.focus_id;
    let everyone = LivenessEvidence::complete(
        (0..=MAX_PINS)
            .map(|n| live("terra", &format!("s{n:02}")))
            .collect(),
    );
    bus.relay
        .focus_pin_put(&check(), &id, &pin("s00", 1), &everyone)
        .unwrap();
    bus.relay
        .focus_pin_put(&check(), &id, &pin("s00", 1), &everyone)
        .unwrap();
    assert_eq!(
        bus.relay.focus_get(&check(), &id).unwrap().pins.len(),
        1,
        "one per incarnation"
    );

    for n in 1..MAX_PINS {
        bus.relay
            .focus_pin_put(&check(), &id, &pin(&format!("s{n:02}"), 1), &everyone)
            .unwrap();
    }
    let full = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(full.pins.len(), MAX_PINS);
    let error = bus
        .relay
        .focus_pin_put(
            &check(),
            &id,
            &pin(&format!("s{MAX_PINS:02}"), 1),
            &everyone,
        )
        .unwrap_err();
    assert!(
        matches!(error, FocusError::Limit { max, .. } if max == MAX_PINS as u64),
        "{error:?}"
    );

    // Replacing an existing incarnation's pin is not a new pin, even at the limit.
    bus.relay
        .focus_pin_put(&check(), &id, &pin("s00", 1), &everyone)
        .unwrap();

    // Compare-and-delete: a pin replaced between read and remove survives.
    let read_entry = full
        .pins
        .iter()
        .find(|p| p.incarnation.contains("s05"))
        .unwrap()
        .clone();
    let mut replacement = pin("s05", 1);
    replacement.adopted_revision = 5;
    bus.relay
        .focus_pin_put(&check(), &id, &replacement, &everyone)
        .unwrap();
    assert!(!bus.relay.focus_pin_remove(&id, &read_entry).unwrap());
    let current = bus.relay.focus_get(&check(), &id).unwrap();
    let fresh = current
        .pins
        .iter()
        .find(|p| p.incarnation.contains("s05"))
        .unwrap()
        .clone();
    assert_eq!(fresh.value.as_ref().map(|v| v.adopted_revision), Some(5));
    assert!(bus.relay.focus_pin_remove(&id, &fresh).unwrap());
    assert_eq!(
        bus.relay.focus_get(&check(), &id).unwrap().pins.len(),
        MAX_PINS - 1
    );
}

#[test]
#[trace("FR-061-AC-16")]
fn dead_pins_are_reclaimed_for_a_33rd_and_live_or_unknown_pins_are_kept() {
    let Some(mut bus) = Bus::connect("focus-reclaim") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let all_live = LivenessEvidence::complete(
        (0..=MAX_PINS)
            .map(|n| live("terra", &format!("s{n:02}")))
            .collect(),
    );
    let newcomer = pin("s32", 1);

    let pinned = |name: &str, on: &str, bus: &mut Bus| -> FocusId {
        let id = create(bus, name, &owner, T0).meta.focus_id;
        for n in 0..MAX_PINS {
            bus.relay
                .focus_pin_put(
                    &check(),
                    &id,
                    &pin_on(on, &format!("s{n:02}"), 1),
                    &all_live,
                )
                .unwrap();
        }
        id
    };

    // Absent: a complete registry read shows none of the 32 sessions.
    let absent = pinned("absent", "terra", &mut bus);
    bus.relay
        .focus_pin_put(&check(), &absent, &newcomer, &nobody())
        .unwrap();
    let pins = bus.relay.focus_get(&check(), &absent).unwrap().pins;
    assert_eq!(pins.len(), 1, "the 32 dead pins are gone");
    assert!(pins[0].incarnation.contains("s32"));

    // Other sessions are registered, but none of the pinned ones: still absent.
    let replaced = pinned("replaced", "terra", &mut bus);
    let others = LivenessEvidence::complete(vec![live("terra", "someone-else")]);
    bus.relay
        .focus_pin_put(&check(), &replaced, &newcomer, &others)
        .unwrap();
    assert_eq!(
        bus.relay.focus_get(&check(), &replaced).unwrap().pins.len(),
        1
    );

    // Live, unknown (failed read), unverified placement and another machine: all kept.
    let unverified_here = LivenessEvidence::complete(
        (0..MAX_PINS)
            .map(|n| unverified("terra", &format!("s{n:02}")))
            .collect(),
    );
    let unverified_elsewhere = LivenessEvidence::complete(
        (0..MAX_PINS)
            .map(|n| unverified("luna", &format!("s{n:02}")))
            .collect(),
    );
    for (name, on, evidence) in [
        ("live", "terra", all_live.clone()),
        ("failed", "terra", LivenessEvidence::unreadable()),
        ("unverified", "terra", unverified_here),
        ("unverified-elsewhere", "luna", unverified_elsewhere),
    ] {
        let id = pinned(name, on, &mut bus);
        let before = dump(&mut bus);
        let error = bus
            .relay
            .focus_pin_put(&check(), &id, &newcomer, &evidence)
            .unwrap_err();
        assert_eq!(code(&error), "limit", "{name}: {error}");
        assert_eq!(dump(&mut bus), before, "{name}: every pin is kept");
    }
}

#[test]
#[trace("FR-061-AC-16")]
fn a_skipped_registry_record_makes_every_pin_unknown_through_the_production_evidence() {
    let Some(mut bus) = Bus::connect("focus-skipped-record") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let everyone = LivenessEvidence::complete(
        (0..=MAX_PINS)
            .map(|n| live("terra", &format!("s{n:02}")))
            .collect(),
    );
    let full = |bus: &mut Bus, name: &str| -> FocusId {
        let id = create(bus, name, &owner, T0).meta.focus_id;
        for n in 0..MAX_PINS {
            bus.relay
                .focus_pin_put(&check(), &id, &pin(&format!("s{n:02}"), 1), &everyone)
                .unwrap();
        }
        id
    };
    let newcomer = pin(&format!("s{MAX_PINS:02}"), 1);

    // Control: a complete, empty registry read through the same adapter reclaims all 32.
    let reclaimed = full(&mut bus, "complete-read");
    let evidence = bus.relay.focus_evidence();
    bus.relay
        .focus_pin_put(&check(), &reclaimed, &newcomer, &evidence)
        .unwrap();
    assert_eq!(
        bus.relay
            .focus_get(&check(), &reclaimed)
            .unwrap()
            .pins
            .len(),
        1
    );

    // One undecodable registry record: the read is partial, so no session may count as absent.
    let kept = full(&mut bus, "partial-read");
    bus.plant("terra-garbage", b"\xff not a registration");
    assert_eq!(
        bus.relay.agents().unwrap().skipped.len(),
        1,
        "the record is skipped"
    );
    let evidence = bus.relay.focus_evidence();
    for n in 0..MAX_PINS {
        let state = evidence.state_of(&machine("terra"), &session(&format!("s{n:02}")), None);
        assert_eq!(state, SessionState::Unknown);
    }
    let before = dump(&mut bus);
    let error = bus
        .relay
        .focus_pin_put(&check(), &kept, &newcomer, &evidence)
        .unwrap_err();
    assert_eq!(code(&error), "limit", "{error}");
    assert_eq!(dump(&mut bus), before, "no pin was reclaimed");
    assert_eq!(
        bus.relay.focus_get(&check(), &kept).unwrap().pins.len(),
        MAX_PINS
    );
    assert_eq!(bus.relay.focus_reclaim_pins(&kept, &evidence).unwrap(), 0);
    assert_eq!(dump(&mut bus), before);
}

// --- AC-18: unknown, undecodable and unsupported stored data --------------------------------

#[test]
#[trace("FR-061-AC-18")]
fn an_unknown_stored_member_survives_an_update() {
    let Some(mut bus) = Bus::connect("focus-unknown-member") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Future", &owner, T0).meta.focus_id;
    for field in ["meta", "owner"] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&raw_field(&mut bus, &id, field)).unwrap();
        value["from_the_future"] = serde_json::json!({"kept": true});
        set_raw_field(&mut bus, &id, field, &serde_json::to_vec(&value).unwrap());
    }
    let read = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(read.summary.meta.name, "Future", "known members are intact");
    update(&mut bus, &id, 1, &content("Future", &["p2"]), &owner, 1).unwrap();
    for field in ["meta", "owner"] {
        let value: serde_json::Value =
            serde_json::from_slice(&raw_field(&mut bus, &id, field)).unwrap();
        assert_eq!(value["from_the_future"]["kept"], true, "{field}");
    }
    assert_eq!(
        bus.relay
            .focus_get(&check(), &id)
            .unwrap()
            .summary
            .meta
            .latest_revision,
        2
    );
}

#[test]
#[trace("FR-061-AC-18")]
fn an_undecodable_or_unsupported_definition_is_listed_by_reason_and_never_overwritten() {
    let Some(mut bus) = Bus::connect("focus-unreadable") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let good = create(&mut bus, "Good", &owner, T0).meta.focus_id;
    let garbled = create(&mut bus, "Garbled", &owner, T0).meta.focus_id;
    let future = create(&mut bus, "Future", &owner, T0).meta.focus_id;

    set_raw_field(&mut bus, &garbled, "meta", b"\xff\xfenot json");
    let mut revision: serde_json::Value =
        serde_json::from_slice(&raw_field(&mut bus, &future, "rev:1")).unwrap();
    revision["selection"]["schema_version"] = serde_json::json!(4);
    set_raw_field(
        &mut bus,
        &future,
        "rev:1",
        &serde_json::to_vec(&revision).unwrap(),
    );

    let listed = bus.relay.focus_list(&check()).unwrap();
    assert_eq!(listed.len(), 3, "the others are still listed");
    let reason_of = |wanted: &FocusId| {
        listed.iter().find_map(|entry| match entry {
            ListEntry::Unavailable { focus, reason } if focus == wanted => Some(*reason),
            _ => None,
        })
    };
    assert_eq!(reason_of(&garbled), Some(UnavailableReason::Undecodable));
    assert_eq!(
        reason_of(&future),
        Some(UnavailableReason::UnsupportedSchema)
    );
    assert!(
        listed
            .iter()
            .any(|e| matches!(e, ListEntry::Available(s) if s.meta.focus_id == good))
    );

    let before = dump(&mut bus);
    for id in [&garbled, &future] {
        let error = update(&mut bus, id, 1, &content("Whatever", &["p2"]), &owner, 1).unwrap_err();
        assert_eq!(code(&error), "unavailable_definition", "{error}");
        let archived = bus
            .relay
            .focus_archive(
                &check(),
                &Resolved::by_id(id.clone()),
                1,
                &caller(&owner, 1, T0),
            )
            .unwrap_err();
        assert_eq!(code(&archived), "unavailable_definition");
        let pinned = bus
            .relay
            .focus_pin_put(&check(), id, &pin("s", 1), &nobody())
            .unwrap_err();
        assert_eq!(code(&pinned), "unavailable_definition");
    }
    assert_eq!(
        dump(&mut bus),
        before,
        "an unreadable record is never overwritten"
    );
    let missing = FocusId::generate().unwrap();
    assert!(matches!(
        bus.relay.focus_get(&check(), &missing),
        Err(FocusError::NotFound { .. })
    ));
}

#[test]
#[trace("FR-061-AC-18")]
fn an_unparseable_pin_value_is_kept_and_an_oversized_pin_is_refused() {
    let Some(mut bus) = Bus::connect("focus-odd-pins") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Odd", &owner, T0).meta.focus_id;
    set_raw_field(&mut bus, &id, "pin:terra/mystery/1", b"not json at all");
    let read = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(read.pins.len(), 1);
    assert!(
        read.pins[0].value.is_none(),
        "an unknown pin is kept, not interpreted"
    );
    let removed = bus.relay.focus_reclaim_pins(&id, &nobody()).unwrap();
    assert_eq!(
        removed, 0,
        "a pin whose incarnation cannot be read is never reclaimed"
    );
    assert_eq!(bus.relay.focus_get(&check(), &id).unwrap().pins.len(), 1);
}

// --- AC-1, AC-3: what a revision holds, read through the codec ---------------------------------

#[test]
#[trace("FR-061-AC-3")]
fn a_revision_holds_content_in_any_member_order_and_not_when_a_member_or_a_field_differs() {
    let id = FocusId::generate().unwrap();
    let content_of = |ids: &[&str]| {
        let mut wanted = content("Held", ids);
        wanted.exclude = vec![Member::new(MemberKind::Issue, "t9", "SWM-9")];
        wanted
    };
    let revision =
        Revision::build(&check(), &id, 1, &content_of(&["p3", "p1", "p2"]), 1, T0).unwrap();
    assert!(revision.holds(&check(), &content_of(&["p1", "p2", "p3"])));
    assert!(revision.holds(&check(), &content_of(&["p2", "p3", "p1"])));
    assert!(
        !revision.holds(&check(), &content_of(&["p1", "p2"])),
        "a member fewer"
    );
    assert!(
        !revision.holds(&check(), &content_of(&["p1", "p2", "p4"])),
        "a member differs"
    );
    let mut renamed = content_of(&["p1", "p2", "p3"]);
    renamed.name = "Other".into();
    assert!(!revision.holds(&check(), &renamed));
    let mut moved_to_include = content_of(&["p1", "p2", "p3"]);
    moved_to_include.exclude.clear();
    assert!(
        !revision.holds(&check(), &moved_to_include),
        "an exclusion fewer"
    );

    // A selection the codec cannot read holds nothing, and says why instead of panicking.
    let mut future: serde_json::Value =
        serde_json::from_slice(&revision.encode().unwrap()).unwrap();
    future["selection"]["schema_version"] = serde_json::json!(4);
    let future: Revision = serde_json::from_value(future).unwrap();
    assert!(!future.holds(&check(), &content_of(&["p1", "p2", "p3"])));
    assert_eq!(
        future.members(&check()).unwrap_err(),
        UnavailableReason::UnsupportedSchema
    );
}

// --- AC-4: a name that is already in use on create ------------------------------------------------

#[test]
#[trace("FR-061-AC-4")]
fn a_second_create_of_a_name_in_any_letter_case_is_name_taken_naming_its_holder() {
    let Some(mut bus) = Bus::connect("focus-create-name-taken") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let first = create(&mut bus, "Plan", &owner, T0).meta.focus_id;
    let before = dump(&mut bus);
    for name in ["Plan", "PLAN", "plAn"] {
        let error = bus
            .relay
            .focus_create(&check(), content(name, &["p2"]), &owner, T0 + 1, &nobody())
            .unwrap_err();
        assert!(
            matches!(&error, FocusError::NameTaken { holder: Some(holder), .. } if *holder == first),
            "{name}: {error:?}"
        );
        assert_eq!(code(&error), "name_taken");
    }
    assert_eq!(dump(&mut bus), before, "a refused create writes nothing");
}

#[test]
#[trace("FR-061-AC-4")]
fn of_two_concurrent_creates_of_one_name_exactly_one_definition_exists() {
    let Some(mut bus) = Bus::connect("focus-create-race") else {
        return;
    };
    let barrier = Barrier::new(2);
    let results: Vec<Result<CreateOutcome, FocusError>> = thread::scope(|scope| {
        let handles: Vec<_> = ["sess-a", "sess-b"]
            .into_iter()
            .map(|session_id| {
                let mut relay = bus.another_connection();
                let barrier = &barrier;
                scope.spawn(move || {
                    let owner = binding(session_id, 11, T0);
                    barrier.wait();
                    relay.focus_create(&check(), content("Same", &["p1"]), &owner, T0, &nobody())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let winners: Vec<&CreateOutcome> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    let loser = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert!(
        matches!(loser, FocusError::NameTaken { holder: Some(holder), .. } if *holder == winners[0].meta.focus_id),
        "{loser:?}"
    );
    let stored = dump(&mut bus);
    assert_eq!(stored[&bus.key("focus:names")].len(), 1, "one name entry");
    assert_eq!(stored[&bus.key("focus:index")].len(), 1, "one definition");
}

// --- AC-8, AC-9: two claimants of one owner, and plans that went stale --------------------------

#[test]
#[trace("FR-061-AC-8")]
fn of_two_claims_made_at_the_same_epoch_one_wins_epoch_two_and_the_other_is_told_who_won() {
    let Some(mut bus) = Bus::connect("focus-claim-vs-claim") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Contested", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    let (b, c) = (binding("sess-b", 12, T0), binding("sess-c", 13, T0));
    let request = ClaimRequest {
        observed_owner_id: "terra/sess-a",
        force_reason: None,
    };

    // Both claims are planned against epoch 1, then committed one after the other.
    let first = bus
        .relay
        .focus_plan_claim(
            &check(),
            &target,
            &caller(&b, 1, T0 + 1),
            &request,
            &nobody(),
        )
        .unwrap();
    let second = bus
        .relay
        .focus_plan_claim(
            &check(),
            &target,
            &caller(&c, 1, T0 + 1),
            &request,
            &nobody(),
        )
        .unwrap();
    assert_eq!(bus.relay.focus_commit(&first).unwrap(), Committed::Applied);
    let after_winner = dump(&mut bus);
    let error = bus.relay.focus_commit(&second).unwrap_err();
    assert!(
        matches!(
            &error,
            FocusError::OwnerEpochConflict { owner_id, epoch: 2, focus } if owner_id == "terra/sess-b" && *focus == id
        ),
        "{error:?}"
    );
    assert_eq!(dump(&mut bus), after_winner, "the loser changed nothing");
    assert_eq!(
        bus.relay
            .focus_get(&check(), &id)
            .unwrap()
            .summary
            .owner
            .owner_id,
        "terra/sess-b"
    );
}

#[test]
#[trace("FR-061-AC-8")]
fn two_claimants_racing_on_two_connections_yield_one_owner_and_one_conflict() {
    let Some(mut bus) = Bus::connect("focus-claim-threads") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Contested", &owner, T0).meta.focus_id;
    let barrier = Barrier::new(2);
    let results: Vec<(&str, Result<relay_core::named_focus::Owner, FocusError>)> =
        thread::scope(|scope| {
            let handles: Vec<_> = ["sess-b", "sess-c"]
                .into_iter()
                .map(|session_id| {
                    let mut relay = bus.another_connection();
                    let (id, barrier) = (id.clone(), &barrier);
                    scope.spawn(move || {
                        let claimant = binding(session_id, 20, T0 + 1);
                        barrier.wait();
                        let claimed = relay.focus_claim(
                            &check(),
                            &Resolved::by_id(id),
                            &caller(&claimant, 1, T0 + 2),
                            &ClaimRequest {
                                observed_owner_id: "terra/sess-a",
                                force_reason: None,
                            },
                            &nobody(),
                        );
                        (session_id, claimed)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
    let wins: Vec<_> = results.iter().filter(|(_, r)| r.is_ok()).collect();
    assert_eq!(wins.len(), 1, "{results:?}");
    let (winner, won) = wins[0];
    let won = won.as_ref().unwrap();
    assert_eq!(won.epoch, 2);
    assert_eq!(won.owner_id, format!("terra/{winner}"));
    let (_, lost) = results.iter().find(|(_, r)| r.is_err()).unwrap();
    assert!(
        matches!(
            lost.as_ref().unwrap_err(),
            FocusError::OwnerEpochConflict { owner_id, epoch: 2, .. } if *owner_id == won.owner_id
        ),
        "{lost:?}"
    );
    let stored = bus.relay.focus_get(&check(), &id).unwrap().summary.owner;
    assert_eq!((stored.owner_id, stored.epoch), (won.owner_id.clone(), 2));
}

#[test]
#[trace("FR-061-AC-9")]
fn a_plan_is_refused_when_the_owner_left_and_came_back_at_a_later_epoch() {
    let Some(mut bus) = Bus::connect("focus-aba-epoch") else {
        return;
    };
    let (a, b, c) = (
        binding("sess-a", 11, T0),
        binding("sess-b", 12, T0),
        binding("sess-c", 13, T0),
    );
    let id = create(&mut bus, "Revolving", &a, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    let Planned::Write(update_plan) = bus
        .relay
        .focus_plan_update(
            &check(),
            &target,
            1,
            content("Revolving", &["p2"]),
            &caller(&a, 1, T0 + 1),
        )
        .unwrap()
    else {
        panic!("the content differs");
    };
    let archive_plan = bus
        .relay
        .focus_plan_archive(&check(), &target, 1, &caller(&a, 1, T0 + 1))
        .unwrap();
    let transfer_plan = bus
        .relay
        .focus_plan_transfer(&check(), &target, &caller(&a, 1, T0 + 1), &c)
        .unwrap();

    // A hands the definition to B, and B hands it back: the owner is A again, at epoch 3.
    bus.relay
        .focus_transfer(&check(), &target, &caller(&a, 1, T0 + 2), &b)
        .unwrap();
    bus.relay
        .focus_transfer(&check(), &target, &caller(&b, 2, T0 + 3), &a)
        .unwrap();
    let stored = bus.relay.focus_get(&check(), &id).unwrap().summary.owner;
    assert_eq!(
        (stored.owner_id.as_str(), stored.epoch),
        ("terra/sess-a", 3)
    );

    let before = dump(&mut bus);
    for (what, plan) in [
        ("update", &*update_plan),
        ("archive", &archive_plan),
        ("transfer", &transfer_plan),
    ] {
        let error = bus.relay.focus_commit(plan).unwrap_err();
        assert!(
            matches!(
                &error,
                FocusError::OwnerEpochConflict { owner_id, epoch: 3, focus }
                    if owner_id == "terra/sess-a" && *focus == id
            ),
            "{what}: the same owner ID is not enough, the epoch moved: {error:?}"
        );
    }
    assert_eq!(dump(&mut bus), before, "nothing was written");
}

#[test]
#[trace("FR-061-AC-9", "FR-061-AC-11")]
fn a_plan_made_before_an_archive_is_refused_as_archived_and_writes_nothing() {
    let Some(mut bus) = Bus::connect("focus-stale-archived") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Closing", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    let Planned::Write(update_plan) = bus
        .relay
        .focus_plan_update(
            &check(),
            &target,
            1,
            content("Closing", &["p2"]),
            &caller(&owner, 1, T0 + 1),
        )
        .unwrap()
    else {
        panic!("the content differs");
    };
    let second_archive = bus
        .relay
        .focus_plan_archive(&check(), &target, 1, &caller(&owner, 1, T0 + 1))
        .unwrap();
    let transfer_plan = bus
        .relay
        .focus_plan_transfer(
            &check(),
            &target,
            &caller(&owner, 1, T0 + 1),
            &binding("sess-b", 12, T0),
        )
        .unwrap();
    archive(&mut bus, &id, &owner, T0 + 2);

    let before = dump(&mut bus);
    for plan in [&*update_plan, &second_archive, &transfer_plan] {
        let error = bus.relay.focus_commit(plan).unwrap_err();
        assert!(
            matches!(&error, FocusError::Archived { focus } if *focus == id),
            "{error:?}"
        );
    }
    assert_eq!(dump(&mut bus), before);
}

#[test]
#[trace("FR-061-AC-4")]
fn a_rename_planned_while_the_name_was_free_is_refused_inside_the_commit_when_it_is_taken() {
    let Some(mut bus) = Bus::connect("focus-name-taken-in-script") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let first = create(&mut bus, "Alpha", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(first.clone());
    let Planned::Write(plan) = bus
        .relay
        .focus_plan_update(
            &check(),
            &target,
            1,
            content("Gamma", &["p1"]),
            &caller(&owner, 1, T0 + 1),
        )
        .unwrap()
    else {
        panic!("a rename is a change");
    };
    // The plan saw "Gamma" free. Another definition takes it before the commit.
    let holder = create(&mut bus, "Gamma", &owner, T0).meta.focus_id;
    let before = dump(&mut bus);
    let error = bus.relay.focus_commit(&plan).unwrap_err();
    assert!(
        matches!(
            &error,
            FocusError::NameTaken { holder: Some(found), name, .. } if *found == holder && name == "Gamma"
        ),
        "{error:?}"
    );
    assert_eq!(
        dump(&mut bus),
        before,
        "the old name and the revision are untouched"
    );
    assert_eq!(bus.relay.focus_resolve("Alpha").unwrap().id, first);
}

#[test]
#[trace("FR-061-AC-3", "FR-061-AC-18")]
fn a_change_the_plan_never_saw_makes_each_commit_retry_and_the_wrappers_replan() {
    let Some(mut bus) = Bus::connect("focus-retry") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let claimant = binding("sess-b", 12, T0);
    let id = create(&mut bus, "Retried", &owner, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    let Planned::Write(update_plan) = bus
        .relay
        .focus_plan_update(
            &check(),
            &target,
            1,
            content("Retried", &["p2"]),
            &caller(&owner, 1, T0 + 1),
        )
        .unwrap()
    else {
        panic!("the content differs");
    };
    let archive_plan = bus
        .relay
        .focus_plan_archive(&check(), &target, 1, &caller(&owner, 1, T0 + 1))
        .unwrap();
    let claim_plan = bus
        .relay
        .focus_plan_claim(
            &check(),
            &target,
            &caller(&claimant, 1, T0 + 1),
            &ClaimRequest {
                observed_owner_id: "terra/sess-a",
                force_reason: None,
            },
            &nobody(),
        )
        .unwrap();
    let transfer_plan = bus
        .relay
        .focus_plan_transfer(&check(), &target, &caller(&owner, 1, T0 + 1), &claimant)
        .unwrap();

    // A newer writer adds a member to `meta`: the owner, the epoch, the revision and the state
    // are all as planned, but the bytes the plan was built from are not.
    let mut meta: serde_json::Value =
        serde_json::from_slice(&raw_field(&mut bus, &id, "meta")).unwrap();
    meta["from_a_newer_writer"] = serde_json::json!(1);
    set_raw_field(&mut bus, &id, "meta", &serde_json::to_vec(&meta).unwrap());
    let before = dump(&mut bus);
    for (what, plan) in [
        ("update", &*update_plan),
        ("archive", &archive_plan),
        ("claim", &claim_plan),
        ("transfer", &transfer_plan),
    ] {
        assert_eq!(
            bus.relay.focus_commit(plan).unwrap(),
            Committed::Retry,
            "{what}"
        );
    }
    assert_eq!(dump(&mut bus), before, "a retry writes nothing");

    // The wrapper plans again from the new bytes and succeeds.
    update(&mut bus, &id, 1, &content("Retried", &["p2"]), &owner, 1).unwrap();
    let meta: serde_json::Value =
        serde_json::from_slice(&raw_field(&mut bus, &id, "meta")).unwrap();
    assert_eq!(meta["from_a_newer_writer"], 1, "the newer member is kept");
    assert_eq!(meta["latest_revision"], 2);
}

// --- AC-7: the three claim fences the other cases do not reach ------------------------------------

#[test]
#[trace("FR-061-AC-7")]
fn a_replaced_owner_can_be_claimed_but_a_resumed_live_owner_and_a_remote_unverified_one_cannot() {
    let Some(mut bus) = Bus::connect("focus-claim-fences") else {
        return;
    };
    let claimant = binding("sess-z", 30, T0 + 1);
    let request = |owner_id: &'static str| ClaimRequest {
        observed_owner_id: owner_id,
        force_reason: None,
    };

    // Replaced: another native session now holds the address the owner recorded.
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Replaced", &owner, T0).meta.focus_id;
    let mut squatter = live("terra", "sess-c");
    squatter.place.as_mut().unwrap().tab = "sess-a".into();
    let replaced = LivenessEvidence::complete(vec![squatter]);
    assert_eq!(
        replaced.owner_state("terra/sess-a", &owner.address),
        SessionState::Replaced
    );
    let claimed = bus
        .relay
        .focus_claim(
            &check(),
            &Resolved::by_id(id),
            &caller(&claimant, 1, T0 + 2),
            &request("terra/sess-a"),
            &replaced,
        )
        .unwrap();
    assert_eq!((claimed.epoch, claimed.change.kind), (2, ChangeKind::Claim));
    assert_eq!(claimed.owner_id, "terra/sess-z");

    // Resumed: the same native session, in a new process at a new address, is still live.
    let id = create(&mut bus, "Resumed", &owner, T0).meta.focus_id;
    let resumed = OwnerBinding::new(
        &machine("terra"),
        &session("sess-a"),
        Some(99),
        "terra:QSL:elsewhere",
        T0 + 5,
    );
    update(&mut bus, &id, 1, &content("Resumed", &["p2"]), &resumed, 1).unwrap();
    let mut moved = live("terra", "sess-a");
    moved.place.as_mut().unwrap().tab = "elsewhere".into();
    let still_live = LivenessEvidence::complete(vec![moved]);
    let stored = bus.relay.focus_get(&check(), &id).unwrap().summary.owner;
    assert_eq!(stored.binding.address, "terra:QSL:elsewhere");
    assert_eq!(
        still_live.owner_state(&stored.owner_id, &stored.binding.address),
        SessionState::Live
    );
    let before = dump(&mut bus);
    let refused = bus
        .relay
        .focus_claim(
            &check(),
            &Resolved::by_id(id.clone()),
            &caller(&claimant, 1, T0 + 6),
            &request("terra/sess-a"),
            &still_live,
        )
        .unwrap_err();
    assert_eq!(code(&refused), "not_owner", "{refused}");
    assert_eq!(dump(&mut bus), before);

    // Another machine, placement not verified: unknown, so refused without `--force`.
    let remote = OwnerBinding::new(
        &machine("luna"),
        &session("sess-r"),
        Some(5),
        "luna:QSL:sess-r",
        T0,
    );
    let id = create(&mut bus, "Remote", &remote, T0).meta.focus_id;
    let unverified_remote = LivenessEvidence::complete(vec![unverified("luna", "sess-r")]);
    assert_eq!(
        unverified_remote.owner_state("luna/sess-r", &remote.address),
        SessionState::Unknown
    );
    let before = dump(&mut bus);
    let refused = bus
        .relay
        .focus_claim(
            &check(),
            &Resolved::by_id(id),
            &caller(&claimant, 1, T0 + 6),
            &request("luna/sess-r"),
            &unverified_remote,
        )
        .unwrap_err();
    assert_eq!(code(&refused), "not_owner", "{refused}");
    assert_eq!(dump(&mut bus), before);
}

#[test]
#[trace("FR-061-AC-7")]
fn an_archived_definition_can_still_change_owner_by_a_claim_and_the_current_owner_may_claim() {
    let Some(mut bus) = Bus::connect("focus-claim-archived") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Closed", &owner, T0).meta.focus_id;
    archive(&mut bus, &id, &owner, T0 + 1);
    let target = Resolved::by_id(id.clone());
    // Archive refuses update, activation, adoption and transfer; it does not list a claim, so
    // the owner of an archived definition is still reassignable. The caller's own ID is never
    // compared with the stored one, so the current owner's claim is also accepted.
    let again = bus
        .relay
        .focus_claim(
            &check(),
            &target,
            &caller(&owner, 1, T0 + 2),
            &ClaimRequest {
                observed_owner_id: "terra/sess-a",
                force_reason: None,
            },
            &nobody(),
        )
        .unwrap();
    assert_eq!((again.owner_id.as_str(), again.epoch), ("terra/sess-a", 2));
    let stored = bus.relay.focus_get(&check(), &id).unwrap();
    assert_eq!(stored.summary.meta.state, DefinitionState::Archived);
    assert_eq!(stored.summary.owner.epoch, 2);
}

// --- AC-16: reclamation at each update -------------------------------------------------------------

#[test]
#[trace("FR-061-AC-16")]
fn an_update_reclaims_pins_of_absent_sessions_before_trimming_and_keeps_live_and_unknown_ones() {
    let Some(mut bus) = Bus::connect("focus-reclaim-on-update") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let both_live = LivenessEvidence::complete(vec![live("terra", "s00"), live("terra", "s01")]);
    for (what, evidence, pins_left) in [
        ("absent", nobody(), 0),
        ("live", both_live.clone(), 2),
        ("unknown", LivenessEvidence::unreadable(), 2),
    ] {
        let id = create(&mut bus, what, &owner, T0).meta.focus_id;
        for session_id in ["s00", "s01"] {
            bus.relay
                .focus_pin_put(&check(), &id, &pin(session_id, 1), &both_live)
                .unwrap();
        }
        // Seven updates with the pins' sessions live bring the definition to revision 8: all
        // eight revisions are kept, and no pin is reclaimed on the way.
        for n in 2..=8_u64 {
            let next = content(what, &[&format!("p{n}")]);
            bus.relay
                .focus_update(
                    &check(),
                    &Resolved::by_id(id.clone()),
                    n - 1,
                    &next,
                    &caller(&owner, 1, T0 + 1),
                    &both_live,
                )
                .unwrap();
        }
        let before = bus.relay.focus_get(&check(), &id).unwrap();
        assert_eq!(before.pins.len(), 2, "{what}");
        assert_eq!(before.kept_revisions, (1..=8).collect::<Vec<_>>(), "{what}");

        // The ninth update trims revision 1 and, in the same call, reclaims what the evidence
        // says is dead.
        bus.relay
            .focus_update(
                &check(),
                &Resolved::by_id(id.clone()),
                8,
                &content(what, &["p9"]),
                &caller(&owner, 1, T0 + 2),
                &evidence,
            )
            .unwrap();
        let after = bus.relay.focus_get(&check(), &id).unwrap();
        assert_eq!(after.pins.len(), pins_left, "{what}");
        assert_eq!(after.kept_revisions, (2..=9).collect::<Vec<_>>(), "{what}");
    }
}

// --- AC-17: how far removal looks ---------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-17")]
fn create_at_the_limit_examines_eight_archived_definitions_and_no_more() {
    let Some(mut bus) = Bus::connect("focus-removal-examined") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let now = T0 + 100 * DAY;
    // Every archived definition is old enough; the oldest eight each carry a live pin, so the
    // ninth, which nothing pins, is outside what one create looks at.
    let ids = fill_to_the_limit(&mut bus, |n| T0 + n as i64);
    let pinned_value = br#"{"incarnation":"terra/sess-p/7","adopted_revision":1,"pinned_at":1}"#;
    for id in &ids[..REMOVAL_EXAMINED] {
        set_raw_field(&mut bus, id, "pin:terra/sess-p/7", pinned_value);
    }
    let evidence = LivenessEvidence::complete(vec![live("terra", "sess-p")]);
    let before = dump(&mut bus);
    let error = bus
        .relay
        .focus_create(&check(), content("extra", &[]), &owner, now, &evidence)
        .unwrap_err();
    assert_eq!(code(&error), "limit", "{error}");
    assert_eq!(
        dump(&mut bus),
        before,
        "nothing was removed, nothing reclaimed"
    );
    assert!(
        bus.relay
            .focus_get(&check(), &ids[REMOVAL_EXAMINED])
            .is_ok()
    );

    // Control: release the pin of the eighth, and that one is the definition removed.
    redis::cmd("HDEL")
        .arg(def_key(&bus, &ids[REMOVAL_EXAMINED - 1]))
        .arg("pin:terra/sess-p/7")
        .exec(&mut bus.raw)
        .unwrap();
    let made = bus
        .relay
        .focus_create(&check(), content("extra", &[]), &owner, now, &evidence)
        .unwrap();
    assert_eq!(made.removed.as_ref(), Some(&ids[REMOVAL_EXAMINED - 1]));
}

#[test]
#[trace("FR-061-AC-17")]
fn a_removal_candidate_with_a_pin_nobody_can_read_is_kept() {
    let Some(mut bus) = Bus::connect("focus-removal-unreadable-pin") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let now = T0 + 100 * DAY;
    let ids = fill_to_the_limit(&mut bus, |n| T0 + n as i64);
    set_raw_field(&mut bus, &ids[0], "pin:terra/mystery/1", b"not json at all");
    let made = bus
        .relay
        .focus_create(&check(), content("extra", &[]), &owner, now, &nobody())
        .unwrap();
    assert_eq!(
        made.removed.as_ref(),
        Some(&ids[1]),
        "the oldest is skipped, the next oldest goes"
    );
    let kept = bus.relay.focus_get(&check(), &ids[0]).unwrap();
    assert_eq!(
        kept.pins.len(),
        1,
        "an unknown pin is kept, never reclaimed"
    );
}

// --- AC-12, AC-18: a record outside the bounds the scripts check ----------------------------------------

#[test]
#[trace("FR-061-AC-12", "FR-061-AC-18")]
fn a_record_over_the_field_count_or_field_size_bound_is_unavailable_and_never_written() {
    let Some(mut bus) = Bus::connect("focus-record-bounds") else {
        return;
    };
    let owner = binding("sess-a", 11, T0);
    let id = create(&mut bus, "Bounded", &owner, T0).meta.focus_id;
    // `meta`, `owner` and `rev:1` are three of the fields the record may hold.
    for n in 0..MAX_RECORD_FIELDS - 3 {
        set_raw_field(&mut bus, &id, &format!("future:{n:03}"), b"x");
    }
    assert!(
        bus.relay.focus_get(&check(), &id).is_ok(),
        "exactly the most fields is still a definition"
    );

    set_raw_field(&mut bus, &id, "future:one-more", b"x");
    let before = dump(&mut bus);
    let error = bus.relay.focus_get(&check(), &id).unwrap_err();
    assert!(
        matches!(
            &error,
            FocusError::UnavailableDefinition {
                reason: UnavailableReason::Undecodable,
                ..
            }
        ),
        "{error:?}"
    );
    let error = update(&mut bus, &id, 1, &content("Bounded", &["p3"]), &owner, 1).unwrap_err();
    assert_eq!(code(&error), "unavailable_definition", "{error}");
    assert_eq!(
        dump(&mut bus),
        before,
        "an unreadable record is never overwritten"
    );

    // One field over the size bound makes the record unavailable in the same way.
    let other = create(&mut bus, "Oversized", &owner, T0).meta.focus_id;
    set_raw_field(
        &mut bus,
        &other,
        "future:big",
        &vec![b'x'; MAX_FIELD_BYTES + 1],
    );
    let error = bus.relay.focus_get(&check(), &other).unwrap_err();
    assert_eq!(code(&error), "unavailable_definition", "{error}");
}

// --- AC-18: a field of the record this build does not know --------------------------------------------

#[test]
#[trace("FR-061-AC-18")]
fn an_unknown_field_of_the_record_survives_every_write_and_reclamation() {
    let Some(mut bus) = Bus::connect("focus-unknown-field") else {
        return;
    };
    let a = binding("sess-a", 11, T0);
    let b = binding("sess-b", 12, T0);
    let id = create(&mut bus, "Future", &a, T0).meta.focus_id;
    let target = Resolved::by_id(id.clone());
    set_raw_field(&mut bus, &id, "future:note", b"keep me");
    let kept = |bus: &mut Bus, what: &str| {
        assert_eq!(raw_field(bus, &id, "future:note"), b"keep me", "{what}");
    };

    update(&mut bus, &id, 1, &content("Future", &["p2"]), &a, 1).unwrap();
    kept(&mut bus, "update");
    bus.relay
        .focus_pin_put(&check(), &id, &pin("s00", 2), &nobody())
        .unwrap();
    assert_eq!(bus.relay.focus_reclaim_pins(&id, &nobody()).unwrap(), 1);
    kept(&mut bus, "reclamation");
    bus.relay
        .focus_transfer(&check(), &target, &caller(&a, 1, T0 + 1), &b)
        .unwrap();
    kept(&mut bus, "transfer");
    bus.relay
        .focus_archive(&check(), &target, 2, &caller(&b, 2, T0 + 2))
        .unwrap();
    kept(&mut bus, "archive");
    assert_eq!(
        bus.relay
            .focus_get(&check(), &id)
            .unwrap()
            .summary
            .meta
            .state,
        DefinitionState::Archived,
        "the record with an unknown field is still a healthy definition"
    );
}
