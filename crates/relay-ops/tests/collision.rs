//! Evidence for name-collision resolution (FR-022) against the real registry.
//!
//! The decision space is small and finite: whether the candidate is held, by
//! whom (same session, another session, no session), on which machine, and
//! whether its pane still hosts it, crossed with the caller's own session, the
//! candidate's length, how many numbered variants are already taken and
//! whether the widened name is taken. Every combination is run against the
//! real Redis-backed registry and compared with the rule stated in the
//! requirement, so this is an exhaustive check of the resolver over that
//! space, not a sample.
//!
//! Skips cleanly without `IX_RELAY_TEST_URL` (`make test-redis` sets it).
#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use relay_core::{
    AgentName, PaneId, Registration, Relay, SessionId,
    naming::{MAX_VARIANT_NUMBER, with_suffix},
};
use relay_ops::naming::{CollisionError, PaneLiveness, ProcessError, resolve_collision};

static CASE: AtomicU64 = AtomicU64::new(0);

/// A relay under a private prefix, and cleanup of that prefix on drop.
struct Bus {
    relay: Relay,
    raw: redis::Connection,
    prefix: String,
}

impl Bus {
    fn connect(test: &str) -> Option<Self> {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping {test}: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return None;
        };
        let prefix = format!(
            "test:relay-ops:collision:{}:{test}:{}:",
            std::process::id(),
            CASE.fetch_add(1, Ordering::Relaxed)
        );
        let relay = Relay::connect(&url, Duration::from_secs(2))
            .expect("connect to test redis")
            .with_prefix(prefix.clone());
        let raw = redis::Client::open(url.as_str())
            .and_then(|c| c.get_connection())
            .expect("second connection to test redis");
        Some(Self { relay, raw, prefix })
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        if let Ok(keys) = redis::cmd("KEYS")
            .arg(format!("{}*", self.prefix))
            .query::<Vec<String>>(&mut self.raw)
            && !keys.is_empty()
        {
            let _ = redis::cmd("DEL").arg(keys).exec(&mut self.raw);
        }
    }
}

/// Answers "does this pane still host this session" from a fixed table.
struct Panes(HashMap<(String, String), bool>);

impl PaneLiveness for Panes {
    fn hosts(&self, pane: &PaneId, session_id: &SessionId) -> Result<bool, ProcessError> {
        Ok(*self
            .0
            .get(&(pane.to_string(), session_id.to_string()))
            .unwrap_or(&false))
    }
}

fn register(
    bus: &mut Bus,
    name: &AgentName,
    machine: &str,
    session: Option<&str>,
    pane: Option<&str>,
) {
    bus.relay
        .register(&Registration {
            name: name.clone(),
            machine: machine.parse().expect("valid id"),
            kind: "claude".into(),
            session_id: session.map(|s| s.parse().expect("valid id")),
            cwd: None,
            herdr_pane: pane.map(|p| p.parse().expect("valid id")),
            place: None,
            last_seen_ms: 1,
        })
        .expect("register");
}

/// How the candidate's current holder looks.
#[derive(Debug, Clone, Copy)]
struct Holder {
    same_machine: bool,
    /// `None`: no session id; `Some(true)`: the caller's own; `Some(false)`: another.
    session: Option<bool>,
    /// `None`: no pane; `Some(true)`: the pane still hosts the session.
    pane: Option<bool>,
}

fn holders() -> Vec<Option<Holder>> {
    let mut all = vec![None];
    for same_machine in [true, false] {
        for session in [None, Some(true), Some(false)] {
            for pane in [None, Some(true), Some(false)] {
                all.push(Some(Holder {
                    same_machine,
                    session,
                    pane,
                }));
            }
        }
    }
    all
}

const ME: &str = "node-a";
const MY_SESSION: &str = "sid-me";

