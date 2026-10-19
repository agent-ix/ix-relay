//! Mail stranded in a replaced or gone registration's inbox, moved or
//! dead-lettered by the store's two scripts (FR-053), against a real Redis.
//!
//! Needs `IX_RELAY_TEST_URL`; each test returns early and says so on stderr
//! without it (`make test-redis` sets it).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

use std::{num::NonZeroUsize, time::Duration};

use redis::Commands;
use relay_core::{
    AddressBook, AgentName, Buried, Claim, Error, Message, Now, Popper, Registration, Relay,
};
use relay_test_support::Bus;

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn reg(n: &str, session: &str, last_seen_ms: i64) -> Registration {
    Registration {
        name: name(n),
        machine: "hub".parse().unwrap(),
        kind: "claude".into(),
        session_id: Some(session.parse().unwrap()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: None,
        last_seen_ms,
    }
}

fn msg(from: &str, text: &str, at: i64) -> Message {
    Message {
        from: name(from),
        to: "session/x".parse().unwrap(),
        text: text.into(),
        sent_at_ms: at,
        forwarded: false,
    }
}

fn queue(relay: &mut Relay, lead: &str, messages: &[Message]) {
    relay.requeue_front(&name(lead), messages).unwrap();
}

/// The inbox of `lead` as stored, entry by entry.
fn stored(bus: &mut Bus, lead: &str) -> Vec<String> {
    let key = bus.key(&format!("inbox:{lead}"));
    bus.raw.lrange(key, 0, -1).unwrap()
}

fn dead(bus: &mut Bus, lead: &str) -> Vec<String> {
    let key = bus.key(&format!("deadletter:{lead}"));
    bus.raw.lrange(key, 0, -1).unwrap()
}

fn texts(bus: &mut Bus, lead: &str) -> Vec<String> {
    let max = NonZeroUsize::new(100).unwrap();
    bus.relay
        .pop_inbox(&name(lead), max)
        .unwrap()
        .messages
        .into_iter()
        .map(|m| m.text)
        .collect()
}

/// Two registrations, the first replaced and holding `old` messages, the second
/// the one that holds its address and already has `own` unpopped.
fn setup(bus: &mut Bus, old: &[Message], own: &[Message]) -> (Registration, Registration) {
    let (a, b) = (reg("old-lead", "s1", 100), reg("new-lead", "s2", 200));
    bus.relay.register(&a).unwrap();
    bus.relay.register(&b).unwrap();
    queue(&mut bus.relay, "old-lead", old);
    queue(&mut bus.relay, "new-lead", own);
    (a, b)
}

/// Tracing: TC-053-01
/// ACs: FR-053-AC-1
#[test]
fn tc_053_01_queued_mail_moves_in_order_before_the_new_sessions_own() {
    let Some(mut bus) = Bus::connect("stranded-order") else {
        return;
    };
    let old = [msg("a", "one", 1), msg("a", "two", 2), msg("b", "three", 3)];
    let (a, b) = setup(&mut bus, &old, &[msg("c", "own", 4)]);
    let before = stored(&mut bus, "old-lead");

    assert_eq!(bus.relay.forward_stranded(&a, &b).unwrap(), Some(3));

    assert!(
        stored(&mut bus, "old-lead").is_empty(),
        "the old inbox is empty"
    );
    let after = stored(&mut bus, "new-lead");
    assert_eq!(after.len(), 4);
    // The mark is the only change to an entry: its other members are byte for byte
    // what was stored.
    for (moved, original) in after.iter().zip(&before) {
        let marked = format!(
            "{},\"forwarded\":true}}",
            original.strip_suffix('}').unwrap()
        );
        assert_eq!(moved, &marked);
    }
    assert_eq!(
        texts(&mut bus, "new-lead"),
        ["one", "two", "three", "own"],
        "order kept, and before what was queued for the new session"
    );
    assert!(
        bus.relay.agent(&name("old-lead")).unwrap().is_some(),
        "the replaced registration stays"
    );
}

/// Tracing: TC-053-07
/// ACs: FR-053-AC-7
#[test]
fn tc_053_07_a_registration_pruned_or_registered_again_since_the_read_moves_nothing() {
    let Some(mut bus) = Bus::connect("stranded-cas") else {
        return;
    };
    let (a, b) = setup(&mut bus, &[msg("a", "one", 1)], &[]);

    // The holder registered again: its record is no longer what was read.
    bus.relay.register(&reg("new-lead", "s2", 300)).unwrap();
    assert_eq!(bus.relay.forward_stranded(&a, &b).unwrap(), None);
    // The replaced one registered again.
    bus.relay.register(&reg("old-lead", "s1", 150)).unwrap();
    assert_eq!(bus.relay.forward_stranded(&a, &b).unwrap(), None);
    assert_eq!(stored(&mut bus, "old-lead").len(), 1);
    assert!(stored(&mut bus, "new-lead").is_empty());

    // Pruned: a moved-away registration is not brought back, and nothing moves.
    let (a, b) = (bus.relay.agent(&name("old-lead")).unwrap().unwrap(), b);
    bus.relay.prune(&name("new-lead"), 300).unwrap();
    assert_eq!(bus.relay.forward_stranded(&a, &b).unwrap(), None);
    assert_eq!(
        bus.relay
            .dead_letter_stranded(&reg("gone", "s", 1))
            .unwrap(),
        Buried::Changed
    );
    assert_eq!(stored(&mut bus, "old-lead").len(), 1);

    // The same for the dead-letter move: a stale snapshot buries nothing.
    let stale = reg("old-lead", "s1", 100);
    assert_eq!(
        bus.relay.dead_letter_stranded(&stale).unwrap(),
        Buried::Changed
    );
    assert_eq!(stored(&mut bus, "old-lead").len(), 1);
    assert!(dead(&mut bus, "old-lead").is_empty());
    assert!(bus.relay.agent(&name("old-lead")).unwrap().is_some());
}

/// Tracing: TC-053-08
/// ACs: FR-053-AC-8
#[test]
fn tc_053_08_a_message_moved_twice_names_its_first_send_time_and_original_sender() {
    let Some(mut bus) = Bus::connect("stranded-twice") else {
        return;
    };
    let sent = 1_700_000_000_000;
    let (a, b) = setup(&mut bus, &[msg("hub:EA:1", "hello", sent)], &[]);
    let c = reg("third-lead", "s3", 300);
    bus.relay.register(&c).unwrap();
    assert_eq!(bus.relay.forward_stranded(&a, &b).unwrap(), Some(1));
    let b = bus.relay.agent(&name("new-lead")).unwrap().unwrap();
    assert_eq!(bus.relay.forward_stranded(&b, &c).unwrap(), Some(1));

    let entries = stored(&mut bus, "third-lead");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].matches("\"forwarded\":true").count(),
        1,
        "the first mark is kept, not repeated: {entries:?}"
    );
    let moved = bus
        .relay
        .pop_inbox(&name("third-lead"), NonZeroUsize::new(5).unwrap())
        .unwrap()
        .messages;
    assert_eq!(
        AddressBook::default().render(&moved[0]),
        "[ix-relay] from hub:EA:1 (sent 2023-11-14T22:13:20Z to a prior session of this \
         address): hello"
    );
}

