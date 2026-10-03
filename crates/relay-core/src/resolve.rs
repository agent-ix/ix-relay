//! Turning what a sender typed into a [`Target`].
//!
//! `agent-msg send hub:EA:1 ...` must work cold, with no `find` first, so
//! resolution happens inside `send`, against the Redis-cached registration
//! snapshot — no Herdr call or SSH per send. Address matches require a Herdr
//! ID-backed place; legacy label-only records are not trusted for routing.
//! Pure over the registry snapshot, so it is tested without Redis.

use std::collections::BTreeSet;

use crate::{
    address::Address,
    error::{Candidate, Error, Result, UnknownCause},
    ids::{MachineName, PaneId, PaneRef},
    message::Registration,
    name::{AgentName, Target},
};

/// The most close matches an unknown-target error suggests.
const MAX_SUGGESTIONS: usize = 5;

/// Resolves `input` to who it addresses, trying, in order (the first form
/// that matches wins, so anything that worked before still does):
///
/// 1. `all`;
/// 2. a bus name or alias of a registered lead;
/// 3. a raw herdr pane id (`wB:p1`) of a lead on `sender_machine` only: pane
///    ids repeat across machines and mean nothing on another one, so a lead
///    elsewhere is addressed by its address. More than one match on the
///    sender's machine is an error;
/// 4. a `server:workspace[:tab[:pane]]` address, case-insensitive; without a tab it
///    selects every tab of the workspace.
///
/// # Errors
///
/// [`Error::AmbiguousTarget`] when more than one lead matches — never a
/// guess. [`Error::UnknownTarget`] when none does, with the likely cause and
/// close matches. [`Error::InvalidAddress`] for text with colons that is not
/// an address.
pub fn resolve_target(
    input: &str,
    sender_machine: &MachineName,
    registrations: &[Registration],
    aliases: &[(AgentName, AgentName)],
) -> Result<Target> {
    resolve_target_scoped(input, sender_machine, None, registrations, aliases)
}

/// Resolves a target with the sender's verified Herdr placement available for
/// relative `:tab` and `:workspace:tab` addresses.
pub fn resolve_target_scoped(
    input: &str,
    sender_machine: &MachineName,
    sender_place: Option<&Address>,
    registrations: &[Registration],
    aliases: &[(AgentName, AgentName)],
) -> Result<Target> {
    if let Ok(target @ Target::All) = input.parse::<Target>() {
        return Ok(target);
    }

    if let Ok(name) = input.parse::<AgentName>() {
        let canonical = aliases
            .iter()
            .find(|(alias, _)| *alias == name)
            .map_or(name, |(_, canonical)| canonical.clone());
        if registrations.iter().any(|r| r.name == canonical) {
            return Ok(Target::Agent(canonical));
        }
    }

    // A pane id is only meaningful with its machine (FR-009): compare the pair.
    let by_pane: Vec<&Registration> = match input.parse::<PaneId>() {
        Ok(pane) => {
            let wanted = PaneRef::new(sender_machine.clone(), pane);
            registrations
                .iter()
                .filter(|r| r.verified_place().is_some() && r.is_on(&wanted))
                .collect()
        }
        Err(_) => Vec::new(),
    };
    if !by_pane.is_empty() {
        return one_or_ambiguous(input, by_pane);
    }

    let expanded;
    let query_input = if let Some(relative) = input.strip_prefix(':') {
        let parts: Vec<_> = relative.split(':').collect();
        if !(1..=2).contains(&parts.len()) || parts.iter().any(|part| part.is_empty()) {
            return Err(Error::InvalidAddress {
                input: input.to_owned(),
            });
        }
        let Some(place) = sender_place else {
            return Err(Error::RelativeAddressNeedsLocation {
                input: input.to_owned(),
            });
        };
        expanded = match parts.as_slice() {
            [tab] if !tab.is_empty() => format!("{}:{}:{}", place.server(), place.workspace(), tab),
            [workspace, tab] if !workspace.is_empty() && !tab.is_empty() => {
                format!("{}:{}:{}", place.server(), workspace, tab)
            }
            _ => {
                return Err(Error::InvalidAddress {
                    input: input.to_owned(),
                });
            }
        };
        expanded.as_str()
    } else {
        input
    };

    if !query_input.contains(':') {
        return Err(unknown(
            input,
            UnknownCause::NotRegistered,
            registrations,
            aliases,
        ));
    }
    let query: Address = query_input.parse()?;
    let matched: Vec<&Registration> = registrations
        .iter()
        .filter(|r| {
            r.full_address()
                .is_some_and(|address| query.selects(&address))
        })
        .collect();
    if matched.is_empty() {
        let cause = classify(&query, registrations);
        return Err(unknown(input, cause, registrations, aliases));
    }
    one_or_ambiguous(input, matched)
}

