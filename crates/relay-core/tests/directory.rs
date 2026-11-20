//! Directory state machine refinement tests against real Redis.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]
use relay_core::{
    AgentName, DirectoryRead, HerdrPlaceIds, IdentityCommit, Message, Place, Registration, Target,
};
use relay_test_support::Bus;

fn registration(session: &str, tab: &str) -> Registration {
    let machine = "terra".parse().unwrap();
    let session = session.parse().unwrap();
    Registration {
        name: AgentName::for_session(&machine, &session),
        machine,
        kind: "codex".into(),
        session_id: Some(session.clone()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: Some(Place {
            workspace: "QSL".into(),
            tab: tab.into(),
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
fn resolved(bus: &mut Bus) -> relay_core::DirectoryRecipient {
    let DirectoryRead::Unique(recipient) = bus
        .relay
        .directory_recipient(&"terra:QSL:plan".parse().unwrap())
        .unwrap()
    else {
        panic!("unique expected")
    };
    *recipient
}
fn message(recipient: &relay_core::DirectoryRecipient) -> Message {
    Message {
        from: "luna:IR:code".parse().unwrap(),
        to: Target::Agent(recipient.registration.name.clone()),
        text: "one-way network proof".into(),
        sent_at_ms: 1,
        forwarded: false,
    }
}
#[test]
fn fenced_enqueue_rejects_replacement_and_preserves_original_session_mail() {
    let Some(mut bus) = Bus::connect("directory-replacement") else {
        return;
    };
    let machine = "terra".parse().unwrap();
    let old = registration("old-session", "plan");
    bus.relay
        .commit_identity(&IdentityCommit {
            registration: Some(old.clone()),
            ..Default::default()
        })
        .unwrap();
    let revision = bus.relay.begin_directory_observation(&machine).unwrap();
    assert!(
        bus.relay
            .publish_directory(&machine, revision, std::slice::from_ref(&old))
            .unwrap()
    );
    let recipient = resolved(&mut bus);
    bus.relay
        .send_directory(&message(&recipient), &recipient)
        .unwrap();
    let new = registration("new-session", "plan");
    let revision = bus.relay.begin_directory_observation(&machine).unwrap();
    assert!(
        bus.relay
            .publish_directory(&machine, revision, std::slice::from_ref(&new))
            .unwrap()
    );
    assert!(
        bus.relay
            .send_directory(&message(&recipient), &recipient)
            .is_err()
    );
    assert_eq!(bus.relay.inbox_len(&old.name).unwrap(), 1);
    assert_eq!(bus.relay.inbox_len(&new.name).unwrap(), 0);
    assert_eq!(bus.relay.log(0, 20).unwrap().len(), 1);
}
#[test]
fn delayed_and_replayed_publications_cannot_restore_expired_ownership() {
    let Some(mut bus) = Bus::connect("directory-fence") else {
        return;
    };
    let machine = "terra".parse().unwrap();
    let old = bus.relay.begin_directory_observation(&machine).unwrap();
    let new = bus.relay.begin_directory_observation(&machine).unwrap();
    assert!(
        !bus.relay
            .publish_directory(&machine, old, &[registration("old", "plan")])
            .unwrap()
    );
    assert!(bus.relay.publish_directory(&machine, new, &[]).unwrap());
    assert!(
        !bus.relay
            .publish_directory(&machine, new, &[registration("replay", "plan")])
            .unwrap()
    );
    let _: i64 = redis::cmd("DEL")
        .arg(bus.key("directory:terra"))
        .query(&mut bus.raw)
        .unwrap();
    assert!(
        !bus.relay
            .publish_directory(&machine, new, &[registration("replay", "plan")])
            .unwrap()
    );
    assert!(matches!(
        bus.relay
            .directory_recipient(&"terra:QSL:plan".parse().unwrap())
            .unwrap(),
        DirectoryRead::Unavailable
    ));
}
#[test]
fn expired_or_ambiguous_directory_does_not_route_even_with_a_registry_record() {
    let Some(mut bus) = Bus::connect("directory-ambiguity-expiry") else {
        return;
    };
    let machine = "terra".parse().unwrap();
    let one = registration("one", "plan");
    let two = registration("two", "plan");
    let revision = bus.relay.begin_directory_observation(&machine).unwrap();
    bus.relay
        .publish_directory(&machine, revision, &[one.clone(), two])
        .unwrap();
    assert!(matches!(
        bus.relay
            .directory_recipient(&"terra:QSL:plan".parse().unwrap())
            .unwrap(),
        DirectoryRead::Ambiguous
    ));
    let revision = bus.relay.begin_directory_observation(&machine).unwrap();
    bus.relay
        .publish_directory(&machine, revision, &[one])
        .unwrap();
    let recipient = resolved(&mut bus);
    let _: bool = redis::cmd("PEXPIRE")
        .arg(bus.key("directory:terra"))
        .arg(1)
        .query(&mut bus.raw)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(
        bus.relay
            .send_directory(&message(&recipient), &recipient)
            .is_err()
    );
    assert!(bus.relay.log(0, 20).unwrap().is_empty());
}
#[test]
fn wrong_log_type_never_leaves_a_partial_enqueue() {
    let Some(mut bus) = Bus::connect("directory-wrong-type") else {
        return;
    };
    let machine = "terra".parse().unwrap();
    let revision = bus.relay.begin_directory_observation(&machine).unwrap();
    bus.relay
        .publish_directory(&machine, revision, &[registration("one", "plan")])
        .unwrap();
    let recipient = resolved(&mut bus);
    let _: () = redis::cmd("SET")
        .arg(bus.key("log"))
        .arg("bad")
        .query(&mut bus.raw)
        .unwrap();
    assert!(
        bus.relay
            .send_directory(&message(&recipient), &recipient)
            .is_err()
    );
    assert_eq!(
        bus.relay.inbox_len(&recipient.registration.name).unwrap(),
        0
    );
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 24, failure_persistence: None, ..Default::default() })]
    #[test]
    fn redis_directory_refines_the_state_machine_over_generated_lifecycle_traces(events in proptest::collection::vec(0u8..10, 1..48)) {
        use relay_core::directory::{Binding, Directory, Fence, Resolution, Snapshot};
        let Some(mut bus) = Bus::connect("directory-model-trace") else { return Ok(()); };
        let machine = "terra".parse().unwrap();
        let address = "terra:QSL:plan".parse().unwrap();
        let mut model = Directory::<AgentName>::default();
        let mut revisions = Vec::new();
        let mut now = 0;
        let mut held: Option<(relay_core::DirectoryRecipient, Fence)> = None;
        let mut queued = 0;
        for event in events {
            match event {
                0 | 9 => {
                    let revision = bus.relay.begin_directory_observation(&machine).unwrap();
                    revisions.push(revision);
                    model.begin_observation(Fence { epoch: 0, revision });
                    // Event 9 is a failed read: no publication and no lease renewal.
                }
                1..=5 => {
                    let revision = if event == 2 && !revisions.is_empty() {
                        revisions[0]
                    } else {
                        let revision = bus.relay.begin_directory_observation(&machine).unwrap();
                        revisions.push(revision);
                        model.begin_observation(Fence { epoch: 0, revision });
                        revision
                    };
                    let entries = match event {
                        3 => vec![],
                        4 => vec![registration("one", "plan"), registration("two", "plan")],
                        5 => vec![registration("one", "review")],
                        _ => vec![registration("one", "plan")],
                    };
                    let mut bindings = std::collections::BTreeMap::new();
                    for entry in &entries {
                        let address = entry.full_address().unwrap().to_string();
                        let binding = if bindings.contains_key(&address) { Binding::Ambiguous } else { Binding::Unique(entry.name.clone()) };
                        bindings.insert(address, binding);
                    }
                    let expected = model.publish(Snapshot { fence: Fence { epoch: 0, revision }, expires_at: now+1000, bindings }, now);
                    let actual = bus.relay.publish_directory(&machine, revision, &entries).unwrap();
                    proptest::prop_assert_eq!(actual, expected);
                }
                6 => {
                    if let DirectoryRead::Unique(recipient) = bus.relay.directory_recipient(&address).unwrap() {
                        let Resolution::Unique { fence, .. } = model.resolve("terra:QSL:plan", now) else { panic!("model/store disagreement"); };
                        held = Some((*recipient, fence));
                    }
                }
                7 => {
                    if let Some((recipient, fence)) = &held {
                        let expected = model.permits_enqueue("terra:QSL:plan", *fence, now);
                        let actual = bus.relay.send_directory(&message(recipient), recipient).is_ok();
                        proptest::prop_assert_eq!(actual, expected);
                        if actual { queued += 1; }
                    }
                }
                8 => {
                    let _: i64 = redis::cmd("DEL").arg(bus.key("directory:terra")).query(&mut bus.raw).unwrap();
                    now += 1000;
                }
                _ => unreachable!(),
            }
            let actual = bus.relay.directory_recipient(&address).unwrap();
            match model.resolve("terra:QSL:plan", now) {
                Resolution::Unknown | Resolution::Expired => proptest::prop_assert!(matches!(actual, DirectoryRead::Unavailable)),
                Resolution::Absent => proptest::prop_assert!(matches!(actual, DirectoryRead::Absent)),
                Resolution::Ambiguous => proptest::prop_assert!(matches!(actual, DirectoryRead::Ambiguous)),
                Resolution::Unique { session, .. } => {
                    let DirectoryRead::Unique(recipient) = actual else { panic!("unique model owner not resolved"); };
                    proptest::prop_assert_eq!(&recipient.registration.name, session);
                }
            }
        }
        proptest::prop_assert_eq!(bus.relay.log(0, 100).unwrap().len(), queued);
        proptest::prop_assert_eq!(bus.relay.inbox_len(&registration("one", "plan").name).unwrap(), queued);
    }
}
