//! Evidence for escalations: id and ticket grammars (FR-023), the status
//! state machine (FR-024) and escalation text safety (FR-025's store half).
//!
//! The state machine is small, so it is checked *exhaustively*: every
//! sequence of up to five operations from a four-operation alphabet, against
//! the real Redis-backed store, compared with the transition table the
//! requirement states. The grammars are compared with independent matchers.
//!
//! Redis-backed tests skip cleanly without `IX_RELAY_TEST_URL`
//! (`make test-redis` sets it).
#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]

use std::{
    sync::{Arc, Barrier},
    thread,
};

use proptest::prelude::*;
use relay_core::{AgentName, Error, EscalationFiling, EscalationId, EscalationStatus, TicketId};

use relay_test_support::Bus;

fn new_escalation(session_id: &str, context: &str) -> EscalationFiling {
    EscalationFiling {
        ticket: "AGE-2029".parse().expect("valid ticket"),
        lead: "qsl-lead".parse::<AgentName>().expect("valid name"),
        machine: "node-a".parse().expect("valid id"),
        session_id: session_id.parse().expect("valid id"),
        cwd: Some("/work".into()),
        kind: "claude".into(),
        context: context.into(),
        options: vec!["a".into(), "b".into()],
        recommendation: "a".into(),
        created_at_ms: 1,
    }
}

// ---------------------------------------------------------------------------
// FR-023: grammars, against independent matchers
// ---------------------------------------------------------------------------

/// `^e[0-9]{1,20}$`, written out.
fn is_escalation_id(s: &str) -> bool {
    let Some(digits) = s.strip_prefix('e') else {
        return false;
    };
    (1..=20).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
}

/// `^[A-Z]+-[0-9]+$`, written out.
fn is_ticket_id(s: &str) -> bool {
    let Some((prefix, number)) = s.split_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// `^[A-Za-z0-9-]{1,64}$`, written out.
fn is_session_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// Tracing: TC-023-02
    /// ACs: FR-023-AC-2
    ///
    /// An escalation id parses exactly when it matches `^e[0-9]{1,20}$`; every
    /// other text, however close, is refused before a Redis key can be built
    /// from it (a `Relay` method takes an `EscalationId`, so an unparsed id
    /// cannot reach one).
    #[test]
    fn tc_023_02_an_escalation_id_parses_iff_it_matches_the_grammar(
        s in prop_oneof![
            "[eE0-9:* a]{0,24}", "e[0-9]{0,22}", "e[0-9]{1,20}[ :\n]?", ".{0,8}",
            proptest::sample::select(vec!["E7", "e", "seq", "index", "e7:attempts", "e7 ", "*", ""]).prop_map(str::to_owned),
        ],
    ) {
        prop_assert_eq!(s.parse::<EscalationId>().is_ok(), is_escalation_id(&s), "{:?}", s);
    }

    /// Tracing: TC-023-03
    /// ACs: FR-023-AC-3
    #[test]
    fn tc_023_03_a_ticket_id_parses_iff_it_matches_the_grammar(
        s in prop_oneof![
            "[A-Z]{1,5}-[0-9]{1,6}", "[A-Za-z0-9 -]{0,10}", ".{0,8}", "[A-Z]{1,3}-[0-9]{1,3}[-a-z ;]{0,3}",
        ],
    ) {
        prop_assert_eq!(s.parse::<TicketId>().is_ok(), is_ticket_id(&s), "{:?}", s);
    }
}

/// Tracing: TC-023-02
/// ACs: FR-023-AC-2
///
/// The named refusals, and the accepted extremes (one digit; twenty digits;
/// twenty-one refused).
#[test]
fn tc_023_02_the_named_ids() {
    for bad in ["E7", "e", "seq", "index", "e7:attempts", "e7 ", "*", ""] {
        assert!(bad.parse::<EscalationId>().is_err(), "{bad:?}");
    }
    assert!(
        format!("e{}", "9".repeat(21))
            .parse::<EscalationId>()
            .is_err()
    );
    assert!(
        format!("e{}", "9".repeat(20))
            .parse::<EscalationId>()
            .is_ok()
    );
    assert!("e0".parse::<EscalationId>().is_ok());
}

