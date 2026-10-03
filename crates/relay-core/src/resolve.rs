//! Turning what a sender typed into a [`Target`].
//!
//! `agent-msg send hub:EA:1 ...` must work cold, with no `find` first, so
//! public send refreshes live Herdr topology and enrolls the verified session.
//! This pure resolver validates the same grammar against a supplied snapshot.

use std::collections::BTreeSet;

use crate::{
    address::Address,
    error::{Candidate, Error, Result, UnknownCause},
    ids::MachineName,
    message::Registration,
    name::Target,
};

/// The most close matches an unknown-target error suggests.
const MAX_SUGGESTIONS: usize = 5;

/// Resolve a full tab address or broadcast against verified session registrations.
/// Shortcuts additionally require the sender's verified placement.
///
/// # Errors
/// Invalid syntax, missing caller placement, absent recipient, or ambiguous tab.
pub fn resolve_target(
    input: &str,
    sender_machine: &MachineName,
    registrations: &[Registration],
) -> Result<Target> {
    resolve_target_scoped(input, sender_machine, None, registrations)
}

/// Resolves a target with the sender's verified Herdr placement available for
/// relative `:tab` and `:workspace:tab` addresses.
pub fn resolve_target_scoped(
    input: &str,
    _sender_machine: &MachineName,
    sender_place: Option<&Address>,
    registrations: &[Registration],
) -> Result<Target> {
    if let Ok(target @ Target::All) = input.parse::<Target>() {
        return Ok(target);
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

    if query_input.split(':').count() != 3
        || query_input
            .split(':')
            .any(|part| part.is_empty() || part.chars().any(char::is_control))
    {
        return Err(unknown(input, UnknownCause::NotRegistered, registrations));
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
        return Err(unknown(input, cause, registrations));
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

fn unknown(input: &str, cause: UnknownCause, registrations: &[Registration]) -> Error {
    Error::UnknownTarget {
        input: input.to_owned(),
        cause,
        suggestions: suggest(input, registrations),
    }
}

/// Full tab addresses close to `input`, best first: one
/// contains the other (ignoring case), or they are a few edits apart.
fn suggest(input: &str, registrations: &[Registration]) -> Vec<String> {
    let wanted = input.to_lowercase();
    let allowed = (wanted.chars().count() / 4).max(2);
    let mut pool: BTreeSet<String> = BTreeSet::new();
    let book = crate::AddressBook::from_registrations(registrations);
    for r in registrations {
        if let Some(address) = book.place_address(&r.name) {
            pool.insert(address.to_string());
        }
    }

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
    #[test]
    fn legacy_names_panes_and_partial_addresses_are_rejected() {
        let machine = "draco".parse().unwrap();
        for input in [
            "filament-code-old-15",
            "wD:p1",
            "draco:misc",
            "draco:misc:code:wT:p1",
        ] {
            assert!(resolve_target(input, &machine, &[]).is_err(), "{input}");
        }
    }
    #[test]
    fn shortcuts_require_verified_sender_location() {
        let machine = "draco".parse().unwrap();
        for input in [":code", ":misc:code"] {
            assert!(matches!(
                resolve_target(input, &machine, &[]),
                Err(Error::RelativeAddressNeedsLocation { .. })
            ));
        }
    }
}
