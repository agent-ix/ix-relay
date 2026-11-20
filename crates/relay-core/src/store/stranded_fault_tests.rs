//! The stranded-mail moves under a bus failure at every step, and under a change
//! of a registration in the very window their compare-and-set guards (FR-053).
//! Unit tests because they run a script body by hand, altered to fail where a
//! lost connection or a Redis error would, and use the seams no caller has.
//! Redis-backed (`IX_RELAY_TEST_URL`); skipped with a message without it.
use std::{collections::BTreeMap, time::Duration};

use proptest::prelude::*;

use super::inflight::script;
use crate::{AgentName, Buried, Message, Registration, Relay, store::decode_bytes};

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn relay(test: &str) -> Option<Relay> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping {test}: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    let relay = Relay::connect(&url, Duration::from_secs(5)).expect("connect to test redis");
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Some(relay.with_prefix(format!("test:{}:{test}:{n}:", std::process::id())))
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

fn message(text: &str) -> Message {
    Message {
        from: name("a"),
        to: "session/x".parse().unwrap(),
        text: text.into(),
        sent_at_ms: 1,
        forwarded: false,
    }
}

/// A script the way production sends it, with the one line `at` followed (or
/// replaced) by `error(...)`: the script then fails exactly there, after the writes
/// above it, as a lost connection between two commands would leave it.
fn failing_after(common_and_own: &str, written: &str, line: &str) -> redis::Script {
    let (Some(write), Some(at)) = (common_and_own.find(written), common_and_own.find(line)) else {
        panic!("the script has no {written:?} before a {line:?} to fail at");
    };
    // The write discipline (FR-020): the destination is written before the line
    // that deletes the source, so a failure at that line duplicates and never loses.
    assert!(write < at, "{written:?} must come before {line:?}");
    redis::Script::new(&common_and_own.replace(line, &format!("error('injected')\n{line}")))
}

fn forward_failing_before_the_source_is_trimmed() -> redis::Script {
    failing_after(
        &script(
            include_str!("lua/common.lua"),
            include_str!("lua/forward.lua"),
        ),
        "redis.call('LPUSH', KEYS[3], marked[i])",
        "redis.call('LTRIM', KEYS[2], #items, -1)",
    )
}

fn bury_failing_before_the_source_is_trimmed() -> redis::Script {
    failing_after(
        &script(include_str!("lua/common.lua"), include_str!("lua/bury.lua")),
        "redis.call('RPUSH', KEYS[4], entry)",
        "if #items > 0 then",
    )
}

impl Relay {
    fn lrange(&mut self, key: &str) -> Vec<String> {
        redis::cmd("LRANGE")
            .arg(key)
            .arg(0)
            .arg(-1)
            .query(&mut self.conn)
            .unwrap()
    }

    fn read(&mut self, lead: &str) -> Option<Registration> {
        let raw: Option<Vec<u8>> = redis::cmd("HGET")
            .arg(self.keys.agents_key())
            .arg(lead)
            .query(&mut self.conn)
            .unwrap();
        raw.map(|raw| decode_bytes(&self.keys.agents_key(), &raw).unwrap())
    }
}