// ---------------------------------------------------------------------------
// FR-023 / FR-025 against the store
// ---------------------------------------------------------------------------

fn cases(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(cases(64))]

    /// Tracing: TC-023-01
    /// ACs: FR-023-AC-1
    ///
    /// N escalations filed on a fresh bus receive exactly `e1` to `eN`, in
    /// order, all distinct, and each id parses as an [`EscalationId`].
    #[test]
    fn tc_023_01_n_escalations_on_a_fresh_bus_get_e1_to_en(n in 1usize..25) {
        let Some(mut bus) = Bus::connect("ids") else { return Ok(()) };
        let mut issued = Vec::new();
        for _ in 0..n {
            issued.push(bus.relay.escalate(new_escalation("sid-1", "ctx")).map_err(|e| TestCaseError::fail(e.to_string()))?.id);
        }
        let want: Vec<String> = (1..=n).map(|i| format!("e{i}")).collect();
        let got: Vec<String> = issued.iter().map(ToString::to_string).collect();
        prop_assert_eq!(got, want);
    }

    /// Tracing: TC-023-04
    /// ACs: FR-023-AC-4
    ///
    /// `escalate` accepts a session id exactly when it is 1 to 64 ASCII
    /// letters, digits and `-`; a refused one writes no key at all.
    #[test]
    fn tc_023_04_a_session_id_is_accepted_iff_it_matches_and_a_refusal_writes_nothing(
        sid in prop_oneof![
            "[A-Za-z0-9-]{1,70}", ".{0,12}", "[a-z;$() /.]{0,12}",
            proptest::sample::select(vec!["sid;rm -rf /", "$(whoami)", "../../etc/passwd", "", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]).prop_map(str::to_owned),
        ],
    ) {
        let Some(mut bus) = Bus::connect("session") else { return Ok(()) };
        // The typed id refuses the shape violations (empty, control characters)
        // at construction, before `escalate` is reachable; the strict rule is a
        // subset of the typed rule, so such an id is never a valid one.
        let Ok(session) = sid.parse() else {
            prop_assert!(!is_session_id(&sid), "{sid:?} is valid but not a SessionId");
            return Ok(());
        };
        let mut filing = new_escalation("sid-1", "ctx");
        filing.session_id = session;
        let got = bus.relay.escalate(filing);
        if is_session_id(&sid) {
            prop_assert!(got.is_ok(), "{sid:?}: {got:?}");
        } else {
            prop_assert!(matches!(got, Err(Error::InvalidSessionId { .. })), "{sid:?}: {got:?}");
            prop_assert_eq!(bus.keys(), Vec::<String>::new(), "a refused escalation wrote keys");
        }
    }

    /// Tracing: TC-025-01, TC-025-04
    /// ACs: FR-025-AC-1, FR-025-AC-4
    ///
    /// An unsafe character (a member of every class of the shared rule, C1
    /// controls included) in the context, any option or the recommendation is
    /// refused naming the field, the code point and the byte offset, and
    /// leaves every key unchanged.
    #[test]
    fn tc_025_01_an_unsafe_character_in_any_text_field_is_refused_and_writes_nothing(
        bad in proptest::sample::select(vec!['\x1b', '\r', '\x00', '\x7f', '\u{85}', '\u{9b}', '\u{2028}', '\u{202e}', '\u{200b}', '\u{e0041}']),
        prefix in "[a-z ]{0,6}", field in 0usize..4,
    ) {
        let Some(mut bus) = Bus::connect("unsafe") else { return Ok(()) };
        let text = format!("{prefix}{bad}tail");
        let mut new = new_escalation("sid-1", "ctx");
        let name = match field {
            0 => { new.context = text; "context" }
            1 => { new.options = vec!["fine".into(), text]; "option" }
            2 => { new.recommendation = text; "recommendation" }
            _ => { new.options = vec![text]; "option" }
        };
        match bus.relay.escalate(new) {
            Err(Error::UnsafeEscalationText { field, codepoint, byte_offset }) => {
                prop_assert_eq!(field, name);
                prop_assert_eq!(codepoint, u32::from(bad));
                prop_assert_eq!(byte_offset, prefix.len());
            }
            other => prop_assert!(false, "expected a refusal, got {other:?}"),
        }
        prop_assert_eq!(bus.keys(), Vec::<String>::new());
    }

    /// Tracing: TC-025-02
    /// ACs: FR-025-AC-2
    ///
    /// Printable text with `\n` and `\t` is filed and read back unchanged.
    #[test]
    fn tc_025_02_printable_text_is_filed_and_read_back_unchanged(
        context in "[a-zA-Z0-9 .,;:!?\n\t\u{e9}\u{200d}]{0,80}",
        option in "[a-zA-Z0-9 .,;:!?\n\t]{0,40}", recommendation in "[a-zA-Z0-9 \n]{0,40}",
    ) {
        let Some(mut bus) = Bus::connect("printable") else { return Ok(()) };
        let mut new = new_escalation("sid-1", &context);
        new.options = vec![option.clone()];
        new.recommendation = recommendation.clone();
        let filed = bus.relay.escalate(new).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let read = bus.relay.get(&filed.id).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&read, &filed);
        prop_assert_eq!(read.filing.context, context);
        prop_assert_eq!(read.filing.options, vec![option]);
        prop_assert_eq!(read.filing.recommendation, recommendation);
    }
}

