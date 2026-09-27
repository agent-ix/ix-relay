//! Property evidence for the address grammar (FR-001), address matching
//! (FR-003), target resolution (FR-002, FR-004), the shown-address rule
//! (FR-005) and pane scoping (FR-009-AC-4's resolver half).
//!
//! Everything here is pure: a registry snapshot goes in, a target or an error
//! comes out, so no test needs Redis, herdr or the environment. The resolver
//! is checked against an independent reference model written from the spec's
//! wording (`model`), not from the implementation.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]
#![expect(clippy::unwrap_used, reason = "fixed valid IDs in test fixtures")]

use std::collections::BTreeSet;

use proptest::prelude::*;
use relay_core::{
    Address, AddressBook, AgentName, Error, HerdrPlaceIds, Place, Registration, Target,
    UnknownCause, duplicated, resolve_target,
};

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Label text weighted towards the characters the grammar cares about:
/// separators, whitespace, control characters and both letter cases.
fn label() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => proptest::sample::select(vec!["EA", "ea", "Code", "code", "plan", "QSL", "1", "2"])
            .prop_map(str::to_owned),
        2 => proptest::sample::select(vec![
            " ", ":", "\n", "\t", "\r", "\x1b", "\u{85}", "\u{200b}", ".", "-", "\u{e9}", "\u{c9}",
        ])
        .prop_map(str::to_owned),
        1 => any::<char>().prop_map(String::from),
    ];
    proptest::collection::vec(piece, 0..5).prop_map(|parts| parts.concat())
}

fn name(s: &str) -> AgentName {
    s.parse().expect("test names are valid")
}

/// A registry over a small alphabet, so that collisions (same pane id, same
/// address in different case, same workspace on two servers) are common.
#[derive(Debug, Clone)]
struct Fleet {
    regs: Vec<Registration>,
    aliases: Vec<(AgentName, AgentName)>,
}

fn fleet() -> impl Strategy<Value = Fleet> {
    let machine =
        proptest::sample::select(vec!["hub", "Hub.local", "node-a", "NODE-A", "\u{c9}cole"]);
    let pane = proptest::option::of(proptest::sample::select(vec!["wB:p1", "wB:p2", "wC:p1"]));
    let workspace = proptest::sample::select(vec!["EA", "ea", "research", "My Work"]);
    let tab = proptest::sample::select(vec!["code", "Code", "plan", "1"]);
    let place = proptest::option::of((workspace, tab, 0u32..4));
    let one = (machine, pane, place);
    (
        proptest::collection::vec(one, 0..8),
        proptest::collection::vec((0usize..10, 0usize..10), 0..4),
    )
        .prop_map(|(ones, alias_ix)| {
            let regs: Vec<Registration> = ones
                .into_iter()
                .enumerate()
                .map(|(i, (machine, pane, place))| Registration {
                    name: name(&format!("lead-{i}")),
                    machine: machine.parse().expect("valid id"),
                    kind: "claude".to_owned(),
                    session_id: Some("sid-test".parse().unwrap()),
                    cwd: None,
                    herdr_pane: pane.map(|p| p.parse().expect("valid id")),
                    place: place.map(|(workspace, tab, _tabs)| Place {
                        workspace: workspace.to_owned(),
                        tab: tab.to_owned(),
                        herdr_ids: Some(HerdrPlaceIds {
                            workspace: format!("workspace-{workspace}").parse().expect("valid id"),
                            tab: format!("workspace-{workspace}:t1")
                                .parse()
                                .expect("valid id"),
                            pane: pane.unwrap_or("wB:p1").parse().unwrap(),
                            session_id: "sid-test".parse().unwrap(),
                            verified_at_ms: 1,
                        }),
                    }),
                    last_seen_ms: 1,
                })
                .collect();
            // Aliases never share a name with a registration (the store keeps
            // the two disjoint, FR-006-AC-4), and point at lead-N which may or
            // may not be registered.
            let aliases = alias_ix
                .into_iter()
                .enumerate()
                .map(|(i, (_, target))| {
                    (name(&format!("old-{i}")), name(&format!("lead-{target}")))
                })
                .collect();
            Fleet { regs, aliases }
        })
}