/// Tracing: TC-022-01, TC-022-02, TC-022-03, TC-022-04
/// ACs: FR-022-AC-1, FR-022-AC-2, FR-022-AC-3, FR-022-AC-4
///
/// Exhaustive over 19 holder shapes x 2 caller session states x 2 candidate
/// lengths x 3 taken-variant counts x 2 widened-name states. For every one:
/// the chosen name is a valid bus name of at most 32 bytes (by type and
/// length); it is the candidate exactly when the candidate is free, the
/// caller's own, or held by a session that is gone from a pane on this
/// machine; otherwise it is not registered to any other session, and it is
/// the first such name in the order the requirement gives (the widened name
/// for another machine, then `-2`, `-3`, ...); and the registry is unchanged.
#[test]
fn tc_022_01_every_collision_resolves_to_the_first_free_name_or_the_candidate() {
    if Bus::connect("probe").is_none() {
        return;
    }
    let mut checked = 0;
    for candidate_len in [3usize, 32] {
        let candidate: AgentName = "q".repeat(candidate_len).parse().expect("valid");
        for holder in holders() {
            for caller_session in [None, Some(MY_SESSION)] {
                for taken_variants in [0u32, 1, 3] {
                    for widened_taken in [false, true] {
                        checked += 1;
                        check_one(
                            &candidate,
                            holder,
                            caller_session,
                            taken_variants,
                            widened_taken,
                        );
                    }
                }
            }
        }
    }
    assert_eq!(checked, 2 * 19 * 2 * 3 * 2);
}

fn check_one(
    candidate: &AgentName,
    holder: Option<Holder>,
    caller_session: Option<&str>,
    taken_variants: u32,
    widened_taken: bool,
) {
    // A fresh registry per combination, so registrations never leak between them.
    let mut bus = Bus::connect("case").expect("redis was reachable a moment ago");
    let mut panes = HashMap::new();
    let holder_session = holder.and_then(|h| {
        h.session
            .map(|mine| if mine { MY_SESSION } else { "sid-other" })
    });
    if let Some(h) = holder {
        let machine = if h.same_machine {
            "Node-A.local"
        } else {
            "hub"
        };
        let pane = h.pane.map(|_| "wE:p9");
        register(&mut bus, candidate, machine, holder_session, pane);
        if let (Some(hosts), Some(session)) = (h.pane, holder_session) {
            panes.insert(("wE:p9".to_owned(), session.to_owned()), hosts);
        }
    }
    // Names already held by other sessions: the widened one, and `-2`..
    let widened = with_suffix(candidate, ME);
    if widened_taken {
        register(&mut bus, &widened, "hub", Some("sid-squatter"), None);
    }
    for n in 2..2 + taken_variants {
        register(
            &mut bus,
            &with_suffix(candidate, &n.to_string()),
            "hub",
            Some("sid-squatter"),
            None,
        );
    }
    let before = bus.relay.agents().expect("agents").registrations;

    let resolved = resolve_collision(
        &mut bus.relay,
        &Panes(panes),
        candidate.clone(),
        &ME.parse().expect("valid id"),
        caller_session
            .map(|s| s.parse().expect("valid id"))
            .as_ref(),
    )
    .expect("a free name exists within the bound");
    // The scripted liveness never fails, so nothing is reported on the side.
    assert!(
        resolved.liveness_error.is_none(),
        "unexpected liveness error: {:?}",
        resolved.liveness_error
    );
    let got = resolved.name;

    // The rule, from the requirement: the caller is the holder only when both
    // sides know a session id and the ids are equal.
    let same_caller = |s: Option<&str>| matches!((s, caller_session), (Some(a), Some(b)) if a == b);
    let keeps_candidate = match holder {
        None => true,
        Some(_) if same_caller(holder_session) => true,
        Some(h) => {
            h.same_machine
                && match (h.pane, holder_session) {
                    // Its pane no longer hosts it: the registration is stale.
                    (Some(hosts), Some(_)) => !hosts,
                    // Unknown is live.
                    _ => false,
                }
        }
    };
    let context = format!(
        "candidate of {} bytes, holder {holder:?}, caller {caller_session:?}, \
         taken {taken_variants}, widened taken {widened_taken}",
        candidate.as_str().len()
    );
    if keeps_candidate {
        assert_eq!(&got, candidate, "{context}");
    } else {
        assert_ne!(&got, candidate, "{context}");
        assert!(got.as_str().len() <= 32, "{context}: {got}");
        let widen = holder.is_some_and(|h| !h.same_machine);
        let mut order: Vec<AgentName> = Vec::new();
        if widen {
            order.push(widened.clone());
        }
        order.extend((2..=MAX_VARIANT_NUMBER).map(|n| with_suffix(candidate, &n.to_string())));
        let first_free = order
            .into_iter()
            .find(|name| match bus.relay.agent(name).expect("agent") {
                None => true,
                Some(other) => same_caller(other.session_id.as_ref().map(|v| v.as_str())),
            })
            .expect("a free name exists");
        assert_eq!(got, first_free, "{context}");
        // Never registered to another live session.
        if let Some(existing) = bus.relay.agent(&got).expect("agent") {
            assert!(
                same_caller(existing.session_id.as_ref().map(|v| v.as_str())),
                "{context}: {got} is held by {existing:?}"
            );
        }
        // A full-length candidate held live on this machine, nothing else taken, ends with `-2`.
        if candidate.as_str().len() == 32
            && holder.is_some_and(|h| h.same_machine)
            && taken_variants == 0
        {
            assert!(got.as_str().ends_with("-2"), "{context}: {got}");
        }
    }
    let after = bus.relay.agents().expect("agents").registrations;
    assert_eq!(
        before, after,
        "resolving a collision must not write the registry: {context}"
    );
}