/// Tracing: TC-053-07
/// ACs: FR-053-AC-7
///
/// A registration pruned or registered again after the store read it and before the
/// script ran: the script's own compare-and-set refuses, whichever registration
/// changed and whichever move it is.
#[test]
fn tc_053_07_a_registration_changed_in_the_window_between_read_and_move_moves_nothing() {
    let Some(mut r) = relay("stranded-window") else {
        return;
    };
    let (a, b) = (reg("old-lead", "s1", 100), reg("new-lead", "s2", 200));
    for lead in [&a, &b] {
        r.register(lead).unwrap();
    }
    r.requeue_front(&a.name, &[message("one"), message("two")])
        .unwrap();
    let inbox_a = r.keys.inbox_key(&a.name);

    // The holder is registered again in the window.
    let again = reg("new-lead", "s2", 300);
    assert_eq!(
        r.forward_stranded_between(&a, &b, |r| r.register(&again))
            .unwrap(),
        None
    );
    // The replaced one is pruned in the window.
    let b = again;
    assert_eq!(
        r.forward_stranded_between(&a, &b, |r| r.prune(&a.name, 100).map(|_| ()))
            .unwrap(),
        None
    );
    assert_eq!(r.lrange(&inbox_a).len(), 2, "the mail did not move");
    assert!(r.lrange(&r.keys.inbox_key(&b.name)).is_empty());

    // The dead-letter move, with the registration pruned and registered again.
    r.register(&a).unwrap();
    assert_eq!(
        r.dead_letter_stranded_between(&a, |r| r.register(&reg("old-lead", "s1", 101)))
            .unwrap(),
        Buried::Changed
    );
    assert_eq!(r.lrange(&inbox_a).len(), 2);
    assert!(r.lrange(&r.keys.dead_letter_key(&a.name)).is_empty());
    assert!(r.read("old-lead").is_some());
}

