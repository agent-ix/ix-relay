//! Deriving a lead's bus name from where it sits in herdr (AGE-2018): the
//! same name every time it comes back to the same tab or pane, so a session
//! needs no `AGENT_MSG_NAME` to be messageable.
//!
//! Pure data in, [`AgentName`] out — no herdr calls live here.
//! `relay-ops::naming` gathers the inputs (shelling out to herdr, working out
//! which tab is "first" and which pane is "second") and calls
//! [`derive_agent_name`] to turn them into a name.

use crate::name::{ALL, AgentName, MAX_NAME_LEN};

/// The prefix herdr labels an escalation's investigation-fork tab with
/// (`esc-<id>-<attempt>`; see `relay-ops::escalate::run_investigate`). A tab
/// with this prefix is skipped entirely: no derived name, no registration,
/// no delivery.
pub const ESCALATION_TAB_PREFIX: &str = "esc-";

/// The workspace a candidate pane sits in.
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceLocation<'a> {
    /// The workspace's own label, e.g. `QSL`.
    pub label: &'a str,
}

/// The tab a candidate pane sits in.
#[derive(Debug, Clone, Copy)]
pub struct TabLocation<'a> {
    /// The tab's label. herdr sets an unlabelled tab's label to its own
    /// *display* number rendered as a string (e.g. `"1"`) — see
    /// `TabLocation::is_labeled` (private). Display numbers renumber when a
    /// sibling tab closes, so this is only used to recognize that
    /// convention, never to build a name (see `id_number`).
    pub label: &'a str,
    /// herdr's own display number: what `label` equals when the tab is
    /// unlabelled. Not used for anything else — in particular, never for
    /// `is_first` or a name's `-t<N>` component, both of which need a
    /// number that never renumbers (see `id_number`).
    pub display_number: u64,
    /// The tab id's own numeric suffix (`<workspace>:t<N>` -> `N`): herdr
    /// never reuses or renumbers this, unlike `display_number`. This is
    /// what appears in a derived name's `-t<N>` component and what
    /// `is_first` is computed from (the operator's 2026-09-25 correction to
    /// AGE-2018: stable location names use herdr's own IDs, not display
    /// numbers).
    pub id_number: u64,
    /// Whether this is the workspace's first tab — the one with the lowest
    /// `id_number` among every tab in the workspace.
    pub is_first: bool,
}

impl TabLocation<'_> {
    /// Whether herdr shows an operator-chosen label, as opposed to "label
    /// equals the tab's own display number", which is how herdr renders an
    /// unlabelled tab.
    fn is_labeled(&self) -> bool {
        self.label != self.display_number.to_string()
    }

    /// Whether this is an escalation investigation-fork tab (see
    /// [`ESCALATION_TAB_PREFIX`]): skipped entirely, never named.
    #[must_use]
    pub fn is_escalation_fork(&self) -> bool {
        self.label.starts_with(ESCALATION_TAB_PREFIX)
    }
}

/// Where a candidate pane sits within its tab.
#[derive(Debug, Clone, Copy)]
pub struct PaneLocation {
    /// This pane's own id number (`<workspace>:p<N>` -> `N`): stable, never
    /// reused. Appears in a derived name's `-p<N>` component when the tab
    /// has more than one pane — see `is_lone`.
    pub id_number: u64,
    /// Whether this tab currently has exactly one pane (this one). A lone
    /// pane gets no `-p<N>` suffix at all, regardless of its `id_number` —
    /// a tab that used to have a lower-numbered sibling pane which has
    /// since closed still names its one remaining pane bare.
    pub is_lone: bool,
}