/// Tracing: TC-053-10
/// ACs: FR-053-AC-10
#[test]
fn tc_053_10_dead_lettering_keeps_every_entry_byte_for_byte_and_prunes_with_it() {
    let Some(mut bus) = Bus::connect("stranded-bury") else {
        return;
    };
    let (a, _) = setup(
        &mut bus,
        &[msg("a", "one", 1), msg("b", "t\"w\\o\n", 2)],
        &[],
    );
    // A redelivered entry carries its count as trailing spaces, and a plain entry
    // that is not a message at all is mail too: both stay exactly as stored.
    let key = bus.key("inbox:old-lead");
    let () = bus.raw.rpush(&key, "{\"redelivered\":1}  ").unwrap();
    let () = bus.raw.rpush(&key, "not json").unwrap();
    let before = stored(&mut bus, "old-lead");
    assert_eq!(before.len(), 4);

    assert_eq!(bus.relay.dead_letter_stranded(&a).unwrap(), Buried::Done(4));

    assert_eq!(dead(&mut bus, "old-lead"), before);
    assert!(stored(&mut bus, "old-lead").is_empty());
    assert!(
        bus.relay.agent(&name("old-lead")).unwrap().is_none(),
        "pruned"
    );
    assert!(bus.relay.agent(&name("new-lead")).unwrap().is_some());
}