/// One step of the model.
#[derive(Debug, Clone)]
enum Step {
    /// A sender writes to the replaced registration's address.
    SendOld,
    /// A sender writes to the new session.
    SendNew,
    /// The new session pops and reads everything queued for it.
    ReadNew,
    /// A pass moves the old inbox to the new one.
    Forward,
    /// A second watcher's pass, with the registrations it read before the first
    /// one's move.
    ForwardStale,
    /// A pass dead-letters the old inbox and prunes the old registration.
    Bury,
    /// A script that fails part-way through the move.
    ForwardFails,
    /// A script that fails part-way through the dead-lettering.
    BuryFails,
    /// The old session registers again (a hook ran).
    RegisterOld,
    /// The old registration is pruned (it was empty).
    PruneOld,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => Just(Step::SendOld),
        2 => Just(Step::SendNew),
        1 => Just(Step::ReadNew),
        3 => Just(Step::Forward),
        1 => Just(Step::ForwardStale),
        2 => Just(Step::Bury),
        1 => Just(Step::ForwardFails),
        1 => Just(Step::BuryFails),
        1 => Just(Step::RegisterOld),
        1 => Just(Step::PruneOld),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: None, ..ProptestConfig::default() })]

    /// Tracing: TC-053-06
    /// ACs: FR-053-AC-6
    ///
    /// Whatever order sends, reads, moves, second passes, prunes and registrations
    /// come in, and whichever script fails part-way, every message that has not been
    /// read is in the old inbox, the new inbox or the dead-letter list at least once,
    /// and exactly once unless a script that failed part-way touched it (then it may
    /// be in two).
    #[test]
    fn tc_053_06_no_message_is_ever_in_none_of_the_inboxes_and_the_dead_letters(
        steps in prop::collection::vec(step(), 1..40),
    ) {
        let Some(mut r) = relay("stranded-model") else { return Ok(()); };
        let (old, new) = (name("old-lead"), name("new-lead"));
        let (mut a, b) = (reg("old-lead", "s1", 100), reg("new-lead", "s2", 200));
        r.register(&a).unwrap();
        r.register(&b).unwrap();
        let mut stale = (a.clone(), b.clone());
        let (inbox_old, inbox_new, letters) = (
            r.keys.inbox_key(&old),
            r.keys.inbox_key(&new),
            r.keys.dead_letter_key(&old),
        );
        // Messages by text: sent, read by the new session, and touched by a failed script.
        let (mut sent, mut read, mut failed): (Vec<String>, Vec<String>, Vec<String>) =
            (Vec::new(), Vec::new(), Vec::new());
        let mut registered = true;
        for step in steps {
            match step {
                Step::SendOld => {
                    let text = format!("m{}", sent.len());
                    // Mail is sent only to a registration that exists: a send to a
                    // pruned one fails with `UnknownAgent` and queues nothing.
                    if registered {
                        r.requeue_front(&old, &[message(&text)]).unwrap();
                        sent.push(text);
                    }
                }
                Step::SendNew => {
                    let text = format!("m{}", sent.len());
                    r.requeue_front(&new, &[message(&text)]).unwrap();
                    sent.push(text);
                }
                Step::ReadNew => {
                    for entry in r.lrange(&inbox_new) {
                        let m: Message = serde_json::from_str(&entry).unwrap();
                        read.push(m.text);
                    }
                    let _: () = redis::cmd("DEL").arg(&inbox_new).query(&mut r.conn).unwrap();
                }
                Step::Forward => {
                    if registered {
                        stale = (a.clone(), b.clone());
                        let _ = r.forward_stranded(&a, &b).unwrap();
                    }
                }
                Step::ForwardStale => {
                    let _ = r.forward_stranded(&stale.0, &stale.1).unwrap();
                }
                Step::Bury => {
                    if r.bury_stranded_for_model(&a) {
                        registered = false;
                    }
                }
                Step::ForwardFails => {
                    if registered {
                        failed.extend(texts_of(&mut r, &inbox_old));
                        let raw_a = raw_record(&mut r, "old-lead");
                        let raw_b = raw_record(&mut r, "new-lead");
                        let outcome: redis::RedisResult<Vec<i64>> =
                            forward_failing_before_the_source_is_trimmed()
                                .key(r.keys.agents_key())
                                .key(&inbox_old)
                                .key(&inbox_new)
                                .arg("old-lead")
                                .arg(raw_a)
                                .arg("new-lead")
                                .arg(raw_b)
                                .invoke(&mut r.conn);
                        // It fails where it should, or finds nothing to move.
                        if let Ok(reply) = outcome {
                            prop_assert_eq!(reply[1], 0);
                        }
                    }
                }
                Step::BuryFails => {
                    if registered {
                        failed.extend(texts_of(&mut r, &inbox_old));
                        let raw_a = raw_record(&mut r, "old-lead");
                        let _: redis::RedisResult<Vec<i64>> = bury_failing_before_the_source_is_trimmed()
                            .key(r.keys.agents_key())
                            .key(&inbox_old)
                            .key(r.keys.inflight_key(&old))
                            .key(&letters)
                            .arg("old-lead")
                            .arg(raw_a)
                            .arg(1000)
                            .invoke(&mut r.conn);
                    }
                }
                Step::RegisterOld => {
                    a = reg("old-lead", "s1", a.last_seen_ms + 1);
                    r.register(&a).unwrap();
                    registered = true;
                }
                Step::PruneOld => {
                    if registered && r.lrange(&inbox_old).is_empty() {
                        r.prune(&old, a.last_seen_ms).unwrap();
                        registered = false;
                    }
                }
            }
            // The invariant, after every step.
            let mut found: BTreeMap<String, usize> = BTreeMap::new();
            for key in [&inbox_old, &inbox_new, &letters] {
                for text in texts_of(&mut r, key) {
                    *found.entry(text).or_default() += 1;
                }
            }
            for text in sent.iter().filter(|t| !read.contains(t)) {
                let n = found.get(text).copied().unwrap_or(0);
                prop_assert!(n >= 1, "{text} is nowhere after {step:?}: {found:?}");
                prop_assert!(
                    n == 1 || failed.contains(text),
                    "{text} is in {n} places without a failed script touching it"
                );
            }
        }
    }
}

fn texts_of(r: &mut Relay, key: &str) -> Vec<String> {
    r.lrange(key)
        .into_iter()
        .map(|entry| serde_json::from_str::<Message>(&entry).unwrap().text)
        .collect()
}

fn raw_record(r: &mut Relay, lead: &str) -> Vec<u8> {
    redis::cmd("HGET")
        .arg(r.keys.agents_key())
        .arg(lead)
        .query(&mut r.conn)
        .unwrap()
}

impl Relay {
    /// The dead-letter move against the registration as the model last registered
    /// it; whether the registration is gone afterwards.
    fn bury_stranded_for_model(&mut self, seen: &Registration) -> bool {
        matches!(self.dead_letter_stranded(seen), Ok(Buried::Done(_)))
    }
}