/// Derives a pane's bus name from its herdr location, per AGE-2018's naming
/// rule:
///
/// | Location | Name |
/// |---|---|
/// | first tab of a workspace labelled `QSL` | `qsl` |
/// | tab labelled `review` in `QSL` | `qsl-review` |
/// | unlabelled 2nd tab (tab id `t3`) | `qsl-t3` |
/// | 2nd (non-lone) pane in that tab (pane id `p5`) | `qsl-t3-p5` (1st tab's 2nd pane, id `p5`: `qsl-p5`) |
///
/// Both the tab and pane numbers that appear in a name are herdr's own,
/// never-reused *id* numbers, not the display numbers herdr renumbers when
/// a sibling closes — see [`TabLocation::id_number`] and
/// [`PaneLocation::id_number`].
///
/// `None` for a tab labelled `esc-*` (see
/// [`TabLocation::is_escalation_fork`]): an escalation investigation fork
/// gets no derived name, no registration and no delivery. The
/// [`AgentName`] this returns is always valid — never the reserved word
/// `all`, never over herdr's own name-length limit (see [`slugify`]'s own
/// proptest).
#[must_use]
pub fn derive_agent_name(
    workspace: &WorkspaceLocation<'_>,
    tab: &TabLocation<'_>,
    pane: &PaneLocation,
) -> Option<AgentName> {
    if tab.is_escalation_fork() {
        return None;
    }
    let mut parts = vec![workspace.label.to_owned()];
    if tab.is_labeled() {
        parts.push(tab.label.to_owned());
    } else if !tab.is_first {
        parts.push(format!("t{}", tab.id_number));
    }
    if !pane.is_lone {
        parts.push(format!("p{}", pane.id_number));
    }
    Some(slugify(&parts.join("-")))
}

/// Turns arbitrary text into a valid [`AgentName`]: lowercase, map anything
/// outside `[a-z0-9_-]` to `-`, collapse repeated `-`, trim leading/trailing
/// `-`, fall back to a fixed placeholder if that leaves nothing, prefix a
/// letter if the result doesn't start with one, dodge the reserved word
/// `all`, and cap at herdr's own name-length limit (`MAX_NAME_LEN`).
///
/// The result always parses as an [`AgentName`] for *any* input, including
/// the empty string, pure punctuation, and non-ASCII text — see this
/// module's `slugify_always_produces_a_valid_agent_name` proptest.
#[must_use]
pub fn slugify(raw: &str) -> AgentName {
    let body = slug_body(raw);
    let mut result = if body.is_empty() {
        "lead".to_owned()
    } else {
        body
    };
    if !result.starts_with(|c: char| c.is_ascii_lowercase()) {
        result.insert(0, 'a');
    }
    cap(&mut result);
    if result == ALL {
        result.push_str("-lead");
        cap(&mut result);
    }
    AgentName::from_constructed(result)
}

/// Lowercases `raw`, maps anything outside `[a-z0-9_-]` to `-`, collapses
/// repeated `-` and trims them from both ends. May be empty, may start with a
/// digit or `_`, and is not length-capped: [`slugify`] and [`with_suffix`]
/// each finish it in their own way.
fn slug_body(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_dash = false;
    for ch in raw.to_lowercase().chars() {
        let mapped = if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' {
            ch
        } else {
            '-'
        };
        if mapped == '-' {
            if last_was_dash {
                continue;
            }
            last_was_dash = true;
        } else {
            last_was_dash = false;
        }
        out.push(mapped);
    }
    out.trim_matches('-').to_owned()
}

/// `base` followed by `-<suffix>`, always a valid [`AgentName`] of at most
/// 32 characters, different from `base`, whose tail is the whole (cleaned)
/// suffix.
///
/// When the two do not fit, the *base* is truncated, never the suffix: the
/// suffix is what tells two colliding names apart, so cutting it off (as
/// [`slugify`] would on `"<32 chars>-2"`) hands back the very name that
/// collided. The suffix is cleaned like [`slugify`] input (`Node-A.local`
/// becomes `node-a-local`; an empty one becomes `x`) and is itself cut to
/// `MAX_NAME_LEN - 2` characters so at least one character of the base always
/// survives. If the truncation happens to rebuild `base` itself (a 31- or
/// 32-character base that already ends in `-<suffix>`), the last kept byte of
/// the base (the one next to the dash) is replaced by `a`, or by `b` when it
/// was `a`, so the result still differs and keeps the base's readable prefix;
/// the suffix is never touched.
///
/// Two variants of one base are distinct when their cleaned suffixes are not
/// dash-suffixes of one another: numbered variants (`-2`, `-3`, ...) are
/// always distinct, but a widening suffix that ends in `-<n>` (a machine named
/// `host-9`) can give the name `-9` gives, which [`free_variant`] skips.
#[must_use]
pub fn with_suffix(base: &AgentName, suffix: &str) -> AgentName {
    let mut suffix = slug_body(suffix);
    suffix.truncate(MAX_NAME_LEN - 2);
    let suffix = suffix.trim_end_matches('-');
    let suffix = if suffix.is_empty() { "x" } else { suffix };
    let room = MAX_NAME_LEN - 1 - suffix.len();
    let mut head = base.as_str().to_owned();
    head.truncate(room);
    // A name starts with a letter, so trimming trailing dashes leaves something.
    let head = head.trim_end_matches('-');
    let name = format!("{head}-{suffix}");
    if name != base.as_str() {
        return AgentName::from_constructed(name);
    }
    // A 31- or 32-byte `base` that already ends in `-<suffix>` is rebuilt
    // exactly by the cut and re-append (at 31 bytes the cut ends on the
    // separator, which the trim drops). A variant must differ and the suffix
    // stays whole, so replace the head's last byte, the one next to the
    // dash, with a letter: the readable prefix is kept, the name still starts
    // with a letter, and no dash is introduced. `name == base` means the head
    // is a non-empty prefix of `base`, so there is always a last byte to change.
    let (kept, last) = head.split_at(head.len().saturating_sub(1));
    let other = if last == "a" { 'b' } else { 'a' };
    AgentName::from_constructed(format!("{kept}{other}-{suffix}"))
}