/// Text a sender might type against `fleet`: every name, alias, pane id and
/// address it holds (in mixed case, with and without the tab), plus noise.
fn typed_input(fleet: &Fleet) -> BoxedStrategy<String> {
    let mut pool: Vec<String> = vec![
        "all".into(),
        "ALL".into(),
        "".into(),
        ":".into(),
        "hub".into(),
        "hub:EA".into(),
        "node-b:EA:code".into(),
        "hub:EAA:code".into(),
        "hub:EA:nope".into(),
        "lead-99".into(),
        "wZ:p9".into(),
        "\u{e9}cole:nope:code".into(),
    ];
    for r in &fleet.regs {
        pool.push(r.name.to_string());
        pool.push(r.name.to_string().to_uppercase());
        if let Some(pane) = &r.herdr_pane {
            pool.push(pane.to_string());
        }
        if let Some(full) = r.full_address() {
            pool.push(full.to_string());
            pool.push(full.to_string().to_uppercase());
            pool.push(full.to_string().to_lowercase());
            pool.push(format!("{}:{}", full.server(), full.workspace()));
        }
    }
    for (alias, _) in &fleet.aliases {
        pool.push(alias.to_string());
    }
    prop_oneof![
        5 => proptest::sample::select(pool),
        1 => "[a-zA-Z0-9:. \\-]{0,14}",
        1 => label(),
    ]
    .boxed()
}

fn machine_of(fleet: &Fleet) -> BoxedStrategy<String> {
    let mut machines: Vec<String> = fleet.regs.iter().map(|r| r.machine.to_string()).collect();
    machines.extend(["hub".into(), "node-a".into(), "node-b".into()]);
    proptest::sample::select(machines).boxed()
}

// ---------------------------------------------------------------------------
// The reference model
// ---------------------------------------------------------------------------

/// What the resolver decided, with the error payloads the spec names.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Broadcast,
    One(String),
    Ambiguous(Vec<String>),
    NotRegistered,
    UnknownServer(BTreeSet<String>),
    UnknownWorkspace(BTreeSet<String>),
    UnknownTab(BTreeSet<String>),
    Invalid,
}

fn observed(result: &Result<Target, Error>) -> Outcome {
    match result {
        Ok(Target::All) => Outcome::Broadcast,
        Ok(Target::Agent(n)) => Outcome::One(n.to_string()),
        Err(Error::AmbiguousTarget { candidates, .. }) => {
            Outcome::Ambiguous(candidates.iter().map(|c| c.name.to_string()).collect())
        }
        Err(Error::UnknownTarget { cause, .. }) => match cause {
            UnknownCause::NotRegistered => Outcome::NotRegistered,
            UnknownCause::UnknownServer { known, .. } => {
                Outcome::UnknownServer(known.iter().cloned().collect())
            }
            UnknownCause::UnknownWorkspace { available, .. } => {
                Outcome::UnknownWorkspace(available.iter().cloned().collect())
            }
            UnknownCause::UnknownTab { available, .. } => {
                Outcome::UnknownTab(available.iter().cloned().collect())
            }
        },
        Err(Error::InvalidAddress { .. }) => Outcome::Invalid,
        Err(other) => panic!("resolution returned an error outside its contract: {other}"),
    }
}

/// FR-001's cleaning, from the spec text: whitespace, control characters and
/// `:` become `-`; an empty segment is `-`.
fn m_clean(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() || c == ':' {
                '-'
            } else {
                c
            }
        })
        .collect();
    if cleaned.is_empty() {
        "-".to_owned()
    } else {
        cleaned
    }
}

/// FR-001's server normalisation: lowercased (ASCII), cut at the first `.`.
fn m_server(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    m_clean(lower.split('.').next().unwrap_or(""))
}