/// Tracing: TC-022-03, TC-022-10
/// ACs: FR-022-AC-3, FR-022-AC-10
///
/// A candidate that itself ends in `-N` and is 31 or 32 bytes long (so the
/// cut for `-2` lands on its own tail): while a live session on this machine
/// holds it, the second lead must get a name ending in `-2`, the first
/// numbered variant, not skip to `-3` because the `-2` name it built was the
/// held candidate, and it keeps the candidate's `qsl-` prefix. Fails on the `with_suffix` that returned its base
/// unchanged (`qsl-` plus 26 `a`s plus `-2` was answered `...-3`).
#[test]
fn tc_022_03_a_candidate_ending_in_a_number_still_gets_the_dash_two_variant() {
    if Bus::connect("probe").is_none() {
        return;
    }
    for (a_count, n) in [(26usize, 2u32), (25, 2), (26, 5), (25, 5)] {
        // A fresh registry per candidate, so the holders never leak between them.
        let mut bus = Bus::connect("ends-in-number").expect("redis was reachable a moment ago");
        let candidate: AgentName = format!("qsl-{}-{n}", "a".repeat(a_count))
            .parse()
            .expect("valid");
        assert!(matches!(candidate.as_str().len(), 31 | 32));
        register(
            &mut bus,
            &candidate,
            "node-a",
            Some("sid-holder"),
            Some("wE:p9"),
        );
        let liveness = Panes(HashMap::from([(
            ("wE:p9".to_owned(), "sid-holder".to_owned()),
            true,
        )]));
        let before = bus.relay.agents().expect("agents").registrations;
        let got = resolve_collision(
            &mut bus.relay,
            &liveness,
            candidate.clone(),
            &"node-a".parse().expect("valid id"),
            Some(&"sid-second".parse().expect("valid id")),
        )
        .expect("a free name exists")
        .name;
        assert_ne!(got, candidate);
        assert!(got.as_str().ends_with("-2"), "{candidate} -> {got}");
        assert_eq!(got, with_suffix(&candidate, "2"));
        // The readable prefix survives: only the byte next to the dash differs.
        assert!(
            got.as_str().starts_with("qsl-aaaaaaaaaaaaaaaaaaaaaaa"),
            "{got}"
        );
        assert_eq!(bus.relay.agents().expect("agents").registrations, before);
    }
}

/// Tracing: TC-022-06
/// ACs: FR-022-AC-6
///
/// The resolver's half of the criterion, which is built: with the candidate
/// and every numbered variant `-2` to `-1000` held by other live sessions,
/// the resolver reports no free name and writes nothing, so the holder's entry
/// is unchanged. (The hook's refusal to register and its stderr report are
/// covered by the hook tests of the collision work, not here.)
#[test]
fn tc_022_06_with_every_variant_taken_the_resolver_reports_no_free_name_and_writes_nothing() {
    let Some(mut bus) = Bus::connect("exhausted") else {
        return;
    };
    let candidate: AgentName = "qsl".parse().expect("valid");
    register(
        &mut bus,
        &candidate,
        "node-a",
        Some("sid-holder"),
        Some("wE:p9"),
    );
    for n in 2..=MAX_VARIANT_NUMBER {
        register(
            &mut bus,
            &with_suffix(&candidate, &n.to_string()),
            "node-a",
            Some(&format!("sid-{n}")),
            None,
        );
    }
    let before = bus.relay.agents().expect("agents").registrations;
    let liveness = Panes(HashMap::from([(
        ("wE:p9".to_owned(), "sid-holder".to_owned()),
        true,
    )]));
    let got = resolve_collision(
        &mut bus.relay,
        &liveness,
        candidate.clone(),
        &"node-a".parse().expect("valid id"),
        Some(&"sid-me".parse().expect("valid id")),
    );
    assert!(
        matches!(got, Err(CollisionError::NoFreeName { candidate: ref c }) if *c == candidate),
        "{got:?}"
    );
    assert_eq!(bus.relay.agents().expect("agents").registrations, before);
}

