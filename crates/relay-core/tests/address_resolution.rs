//! Address-only routing contract, including shortcut and conflict regressions.
#![allow(clippy::unwrap_used, reason = "test fixtures")]
use relay_core::{
    Address, AgentName, Error, HerdrPlaceIds, Place, Registration, Target, resolve_target,
    resolve_target_scoped,
};
fn lead(session: &str, tab: &str) -> Registration {
    let machine = "draco".parse().unwrap();
    let sid = session.parse().unwrap();
    Registration {
        name: AgentName::for_session(&machine, &sid),
        machine,
        kind: "codex".into(),
        session_id: Some(sid.clone()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        last_seen_ms: 1,
        place: Some(Place {
            workspace: "misc".into(),
            tab: tab.into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "w1".parse().unwrap(),
                tab: "w1:t1".parse().unwrap(),
                pane: "w1:p1".parse().unwrap(),
                session_id: sid,
                verified_at_ms: 1,
            }),
        }),
    }
}
#[test]
fn full_and_shortcut_addresses_resolve_the_same_live_session() {
    let lead = lead("genuine-session", "agent-skills");
    let sender: Address = "draco:ix-relay:code".parse().unwrap();
    for input in ["DRACO:MISC:AGENT-SKILLS", ":misc:agent-skills"] {
        assert_eq!(
            resolve_target_scoped(
                input,
                &lead.machine,
                Some(&sender),
                std::slice::from_ref(&lead)
            )
            .unwrap(),
            Target::Agent(lead.name.clone())
        );
    }
    let sender: Address = "draco:misc:code".parse().unwrap();
    assert_eq!(
        resolve_target_scoped(
            ":agent-skills",
            &lead.machine,
            Some(&sender),
            std::slice::from_ref(&lead)
        )
        .unwrap(),
        Target::Agent(lead.name)
    );
}
#[test]
fn names_and_panes_never_route_even_when_registered() {
    let lead = lead("genuine-session", "code");
    for input in [
        lead.name.as_str(),
        "filament-code-old-15",
        "w1:p1",
        "draco:misc",
        "draco:misc:code:w1:p1",
    ] {
        assert!(
            resolve_target(input, &lead.machine, std::slice::from_ref(&lead)).is_err(),
            "{input}"
        );
    }
}
#[test]
fn duplicate_tab_sessions_are_ambiguous_and_unverified_records_do_not_route() {
    let a = lead("session-a", "code");
    let b = lead("session-b", "code");
    assert!(matches!(
        resolve_target("draco:misc:code", &a.machine, &[a.clone(), b]),
        Err(Error::AmbiguousTarget { .. })
    ));
    let mut unverified = a.clone();
    unverified.place.as_mut().unwrap().herdr_ids = None;
    assert!(resolve_target("draco:misc:code", &a.machine, &[unverified]).is_err());
}
#[test]
fn moving_labels_changes_address_but_not_queue_identity() {
    let before = lead("session-a", "code");
    let after = lead("session-a", "review");
    assert_eq!(before.name, after.name);
    assert_eq!(
        after.full_address().unwrap().to_string(),
        "draco:misc:review"
    );
}