/// The most numbered variants [`free_variant`] tries (`-2` to `-1000`).
pub const MAX_VARIANT_NUMBER: u32 = 1000;

/// Picks a name for a lead whose `candidate` is taken by someone else:
/// `<candidate>-<widen>` first when `widen` is given (a cross-machine
/// collision widens by machine name), then `<candidate>-2`, `-3`, ... — each
/// built with [`with_suffix`] so the variant is never the colliding
/// `candidate` again — and returns the first for which `usable` says yes. A
/// numbered variant equal to the widened name (a machine name ending in
/// `-<n>`) is not asked about a second time: `usable` sees each name once.
///
/// `None` when [`MAX_VARIANT_NUMBER`] variants were all unusable, so the
/// search is bounded and never hands back a taken name. `usable` may fail
/// (it is normally a registry lookup); that error is returned as is.
///
/// # Errors
///
/// Whatever `usable` returns.
pub fn free_variant<E>(
    candidate: &AgentName,
    widen: Option<&str>,
    mut usable: impl FnMut(&AgentName) -> Result<bool, E>,
) -> Result<Option<AgentName>, E> {
    let widened = widen.map(|widen| with_suffix(candidate, widen));
    if let Some(widened) = &widened
        && usable(widened)?
    {
        return Ok(Some(widened.clone()));
    }
    for n in 2..=MAX_VARIANT_NUMBER {
        let numbered = with_suffix(candidate, &n.to_string());
        // A widening suffix that ends in `-<n>` gives the widened name again.
        if widened.as_ref() == Some(&numbered) {
            continue;
        }
        if usable(&numbered)? {
            return Ok(Some(numbered));
        }
    }
    Ok(None)
}

