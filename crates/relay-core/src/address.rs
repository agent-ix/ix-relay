//! Public lead locations are `server:workspace:tab`.
//! Labels are preserved; public parsing rejects missing levels and unsafe text.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

use crate::{
    error::Error,
    ids::{MachineName, PaneId, SessionId, TabId, WorkspaceId},
    message::{Message, Registration},
    name::{AgentName, Target},
};

/// A verified Herdr location for one registered lead.
///
/// Stored on the [`Registration`] after Herdr confirms the location and its
/// harness session matches. Address resolution and listings use this verified
/// value without a Herdr call per send.
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
    /// Herdr's stable workspace and tab ids, plus when Herdr confirmed
    /// this location. Legacy label-only records are decoded as unplaced.
    pub herdr_ids: Option<HerdrPlaceIds>,
}

/// Herdr identity backing a human-readable [`Place`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HerdrPlaceIds {
    /// Stable identity of the workspace on this machine's Herdr instance.
    pub workspace: WorkspaceId,
    /// Stable identity of the tab within its workspace.
    pub tab: TabId,
    /// Herdr pane whose current location this identity describes.
    pub pane: PaneId,
    /// Harness session Herdr reported in the pane when this location was read.
    pub session_id: SessionId,
    /// Unix milliseconds when Herdr last confirmed these IDs and labels.
    pub verified_at_ms: i64,
}

impl HerdrPlaceIds {
    /// Whether this mapping is past the three-refresh-interval freshness window.
    #[must_use]
    pub fn is_stale(&self, now_ms: i64) -> bool {
        now_ms.saturating_sub(self.verified_at_ms) > 90_000
    }
}

impl Place {
    /// The human-readable server/workspace/tab prefix for this place.
    #[must_use]
    pub fn address(&self, machine: &MachineName) -> Address {
        self.tab_address(machine)
    }

    /// The preferred address for a tab, without a pane qualifier.
    #[must_use]
    pub fn tab_address(&self, machine: &MachineName) -> Address {
        Address::new(machine.as_str(), &self.workspace, Some(&self.tab))
    }
}