/// A scripted liveness that cannot be asked.
struct BrokenPanes;

impl PaneLiveness for BrokenPanes {
    fn hosts(&self, _pane: &PaneId, _session_id: &SessionId) -> Result<bool, ProcessError> {
        Err(ProcessError::Timeout {
            program: "herdr".into(),
            secs: 10,
        })
    }
}

/// Tracing: TC-022-07
/// ACs: FR-022-AC-7
///
/// The resolver's half of the criterion: when herdr cannot say whether the
/// same-machine holder's pane still hosts it, the holder is treated as live,
/// so the candidate is not chosen, the next free `-N` name is, the failure is
/// handed back for the caller to record, and the holder's registration is
/// unchanged.
#[test]
fn tc_022_07_a_failing_liveness_call_treats_the_holder_as_live() {
    let Some(mut bus) = Bus::connect("liveness-fails") else {
        return;
    };
    let candidate: AgentName = "qsl".parse().expect("valid");
    register(
        &mut bus,
        &candidate,
        "node-a",
        Some("sid-holder"),
        Some("wE:p9"),
    );
    let before = bus.relay.agents().expect("agents").registrations;
    let resolved = resolve_collision(
        &mut bus.relay,
        &BrokenPanes,
        candidate.clone(),
        &"node-a".parse().expect("valid id"),
        Some(&"sid-me".parse().expect("valid id")),
    )
    .expect("a free name exists");
    assert_eq!(resolved.name.as_str(), "qsl-2");
    assert!(matches!(
        resolved.liveness_error,
        Some(ProcessError::Timeout { .. })
    ));
    assert_eq!(bus.relay.agents().expect("agents").registrations, before);
}

/// Tracing: TC-022-06
/// ACs: FR-022-AC-6
///
/// A record this binary cannot read (an older writer's empty session id, a
/// 300-byte one, a tab, a zero-width character in the pane) still holds its
/// name: a hook whose derived name matches it gets another name and never
/// registers over it (AGE-2027 PR review, finding 1).
#[test]
fn a_name_held_by_an_unreadable_record_is_taken_never_registered_over() {
    let Some(mut bus) = Bus::connect("unreadable-holder") else {
        return;
    };
    let long = format!("\"{}\"", "x".repeat(300));
    for (i, (session, pane)) in [
        ("\"\"", "\"w1:p1\""),
        (long.as_str(), "\"w1:p1\""),
        ("\"a\\tb\"", "\"w1:p1\""),
        ("\"a\\u200bb\"", "\"w1:p1\""),
        ("\"s-ok\"", "\"w1:p\\u200b1\""),
    ]
    .into_iter()
    .enumerate()
    {
        let json = format!(
            r#"{{"name":"qsl","machine":"hub","kind":"claude","session_id":{session},"cwd":null,"herdr_pane":{pane},"place":null,"last_seen_ms":1}}"#
        );
        let key = format!("{}agents", bus.prefix);
        let _: () = redis::cmd("HSET")
            .arg(&key)
            .arg("qsl")
            .arg(&json)
            .query(&mut bus.raw)
            .unwrap();
        let resolved = resolve_collision(
            &mut bus.relay,
            &Panes(HashMap::new()),
            "qsl".parse().unwrap(),
            &"node-a".parse().unwrap(),
            Some(&"sid-me".parse().unwrap()),
        )
        .expect("a free variant exists");
        assert_ne!(
            resolved.name.as_str(),
            "qsl",
            "case {i}: the held name is not reused"
        );
        let stored: String = redis::cmd("HGET")
            .arg(&key)
            .arg("qsl")
            .query(&mut bus.raw)
            .unwrap();
        assert_eq!(stored, json, "case {i}: the record is untouched");
    }
}