/// Truncates `s` to [`MAX_NAME_LEN`] (byte-index truncation is char-safe
/// here: every char `slugify` can produce is single-byte ASCII), then trims
/// a trailing `-` truncation may have exposed. Cosmetic only — herdr/
/// [`AgentName`] allow a trailing `-`, but a name ending mid-thought reads
/// worse than one trimmed cleanly.
fn cap(s: &mut String) {
    if s.len() > MAX_NAME_LEN {
        s.truncate(MAX_NAME_LEN);
        while s.ends_with('-') {
            s.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn ws(label: &str) -> WorkspaceLocation<'_> {
        WorkspaceLocation { label }
    }

    fn tab<'a>(
        label: &'a str,
        display_number: u64,
        id_number: u64,
        is_first: bool,
    ) -> TabLocation<'a> {
        TabLocation {
            label,
            display_number,
            id_number,
            is_first,
        }
    }

    fn pane(id_number: u64, is_lone: bool) -> PaneLocation {
        PaneLocation { id_number, is_lone }
    }

    #[test]
    fn first_tab_first_pane_is_just_the_workspace_slug() {
        let name = derive_agent_name(&ws("QSL"), &tab("1", 1, 1, true), &pane(1, true)).unwrap();
        assert_eq!(name.as_str(), "qsl");
    }

    #[test]
    fn labeled_tab_appends_its_label() {
        let name =
            derive_agent_name(&ws("QSL"), &tab("review", 2, 2, false), &pane(1, true)).unwrap();
        assert_eq!(name.as_str(), "qsl-review");
    }

    #[test]
    fn a_labeled_first_tab_still_uses_its_label_not_bare_workspace() {
        // A first tab that also carries an explicit label is named by that
        // label, same as any other labeled tab — "first tab" only matters
        // for an *unlabelled* tab.
        let name =
            derive_agent_name(&ws("QSL"), &tab("review", 1, 1, true), &pane(1, true)).unwrap();
        assert_eq!(name.as_str(), "qsl-review");
    }

    #[test]
    fn unlabelled_second_tab_uses_its_stable_id_number_not_its_display_number() {
        // Display number 2 (herdr renumbers these), tab id number 3 (a
        // sibling with id number 2 already closed) — the name must use the
        // stable id number, 3, not the display number.
        let name = derive_agent_name(&ws("QSL"), &tab("2", 2, 3, false), &pane(1, true)).unwrap();
        assert_eq!(name.as_str(), "qsl-t3");
    }

    #[test]
    fn non_lone_pane_uses_its_own_stable_id_number() {
        let name = derive_agent_name(&ws("QSL"), &tab("2", 2, 2, false), &pane(5, false)).unwrap();
        assert_eq!(name.as_str(), "qsl-t2-p5");
    }

    #[test]
    fn non_lone_pane_in_the_first_tab() {
        let name = derive_agent_name(&ws("QSL"), &tab("1", 1, 1, true), &pane(5, false)).unwrap();
        assert_eq!(name.as_str(), "qsl-p5");
    }

    #[test]
    fn a_lone_pane_gets_no_suffix_even_with_a_high_id_number() {
        // A sibling pane with a lower id closed, leaving this one alone in
        // the tab: still no `-p<N>` suffix, regardless of its own id.
        let name = derive_agent_name(&ws("QSL"), &tab("1", 1, 1, true), &pane(9, true)).unwrap();
        assert_eq!(name.as_str(), "qsl");
    }

    #[test]
    fn non_lone_pane_in_a_labeled_tab() {
        let name =
            derive_agent_name(&ws("QSL"), &tab("review", 2, 2, false), &pane(5, false)).unwrap();
        assert_eq!(name.as_str(), "qsl-review-p5");
    }

    #[test]
    fn esc_prefixed_tabs_are_skipped_entirely() {
        assert!(
            derive_agent_name(&ws("QSL"), &tab("esc-e7-1", 3, 3, false), &pane(1, true)).is_none()
        );
        // Even as the (otherwise) first tab, or with a pane suffix.
        assert!(
            derive_agent_name(&ws("QSL"), &tab("esc-e7-1", 1, 1, true), &pane(2, false)).is_none()
        );
    }

    #[test]
    fn workspace_labels_are_slugified() {
        let name = derive_agent_name(
            &ws("My Cool Workspace!!"),
            &tab("1", 1, 1, true),
            &pane(1, true),
        )
        .unwrap();
        assert_eq!(name.as_str(), "my-cool-workspace");
    }

    #[test]
    fn a_very_long_combined_name_is_capped_at_32() {
        let long_label = "a".repeat(40);
        let name = derive_agent_name(
            &ws(&long_label),
            &tab("review", 2, 2, false),
            &pane(5, false),
        )
        .unwrap();
        assert!(name.as_str().len() <= 32, "{name}");
        assert!(name.as_str().starts_with("aaaa"));
    }

    // ---- slugify ---------------------------------------------------------

    #[test]
    fn slugify_examples() {
        assert_eq!(slugify("QSL").as_str(), "qsl");
        assert_eq!(slugify("My Cool Workspace!!").as_str(), "my-cool-workspace");
        assert_eq!(slugify("---").as_str(), "lead");
        assert_eq!(slugify("").as_str(), "lead");
        assert_eq!(slugify("1lead").as_str(), "a1lead");
        assert_eq!(slugify("_lead").as_str(), "a_lead");
        assert_eq!(slugify("all").as_str(), "all-lead");
        assert_eq!(slugify("ALL").as_str(), "all-lead");
        assert_eq!(slugify("a  b   c").as_str(), "a-b-c");
        assert_eq!(slugify(&"x".repeat(50)).as_str().len(), 32);
    }

    // ---- with_suffix / free_variant ---------------------------------------

    #[test]
    fn with_suffix_truncates_the_base_so_the_suffix_survives() {
        let full = slugify(&"a".repeat(40));
        assert_eq!(full.as_str().len(), 32);
        assert_eq!(
            with_suffix(&full, "2").as_str(),
            format!("{}-2", "a".repeat(30))
        );
        assert_eq!(
            with_suffix(&slugify("qsl"), "Node-A.local").as_str(),
            "qsl-node-a-local"
        );
        assert_eq!(with_suffix(&slugify("qsl"), "").as_str(), "qsl-x");
        // A base whose cut lands on a dash does not leave "--".
        let dashed = slugify(&format!("{}-b", "a".repeat(29)));
        assert_eq!(
            with_suffix(&dashed, "22").as_str(),
            format!("{}-22", "a".repeat(29))
        );
    }

    proptest! {
        /// Whatever the candidate, the taken set (which always includes the
        /// candidate itself and the variants a naive scheme would pick) and
        /// the machine, the result is a valid name outside the taken set.
        #[test]
        fn free_variant_never_returns_a_taken_name(
            raw in ".{0,80}",
            machine in ".{0,50}",
            widen in any::<bool>(),
            taken_count in 0u32..40,
            extra in proptest::collection::vec("[a-z][a-z0-9-]{0,31}", 0..20),
        ) {
            let candidate = slugify(&raw);
            let widen_with = widen.then_some(machine.as_str());
            let mut taken: std::collections::HashSet<String> = extra.into_iter().collect();
            taken.insert(candidate.as_str().to_owned());
            taken.insert(with_suffix(&candidate, &machine).as_str().to_owned());
            for n in 2..2 + taken_count {
                taken.insert(with_suffix(&candidate, &n.to_string()).as_str().to_owned());
            }
            let got = free_variant::<()>(&candidate, widen_with, |n| Ok(!taken.contains(n.as_str())))
                .unwrap()
                .expect("fewer than MAX_VARIANT_NUMBER names are taken");
            prop_assert!(got.as_str().parse::<AgentName>().is_ok());
            prop_assert!(!taken.contains(got.as_str()));
            prop_assert_ne!(got.as_str(), candidate.as_str());
        }

        /// `with_suffix` keeps the suffix intact and the name in bounds.
        #[test]
        fn with_suffix_keeps_the_suffix_and_the_length_bound(raw in ".{0,80}", n in 2u32..100_000) {
            let base = slugify(&raw);
            let got = with_suffix(&base, &n.to_string());
            prop_assert!(got.as_str().len() <= MAX_NAME_LEN);
            let expected_tail = format!("-{n}");
            prop_assert!(got.as_str().ends_with(&expected_tail));
            prop_assert!(got.as_str().starts_with(|c: char| c.is_ascii_lowercase()));
            prop_assert_ne!(got.as_str(), base.as_str());
        }

        /// Exhausting every variant is reported, not papered over with a taken name.
        #[test]
        fn free_variant_gives_up_when_everything_is_taken(raw in ".{0,40}") {
            let candidate = slugify(&raw);
            let got = free_variant::<()>(&candidate, None, |_| Ok(false)).unwrap();
            prop_assert!(got.is_none());
        }
    }

    proptest! {
        /// The whole point of slugify: no matter what text comes back from
        /// herdr (a workspace/tab label an operator typed by hand, any
        /// Unicode, any length), the result always parses as a valid
        /// AgentName. This is what lets `derive_agent_name` return `Some`
        /// unconditionally (aside from the esc-* skip) instead of a
        /// fallible result a caller has to handle.
        #[test]
        fn slugify_always_produces_a_valid_agent_name(raw in ".{0,200}") {
            let name = slugify(&raw);
            prop_assert!(name.as_str().parse::<AgentName>().is_ok());
            prop_assert!(name.as_str().len() <= MAX_NAME_LEN);
            prop_assert_ne!(name.as_str(), ALL);
        }

        /// Same property, but composed the way `derive_agent_name` actually
        /// builds its input: a workspace label plus an optional tab/pane
        /// suffix, joined with '-', arbitrary text throughout.
        #[test]
        fn derive_agent_name_always_produces_a_valid_agent_name(
            workspace_label in ".{1,60}",
            tab_label in ".{1,30}",
            display_number in 1u64..1000,
            id_number in 1u64..1000,
            is_first in any::<bool>(),
            pane_id_number in 1u64..1000,
            is_lone in any::<bool>(),
        ) {
            let name = derive_agent_name(
                &WorkspaceLocation { label: &workspace_label },
                &TabLocation { label: &tab_label, display_number, id_number, is_first },
                &PaneLocation { id_number: pane_id_number, is_lone },
            );
            if let Some(name) = name {
                prop_assert!(name.as_str().parse::<AgentName>().is_ok());
                prop_assert!(name.as_str().len() <= MAX_NAME_LEN);
            }
            // else: an esc-* tab label, which is a valid "skip" outcome.
        }
    }
}