/// A Herdr location, with exactly three levels in every public address.
///
/// Equality and hashing ignore case — the one definition of "the same
/// address", so `hub:EA:Code:w1-p1` equals `HUB:ea:code:W1-P1`. Prefix queries
/// select wider Herdr containers. [`Address::selects`] is the query form of the same
/// comparison. Neither allocates.
#[derive(Debug, Clone)]
pub struct Address {
    server: String,
    workspace: String,
    tab: Option<String>,
    pane: Option<String>,
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
            pane: None,
        }
    }

    /// Builds an address down to an optional Herdr pane id.
    #[must_use]
    pub fn with_pane(server: &str, workspace: &str, tab: &str, pane: &str) -> Self {
        let mut address = Self::new(server, workspace, Some(tab));
        address.pane = Some(clean(pane));
        address
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

    /// The Herdr pane id, when the address names one.
    #[must_use]
    pub fn pane(&self) -> Option<&str> {
        self.pane.as_deref()
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
                (Some(want), Some(have)) if eq_fold(want, have) => match (&self.pane, &target.pane)
                {
                    (None, _) => true,
                    (Some(want), Some(have)) => eq_fold(want, have),
                    (Some(_), None) => false,
                },
                (Some(_), Some(_)) => false,
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
            && match (&self.pane, &other.pane) {
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
        self.pane.is_some().hash(state);
        if let Some(pane) = &self.pane {
            hash_fold(pane, state);
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
    raw.to_owned()
}

impl FromStr for Address {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let bad = || Error::InvalidAddress {
            input: s.to_owned(),
        };
        let parts: Vec<&str> = s.split(':').collect();
        if s.len() > MAX_LABEL_BYTES || s.chars().any(char::is_control) {
            return Err(bad());
        }
        match parts.as_slice() {
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
        if let Some(pane) = &self.pane {
            write!(f, ":{pane}")?;
        }
        Ok(())
    }
}

impl Registration {
    /// This lead's address with its tab always included — what an address
    /// query is matched against.
    #[must_use]
    pub fn full_address(&self) -> Option<Address> {
        let place = self.verified_place()?;
        Some(place.tab_address(&self.machine))
    }
}

/// Maximum public address length. Longer or ambiguous placements display as
/// unverified; enrollment rejects them without inventing a substitute name.
pub const MAX_LABEL_BYTES: usize = 96;

/// How to name each registered lead when printing: its address when known,
/// else an unverified marker. Built once per command from the registry.
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
    /// A lead's canonical displayed address always includes its tab, so the
    /// printed address can be pasted back into `send` without depending on a
    /// tab-count snapshot.
    ///
    /// Leads whose address another lead shares (two panes in one tab) are then
    /// printed as unverified: an address that names two leads cannot say who sent
    /// a message, and `send` would refuse it as ambiguous. So are those whose
    /// address is longer than [`MAX_LABEL_BYTES`].
    #[must_use]
    pub fn from_registrations(registrations: &[Registration]) -> Self {
        let placed: Vec<(&Registration, &Place)> = registrations
            .iter()
            .filter_map(|r| {
                let place = r.verified_place()?;
                Some((r, place))
            })
            .collect();
        let addressed: Vec<(&Registration, Address)> = placed
            .iter()
            .map(|(r, place)| (*r, place.tab_address(&r.machine)))
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

    /// A canonical location, the reserved human operator actor, or unverified.
    #[must_use]
    pub fn label(&self, name: &AgentName) -> String {
        self.address(name).map_or_else(
            || {
                if name.as_str() == crate::config::DEFAULT_HUMAN
                    || name.as_str().split(':').count() == 3
                {
                    name.to_string()
                } else {
                    "unverified".to_owned()
                }
            },
            ToString::to_string,
        )
    }

    /// `name` with its address for a confirmation line: `hub:EA:1 (ea-code)`,
    /// or just the name when it has no address.
    #[must_use]
    pub fn describe(&self, name: &AgentName) -> String {
        self.label(name)
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
            session_id: Some("test-session".parse().unwrap()),
            cwd: None,
            herdr_pane: Some("wB:p1".parse().unwrap()),
            place,
            last_seen_ms: 1,
        }
    }

    fn place(workspace: &str, tab: &str, _tabs: u32) -> Place {
        Place {
            workspace: workspace.into(),
            tab: tab.into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: format!("workspace-{workspace}").parse().unwrap(),
                tab: format!("workspace-{workspace}:t1").parse().unwrap(),
                pane: "wB:p1".parse().unwrap(),
                session_id: "test-session".parse().unwrap(),
                verified_at_ms: 1,
            }),
        }
    }

    #[test]
    fn parses_and_prints_optional_tab_and_pane() {
        let a: Address = "hub:EA:1".parse().unwrap();
        assert_eq!(
            (a.server(), a.workspace(), a.tab()),
            ("hub", "EA", Some("1"))
        );
        assert_eq!(a.to_string(), "hub:EA:1");
        assert!("hub:EA".parse::<Address>().is_err());
        assert!("hub:EA:code:wA-p1".parse::<Address>().is_err());
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for bad in ["", "hub", "hub:", ":EA", "hub::1", "a:b:c:d:", "hub:EA:"] {
            assert!(
                matches!(bad.parse::<Address>(), Err(Error::InvalidAddress { input }) if input == bad),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn the_server_segment_is_normalized_like_a_machine_name() {
        let a: Address = "Hub.local:EA:code".parse().unwrap();
        assert_eq!(a.server(), "hub");
    }

    #[test]
    fn whitespace_is_preserved_and_unaddressable_labels_are_rejected() {
        assert_eq!(
            Address::new("hub", "My Work", Some("code")).to_string(),
            "hub:My Work:code"
        );
        assert!("hub:My Work\nspace:code".parse::<Address>().is_err());
        assert!("hub:My Work:a:b".parse::<Address>().is_err());
    }

    #[test]
    fn selects_only_the_exact_tab_ignoring_case() {
        let target: Address = "hub:EA:code".parse().unwrap();
        assert!("HUB:ea:Code".parse::<Address>().unwrap().selects(&target));
        assert!(!"hub:EA:plan".parse::<Address>().unwrap().selects(&target));
        assert!(
            !"node-a:EA:code"
                .parse::<Address>()
                .unwrap()
                .selects(&target)
        );
        assert!(!"hub:QSL:code".parse::<Address>().unwrap().selects(&target));
    }

    #[test]
    fn the_shown_address_always_includes_the_tab() {
        let a = |regs: &[Registration]| AddressBook::from_registrations(regs).label(&name("a"));
        let one = reg("a", "node-a", Some(place("research", "1", 1)));
        assert_eq!(a(std::slice::from_ref(&one)), "node-a:research:1");
        assert_eq!(one.full_address().unwrap().to_string(), "node-a:research:1");
        let many = reg("a", "hub", Some(place("EA", "1", 3)));
        assert_eq!(a(&[many]), "hub:EA:1");
    }

    /// Tracing: TC-005-01
    /// ACs: FR-005-AC-1
    #[test]
    fn a_shared_tab_uses_herdr_pane_ids_to_disambiguate() {
        let mut a = reg("a", "hub", Some(place("EA", "code", 2)));
        let mut b = reg("b", "hub", Some(place("EA", "code", 2)));
        a.herdr_pane = Some("wA:p1".parse().unwrap());
        b.herdr_pane = Some("wA:p2".parse().unwrap());
        a.place.as_mut().unwrap().herdr_ids.as_mut().unwrap().pane = "wA:p1".parse().unwrap();
        b.place.as_mut().unwrap().herdr_ids.as_mut().unwrap().pane = "wA:p2".parse().unwrap();
        let registrations = [a, b];
        let book = AddressBook::from_registrations(&registrations);
        assert_eq!(book.label(&name("a")), "unverified");
        assert_eq!(book.label(&name("b")), "unverified");
        assert!(matches!(
            crate::resolve_target("hub:EA:code", &"hub".parse().unwrap(), &registrations),
            Err(Error::AmbiguousTarget { .. })
        ));
    }

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    /// Tracing: TC-005-08
    /// ACs: FR-005-AC-8
    ///
    /// Tab cardinality is not part of the stored address model. Leads in
    /// different tabs retain their tab-qualified addresses.
    #[test]
    fn tc_005_08_different_tabs_keep_distinct_tab_addresses() {
        let stale = reg("stale", "hub", Some(place("EA", "code", 1)));
        let fresh = reg("fresh", "hub", Some(place("EA", "plan", 2)));
        let book = AddressBook::from_registrations(&[stale.clone(), fresh]);
        assert_eq!(book.label(&name("stale")), "hub:EA:code");
        assert_eq!(book.label(&name("fresh")), "hub:EA:plan");
        assert_eq!(
            book.place_address(&name("stale")).map(ToString::to_string),
            Some("hub:EA:code".to_owned())
        );

        // The same address remains stable when alone or when another server has
        // the same workspace label.
        let alone = AddressBook::from_registrations(std::slice::from_ref(&stale));
        assert_eq!(alone.label(&name("stale")), "hub:EA:code");
        let other_machine = reg("far", "node-a", Some(place("EA", "plan", 2)));
        let apart = AddressBook::from_registrations(&[stale.clone(), other_machine]);
        assert_eq!(
            apart.label(&name("stale")),
            "hub:EA:code",
            "another server's EA does not change its canonical address"
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
        assert_eq!(book.label(&"long".parse().unwrap()), "unverified");
    }

    #[test]
    fn two_leads_sharing_an_address_are_shown_by_name_not_by_a_shared_address() {
        let book = AddressBook::from_registrations(&[
            reg("pane-one", "hub", Some(place("EA", "code", 2))),
            reg("pane-two", "hub", Some(place("ea", "Code", 2))),
            reg("alone", "hub", Some(place("EA", "plan", 2))),
        ]);
        assert_eq!(book.label(&"pane-one".parse().unwrap()), "unverified");
        assert_eq!(book.label(&"pane-two".parse().unwrap()), "unverified");
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
        assert!("hub:EA".parse::<Address>().is_err());
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
            .filter_map(|r| {
                let place = r.verified_place()?;
                Some((r, place))
            })
            .collect();
        let mut shown: Vec<(&Registration, Address)> = placed
            .iter()
            .map(|(r, place)| (*r, place.tab_address(&r.machine)))
            .filter(|(_, address)| address.to_string().len() <= MAX_LABEL_BYTES)
            .collect();
        let shared_tabs: HashSet<Address> = duplicated(shown.iter().map(|(_, address)| address))
            .into_iter()
            .cloned()
            .collect();
        for (registration, address) in &mut shown {
            if shared_tabs.contains(address)
                && let Some(full) = registration.full_address()
            {
                *address = full;
            }
        }
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
        let place = (label, label).prop_map(|(workspace, tab)| Place {
            herdr_ids: Some(HerdrPlaceIds {
                workspace: format!("workspace-{workspace}").parse().unwrap(),
                tab: format!("workspace-{workspace}:t1").parse().unwrap(),
                pane: "wB:p1".parse().unwrap(),
                session_id: "test-session".parse().unwrap(),
                verified_at_ms: 1,
            }),
            workspace,
            tab,
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

    /// Tracing: TC-005-10
    /// ACs: FR-005-AC-10
    #[test]
    fn a_registration_without_herdr_ids_is_not_advertised_as_an_address() {
        let mut registration = reg("legacy", "hub", Some(place("EA", "code", 2)));
        registration.place = None;
        let book = AddressBook::from_registrations(&[registration]);
        assert_eq!(book.label(&name("legacy")), "unverified");
        assert_eq!(book.place_address(&name("legacy")), None);
    }

    #[test]
    fn cache_freshness_uses_three_watcher_refresh_intervals() {
        let ids = HerdrPlaceIds {
            workspace: "w1".parse().unwrap(),
            tab: "w1:t1".parse().unwrap(),
            pane: "w1:p1".parse().unwrap(),
            session_id: "test-session".parse().unwrap(),
            verified_at_ms: 1_000,
        };
        assert!(!ids.is_stale(91_000));
        assert!(ids.is_stale(91_001));
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
        assert_eq!(book.render(&msg("plain")), "[ix-relay] from unverified: hi");
        assert_eq!(book.render(&msg("gone")), "[ix-relay] from unverified: hi");
    }
}
