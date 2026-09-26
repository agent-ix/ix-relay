//! Human-readable agent addresses: `server:workspace[:tab]`, e.g. `hub:EA:1`
//! or `node-a:research`.
//!
//! Bus names and herdr pane ids stay valid, but neither tells a person where
//! an agent is: pane ids repeat across machines, and derived names are slugs.
//! An [`Address`] is built from what herdr shows on screen — the machine, the
//! workspace label and the tab label — so what an agent reports is what the operator
//! sees.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

use crate::{
    error::Error,
    ids::MachineName,
    message::{Message, Registration},
    name::{AgentName, Target},
};

/// Where a lead's pane sits in herdr, captured when it registered.
///
/// Stored on the [`Registration`] so `send` can resolve an address, and every
/// listing can print one, without shelling out to herdr (over SSH, for a
/// remote machine) on each call.
///
/// Tolerant of unknown fields, like [`Registration`]: a newer binary may record
/// more about the pane than this one knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    /// The workspace's label, e.g. `EA`.
    pub workspace: String,
    /// The tab's label, e.g. `code`; herdr's own display number, rendered as a
    /// string (e.g. `1`), for an unlabelled tab.
    pub tab: String,
    /// How many tabs the workspace had when this was captured. The tab is left
    /// out of the shown address when it is `1`: there is nothing to tell apart.
    pub tabs_in_workspace: u32,
}

impl Place {
    /// The address to show for a lead at this place on `machine` *when this is
    /// all that is known*: the tab is included unless this place's recorded count
    /// says its workspace has exactly one — with nothing to tell apart it is
    /// noise, but when the count is unknown (`0`) a missing tab could make two
    /// leads look like one, so it stays.
    ///
    /// A recorded count can be stale, so this is not the rule a listing of
    /// registered leads uses: [`AddressBook`] drops the tab only when no other
    /// registered lead shares the workspace either. This is for a place seen
    /// live, with no registry to compare against.
    #[must_use]
    pub fn address(&self, machine: &MachineName) -> Address {
        let tab = (self.tabs_in_workspace != 1).then_some(self.tab.as_str());
        Address::new(machine.as_str(), &self.workspace, tab)
    }

    /// The address with its tab always included — what an address query is
    /// matched against.
    #[must_use]
    pub fn full_address(&self, machine: &MachineName) -> Address {
        Address::new(machine.as_str(), &self.workspace, Some(&self.tab))
    }
}

/// `server:workspace[:tab]`.
///
/// Segments are made safe to type and to print on one line: whitespace,
/// control characters and `:` become `-`. Both sides of a comparison go
/// through the same cleaning, so a workspace called `My Work` is addressed as
/// `hub:My-Work`.
///
/// Equality and hashing ignore case — the one definition of "the same
/// address", so `hub:EA:Code` equals `HUB:ea:code` and either finds the
/// other in a set or map. [`Address::selects`] is the query form of the same
/// comparison. Neither allocates.
#[derive(Debug, Clone)]
pub struct Address {
    server: String,
    workspace: String,
    tab: Option<String>,
}

impl Address {
    /// Builds an address; `server` is normalized like every machine name on
    /// the bus (see [`MachineName`]).
    #[must_use]
    pub fn new(server: &str, workspace: &str, tab: Option<&str>) -> Self {
        Self {
            server: clean(&MachineName::normalise(server)),
            workspace: clean(workspace),
            tab: tab.map(clean),
        }
    }

    /// The machine the agent runs on.
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }

    /// The workspace label.
    #[must_use]
    pub fn workspace(&self) -> &str {
        &self.workspace
    }

    /// The tab label, when the address names one.
    #[must_use]
    pub fn tab(&self) -> Option<&str> {
        self.tab.as_deref()
    }

    /// Whether this address, read as a query, selects `target`: the same
    /// server and workspace, ignoring case, and — only when this address
    /// names a tab — the same tab. An address without a tab selects every tab
    /// of the workspace.
    #[must_use]
    pub fn selects(&self, target: &Address) -> bool {
        eq_fold(&self.server, &target.server)
            && eq_fold(&self.workspace, &target.workspace)
            && match (&self.tab, &target.tab) {
                (None, _) => true,
                (Some(want), Some(have)) => eq_fold(want, have),
                (Some(_), None) => false,
            }
    }
}

impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        eq_fold(&self.server, &other.server)
            && eq_fold(&self.workspace, &other.workspace)
            && match (&self.tab, &other.tab) {
                (None, None) => true,
                (Some(a), Some(b)) => eq_fold(a, b),
                _ => false,
            }
    }
}

impl Eq for Address {}

impl Hash for Address {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_fold(&self.server, state);
        hash_fold(&self.workspace, state);
        // `None` and `Some("")` must differ, as they do under `==`.
        self.tab.is_some().hash(state);
        if let Some(tab) = &self.tab {
            hash_fold(tab, state);
        }
    }
}

/// `s` lowercased, one char at a time, without building a `String`.
fn folded(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().flat_map(char::to_lowercase)
}

fn eq_fold(a: &str, b: &str) -> bool {
    folded(a).eq(folded(b))
}

/// Hashes the folded chars plus a terminator, consistently with [`eq_fold`]:
/// strings that fold equal hash equal.
fn hash_fold<H: Hasher>(s: &str, state: &mut H) {
    folded(s).for_each(|c| c.hash(state));
    state.write_u8(0xff);
}

/// The addresses that appear more than once in `addresses` (compared as
/// [`Address`] equality: ignoring case, tab included). An address two leads
/// share names neither of them: `send` refuses it as ambiguous, so a listing
/// must not offer it — every "show this lead by address" rule uses this set.
pub fn duplicated<'a>(addresses: impl IntoIterator<Item = &'a Address>) -> HashSet<&'a Address> {
    let mut seen = HashSet::new();
    let mut dup = HashSet::new();
    for address in addresses {
        if !seen.insert(address) {
            dup.insert(address);
        }
    }
    dup
}