fn one_or_ambiguous(input: &str, mut matched: Vec<&Registration>) -> Result<Target> {
    if let [only] = matched.as_slice() {
        return Ok(Target::Agent(only.name.clone()));
    }
    matched.sort_by(|a, b| a.name.cmp(&b.name));
    Err(Error::AmbiguousTarget {
        input: input.to_owned(),
        candidates: matched
            .into_iter()
            .map(|r| Candidate {
                name: r.name.clone(),
                address: r.full_address(),
                workspace_id: r
                    .place
                    .as_ref()
                    .and_then(|p| p.herdr_ids.as_ref().map(|ids| ids.workspace.clone())),
                tab_id: r
                    .place
                    .as_ref()
                    .and_then(|p| p.herdr_ids.as_ref().map(|ids| ids.tab.clone())),
            })
            .collect(),
    })
}

/// The most specific reason an address matched no registration.
fn classify(query: &Address, registrations: &[Registration]) -> UnknownCause {
    // Compared the way `Address` cleans a server, so a machine name the address
    // grammar would alter still matches itself.
    let server_of = |r: &Registration| {
        Address::new(r.machine.as_str(), "-", None)
            .server()
            .to_owned()
    };
    // Equality on `Address` folds case, like `selects` did when it matched the
    // server: comparing the strings exactly would call a server unknown while
    // listing it among `known` when its name has a non-ASCII capital.
    let query_server = Address::new(query.server(), "-", None);
    let on_server: Vec<&Registration> = registrations
        .iter()
        .filter(|r| Address::new(r.machine.as_str(), "-", None) == query_server)
        .collect();
    if on_server.is_empty() {
        let known: BTreeSet<String> = registrations.iter().map(server_of).collect();
        return UnknownCause::UnknownServer {
            server: query.server().to_owned(),
            known: known.into_iter().collect(),
        };
    }
    let placed: Vec<Address> = on_server.iter().filter_map(|r| r.full_address()).collect();
    let in_workspace: Vec<&Address> = placed
        .iter()
        .filter(|a| Address::new(query.server(), query.workspace(), None).selects(a))
        .collect();
    if in_workspace.is_empty() {
        let available: BTreeSet<String> = placed.iter().map(|a| a.workspace().to_owned()).collect();
        return UnknownCause::UnknownWorkspace {
            server: query.server().to_owned(),
            available: available.into_iter().collect(),
        };
    }
    let available: BTreeSet<String> = in_workspace
        .iter()
        .filter_map(|a| a.tab().map(str::to_owned))
        .collect();
    UnknownCause::UnknownTab {
        workspace: query.workspace().to_owned(),
        available: available.into_iter().collect(),
    }
}

fn unknown(
    input: &str,
    cause: UnknownCause,
    registrations: &[Registration],
    aliases: &[(AgentName, AgentName)],
) -> Error {
    Error::UnknownTarget {
        input: input.to_owned(),
        cause,
        suggestions: suggest(input, registrations, aliases),
    }
}

/// Names, aliases and full addresses close to `input`, best first: one
/// contains the other (ignoring case), or they are a few edits apart.
fn suggest(
    input: &str,
    registrations: &[Registration],
    aliases: &[(AgentName, AgentName)],
) -> Vec<String> {
    let wanted = input.to_lowercase();
    let allowed = (wanted.chars().count() / 4).max(2);
    let mut pool: BTreeSet<String> = BTreeSet::new();
    let book = crate::AddressBook::from_registrations(registrations);
    for r in registrations {
        pool.insert(r.name.to_string());
        if let Some(address) = book.place_address(&r.name) {
            pool.insert(address.to_string());
        }
    }
    pool.extend(aliases.iter().map(|(alias, _)| alias.to_string()));

    let mut scored: Vec<(usize, String)> = pool
        .into_iter()
        .filter_map(|candidate| {
            let lower = candidate.to_lowercase();
            let distance = edit_distance(&wanted, &lower);
            let contained = lower.contains(&wanted) || wanted.contains(&lower);
            (contained || distance <= allowed).then_some((distance, candidate))
        })
        .collect();
    scored.sort();
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, candidate)| candidate)
        .collect()
}