/// Tracing: TC-053-10
/// ACs: FR-053-AC-10
#[test]
fn tc_053_10_a_dead_letter_list_that_cannot_take_the_mail_prunes_nothing() {
    let Some(mut bus) = Bus::connect("stranded-bury-fails") else {
        return;
    };
    let (a, _) = setup(&mut bus, &[msg("a", "one", 1), msg("a", "two", 2)], &[]);
    // The dead-letter key holds the wrong type: the script refuses before it writes.
    let () = bus.raw.set(bus.key("deadletter:old-lead"), "oops").unwrap();

    let err = bus.relay.dead_letter_stranded(&a).unwrap_err();
    assert!(matches!(err, Error::MoveUnreadable { .. }), "{err}");
    assert_eq!(stored(&mut bus, "old-lead").len(), 2, "the mail stays");
    assert!(
        bus.relay.agent(&name("old-lead")).unwrap().is_some(),
        "not pruned"
    );
}

/// Tracing: TC-053-11
/// ACs: FR-053-AC-11
#[test]
fn tc_053_11_a_leased_batch_is_not_moved_until_its_lease_expires() {
    let Some(mut bus) = Bus::connect("stranded-lease") else {
        return;
    };
    let (a, b) = setup(&mut bus, &[msg("a", "one", 1), msg("a", "two", 2)], &[]);
    let claim = Claim::new(Popper::Watcher)
        .with_lease(Duration::from_secs(60))
        .at(Now::AtMs(1_000));
    let popped = bus
        .relay
        .pop_inbox_budget(
            &name("old-lead"),
            NonZeroUsize::new(1).unwrap(),
            usize::MAX,
            claim,
        )
        .unwrap();
    assert_eq!(popped.messages.len(), 1);

    // One message is queued, one is leased: the pass moves the queued one only.
    assert_eq!(bus.relay.forward_stranded(&a, &b).unwrap(), Some(1));
    assert_eq!(bus.relay.inflight_len(&name("old-lead")).unwrap(), 1);
    assert_eq!(bus.relay.inbox_len(&name("new-lead")).unwrap(), 1);

    // The lease runs out: the batch returns to the old inbox, and the next pass moves it.
    let reaped = bus
        .relay
        .reap_expired(&name("old-lead"), Now::AtMs(1_000 + 61_000))
        .unwrap();
    assert_eq!(reaped.requeued, 1);
    assert_eq!(bus.relay.forward_stranded(&a, &b).unwrap(), Some(1));
    assert_eq!(bus.relay.inbox_len(&name("new-lead")).unwrap(), 2);
    assert_eq!(bus.relay.inflight_len(&name("old-lead")).unwrap(), 0);
}

/// Tracing: TC-053-11
/// ACs: FR-053-AC-11
#[test]
fn tc_053_11_a_registration_with_a_batch_in_flight_is_not_pruned_with_its_mail() {
    let Some(mut bus) = Bus::connect("stranded-lease-bury") else {
        return;
    };
    let (a, _) = setup(&mut bus, &[msg("a", "one", 1), msg("a", "two", 2)], &[]);
    let claim = Claim::new(Popper::Watcher)
        .with_lease(Duration::from_secs(60))
        .at(Now::AtMs(1_000));
    bus.relay
        .pop_inbox_budget(
            &name("old-lead"),
            NonZeroUsize::new(1).unwrap(),
            usize::MAX,
            claim,
        )
        .unwrap();

    assert_eq!(
        bus.relay.dead_letter_stranded(&a).unwrap(),
        Buried::InFlight
    );
    assert!(bus.relay.agent(&name("old-lead")).unwrap().is_some());
    assert_eq!(bus.relay.inbox_len(&name("old-lead")).unwrap(), 1);

    bus.relay
        .reap_expired(&name("old-lead"), Now::AtMs(1_000 + 61_000))
        .unwrap();
    assert_eq!(bus.relay.dead_letter_stranded(&a).unwrap(), Buried::Done(2));
}