/// Replaces whitespace, control characters and `:` with `-`; an empty result
/// becomes `-` so a segment is never empty.
fn clean(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_whitespace() || crate::text::is_unsafe_char(c) || c == ':' {
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

impl FromStr for Address {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let bad = || Error::InvalidAddress {
            input: s.to_owned(),
        };
        let parts: Vec<&str> = s.split(':').collect();
        match parts.as_slice() {
            [server, workspace] if !server.is_empty() && !workspace.is_empty() => {
                Ok(Self::new(server, workspace, None))
            }
            [server, workspace, tab]
                if !server.is_empty() && !workspace.is_empty() && !tab.is_empty() =>
            {
                Ok(Self::new(server, workspace, Some(tab)))
            }
            _ => Err(bad()),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.server, self.workspace)?;
        if let Some(tab) = &self.tab {
            write!(f, ":{tab}")?;
        }
        Ok(())
    }
}

impl Registration {
    /// This lead's address with its tab always included — what an address
    /// query is matched against.
    #[must_use]
    pub fn full_address(&self) -> Option<Address> {
        Some(self.place.as_ref()?.full_address(&self.machine))
    }
}

/// Longest address (`server:workspace[:tab]`) shown in place of a bus name, in
/// bytes. A workspace or tab label is whatever a person typed, so an address has
/// no length of its own; a longer one is not shown and the sender's bus name
/// (at most 32 bytes) is printed instead. This bounds a rendered message, which
/// is what the hook and watcher byte budgets count (`Message::rendered_len_max`, test-only).
pub const MAX_LABEL_BYTES: usize = 96;

/// How to name each registered lead when printing: its address when known,
/// else its bus name. Built once per command from the registry.
#[derive(Debug, Default)]
pub struct AddressBook {
    /// Where each lead with a place sits, tab per the shown-address rule: every
    /// lead that fits [`MAX_LABEL_BYTES`], shared or not.
    placed: HashMap<AgentName, Address>,
    /// The subset of [`Self::placed`] whose address no other lead shares: the
    /// leads that are printed by address.
    by_name: HashMap<AgentName, Address>,
}

impl AddressBook {
    /// Indexes every registration that has a place, in time linear in the
    /// registry.
    ///
    /// A lead's shown address omits the tab only when its recorded count says
    /// its workspace has one tab **and** no other registered lead is in that
    /// server and workspace. A count is written when the lead registered and
    /// refreshed by the watcher, so it can be stale: trusting it alone printed a
    /// lead as `hub:EA` beside a lead at `hub:EA:plan`, an address that
    /// selects both. An unknown count (`0`) keeps the tab, as before.
    ///
    /// Leads whose address another lead shares (two panes in one tab) are then
    /// printed by bus name: an address that names two leads cannot say who sent
    /// a message, and `send` would refuse it as ambiguous. So are those whose
    /// address is longer than [`MAX_LABEL_BYTES`].
    #[must_use]
    pub fn from_registrations(registrations: &[Registration]) -> Self {
        let placed: Vec<(&Registration, &Place)> = registrations
            .iter()
            .filter_map(|r| Some((r, r.place.as_ref()?)))
            .collect();
        // How many placed leads share each server and workspace (a tab-less
        // address compares equal for a shared workspace whatever the case).
        let mut in_workspace: HashMap<Address, usize> = HashMap::new();
        for (r, place) in &placed {
            *in_workspace
                .entry(Address::new(r.machine.as_str(), &place.workspace, None))
                .or_default() += 1;
        }
        let addressed: Vec<(&Registration, Address)> = placed
            .iter()
            .map(|(r, place)| {
                let workspace = Address::new(r.machine.as_str(), &place.workspace, None);
                let alone = in_workspace.get(&workspace).copied() == Some(1);
                let address = if place.tabs_in_workspace == 1 && alone {
                    workspace
                } else {
                    place.full_address(&r.machine)
                };
                (*r, address)
            })
            .filter(|(_, address)| address.to_string().len() <= MAX_LABEL_BYTES)
            .collect();
        let shared = duplicated(addressed.iter().map(|(_, address)| address));
        Self {
            placed: addressed
                .iter()
                .map(|(r, address)| (r.name.clone(), address.clone()))
                .collect(),
            by_name: addressed
                .iter()
                .filter(|(_, address)| !shared.contains(address))
                .map(|(r, address)| (r.name.clone(), address.clone()))
                .collect(),
        }
    }

    /// Where `name` sits, tab per the rule of [`AddressBook::from_registrations`],
    /// whether or not another lead shares it: for a listing that prints the
    /// address as a column beside the bus name (`who`, `whoami`), not as the
    /// lead's name.
    #[must_use]
    pub fn place_address(&self, name: &AgentName) -> Option<&Address> {
        self.placed.get(name)
    }

    /// `name`'s address, if it registered one.
    #[must_use]
    pub fn address(&self, name: &AgentName) -> Option<&Address> {
        self.by_name.get(name)
    }

    /// What to print for `name`: its address, else the bare bus name.
    #[must_use]
    pub fn label(&self, name: &AgentName) -> String {
        self.address(name)
            .map_or_else(|| name.to_string(), ToString::to_string)
    }

    /// `name` with its address for a confirmation line: `hub:EA:1 (ea-code)`,
    /// or just the name when it has no address.
    #[must_use]
    pub fn describe(&self, name: &AgentName) -> String {
        match self.address(name) {
            Some(address) => format!("{address} ({name})"),
            None => name.to_string(),
        }
    }

    /// `message` as [`Message::render`] shows it, signed with the sender's
    /// address when it has one.
    #[must_use]
    pub fn render(&self, message: &Message) -> String {
        message.render_from(&self.label(&message.from))
    }

    /// [`AddressBook::render`], marked as a redelivery when `redelivered`
    /// (see [`Message::render_from_marked`]).
    #[must_use]
    pub fn render_marked(&self, message: &Message, redelivered: bool) -> String {
        message.render_from_marked(&self.label(&message.from), redelivered)
    }

    /// A message's recipient as printed by `log`: an address for one lead,
    /// `all` for a broadcast.
    #[must_use]
    pub fn label_target(&self, target: &Target) -> String {
        match target {
            Target::Agent(name) => self.label(name),
            Target::All => target.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(name: &str, machine: &str, place: Option<Place>) -> Registration {
        Registration {
            name: name.parse().unwrap(),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: Some("wB:p1".parse().unwrap()),
            place,
            last_seen_ms: 1,
        }
    }

    fn place(workspace: &str, tab: &str, tabs: u32) -> Place {
        Place {
            workspace: workspace.into(),
            tab: tab.into(),
            tabs_in_workspace: tabs,
        }
    }

    #[test]
    fn parses_and_prints_server_workspace_and_optional_tab() {
        let a: Address = "hub:EA:1".parse().unwrap();
        assert_eq!(
            (a.server(), a.workspace(), a.tab()),
            ("hub", "EA", Some("1"))
        );
        assert_eq!(a.to_string(), "hub:EA:1");
        let b: Address = "node-a:research".parse().unwrap();
        assert_eq!(b.tab(), None);
        assert_eq!(b.to_string(), "node-a:research");
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for bad in ["", "hub", "hub:", ":EA", "hub::1", "a:b:c:d", "hub:EA:"] {
            assert!(
                matches!(bad.parse::<Address>(), Err(Error::InvalidAddress { input }) if input == bad),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn the_server_segment_is_normalized_like_a_machine_name() {
        let a: Address = "Hub.local:EA".parse().unwrap();
        assert_eq!(a.server(), "hub");
    }

    #[test]
    fn whitespace_in_a_label_is_cleaned_so_the_address_stays_one_token() {
        let a = Address::new("hub", "My Work\nspace", Some("a:b"));
        assert_eq!(a.to_string(), "hub:My-Work-space:a-b");
        assert_eq!("hub:My-Work-space:a-b".parse::<Address>().unwrap(), a);
    }

    #[test]
    fn selects_ignores_case_and_a_missing_tab_selects_every_tab() {
        let target: Address = "hub:EA:Code".parse().unwrap();
        assert!("hub:ea:code".parse::<Address>().unwrap().selects(&target));
        assert!("HUB:EA".parse::<Address>().unwrap().selects(&target));
        assert!(!"hub:EA:plan".parse::<Address>().unwrap().selects(&target));
        assert!(!"node-a:EA".parse::<Address>().unwrap().selects(&target));
        assert!(!"hub:QSL".parse::<Address>().unwrap().selects(&target));
    }

    #[test]
    fn the_shown_address_drops_the_tab_only_for_a_lone_lead_in_a_one_tab_workspace() {
        let a = |regs: &[Registration]| AddressBook::from_registrations(regs).label(&name("a"));
        let one = reg("a", "node-a", Some(place("research", "1", 1)));
        assert_eq!(a(std::slice::from_ref(&one)), "node-a:research");
        assert_eq!(one.full_address().unwrap().to_string(), "node-a:research:1");
        let many = reg("a", "hub", Some(place("EA", "1", 3)));
        assert_eq!(a(&[many]), "hub:EA:1");
    }

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    /// Tracing: TC-005-08
    /// ACs: FR-005-AC-8
    ///
    /// A recorded count of 1 is not enough to drop the tab: `stale` registered
    /// while `code` was the workspace's only tab and was never refreshed, and
    /// `fresh` registered later in `plan` with the real count 2. Trusting the
    /// count printed `stale` as `hub:EA`, which selects both. The tab is kept
    /// while another lead shares the workspace, and dropped once it is alone.
    #[test]
    fn tc_005_08_a_stale_tab_count_does_not_print_an_address_that_selects_two_leads() {
        let stale = reg("stale", "hub", Some(place("EA", "code", 1)));
        let fresh = reg("fresh", "hub", Some(place("EA", "plan", 2)));
        let book = AddressBook::from_registrations(&[stale.clone(), fresh]);
        assert_eq!(book.label(&name("stale")), "hub:EA:code");
        assert_eq!(book.label(&name("fresh")), "hub:EA:plan");
        assert_eq!(
            book.place_address(&name("stale")).map(ToString::to_string),
            Some("hub:EA:code".to_owned())
        );

        // Alone in the workspace with a count of 1, the tab is noise again. The
        // workspace is compared as the resolver compares it: server and label,
        // ignoring case.
        let alone = AddressBook::from_registrations(std::slice::from_ref(&stale));
        assert_eq!(alone.label(&name("stale")), "hub:EA");
        let other_machine = reg("far", "node-a", Some(place("EA", "plan", 2)));
        let apart = AddressBook::from_registrations(&[stale.clone(), other_machine]);
        assert_eq!(
            apart.label(&name("stale")),
            "hub:EA",
            "another server's EA is not shared"
        );
        let cased = reg("cased", "HUB", Some(place("ea", "plan", 2)));
        let same = AddressBook::from_registrations(&[stale, cased]);
        assert_eq!(
            same.label(&name("stale")),
            "hub:EA:code",
            "case does not hide a shared workspace"
        );
    }

    /// Tracing: TC-005-07
    /// ACs: FR-005-AC-7
    #[test]
    fn tc_005_07_an_address_over_the_label_cap_is_shown_as_the_bus_name() {
        // `hub:` + workspace + `:1` is 6 bytes longer than the workspace.
        let fits = "w".repeat(MAX_LABEL_BYTES - 6);
        let book = AddressBook::from_registrations(&[
            reg("fits", "hub", Some(place(&fits, "1", 2))),
            reg("long", "hub", Some(place(&format!("{fits}w"), "1", 2))),
        ]);
        let fits_name: AgentName = "fits".parse().unwrap();
        assert_eq!(book.label(&fits_name).len(), MAX_LABEL_BYTES);
        assert!(
            book.place_address(&"long".parse().unwrap()).is_none(),
            "who and whoami use this too"
        );
        assert_eq!(book.label(&"long".parse().unwrap()), "long");
    }

    #[test]
    fn two_leads_sharing_an_address_are_shown_by_name_not_by_a_shared_address() {
        let book = AddressBook::from_registrations(&[
            reg("pane-one", "hub", Some(place("EA", "code", 2))),
            reg("pane-two", "hub", Some(place("ea", "Code", 2))),
            reg("alone", "hub", Some(place("EA", "plan", 2))),
        ]);
        assert_eq!(book.label(&"pane-one".parse().unwrap()), "pane-one");
        assert_eq!(book.label(&"pane-two".parse().unwrap()), "pane-two");
        assert_eq!(book.label(&"alone".parse().unwrap()), "hub:EA:plan");
    }

    #[test]
    fn an_unknown_tab_count_keeps_the_tab_rather_than_risk_merging_two_leads() {
        let unknown = reg("a", "hub", Some(place("EA", "code", 0)));
        let book = AddressBook::from_registrations(&[unknown]);
        assert_eq!(book.label(&name("a")), "hub:EA:code");
    }

    #[test]
    fn equality_and_hashing_ignore_case_and_agree_with_each_other() {
        use std::hash::{BuildHasher, RandomState};
        let a: Address = "hub:EA:Code".parse().unwrap();
        let b: Address = "HUB:ea:code".parse().unwrap();
        assert_eq!(a, b);
        let state = RandomState::new();
        assert_eq!(state.hash_one(&a), state.hash_one(&b));
        assert_ne!(
            a,
            "hub:EA".parse::<Address>().unwrap(),
            "a tab is part of identity"
        );
        assert_ne!(a, "hub:EA:plan".parse::<Address>().unwrap());
        assert_eq!(duplicated([&a, &b]).len(), 1);
        assert!(duplicated([&a]).is_empty());
    }

    proptest::proptest! {
        /// Labels that differ only in ASCII case are the same address, and
        /// hash the same, so a set or map keyed by an address finds its twin.
        #[test]
        fn addresses_differing_only_in_case_are_equal_and_hash_equal(
            ws in "[a-zA-Z0-9-]{1,12}", tab in proptest::option::of("[a-zA-Z0-9-]{1,8}"),
        ) {
            use std::hash::{BuildHasher, RandomState};
            let a = Address::new("hub", &ws, tab.as_deref());
            let b = Address::new(
                "HUB",
                &ws.to_uppercase(),
                tab.as_deref().map(str::to_lowercase).as_deref(),
            );
            proptest::prop_assert_eq!(&a, &b);
            let state = RandomState::new();
            proptest::prop_assert_eq!(state.hash_one(&a), state.hash_one(&b));
        }
    }

    /// The definition [`AddressBook::from_registrations`] must equal, written
    /// from the rule's wording: a lead's tab is dropped when its recorded count
    /// is 1 and *no other* placed lead is in its server and workspace; it is
    /// printed by address when that address fits the cap and *no other* lead
    /// shows an equal one. Quadratic on purpose — every lead is compared with
    /// every other — so it is obviously the rule, not fast.
    fn quadratic_oracle(registrations: &[Registration]) -> HashMap<AgentName, String> {
        let placed: Vec<(&Registration, &Place)> = registrations
            .iter()
            .filter_map(|r| Some((r, r.place.as_ref()?)))
            .collect();
        let shown: Vec<(&Registration, Address)> = placed
            .iter()
            .enumerate()
            .map(|(i, (r, place))| {
                let workspace = Address::new(r.machine.as_str(), &place.workspace, None);
                let shares_workspace = placed.iter().enumerate().any(|(j, (other, p))| {
                    i != j && Address::new(other.machine.as_str(), &p.workspace, None) == workspace
                });
                let address = if place.tabs_in_workspace == 1 && !shares_workspace {
                    workspace
                } else {
                    place.full_address(&r.machine)
                };
                (*r, address)
            })
            .filter(|(_, address)| address.to_string().len() <= MAX_LABEL_BYTES)
            .collect();
        shown
            .iter()
            .enumerate()
            .filter(|(i, (_, address))| {
                !shown
                    .iter()
                    .enumerate()
                    .any(|(j, (_, other))| *i != j && other == address)
            })
            .map(|(_, (r, address))| (r.name.clone(), address.to_string()))
            .collect()
    }

    fn shown(book: &AddressBook) -> HashMap<AgentName, String> {
        book.by_name
            .iter()
            .map(|(name, address)| (name.clone(), address.to_string()))
            .collect()
    }

    fn arbitrary_registration() -> impl proptest::strategy::Strategy<Value = (String, Option<Place>)>
    {
        use proptest::prelude::*;
        // A small alphabet in both cases so collisions, case-only collisions
        // and near-misses are common, not rare.
        let label = "[a-cA-C]{1,2}";
        let place = (label, label, 0u32..3).prop_map(|(workspace, tab, tabs_in_workspace)| Place {
            workspace,
            tab,
            tabs_in_workspace,
        });
        (
            prop::sample::select(vec!["hub", "HUB", "node-a", "Node-A.local"])
                .prop_map(str::to_owned),
            proptest::option::weighted(0.8, place),
        )
    }

    proptest::proptest! {
        /// Tracing: TC-005-09
        /// ACs: FR-005-AC-9
        ///
        /// The linear-time book equals the quadratic definition on any
        /// registry: same leads shown by address, same addresses, and every
        /// lead that shares one is shown by name.
        #[test]
        fn the_linear_book_equals_the_quadratic_definition(
            registrations in proptest::collection::vec(arbitrary_registration(), 0..40),
        ) {
            let regs: Vec<Registration> = registrations
                .into_iter()
                .enumerate()
                .map(|(i, (machine, place))| reg(&format!("lead-{i}"), &machine, place))
                .collect();
            proptest::prop_assert_eq!(
                shown(&AddressBook::from_registrations(&regs)),
                quadratic_oracle(&regs)
            );
        }
    }

    /// Benchmark lane (`cargo test -p relay-core -- --ignored`): building the
    /// book for a 10 000-lead registry, a tenth of them sharing an address, stays
    /// far inside a generous bound. A quadratic build does 10^8 address
    /// comparisons here and takes many seconds; the linear one takes
    /// milliseconds. The bound is wall-clock, so this is not in the default lane.
    #[test]
    #[ignore = "benchmark lane: wall-clock bound, run with --ignored"]
    fn building_the_book_for_ten_thousand_leads_is_fast() {
        let regs: Vec<Registration> = (0..10_000)
            .map(|i| {
                // Every tenth lead shares its address with the one before it.
                let unique = i - i % 10 * usize::from(i % 10 == 1);
                reg(
                    &format!("lead-{i}"),
                    "hub",
                    Some(place(&format!("ws-{unique}"), "code", 2)),
                )
            })
            .collect();
        let started = std::time::Instant::now();
        let book = AddressBook::from_registrations(&regs);
        let elapsed = started.elapsed();
        assert!(book.by_name.len() < regs.len(), "some addresses are shared");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "10 000 leads took {elapsed:?}"
        );
    }

    #[test]
    fn a_registration_without_a_place_has_no_address() {
        let r = reg("a", "hub", None);
        assert!(r.full_address().is_none());
        assert!(
            AddressBook::from_registrations(&[r])
                .place_address(&name("a"))
                .is_none()
        );
    }

    #[test]
    fn the_book_signs_with_an_address_when_known_and_the_name_otherwise() {
        let book = AddressBook::from_registrations(&[
            reg("ea-lead", "hub", Some(place("EA", "1", 2))),
            reg("plain", "hub", None),
        ]);
        let msg = |from: &str| Message {
            from: from.parse().unwrap(),
            to: "x".parse().unwrap(),
            text: "hi".into(),
            sent_at_ms: 0,
        };
        assert_eq!(book.render(&msg("ea-lead")), "[ix-relay] from hub:EA:1: hi");
        assert_eq!(book.render(&msg("plain")), "[ix-relay] from plain: hi");
        assert_eq!(book.render(&msg("gone")), "[ix-relay] from gone: hi");
    }
}