fn m_eq(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

struct Placed {
    name: String,
    server: String,
    workspace: String,
    tab: String,
    pane: String,
}

fn placed(regs: &[Registration]) -> Vec<Placed> {
    regs.iter()
        .filter_map(|r| {
            let p = r.verified_place()?;
            let pane = r.herdr_pane.as_ref()?;
            Some(Placed {
                name: r.name.to_string(),
                server: m_server(r.machine.as_str()),
                workspace: m_clean(&p.workspace),
                tab: m_clean(&p.tab),
                pane: m_clean(pane.as_str()),
            })
        })
        .collect()
}

/// FR-002's order and FR-003/FR-004's selection and error rules, written as
/// plainly as the spec states them.
fn model(input: &str, sender: &str, fleet: &Fleet) -> Outcome {
    if input == "all" {
        return Outcome::Broadcast;
    }
    let registered = |n: &AgentName| fleet.regs.iter().any(|r| r.name == *n);
    if let Ok(n) = input.parse::<AgentName>() {
        // Exactly one alias hop.
        let canonical = fleet
            .aliases
            .iter()
            .find(|(alias, _)| *alias == n)
            .map_or(n, |(_, canonical)| canonical.clone());
        if registered(&canonical) {
            return Outcome::One(canonical.to_string());
        }
    }
    let sender = m_server(sender);
    let by_pane: Vec<&Registration> = fleet
        .regs
        .iter()
        .filter(|r| {
            r.verified_place().is_some()
                && r.herdr_pane.as_ref().map(|v| v.as_str()) == Some(input)
                && m_server(r.machine.as_str()) == sender
        })
        .collect();
    match by_pane.as_slice() {
        [] => {}
        [only] => return Outcome::One(only.name.to_string()),
        many => {
            let mut names: Vec<String> = many.iter().map(|r| r.name.to_string()).collect();
            names.sort();
            return Outcome::Ambiguous(names);
        }
    }
    if !input.contains(':') {
        return Outcome::NotRegistered;
    }
    let parts: Vec<&str> = input.split(':').collect();
    let (server, workspace, tab, pane) = match parts.as_slice() {
        [s, w] if !s.is_empty() && !w.is_empty() => (m_server(s), m_clean(w), None, None),
        [s, w, t] if !s.is_empty() && !w.is_empty() && !t.is_empty() => {
            (m_server(s), m_clean(w), Some(m_clean(t)), None)
        }
        [s, w, t, p] if !s.is_empty() && !w.is_empty() && !t.is_empty() && !p.is_empty() => {
            (m_server(s), m_clean(w), Some(m_clean(t)), Some(m_clean(p)))
        }
        _ => return Outcome::Invalid,
    };
    let leads = placed(&fleet.regs);
    let selected: Vec<&Placed> = leads
        .iter()
        .filter(|l| {
            m_eq(&l.server, &server)
                && m_eq(&l.workspace, &workspace)
                && tab.as_ref().is_none_or(|t| m_eq(&l.tab, t))
                && pane.as_ref().is_none_or(|p| m_eq(&l.pane, p))
        })
        .collect();
    match selected.as_slice() {
        [only] => return Outcome::One(only.name.clone()),
        [] => {}
        many => {
            let mut names: Vec<String> = many.iter().map(|l| l.name.clone()).collect();
            names.sort();
            return Outcome::Ambiguous(names);
        }
    }
    // Nothing matched: the most specific cause.
    let on_server = |r: &Registration| m_eq(&m_server(r.machine.as_str()), &server);
    if !fleet.regs.iter().any(on_server) {
        return Outcome::UnknownServer(
            fleet
                .regs
                .iter()
                .map(|r| m_server(r.machine.as_str()))
                .collect(),
        );
    }
    let here: Vec<&Placed> = leads.iter().filter(|l| m_eq(&l.server, &server)).collect();
    let in_workspace: Vec<&&Placed> = here
        .iter()
        .filter(|l| m_eq(&l.workspace, &workspace))
        .collect();
    if in_workspace.is_empty() {
        return Outcome::UnknownWorkspace(here.iter().map(|l| l.workspace.clone()).collect());
    }
    Outcome::UnknownTab(in_workspace.iter().map(|l| l.tab.clone()).collect())
}

// ---------------------------------------------------------------------------
// FR-001: address grammar and cleaning
// ---------------------------------------------------------------------------

/// Whether `s` is one of the two forms FR-001 accepts, stated independently.
fn is_address_text(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    matches!(parts.len(), 2..=4) && parts.iter().all(|p| !p.is_empty())
}

/// The property FR-001-AC-1 states over every input, shared by the always-run
/// tests and the `fuzz_address_parse_*` lanes at the end of this file.
fn check_parse_accepts_exactly_the_grammar(s: &str) -> Result<(), TestCaseError> {
    match s.parse::<Address>() {
        Ok(a) => {
            prop_assert!(
                is_address_text(s),
                "accepted {:?} which is not the grammar",
                s
            );
            // Accepted text prints as three or two well-formed segments.
            prop_assert!(a.to_string().split(':').count() == parts_of(s));
        }
        Err(Error::InvalidAddress { input }) => {
            prop_assert!(!is_address_text(s), "refused {:?} which is the grammar", s);
            prop_assert_eq!(input, s, "the error must carry the input verbatim");
        }
        Err(other) => prop_assert!(false, "wrong error kind: {other}"),
    }
    Ok(())
}

fn parts_of(s: &str) -> usize {
    s.split(':').count()
}

/// Tracing: TC-001-01
/// ACs: FR-001-AC-1
///
/// The named malformed forms, then the same predicate over generated text.
#[test]
fn tc_001_01_the_named_malformed_forms_are_refused_verbatim() {
    for bad in [
        "",
        "hub",
        "hub:",
        ":EA",
        "hub::1",
        "hub:EA:",
        "hub:EA:code:",
        "a:b:c:d:e",
    ] {
        let err = bad.parse::<Address>().expect_err(bad);
        assert!(
            matches!(err, Error::InvalidAddress { ref input } if input == bad),
            "{bad:?}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// Tracing: TC-001-01
    /// ACs: FR-001-AC-1
    #[test]
    fn tc_001_01_parse_accepts_two_to_four_nonempty_segments(
        s in prop_oneof![label(), "[a-c:]{0,8}", ".{0,20}"],
    ) {
        check_parse_accepts_exactly_the_grammar(&s)?;
    }

    /// Tracing: TC-001-02
    /// ACs: FR-001-AC-2
    #[test]
    fn tc_001_02_a_built_address_parses_back_from_its_printed_form(
        server in label(), workspace in label(), tab in proptest::option::of(label()), pane in proptest::option::of(label()),
    ) {
        let built = match (tab.as_deref(), pane.as_deref()) {
            (Some(tab), Some(pane)) => Address::with_pane(&server, &workspace, tab, pane),
            (tab, _) => Address::new(&server, &workspace, tab),
        };
        let printed = built.to_string();
        let reparsed: Address = printed.parse().map_err(|e| TestCaseError::fail(format!("{printed:?}: {e}")))?;
        prop_assert_eq!(&reparsed, &built);
        prop_assert_eq!(reparsed.to_string(), printed);
    }

    /// Tracing: TC-001-03
    /// ACs: FR-001-AC-3
    #[test]
    fn tc_001_03_no_printed_segment_is_empty_or_holds_whitespace_control_or_colon(
        server in label(), workspace in label(), tab in proptest::option::of(label()),
    ) {
        let built = Address::new(&server, &workspace, tab.as_deref());
        let segments: Vec<&str> = std::iter::once(built.server())
            .chain(std::iter::once(built.workspace()))
            .chain(built.tab())
            .collect();
        for segment in segments {
            prop_assert!(!segment.is_empty());
            prop_assert!(
                !segment.chars().any(|c| c.is_whitespace() || c.is_control() || c == ':'),
                "segment {segment:?} of {built}"
            );
        }
        // The printed form has exactly the segments the address has.
        prop_assert_eq!(built.to_string().split(':').count(), 2 + usize::from(built.tab().is_some()));
    }
}

/// Tracing: TC-001-04
/// ACs: FR-001-AC-4
#[test]
fn tc_001_04_the_server_segment_is_normalised_like_a_machine_name() {
    assert_eq!(Address::new("Hub.local", "EA", None).server(), "hub");
}

/// Tracing: TC-001-05
/// ACs: FR-001-AC-5
#[test]
fn tc_001_05_labels_are_cleaned_in_the_printed_address() {
    let a = Address::new("hub", "My Work\nspace", Some("a:b"));
    assert_eq!(a.to_string(), "hub:My-Work-space:a-b");
}

// ---------------------------------------------------------------------------
// FR-002 / FR-003 / FR-004: resolution against the reference model
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// Tracing: TC-002-01
    /// ACs: FR-002-AC-1
    #[test]
    fn tc_002_01_all_is_the_broadcast_target_for_any_registry(
        f in fleet(), sender in "[a-z][a-z.]{0,5}",
    ) {
        let got = resolve_target("all", &sender.parse().expect("valid id"), &f.regs, &f.aliases);
        prop_assert!(matches!(got, Ok(Target::All)), "{got:?}");
    }

    /// Tracing: TC-002-02
    /// ACs: FR-002-AC-2
    #[test]
    fn tc_002_02_every_registered_name_resolves_to_itself(f in fleet(), sender in "[a-z][a-z.]{0,5}") {
        for r in &f.regs {
            let got = resolve_target(r.name.as_str(), &sender.parse().expect("valid id"), &f.regs, &f.aliases);
            prop_assert_eq!(got.unwrap(), Target::Agent(r.name.clone()));
        }
    }

    /// Tracing: TC-002-03
    /// ACs: FR-002-AC-3, FR-002-AC-4
    ///
    /// An alias follows exactly one hop: to its canonical name when that is
    /// registered, and otherwise it is an error, never a second hop.
    #[test]
    fn tc_002_03_an_alias_resolves_one_hop_to_a_registered_canonical_name(
        f in fleet(), sender in "[a-z][a-z.]{0,5}",
    ) {
        for (alias, canonical) in &f.aliases {
            let got = resolve_target(alias.as_str(), &sender.parse().expect("valid id"), &f.regs, &f.aliases);
            if f.regs.iter().any(|r| r.name == *canonical) {
                prop_assert_eq!(got.unwrap(), Target::Agent(canonical.clone()));
            } else {
                prop_assert!(got.is_err(), "{alias} -> {canonical} is not registered: {got:?}");
            }
        }
    }

    /// Tracing: TC-002-06
    /// ACs: FR-002-AC-6
    #[test]
    fn tc_002_06_colonless_unregistered_text_is_not_registered(
        f in fleet(), sender in "[a-z][a-z.]{0,5}", text in "[a-z0-9_-]{1,10}",
    ) {
        let known = f.regs.iter().any(|r| r.name.as_str() == text)
            || f.aliases.iter().any(|(a, c)| a.as_str() == text && f.regs.iter().any(|r| r.name == *c));
        prop_assume!(text != "all" && !known);
        let got = resolve_target(&text, &sender.parse().expect("valid id"), &f.regs, &f.aliases);
        prop_assert!(
            matches!(got, Err(Error::UnknownTarget { cause: UnknownCause::NotRegistered, .. })),
            "{got:?}"
        );
    }

    /// Tracing: TC-002-01, TC-002-02, TC-002-03, TC-002-05, TC-002-06, TC-002-08
    /// ACs: FR-002-AC-1, FR-002-AC-2, FR-002-AC-3, FR-002-AC-5, FR-002-AC-6, FR-002-AC-8
    ///
    /// Priority order: for any registry, sender and typed text the resolver
    /// agrees with the reference model, which tries `all`, name or alias, pane
    /// id on the sender's machine, then address, and stops at the first form
    /// that matches (FR-002), including which error and which candidates.
    #[test]
    fn tc_002_05_resolution_matches_the_priority_ordered_reference_model(
        (f, sender, input) in fleet().prop_flat_map(|f| {
            let sender = machine_of(&f);
            let input = typed_input(&f);
            (Just(f), sender, input)
        }),
    ) {
        let got = observed(&resolve_target(&input, &sender.parse().expect("valid id"), &f.regs, &f.aliases));
        prop_assert_eq!(got, model(&input, &sender, &f), "input {:?} sender {:?}", input, sender);
    }

    /// Tracing: TC-004-01, TC-004-03
    /// ACs: FR-004-AC-1, FR-004-AC-3
    ///
    /// A single-lead result is the only lead the first matching form selects;
    /// an ambiguous result lists every match, sorted by name, at least two,
    /// and none twice.
    #[test]
    fn tc_004_01_a_single_lead_is_the_only_match_and_an_ambiguity_lists_all(
        (f, sender, input) in fleet().prop_flat_map(|f| {
            let sender = machine_of(&f);
            let input = typed_input(&f);
            (Just(f), sender, input)
        }),
    ) {
        let want = model(&input, &sender, &f);
        let got = resolve_target(&input, &sender.parse().expect("valid id"), &f.regs, &f.aliases);
        // Unconditionally: a single lead only where the model finds exactly
        // one match, an ambiguity exactly where it finds several, and no
        // single lead otherwise (a first-of-several guess fails here).
        match (&want, &got) {
            (Outcome::One(n), Ok(Target::Agent(g))) => prop_assert_eq!(n, g.as_str()),
            (Outcome::One(_) | Outcome::Ambiguous(_), _) if !matches!((&want, &got), (Outcome::Ambiguous(_), Err(Error::AmbiguousTarget { .. }))) => {
                return Err(TestCaseError::fail(format!("model {want:?}, resolver {got:?}")));
            }
            (_, Ok(Target::Agent(g))) => {
                return Err(TestCaseError::fail(format!("model {want:?}, but resolved to {g}")));
            }
            _ => {}
        }
        match got {
            Err(Error::AmbiguousTarget { input: echoed, candidates }) => {
                prop_assert_eq!(echoed, input);
                prop_assert!(candidates.len() >= 2);
                let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
                let mut sorted = names.clone();
                sorted.sort_unstable();
                sorted.dedup();
                prop_assert_eq!(&names, &sorted, "sorted by bus name, no duplicates");
                for c in &candidates {
                    let reg = f.regs.iter().find(|r| r.name == c.name).unwrap();
                    prop_assert_eq!(&c.address, &reg.full_address());
                }
            }
            Ok(Target::Agent(n)) => prop_assert!(f.regs.iter().any(|r| r.name == n)),
            _ => {}
        }
    }

    /// Tracing: TC-003-05, TC-003-06
    /// ACs: FR-003-AC-5, FR-003-AC-6
    ///
    /// Whatever address is typed, a lead it resolves to is on that address's
    /// server and has a recorded place; a lead with no place is never
    /// selected by an address.
    #[test]
    fn tc_003_05_an_address_never_selects_another_server_or_a_lead_without_a_place(
        f in fleet(), ws in label(), tab in proptest::option::of(label()),
        server in proptest::sample::select(vec!["hub", "HUB", "node-a", "Hub.local", "node-b", "\u{e9}cole"]),
    ) {
        let query = Address::new(server, &ws, tab.as_deref());
        // A sender machine no pane is registered on, so only the address form applies.
        let got = resolve_target(&query.to_string(), &"nowhere".parse().expect("valid id"), &f.regs, &[]);
        let candidates: Vec<AgentName> = match got {
            Ok(Target::Agent(n)) => vec![n],
            Err(Error::AmbiguousTarget { candidates, .. }) => candidates.into_iter().map(|c| c.name).collect(),
            _ => vec![],
        };
        for n in candidates {
            let reg = f.regs.iter().find(|r| r.name == n).unwrap();
            let full = reg.full_address();
            prop_assert!(full.is_some(), "{n} has no place yet was selected");
            prop_assert!(query.selects(&full.unwrap()));
            prop_assert!(
                Address::new(reg.machine.as_str(), "-", None).server().to_lowercase() == query.server().to_lowercase(),
                "{n} is on {} not {}", reg.machine, query.server()
            );
        }
    }

    /// Tracing: TC-004-09
    /// ACs: FR-004-AC-9
    #[test]
    fn tc_004_09_an_unknown_target_has_at_most_five_suggestions_from_the_registry(
        (f, sender, input) in fleet().prop_flat_map(|f| {
            let sender = machine_of(&f);
            let input = typed_input(&f);
            (Just(f), sender, input)
        }),
    ) {
        let want = model(&input, &sender, &f);
        let unknown = matches!(
            want,
            Outcome::NotRegistered | Outcome::UnknownServer(_) | Outcome::UnknownWorkspace(_) | Outcome::UnknownTab(_)
        );
        let got = resolve_target(&input, &sender.parse().expect("valid id"), &f.regs, &f.aliases);
        // Unconditionally: an unknown target is an unknown-target error, and
        // only then are suggestions checked.
        prop_assert_eq!(unknown, matches!(got, Err(Error::UnknownTarget { .. })), "model {:?}, resolver {:?}", want, got);
        if let Err(Error::UnknownTarget { suggestions, .. }) = got {
            prop_assert!(suggestions.len() <= 5, "{suggestions:?}");
            let mut pool: BTreeSet<String> = f.regs.iter().map(|r| r.name.to_string()).collect();
            pool.extend(f.aliases.iter().map(|(a, _)| a.to_string()));
            let book = AddressBook::from_registrations(&f.regs);
            pool.extend(f.regs.iter().filter_map(|r| book.place_address(&r.name)).map(|a| a.to_string()));
            for s in &suggestions {
                prop_assert!(pool.contains(s), "{s:?} is not a registered name, alias or full address");
            }
            let unique: BTreeSet<&String> = suggestions.iter().collect();
            prop_assert_eq!(unique.len(), suggestions.len());
        }
    }

    /// Tracing: TC-004-09
    /// ACs: FR-004-AC-9
    ///
    /// Resolution is total: no registry, sender or text, however hostile,
    /// makes it panic or return an error outside its contract (`observed`
    /// panics on any other error kind).
    #[test]
    fn tc_004_09_resolution_is_total_over_arbitrary_text(
        f in fleet(), sender in "[a-z][a-z.]{0,5}", input in ".{0,40}",
    ) {
        let _ = observed(&resolve_target(&input, &sender.parse().expect("valid id"), &f.regs, &f.aliases));
    }
}

/// Tracing: TC-003-04
/// ACs: FR-003-AC-4
///
/// Workspace `EA` on both machines with one lead each: an address never
/// crosses servers (also asserted over generated fleets by TC-003-05).
#[test]
fn tc_003_04_the_same_workspace_on_two_servers_never_crosses_servers() {
    let mk = |n: &str, machine: &str| Registration {
        name: name(n),
        machine: machine.parse().expect("valid id"),
        kind: "claude".into(),
        session_id: Some("sid-test".parse().unwrap()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: Some(Place {
            workspace: "EA".into(),
            tab: "code".into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: if machine == "hub" { "wA" } else { "wB" }.parse().unwrap(),
                tab: if machine == "hub" { "wA:t1" } else { "wB:t1" }
                    .parse()
                    .unwrap(),
                pane: "w1:p1".parse().unwrap(),
                session_id: "sid-test".parse().unwrap(),
                verified_at_ms: 1,
            }),
        }),
        last_seen_ms: 1,
    };
    let regs = [mk("on-hub", "hub"), mk("on-node-a", "node-a")];
    assert_eq!(
        resolve_target("node-a:EA", &"hub".parse().expect("valid id"), &regs, &[]).unwrap(),
        Target::Agent(name("on-node-a"))
    );
    assert_eq!(
        resolve_target("hub:EA", &"node-a".parse().expect("valid id"), &regs, &[]).unwrap(),
        Target::Agent(name("on-hub"))
    );
}

// ---------------------------------------------------------------------------
// FR-005: the shown-address rule
// ---------------------------------------------------------------------------

/// The address FR-005 says lead `i` is printed with: none without a verified
/// place; always include the tab; none when it is longer than the label cap.
fn model_shown(regs: &[Registration], i: usize) -> Option<Address> {
    let r = &regs[i];
    let place = r.verified_place()?;
    let tab_address = place.tab_address(&r.machine);
    let shared_tab = regs.iter().enumerate().any(|(j, other)| {
        j != i
            && other
                .verified_place()
                .is_some_and(|p| p.tab_address(&other.machine) == tab_address)
    });
    let shown = if shared_tab {
        r.full_address()?
    } else {
        tab_address
    };
    (shown.to_string().len() <= relay_core::MAX_LABEL_BYTES).then_some(shown)
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// Tracing: TC-005-02, TC-005-06
    /// ACs: FR-005-AC-2, FR-005-AC-6
    ///
    /// The full address always includes a tab, independent of cached tab count.
    #[test]
    fn tc_005_02_the_tab_is_always_included(ws in label(), tab in label()) {
        let place = Place { workspace: ws.clone(), tab: tab.clone(), herdr_ids: Some(HerdrPlaceIds { workspace: "wA".parse().unwrap(), tab: "wA:t1".parse().unwrap(), pane: "w1:p1".parse().unwrap(), session_id: "sid-test".parse().unwrap(), verified_at_ms: 1 }) };
        let shown = place.address(&"hub".parse().expect("valid id"));
        prop_assert_eq!(&shown, &Address::new("hub", &ws, Some(&tab)));
        // The full address always carries the tab.
        prop_assert!(place.address(&"hub".parse().expect("valid id")).tab().is_some());
    }

    /// Tracing: TC-005-01, TC-005-03
    /// ACs: FR-005-AC-1, FR-005-AC-3
    ///
    /// One rule for every command: a lead is labelled with its shown address
    /// (the tab dropped only for a lone lead in a one-tab workspace, see
    /// `model_shown`) exactly when it has one and no other lead has an equal
    /// one, and with its bus name otherwise; the label is the same on every
    /// call and in `describe`, `render` and `label_target`.
    #[test]
    fn tc_005_03_the_label_is_the_address_unless_shared_or_unplaced(f in fleet()) {
        let book = AddressBook::from_registrations(&f.regs);
        let shown: Vec<Option<Address>> = (0..f.regs.len()).map(|i| model_shown(&f.regs, i)).collect();
        for (i, r) in f.regs.iter().enumerate() {
            let shared = shown[i].as_ref().is_some_and(|a| {
                shown.iter().enumerate().any(|(j, b)| j != i && b.as_ref() == Some(a))
            });
            let label = book.label(&r.name);
            match (&shown[i], shared) {
                (Some(address), false) => prop_assert_eq!(&label, &address.to_string()),
                _ => prop_assert_eq!(&label, r.name.as_str()),
            }
            prop_assert_eq!(book.label(&r.name), label.clone(), "stable across calls");
            prop_assert_eq!(book.label_target(&Target::Agent(r.name.clone())), label.clone());
            let described = book.describe(&r.name);
            prop_assert!(described.starts_with(&label));
            let signed = book.render(&relay_core::Message {
                from: r.name.clone(),
                to: Target::All,
                text: "hi".into(),
                sent_at_ms: 0,
            });
            prop_assert_eq!(signed, format!("[ix-relay] from {label} (to all): hi"));
        }
    }

    /// Tracing: TC-005-01
    /// ACs: FR-005-AC-1
    ///
    /// `duplicated` is exactly the set of addresses more than one lead holds.
    #[test]
    fn tc_005_01_duplicated_is_exactly_the_addresses_held_more_than_once(f in fleet()) {
        let addresses: Vec<Address> = f.regs.iter().filter_map(Registration::full_address).collect();
        let dup = duplicated(addresses.iter());
        for a in &addresses {
            let held = addresses.iter().filter(|b| *b == a).count();
            prop_assert_eq!(dup.contains(a), held > 1, "address {}", a);
        }
        prop_assert!(dup.len() <= addresses.len());
    }

    /// Tracing: TC-005-04
    /// ACs: FR-005-AC-4
    ///
    /// Every printed address resolves back, through the resolver, to exactly
    /// the lead it labels, for every registry snapshot.
    #[test]
    fn tc_005_04_every_printed_address_resolves_back_to_its_lead(f in fleet()) {
        let book = AddressBook::from_registrations(&f.regs);
        for r in &f.regs {
            let label = book.label(&r.name);
            // No alias is involved: the store keeps aliases disjoint from names.
            let got = resolve_target(&label, &r.machine, &f.regs, &[]);
            prop_assert_eq!(
                got.map_err(|e| e.to_string()),
                Ok(Target::Agent(r.name.clone())),
                "label {:?}", label
            );
        }
    }
}

/// Tracing: TC-005-04
/// ACs: FR-005-AC-4
///
/// The shown addresses include tab labels regardless of Herdr topology counts.
#[test]
fn tc_005_04_tab_addresses_resolve_back_to_their_verified_lead() {
    let mk = |n: &str, tab: &str, pane: &str| Registration {
        name: name(n),
        machine: "hub".parse().expect("valid id"),
        kind: "claude".into(),
        session_id: Some("sid-test".parse().unwrap()),
        cwd: None,
        herdr_pane: Some(pane.parse().unwrap()),
        place: Some(Place {
            workspace: "EA".into(),
            tab: tab.into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "wA".parse().unwrap(),
                tab: "wA:t1".parse().unwrap(),
                pane: pane.parse().unwrap(),
                session_id: "sid-test".parse().unwrap(),
                verified_at_ms: 1,
            }),
        }),
        last_seen_ms: 1,
    };
    let regs = [mk("stale", "code", "wA:p1"), mk("fresh", "plan", "wA:p2")];
    let book = AddressBook::from_registrations(&regs);
    for r in &regs {
        let label = book.label(&r.name);
        assert_eq!(
            resolve_target(&label, &"hub".parse().expect("valid id"), &regs, &[])
                .map_err(|e| e.to_string()),
            Ok(Target::Agent(r.name.clone())),
            "label {label:?}"
        );
    }
}