/// Levenshtein distance over chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if ca == *cb {
                diagonal
            } else {
                1 + diagonal.min(above).min(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Place;

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    fn reg(n: &str, machine: &str, pane: &str, place: Option<(&str, &str, u32)>) -> Registration {
        Registration {
            name: name(n),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: Some("test-session".parse().unwrap()),
            cwd: None,
            herdr_pane: Some(pane.parse().unwrap()),
            place: place.map(|(workspace, tab, _tabs)| Place {
                workspace: workspace.into(),
                tab: tab.into(),
                herdr_ids: Some(crate::HerdrPlaceIds {
                    workspace: format!("workspace-{workspace}").parse().unwrap(),
                    tab: format!("workspace-{workspace}:t1").parse().unwrap(),
                    pane: pane.parse().unwrap(),
                    session_id: "test-session".parse().unwrap(),
                    verified_at_ms: 1,
                }),
            }),
            last_seen_ms: 1,
        }
    }

    fn fleet() -> Vec<Registration> {
        vec![
            reg("ea-code", "hub", "wB:p1", Some(("EA", "code", 2))),
            reg("ea-plan", "hub", "wB:p2", Some(("EA", "plan", 2))),
            reg("research", "node-a", "wB:p1", Some(("research", "1", 1))),
            reg("old-lead", "node-a", "wC:p1", None),
        ]
    }

    fn target(input: &str) -> Result<Target> {
        resolve_target(
            input,
            &"hub".parse().unwrap(),
            &fleet(),
            &[(name("ea"), name("ea-code"))],
        )
    }

    fn agent(n: &str) -> Target {
        Target::Agent(name(n))
    }

    #[test]
    fn shorthand_uses_the_senders_machine_and_workspace() {
        let place: Address = "hub:EA:code".parse().unwrap();
        let regs = fleet();
        for (input, expected) in [(":plan", "ea-plan"), (":EA:plan", "ea-plan")] {
            assert_eq!(
                resolve_target_scoped(input, &"hub".parse().unwrap(), Some(&place), &regs, &[])
                    .unwrap(),
                agent(expected)
            );
        }
        let other: Address = "node-a:research:1".parse().unwrap();
        assert!(matches!(
            resolve_target_scoped(
                ":EA:plan",
                &"node-a".parse().unwrap(),
                Some(&other),
                &regs,
                &[]
            ),
            Err(Error::UnknownTarget { .. })
        ));
    }

    #[test]
    fn shorthand_without_location_is_a_context_error() {
        assert!(matches!(
            target(":plan"),
            Err(Error::RelativeAddressNeedsLocation { .. })
        ));
    }

    #[test]
    fn shorthand_does_not_guess_an_ambiguous_target() {
        let place: Address = "hub:EA:code".parse().unwrap();
        let mut regs = fleet();
        regs.push(reg("another-plan", "hub", "wB:p3", Some(("EA", "plan", 2))));
        assert!(matches!(
            resolve_target_scoped(":plan", &"hub".parse().unwrap(), Some(&place), &regs, &[]),
            Err(Error::AmbiguousTarget { .. })
        ));
    }

    #[test]
    fn all_is_a_broadcast() {
        assert_eq!(target("all").unwrap(), Target::All);
    }

    #[test]
    fn a_bus_name_or_an_alias_still_resolves() {
        assert_eq!(target("ea-plan").unwrap(), agent("ea-plan"));
        assert_eq!(target("ea").unwrap(), agent("ea-code"));
        assert_eq!(target("old-lead").unwrap(), agent("old-lead"));
    }

    #[test]
    fn a_full_address_resolves_ignoring_case() {
        assert_eq!(target("hub:EA:code").unwrap(), agent("ea-code"));
        assert_eq!(target("HUB:ea:PLAN").unwrap(), agent("ea-plan"));
    }

    /// Tracing: TC-003-08
    /// ACs: FR-003-AC-8
    #[test]
    fn workspace_and_tab_prefixes_require_specificity_only_when_multiple_leads_match() {
        let at = |name: &str, tab: &str, tab_id: &str, pane: &str| {
            let mut registration = reg(name, "hub", pane, Some(("EA", tab, 2)));
            registration.place.as_mut().unwrap().herdr_ids = Some(crate::HerdrPlaceIds {
                workspace: "wA".parse().unwrap(),
                tab: tab_id.parse().unwrap(),
                pane: pane.parse().unwrap(),
                session_id: "test-session".parse().unwrap(),
                verified_at_ms: 1,
            });
            registration
        };
        let registrations = [
            at("ea-code-1", "code", "wA:t1", "wA:p1"),
            at("ea-code-2", "code", "wA:t1", "wA:p2"),
            at("ea-plan", "plan", "wA:t2", "wA:p3"),
        ];
        for input in ["hub:EA", "hub:EA:code"] {
            let Err(Error::AmbiguousTarget { candidates, .. }) =
                resolve_target(input, &"hub".parse().unwrap(), &registrations, &[])
            else {
                panic!("{input} should be a valid broader Herdr address, then ambiguous by lead");
            };
            assert!(!candidates.is_empty());
            assert!(candidates.iter().all(|candidate| {
                candidate
                    .address
                    .as_ref()
                    .is_some_and(|address| address.pane().is_some())
            }));
        }
        assert_eq!(
            resolve_target(
                "hub:EA:code:wA-p1",
                &"hub".parse().unwrap(),
                &registrations,
                &[]
            )
            .unwrap(),
            agent("ea-code-1")
        );
        assert_eq!(
            resolve_target("hub:EA:plan", &"hub".parse().unwrap(), &registrations, &[]).unwrap(),
            agent("ea-plan")
        );
    }

    #[test]
    fn the_tab_is_optional_when_the_workspace_has_one_lead() {
        assert_eq!(target("node-a:research").unwrap(), agent("research"));
        assert_eq!(target("node-a:research:1").unwrap(), agent("research"));
    }

    #[test]
    fn a_missing_tab_is_ambiguous_when_the_workspace_has_several_and_lists_them() {
        let Err(Error::AmbiguousTarget { input, candidates }) = target("hub:EA") else {
            panic!("expected AmbiguousTarget");
        };
        assert_eq!(input, "hub:EA");
        let names: Vec<_> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["ea-code", "ea-plan"]);
        assert_eq!(
            candidates[0].address.as_ref().unwrap().to_string(),
            "hub:EA:code:wB-p1"
        );
    }

    /// Tracing: TC-003-07
    /// ACs: FR-003-AC-7
    #[test]
    fn tc_003_07_equal_labels_on_distinct_herdr_ids_are_ambiguous_with_ids() {
        let mut regs = vec![
            reg("dev-one", "hub", "wA:p1", Some(("dev", "code", 1))),
            reg("dev-two", "hub", "wB:p1", Some(("dev", "code", 1))),
        ];
        for (reg, workspace) in regs.iter_mut().zip(["wA", "wB"]) {
            let place = reg.place.as_mut().unwrap();
            place.herdr_ids = Some(crate::HerdrPlaceIds {
                workspace: workspace.parse().unwrap(),
                tab: format!("{workspace}:t1").parse().unwrap(),
                pane: reg.herdr_pane.clone().unwrap(),
                session_id: "test-session".parse().unwrap(),
                verified_at_ms: 1,
            });
        }
        let Err(Error::AmbiguousTarget { candidates, .. }) =
            resolve_target("hub:dev:code", &"hub".parse().unwrap(), &regs, &[])
        else {
            panic!("duplicate human labels must not choose a Herdr identity");
        };
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].workspace_id.as_ref().unwrap().as_str(), "wA");
        assert_eq!(candidates[1].workspace_id.as_ref().unwrap().as_str(), "wB");
    }

    /// Tracing: TC-005-10
    /// ACs: FR-005-AC-10
    #[test]
    fn legacy_label_only_registration_is_not_routable_by_address() {
        let mut legacy = reg("legacy", "hub", "wB:p1", Some(("EA", "code", 1)));
        legacy.place = None;
        assert!(matches!(
            resolve_target("hub:EA:code", &"hub".parse().unwrap(), &[legacy], &[]),
            Err(Error::UnknownTarget { .. })
        ));
    }

    /// Tracing: TC-009-05
    /// ACs: FR-009-AC-5
    #[test]
    fn tc_009_05_a_pane_claim_without_session_bound_herdr_proof_is_not_a_send_target() {
        let claimed = reg("claimed", "hub", "wC:p1", None);
        let verified = reg("verified", "hub", "wB:p1", Some(("EA", "code", 1)));

        assert!(matches!(
            resolve_target("wC:p1", &"hub".parse().unwrap(), &[claimed], &[]),
            Err(Error::UnknownTarget { .. })
        ));
        assert_eq!(
            resolve_target("wB:p1", &"hub".parse().unwrap(), &[verified], &[]).unwrap(),
            agent("verified")
        );
    }

    #[test]
    fn a_pane_id_resolves_on_the_senders_machine_only() {
        // `wB:p1` is on both hub (ea-code) and node-a (research): each sender
        // reaches its own machine's lead, never the other's.
        assert_eq!(target("wB:p1").unwrap(), agent("ea-code"));
        assert_eq!(
            resolve_target("wB:p1", &"Node-A.local".parse().unwrap(), &fleet(), &[]).unwrap(),
            agent("research")
        );
        // hub's `wB:p2` is not reachable by pane id from node-a.
        assert!(resolve_target("wB:p2", &"node-a".parse().unwrap(), &fleet(), &[]).is_err());
        // An unplaced pane claim is not routing proof, even on the sender's machine.
        assert!(resolve_target("wC:p1", &"hub".parse().unwrap(), &fleet(), &[]).is_err());
    }

    #[test]
    fn two_leads_on_the_senders_machine_with_one_pane_id_are_ambiguous() {
        let mut regs = fleet();
        regs.push(reg("ea-dup", "hub", "wB:p1", Some(("EA", "duplicate", 1))));
        assert!(matches!(
            resolve_target("wB:p1", &"hub".parse().unwrap(), &regs, &[]),
            Err(Error::AmbiguousTarget { candidates, .. }) if candidates.len() == 2
        ));
    }

    #[test]
    fn the_same_workspace_label_on_two_servers_never_crosses_servers() {
        let mut regs = fleet();
        regs.push(reg("ea-node-a", "node-a", "wZ:p1", Some(("EA", "code", 1))));
        assert_eq!(
            resolve_target("node-a:EA", &"hub".parse().unwrap(), &regs, &[]).unwrap(),
            agent("ea-node-a")
        );
        assert_eq!(
            resolve_target("hub:EA:code", &"hub".parse().unwrap(), &regs, &[]).unwrap(),
            agent("ea-code")
        );
    }

    #[test]
    fn two_leads_in_one_tab_are_ambiguous() {
        let mut regs = fleet();
        regs.push(reg("ea-code-2", "hub", "wB:p9", Some(("EA", "code", 2))));
        assert!(matches!(
            resolve_target("hub:EA:code", &"hub".parse().unwrap(), &regs, &[]),
            Err(Error::AmbiguousTarget { candidates, .. }) if candidates.len() == 2
        ));
    }

    #[test]
    fn a_lead_with_no_recorded_place_has_no_address() {
        assert!(matches!(
            target("node-a:wC"),
            Err(Error::UnknownTarget {
                cause: UnknownCause::UnknownWorkspace { .. },
                ..
            })
        ));
    }

    #[test]
    fn an_unknown_server_names_the_servers_that_have_leads() {
        let Err(Error::UnknownTarget { cause, .. }) = target("node-b:EA:code") else {
            panic!("expected UnknownTarget");
        };
        assert_eq!(
            cause,
            UnknownCause::UnknownServer {
                server: "node-b".into(),
                known: vec!["hub".into(), "node-a".into()],
            }
        );
    }

    #[test]
    fn an_unknown_workspace_lists_the_workspaces_on_that_server_and_suggests_the_near_miss() {
        let Err(Error::UnknownTarget {
            cause, suggestions, ..
        }) = target("hub:EAA:code")
        else {
            panic!("expected UnknownTarget");
        };
        assert_eq!(
            cause,
            UnknownCause::UnknownWorkspace {
                server: "hub".into(),
                available: vec!["EA".into()],
            }
        );
        assert!(
            suggestions.contains(&"hub:EA:code".to_owned()),
            "{suggestions:?}"
        );
    }

    #[test]
    fn an_unknown_tab_lists_the_tabs_that_have_a_lead() {
        let Err(Error::UnknownTarget { cause, .. }) = target("hub:EA:nope") else {
            panic!("expected UnknownTarget");
        };
        assert_eq!(
            cause,
            UnknownCause::UnknownTab {
                workspace: "EA".into(),
                available: vec!["code".into(), "plan".into()],
            }
        );
    }

    #[test]
    fn a_mistyped_name_is_not_registered_and_suggests_the_close_name() {
        let Err(Error::UnknownTarget {
            cause, suggestions, ..
        }) = target("ea-cod")
        else {
            panic!("expected UnknownTarget");
        };
        assert_eq!(cause, UnknownCause::NotRegistered);
        assert_eq!(suggestions.first().map(String::as_str), Some("ea-code"));
    }

    #[test]
    fn colon_text_that_is_not_an_address_is_invalid() {
        assert!(matches!(
            target("a:b:c:d:e"),
            Err(Error::InvalidAddress { .. })
        ));
    }

    #[test]
    fn the_error_text_carries_the_cause_and_the_suggestion() {
        let text = target("hub:EAA:code").unwrap_err().to_string();
        assert!(text.contains("workspaces: EA"), "{text}");
        assert!(text.contains("did you mean"), "{text}");
        assert!(text.contains("hub:EA:code"), "{text}");
    }

    #[test]
    fn edit_distance_counts_single_character_edits() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
    }
}