// ---------------------------------------------------------------------------
// FR-024: the state machine, exhaustively
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Investigate,
    Resolve(&'static str),
}

/// Every event the alphabet holds: investigate, and resolve with two distinct
/// decisions and with the empty decision.
const EVENTS: [Event; 5] = [
    Event::Investigate,
    Event::Resolve("first"),
    Event::Resolve("second"),
    Event::Resolve(""),
    Event::Resolve(" \n\t"),
];

/// The requirement's table (FR-024): what `state` does on `event`, as the
/// next state and decision, or `None` for a refusal.
///
/// waiting -> investigating; investigating -> investigating; waiting ->
/// resolved and investigating -> resolved (each with a non-empty decision);
/// every transition out of resolved is refused.
fn table(
    state: EscalationStatus,
    decision: Option<&str>,
    event: Event,
) -> Result<(EscalationStatus, Option<String>), Refusal> {
    use EscalationStatus::{Investigating, Resolved, Waiting};
    match (state, event) {
        // An empty decision is refused whatever the state: the argument is
        // validated before the status is consulted.
        (_, Event::Resolve(d)) if d.trim().is_empty() => Err(Refusal::EmptyDecision),
        (Resolved, _) => Err(Refusal::AlreadyResolved),
        (Waiting | Investigating, Event::Investigate) => {
            Ok((Investigating, decision.map(str::to_owned)))
        }
        (Waiting | Investigating, Event::Resolve(d)) => Ok((Resolved, Some(d.to_owned()))),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    AlreadyResolved,
    EmptyDecision,
}

/// Every sequence of at most `depth` events.
fn sequences(depth: usize) -> Vec<Vec<Event>> {
    let mut all = vec![vec![]];
    let mut layer: Vec<Vec<Event>> = vec![vec![]];
    for _ in 0..depth {
        layer = layer
            .iter()
            .flat_map(|s| EVENTS.iter().map(move |e| [s.as_slice(), &[*e]].concat()))
            .collect();
        all.extend(layer.iter().cloned());
    }
    all
}

/// Tracing: TC-024-01, TC-024-02
/// ACs: FR-024-AC-1, FR-024-AC-2
///
/// Exhaustive over all 3,906 event sequences of length 0 to 5 from the
/// alphabet {investigate, resolve "first", resolve "second", resolve "", resolve " \n\t"} on a
/// fresh escalation each, against the real Lua transition script. After every
/// event the store's answer and the stored status and decision equal the
/// requirement's table, so: `resolved` is terminal, `waiting` is never
/// re-entered, the decision is non-empty exactly when the status is
/// `resolved`, and it never changes after the first resolve.
///
/// This is a complete check of the machine up to depth 5 (every state and
/// event pair is reached, and the machine has no memory beyond its state and
/// decision); Kani would prove the same for unbounded sequences.
#[test]
fn tc_024_01_every_event_sequence_up_to_depth_five_follows_the_transition_table() {
    let Some(mut bus) = Bus::connect("state-machine") else {
        return;
    };
    for sequence in sequences(5) {
        let filed = bus
            .relay
            .escalate(new_escalation("sid-1", "ctx"))
            .expect("file");
        let mut state = EscalationStatus::Waiting;
        let mut decision: Option<String> = None;
        assert_eq!(filed.status, state);
        assert_eq!(filed.decision, None, "a filed escalation has no decision");
        for (i, event) in sequence.iter().enumerate() {
            let got = match event {
                Event::Investigate => bus.relay.set_investigating(&filed.id),
                Event::Resolve(d) => bus.relay.resolve(&filed.id, d),
            };
            let expected = table(state, decision.as_deref(), *event);
            let context = format!("{sequence:?} at {i}");
            match (expected, got) {
                (Ok((next, next_decision)), Ok(escalation)) => {
                    state = next;
                    decision = next_decision;
                    assert_eq!(escalation.status, state, "{context}");
                    assert_eq!(escalation.decision, decision, "{context}");
                }
                (Err(Refusal::AlreadyResolved), Err(Error::EscalationAlreadyResolved(id))) => {
                    assert_eq!(id, filed.id.to_string(), "{context}");
                }
                (Err(Refusal::EmptyDecision), Err(Error::EmptyDecision { id })) => {
                    assert_eq!(id, filed.id.to_string(), "{context}");
                }
                (expected, got) => panic!("{context}: table says {expected:?}, store says {got:?}"),
            }
            // The stored record agrees with the answer, and obeys AC-2.
            let stored = bus.relay.get(&filed.id).expect("get");
            assert_eq!(stored.status, state, "{context}");
            assert_eq!(stored.decision, decision, "{context}");
            assert_eq!(
                stored.decision.as_deref().is_some_and(|d| !d.is_empty()),
                state == EscalationStatus::Resolved,
                "decision non-empty iff resolved: {context}"
            );
        }
    }
}

/// Tracing: TC-024-04
/// ACs: FR-024-AC-4
///
/// An escalation that does not exist: investigate and resolve report
/// not-found and create no key.
#[test]
fn tc_024_04_an_unknown_id_is_not_found_and_creates_nothing() {
    let Some(mut bus) = Bus::connect("not-found") else {
        return;
    };
    let id: EscalationId = "e424242".parse().expect("valid id");
    assert!(matches!(
        bus.relay.set_investigating(&id),
        Err(Error::EscalationNotFound(_))
    ));
    assert!(matches!(
        bus.relay.resolve(&id, "d"),
        Err(Error::EscalationNotFound(_))
    ));
    assert!(matches!(
        bus.relay.get(&id),
        Err(Error::EscalationNotFound(_))
    ));
    assert_eq!(
        bus.keys(),
        Vec::<String>::new(),
        "a failed lookup created keys"
    );
}

/// Tracing: TC-024-03
/// ACs: FR-024-AC-3
///
/// Two, three and four consumers resolving one escalation at the same instant
/// (a barrier releases them together) with different decisions, over many
/// fresh escalations: exactly one succeeds, the rest get already-resolved, and
/// the stored decision is the winner's.
#[test]
fn tc_024_03_concurrent_resolves_have_exactly_one_winner_and_its_decision_is_stored() {
    let Some(mut bus) = Bus::connect("race") else {
        return;
    };
    for contenders in [2usize, 3, 4] {
        for _ in 0..25 {
            let filed = bus
                .relay
                .escalate(new_escalation("sid-1", "ctx"))
                .expect("file");
            let barrier = Arc::new(Barrier::new(contenders));
            let handles: Vec<_> = (0..contenders)
                .map(|i| {
                    let mut relay = bus.another_connection();
                    let (id, barrier) = (filed.id.clone(), Arc::clone(&barrier));
                    thread::spawn(move || {
                        barrier.wait();
                        (i, relay.resolve(&id, &format!("decision-{i}")))
                    })
                })
                .collect();
            let results: Vec<_> = handles
                .into_iter()
                .map(|h| h.join().expect("thread"))
                .collect();
            let winners: Vec<usize> = results
                .iter()
                .filter(|(_, r)| r.is_ok())
                .map(|(i, _)| *i)
                .collect();
            assert_eq!(winners.len(), 1, "{results:?}");
            for (_, r) in results.iter().filter(|(_, r)| r.is_err()) {
                assert!(
                    matches!(r, Err(Error::EscalationAlreadyResolved(_))),
                    "{r:?}"
                );
            }
            let stored = bus.relay.get(&filed.id).expect("get");
            assert_eq!(stored.decision, Some(format!("decision-{}", winners[0])));
            assert_eq!(stored.status, EscalationStatus::Resolved);
        }
    }
}

/// Tracing: TC-024-06
/// ACs: FR-024-AC-6
///
/// A decision that is empty or only Unicode whitespace (`str::trim`'s notion:
/// including U+00A0 and U+2003) is refused with the empty-decision error, on a
/// waiting, an investigating, a resolved and an unknown escalation alike, and
/// changes nothing: the empty decision is checked before existence and status.
#[test]
fn tc_024_06_an_empty_or_whitespace_decision_is_refused_in_every_state_and_changes_nothing() {
    let Some(mut bus) = Bus::connect("empty-decision") else {
        return;
    };
    let unknown: EscalationId = "e424242".parse().expect("valid id");
    for decision in ["", " ", "\n", "\t \r\n", "\u{a0}", "\u{2003}\u{a0}"] {
        let waiting = bus
            .relay
            .escalate(new_escalation("sid-1", "ctx"))
            .expect("file");
        let investigating = bus
            .relay
            .escalate(new_escalation("sid-1", "ctx"))
            .expect("file");
        bus.relay
            .set_investigating(&investigating.id)
            .expect("investigate");
        let resolved = bus
            .relay
            .escalate(new_escalation("sid-1", "ctx"))
            .expect("file");
        bus.relay.resolve(&resolved.id, "real").expect("resolve");
        let before: Vec<_> = [&waiting.id, &investigating.id, &resolved.id]
            .iter()
            .map(|id| bus.relay.get(id).expect("get"))
            .collect();
        for id in [&waiting.id, &investigating.id, &resolved.id, &unknown] {
            let got = bus.relay.resolve(id, decision);
            assert!(
                matches!(got, Err(Error::EmptyDecision { .. })),
                "{decision:?} on {id}: {got:?}"
            );
        }
        let after: Vec<_> = [&waiting.id, &investigating.id, &resolved.id]
            .iter()
            .map(|id| bus.relay.get(id).expect("get"))
            .collect();
        assert_eq!(before, after, "{decision:?} changed a record");
    }
    assert!(
        matches!(bus.relay.get(&unknown), Err(Error::EscalationNotFound(_))),
        "the unknown id must not have been created"
    );
    // A decision with any non-whitespace character is not refused.
    let filed = bus
        .relay
        .escalate(new_escalation("sid-1", "ctx"))
        .expect("file");
    assert!(bus.relay.resolve(&filed.id, " go ").is_ok());
}
