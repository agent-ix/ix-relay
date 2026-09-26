//! Evidence for derived bus names (FR-021) and collision variants
//! (FR-022's pure half: `with_suffix` and `free_variant`; the registry half
//! is `relay-ops/tests/collision.rs`).
//!
//! `slugify` and `with_suffix` are total functions over strings, so besides
//! generated text they are checked *exhaustively* over every string up to a
//! length bound drawn from an alphabet with one representative per character
//! class the functions distinguish. That is a proof over the bound; a Kani
//! harness would lift it to every string (see `spec/reviews/evidence.md`).

use proptest::prelude::*;
use relay_core::{
    AgentName,
    naming::{
        ESCALATION_TAB_PREFIX, MAX_VARIANT_NUMBER, PaneLocation, TabLocation, WorkspaceLocation,
        derive_agent_name, free_variant, slugify, with_suffix,
    },
};

/// One representative per class `slugify` treats differently: lowercase and
/// uppercase ASCII letters, a digit, `_`, `-`, space, a non-ASCII letter
/// whose lowercase is multi-character (U+0130), a non-ASCII lowercase letter,
/// NUL, and an astral scalar.
const CLASSES: [char; 10] = [
    'a',
    'Z',
    '7',
    '_',
    '-',
    ' ',
    '\u{130}',
    '\u{e9}',
    '\0',
    '\u{1F600}',
];

/// Every string over `alphabet` with at most `max_len` characters.
fn strings_over(alphabet: &[char], max_len: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..max_len {
        layer = layer
            .iter()
            .flat_map(|s| alphabet.iter().map(move |c| format!("{s}{c}")))
            .collect();
        all.extend(layer.iter().cloned());
    }
    all
}

/// The property of FR-021-AC-1, shared by the always-run tests and the fuzz lane.
fn check_slug_is_a_valid_bus_name(raw: &str) -> Result<(), TestCaseError> {
    let slug = slugify(raw);
    let s = slug.as_str();
    prop_assert!(s.parse::<AgentName>().is_ok(), "{s:?} from {raw:?}");
    prop_assert!(s.len() <= 32, "{s:?} from {raw:?}");
    prop_assert!(s != "all");
    prop_assert!(s.starts_with(|c: char| c.is_ascii_lowercase()));
    prop_assert!(
        s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    );
    Ok(())
}

/// Tracing: TC-021-01, TC-021-05
/// ACs: FR-021-AC-1, FR-021-AC-5
///
/// Exhaustive over three alphabets, each a proof over its own bound only: every
/// string of up to four characters over the ten character classes above
/// (11,111 strings); every string of up to five characters over
/// `a`, `l`, `A`, `L`, `-`, `1` (9,331 strings, which spell `all`, `ALL`,
/// `a-l-l`, `all-` and `1all`); and every 26 to 40 character run of `a`
/// followed by every string of up to three characters over `a`, `l`, `-`
/// (so the 32-byte cap falls on every combination of letter and dash, and on
/// `all`). Each result is a valid bus name of at most 32 bytes that is not
/// `all`, and a fixed point of `slugify`. It is not a proof over every
/// character class or length; a Kani harness would be.
#[test]
fn tc_021_01_every_short_string_slugs_to_a_valid_fixed_point() {
    let mut inputs = strings_over(&CLASSES, 4);
    inputs.extend(strings_over(&['a', 'l', 'A', 'L', '-', '1'], 5));
    let tails = strings_over(&['a', 'l', '-'], 3);
    for run in 26..=40 {
        for tail in &tails {
            inputs.push(format!("{}{tail}", "a".repeat(run)));
        }
    }
    assert!(inputs.iter().any(|raw| slugify(raw).as_str() == "all-lead"));
    assert!(inputs.iter().any(|raw| slugify(raw).as_str().len() == 32));
    for raw in inputs {
        check_slug_is_a_valid_bus_name(&raw).unwrap_or_else(|e| panic!("{raw:?}: {e}"));
        let once = slugify(&raw);
        assert_eq!(
            slugify(once.as_str()),
            once,
            "not a fixed point for {raw:?}"
        );
    }
}

/// Tracing: TC-021-01, TC-021-05
/// ACs: FR-021-AC-1, FR-021-AC-5
///
/// Every string length from 0 to 80 built from one repeated class character,
/// which walks the 32-byte cap and the `all` and leading-character rules
/// across every boundary.
#[test]
fn tc_021_01_the_length_cap_holds_at_every_length_for_every_class() {
    for c in CLASSES {
        for n in 0..=80 {
            let raw: String = std::iter::repeat_n(c, n).collect();
            check_slug_is_a_valid_bus_name(&raw).unwrap_or_else(|e| panic!("{raw:?}: {e}"));
            let once = slugify(&raw);
            assert_eq!(slugify(once.as_str()), once, "{raw:?}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// Tracing: TC-021-01
    /// ACs: FR-021-AC-1
    #[test]
    fn tc_021_01_every_slug_is_a_valid_bus_name(raw in prop_oneof![".{0,200}", "[a-zA-Z0-9 _.:/-]{0,80}"]) {
        check_slug_is_a_valid_bus_name(&raw)?;
    }

    /// Tracing: TC-021-05
    /// ACs: FR-021-AC-5
    #[test]
    fn tc_021_05_slugify_is_idempotent(raw in prop_oneof![".{0,200}", "[a-zA-Z0-9 _.:/-]{0,80}"]) {
        let once = slugify(&raw);
        prop_assert_eq!(slugify(once.as_str()), once);
    }

    /// Tracing: TC-021-02
    /// ACs: FR-021-AC-2
    ///
    /// A derived name is a valid bus name, or absent exactly when the tab
    /// label starts with `esc-`. (The pre-existing unit test only checked the
    /// "valid when present" half.)
    #[test]
    fn tc_021_02_a_derived_name_is_valid_or_absent_exactly_for_an_escalation_fork(
        workspace in prop_oneof![".{0,40}", "[A-Za-z0-9 -]{0,12}"],
        tab in prop_oneof![".{0,30}", "[a-z0-9-]{0,8}", "esc-[a-z0-9]{0,6}"],
        display_number in 0u64..1000, id_number in 0u64..1000,
        is_first in any::<bool>(), pane_id in 0u64..1000, is_lone in any::<bool>(),
    ) {
        let name = derive_agent_name(
            &WorkspaceLocation { label: &workspace },
            &TabLocation { label: &tab, display_number, id_number, is_first },
            &PaneLocation { id_number: pane_id, is_lone },
        );
        let fork = tab.starts_with(ESCALATION_TAB_PREFIX);
        prop_assert_eq!(name.is_none(), fork);
        if let Some(name) = name {
            prop_assert!(name.as_str().parse::<AgentName>().is_ok());
            prop_assert!(name.as_str().len() <= 32);
        }
    }

    /// Tracing: TC-021-02
    /// ACs: FR-021-AC-2
    ///
    /// The tab and pane numbers in a name come from the never-reused id
    /// numbers, never the display number (FR-021's behaviour): changing only
    /// the display number of an unlabelled tab never changes the name.
    #[test]
    fn tc_021_02_the_display_number_never_changes_the_name(
        workspace in "[A-Za-z]{1,8}", id_number in 1u64..500,
        is_first in any::<bool>(), pane_id in 1u64..500, is_lone in any::<bool>(),
        d1 in 1u64..500, d2 in 1u64..500,
    ) {
        let name = |display_number: u64| {
            derive_agent_name(
                &WorkspaceLocation { label: &workspace },
                &TabLocation { label: &display_number.to_string(), display_number, id_number, is_first },
                &PaneLocation { id_number: pane_id, is_lone },
            )
        };
        prop_assert_eq!(name(d1), name(d2));
    }

    /// Tracing: TC-022-01
    /// ACs: FR-022-AC-1
    ///
    /// `with_suffix` (the constructor of every widened or numbered
    /// candidate) yields a valid bus name of at most 32 bytes that ends with
    /// the cleaned suffix and differs from its base, for any base and suffix.
    #[test]
    fn tc_022_01_a_suffixed_name_is_valid_short_and_ends_with_the_suffix(
        base in ".{0,60}", suffix in prop_oneof![".{0,60}", "[a-z0-9-]{0,40}", "[0-9]{1,4}"],
    ) {
        let base = slugify(&base);
        let got = with_suffix(&base, &suffix);
        prop_assert!(got.as_str().parse::<AgentName>().is_ok());
        prop_assert!(got.as_str().len() <= 32);
        prop_assert_ne!(&got, &base, "a variant must differ from the name it varies");
        // A suffix that cleans to nothing becomes `x`; a valid one survives whole
        // (up to 30 bytes), whatever the base.
        if !suffix.is_empty() && suffix.len() <= 30 && suffix.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) {
            prop_assert!(got.as_str().ends_with(&format!("-{suffix}")), "{got} vs {suffix}");
        }
    }

    /// Tracing: TC-022-01
    /// ACs: FR-022-AC-1
    ///
    /// `free_variant` never returns a name the predicate refused, however many
    /// it refuses, and gives up (`None`) rather than return a refused one.
    #[test]
    fn tc_022_01_free_variant_returns_only_a_name_the_caller_accepted(
        candidate in "[a-z][a-z0-9-]{0,31}", widen in proptest::option::of("[a-zA-Z.]{0,12}"),
        refuse_first in 0u32..1200, refuse_widened in any::<bool>(),
    ) {
        let candidate: AgentName = candidate.parse().unwrap();
        let mut asked = Vec::new();
        let got = free_variant::<()>(&candidate, widen.as_deref(), |n| {
            asked.push(n.clone());
            let ordinal = u32::try_from(asked.len()).unwrap();
            Ok(!(ordinal <= refuse_first || (refuse_widened && ordinal == 1 && widen.is_some())))
        }).unwrap();
        // Every name is asked about at most once per attempt, at most 1000
        // variants plus the widened one are tried.
        prop_assert!(asked.len() <= MAX_VARIANT_NUMBER as usize);
        match got {
            Some(name) => {
                prop_assert_eq!(asked.last(), Some(&name), "the accepted name is the last one asked about");
                prop_assert_ne!(&name, &candidate);
            }
            None => prop_assert!(asked.len() >= MAX_VARIANT_NUMBER as usize - 1),
        }
    }
}

/// What every `with_suffix` result must satisfy, whatever the base: a valid
/// bus name of at most 32 bytes, different from the base, ending in the whole
/// cleaned suffix, starting with a letter, and containing no `--` the base
/// did not already have.
fn check_variant(base: &AgentName, tail: &str, got: &AgentName) {
    let s = got.as_str();
    assert!(s.parse::<AgentName>().is_ok(), "{base} + {tail}: {s}");
    assert!(s.len() <= 32, "{base} + {tail}: {s}");
    assert_ne!(got, base, "{base} + {tail}");
    assert!(s.ends_with(&format!("-{tail}")), "{base} + {tail}: {s}");
    assert!(s.starts_with(|c: char| c.is_ascii_lowercase()), "{s}");
    assert_ne!(s, "all", "{base} + {tail}");
    // When the cut rebuilt the base, only the byte next to the dash changed:
    // the readable prefix is kept.
    if s.len() == base.as_str().len() && base.as_str().ends_with(&format!("-{tail}")) {
        let kept = s.len() - tail.len() - 2;
        assert!(
            s.chars().take(kept).eq(base.as_str().chars().take(kept)),
            "{base} + {tail}: {s}"
        );
    }
    assert!(
        !s.contains("--") || base.as_str().contains("--"),
        "{base} + {tail}: {s}"
    );
}

/// Tracing: TC-022-01, TC-022-10
/// ACs: FR-022-AC-1, FR-022-AC-10
///
/// Exhaustive over the base-length and suffix-length grid: for every base
/// length from 1 to 32 (all letters, with a `-` at every interior position so
/// truncation can expose a trailing dash, and, for lengths 28 to 32, every
/// base that already ends in the suffix being added: `<a...>-<b...>`) and
/// every suffix length from 0 to 40, the result is a valid name of at most 32
/// bytes, differs from the base, and ends with the whole suffix (cut to 30
/// bytes at most). A proof over that grid; the invariant is a pure integer
/// property of the two lengths, which Kani would prove for all lengths.
/// Includes the widen-path inputs `a-` plus a 29 or 30 byte suffix.
#[test]
fn tc_022_01_with_suffix_holds_the_length_invariant_over_the_whole_grid() {
    let tail_of = |suffix_len: usize| {
        if suffix_len == 0 {
            "x".to_owned()
        } else {
            "b".repeat(suffix_len.clamp(1, 30))
        }
    };
    for base_len in 1..=32 {
        let mut bases = vec!["a".repeat(base_len)];
        for dash in 1..base_len.saturating_sub(1) {
            let mut b = "a".repeat(base_len);
            b.replace_range(dash..=dash, "-");
            bases.push(b);
        }
        // Bases that already end in `-<suffix>`.
        if base_len >= 28 {
            for kept in 1..=30usize.min(base_len - 2) {
                bases.push(format!(
                    "{}-{}",
                    "a".repeat(base_len - 1 - kept),
                    "b".repeat(kept)
                ));
            }
        }
        for base in bases {
            let base: AgentName = base.parse().unwrap();
            for suffix_len in 0..=40 {
                let got = with_suffix(&base, &"b".repeat(suffix_len));
                check_variant(&base, &tail_of(suffix_len), &got);
            }
        }
    }
    for kept in [29, 30] {
        let base: AgentName = format!("a-{}", "b".repeat(kept)).parse().unwrap();
        check_variant(
            &base,
            &"b".repeat(kept),
            &with_suffix(&base, &"b".repeat(kept)),
        );
    }
}

/// The bases of the numbered-path sweep, each with the n values to suffix it
/// with: every string over `{a, 0, 2, -}` of length 1 to 7 that starts with
/// `a`; for every length 8 to 32 an `a` run followed by every 5-byte tail over
/// that alphabet; and every `<prefix>-<n>` base (prefixes of `a`, of `z`, and
/// with an interior dash) at lengths 28 to 32 for n = 1 to 1000. `full` gives
/// every base every n from 1 to 1000; otherwise the bases numbered 2, 10, 100,
/// 999 and 1000 get every n, the `<prefix>-<n>` bases n up to 12 and their own
/// n, 999 and 1000, and the others n up to 40 and 99, 100, 999, 1000.
fn sweep_bases(full: bool) -> Vec<(String, Vec<u32>)> {
    let alphabet = ['a', '0', '2', '-'];
    let all_n: Vec<u32> = (1..=1000).collect();
    let some_n: Vec<u32> = if full {
        all_n.clone()
    } else {
        (1..=40).chain([99, 100, 999, 1000]).collect()
    };
    let mut bases: Vec<(String, Vec<u32>)> = Vec::new();
    for s in strings_over(&alphabet, 6) {
        bases.push((format!("a{s}"), some_n.clone()));
    }
    let tails: Vec<String> = strings_over(&alphabet, 5)
        .into_iter()
        .filter(|t| t.len() == 5)
        .collect();
    for len in 8..=32 {
        for tail in &tails {
            bases.push((format!("{}{tail}", "a".repeat(len - 5)), some_n.clone()));
        }
    }
    for len in 28..=32usize {
        for n in 1..=1000u32 {
            let digits = n.to_string().len();
            let Some(prefix_len) = len.checked_sub(digits + 1).filter(|l| *l >= 1) else {
                continue;
            };
            for letter in ['a', 'z'] {
                let plain = letter.to_string().repeat(prefix_len);
                let mut dashed = plain.clone();
                if prefix_len > 2 {
                    dashed.replace_range(1..2, "-");
                }
                for prefix in [plain, dashed] {
                    let mut ns: Vec<u32> = if full || [2, 10, 100, 999, 1000].contains(&n) {
                        all_n.clone()
                    } else {
                        (1..=12).chain([n, 999, 1000]).collect()
                    };
                    ns.sort_unstable();
                    ns.dedup();
                    bases.push((format!("{prefix}-{n}"), ns));
                }
            }
        }
    }
    bases
}

/// Every result for every `(base, n)` satisfies `check_variant`, and the
/// results for one base are pairwise distinct across n.
fn check_sweep(bases: Vec<(String, Vec<u32>)>) {
    for (base, ns) in bases {
        let Ok(base) = base.parse::<AgentName>() else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for n in ns {
            let got = with_suffix(&base, &n.to_string());
            check_variant(&base, &n.to_string(), &got);
            assert!(seen.insert(got.clone()), "{base}: {n} repeats {got}");
        }
    }
}

/// Tracing: TC-022-01, TC-022-10
/// ACs: FR-022-AC-1, FR-022-AC-10
///
/// Bounded-exhaustive over the numbered path `free_variant` uses, over the
/// bases and n values of `sweep_bases(false)`: every result is valid, at most
/// 32 bytes, differs from the base, ends in the whole `-<n>`, keeps the base's
/// prefix when the cut rebuilt it, introduces no `--`, is never `all`, and
/// the results for one base are pairwise distinct across n.
#[test]
fn tc_022_01_numbered_variants_are_valid_different_and_distinct_for_every_base_shape() {
    check_sweep(sweep_bases(false));
}

/// Tracing: TC-022-01, TC-022-10
/// ACs: FR-022-AC-1, FR-022-AC-10
///
/// The same sweep with every n from 1 to 1000 for every base (about 32 million
/// `with_suffix` calls, minutes in a debug build), so it runs in the fuzz lane
/// (`make fuzz`), not in `make test`.
#[test]
#[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
fn fuzz_with_suffix_every_n_for_every_base_shape() {
    check_sweep(sweep_bases(true));
}

/// Tracing: TC-022-01, TC-022-10
/// ACs: FR-022-AC-1, FR-022-AC-10
///
/// Regression for the flaky `tc_022_01_free_variant_returns_only_a_name_the_caller_accepted`
/// (proptest shrank it to the 31-byte candidate `aa0--a-0a-aa0aa0-a-0-a0-0-00a-2`,
/// widen `None`, nothing refused). A 31 or 32 byte candidate that already ends
/// in the suffix being added ("...-2") used to come back unchanged, because
/// the base was cut and `-2` re-appended: `free_variant` handed back the very
/// name that collided. Also the widening cases at the extreme (a 29 or 30 byte
/// suffix leaving one byte of base), where the suffix must still stay whole,
/// and `check_variant`'s rule that only the byte next to the dash changes.
#[test]
fn tc_022_01_a_variant_never_equals_a_31_or_32_byte_base_ending_in_the_suffix() {
    let cases = [
        ("aa0--a-0a-aa0aa0-a-0-a0-0-00a-2".to_owned(), "2".to_owned()),
        (format!("{}-8", "a".repeat(29)), "8".to_owned()),
        (format!("{}-8", "a".repeat(30)), "8".to_owned()),
        (format!("a-{}", "b".repeat(30)), "b".repeat(30)),
        (format!("a-{}", "b".repeat(29)), "b".repeat(29)),
    ];
    for (base, suffix) in cases {
        let base: AgentName = base.parse().unwrap();
        check_variant(&base, &suffix, &with_suffix(&base, &suffix));
    }
    let candidate: AgentName = "aa0--a-0a-aa0aa0-a-0-a0-0-00a-2".parse().unwrap();
    let got = free_variant::<()>(&candidate, None, |_| Ok(true))
        .unwrap()
        .unwrap();
    assert_ne!(got, candidate);
}

/// Tracing: TC-022-01
/// ACs: FR-022-AC-1
///
/// A widening suffix that ends in `-<n>` (a machine named `0-9`) makes the
/// widened name equal to the numbered variant `-9`: base 27 `a` plus `-0-zz`
/// gives `<27 a>-0-9` both ways. `free_variant` must ask about each name once,
/// so the widened name is not asked again as `-9` (999 asks, not 1000), and no
/// name is ever asked twice.
#[test]
fn tc_022_01_free_variant_asks_each_name_once_when_the_widened_name_is_a_numbered_variant() {
    let base: AgentName = format!("{}-0-zz", "a".repeat(27)).parse().unwrap();
    assert_eq!(with_suffix(&base, "0-9"), with_suffix(&base, "9"));
    let mut asked: Vec<AgentName> = Vec::new();
    let got = free_variant::<()>(&base, Some("0-9"), |n| {
        asked.push(n.clone());
        Ok(false)
    })
    .unwrap();
    assert!(got.is_none());
    let distinct: std::collections::HashSet<_> = asked.iter().collect();
    assert_eq!(distinct.len(), asked.len(), "a name was asked twice");
    assert_eq!(asked.len(), MAX_VARIANT_NUMBER as usize - 1);
}

/// Tracing: TC-021-03
/// ACs: FR-021-AC-3
#[test]
fn tc_021_03_the_named_locations_of_workspace_qsl() {
    let ws = WorkspaceLocation { label: "QSL" };
    let lone = PaneLocation {
        id_number: 1,
        is_lone: true,
    };
    // herdr labels an unlabelled tab with its own display number.
    let (one, three) = ("1", "3");
    let unlabelled = |label: &'static str, n: u64, first: bool| TabLocation {
        label,
        display_number: n,
        id_number: n,
        is_first: first,
    };
    let name = |tab: &TabLocation<'_>, pane: &PaneLocation| {
        derive_agent_name(&ws, tab, pane).map(|n| n.to_string())
    };
    assert_eq!(
        name(&unlabelled(one, 1, true), &lone).as_deref(),
        Some("qsl")
    );
    let review = TabLocation {
        label: "review",
        display_number: 2,
        id_number: 2,
        is_first: false,
    };
    assert_eq!(name(&review, &lone).as_deref(), Some("qsl-review"));
    assert_eq!(
        name(&unlabelled(three, 3, false), &lone).as_deref(),
        Some("qsl-t3")
    );
    let second = PaneLocation {
        id_number: 5,
        is_lone: false,
    };
    assert_eq!(
        name(&unlabelled(three, 3, false), &second).as_deref(),
        Some("qsl-t3-p5")
    );
}

/// Tracing: TC-021-04
/// ACs: FR-021-AC-4
#[test]
fn tc_021_04_the_named_degenerate_texts() {
    let fifty = "x".repeat(50);
    for (raw, want) in [
        ("", "lead"),
        ("---", "lead"),
        ("1lead", "a1lead"),
        ("ALL", "all-lead"),
        (fifty.as_str(), &"x".repeat(32)),
    ] {
        assert_eq!(slugify(raw).as_str(), want, "{raw:?}");
    }
}

// ---------------------------------------------------------------------------
// Seed corpus and fuzz lane (FR-021-AC-1)
// ---------------------------------------------------------------------------

/// Tracing: TC-021-01
/// ACs: FR-021-AC-1
///
/// Known-nasty inputs, always run: a megabyte of one character (time and
/// allocation stay linear and the output stays 32 bytes), embedded NULs,
/// combining marks, right-to-left text, and every reserved-word spelling.
#[test]
fn tc_021_01_the_seed_corpus_of_nasty_inputs_slugs_to_valid_names() {
    let megabyte = |c: char| std::iter::repeat_n(c, 1 << 20).collect::<String>();
    let corpus = [
        megabyte('-'),
        megabyte('a'),
        megabyte('\u{130}'),
        "\0\0\0".to_owned(),
        "e\u{301}\u{301}\u{301}".to_owned(),
        "\u{5d0}\u{5d1}\u{5d2}".to_owned(),
        "all".to_owned(),
        "All".to_owned(),
        "a-l-l".to_owned(),
        "ALL-".to_owned(),
        "-all-".to_owned(),
        "\u{feff}all".to_owned(),
        format!("{}-all", "x".repeat(28)),
        "esc-e7-1".to_owned(),
    ];
    for raw in &corpus {
        check_slug_is_a_valid_bus_name(raw)
            .unwrap_or_else(|e| panic!("{:?}: {e}", &raw[..raw.len().min(20)]));
        let once = slugify(raw);
        assert_eq!(slugify(once.as_str()), once);
    }
}

/// The suffix `with_suffix` appends for `raw`, from the requirement rather
/// than the implementation: lowercased, anything outside `[a-z0-9_-]` mapped
/// to `-`, repeated `-` collapsed and trimmed, cut to 30 bytes, trailing `-`
/// trimmed, `x` when nothing is left.
fn cleaned_suffix(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.to_lowercase().chars() {
        let mapped = if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' {
            ch
        } else {
            '-'
        };
        if mapped != '-' || !out.ends_with('-') {
            out.push(mapped);
        }
    }
    let mut out = out.trim_matches('-').to_owned();
    out.truncate(30);
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "x".to_owned()
    } else {
        out.to_owned()
    }
}

/// The property of FR-022-AC-1 for one base and suffix, shared by the seed
/// corpus and the fuzz lanes: valid, at most 32 bytes, different from the
/// base, ending in the whole cleaned suffix, no new `--`.
fn check_with_suffix(base: &str, suffix: &str) -> Result<(), TestCaseError> {
    // A base that is already a valid name is used as it is (the flaky case
    // has a `--` that `slugify` would collapse); anything else is slugified.
    let base = base.parse::<AgentName>().unwrap_or_else(|_| slugify(base));
    let got = with_suffix(&base, suffix);
    let s = got.as_str();
    prop_assert!(s.parse::<AgentName>().is_ok(), "{s:?}");
    prop_assert!(s.len() <= 32, "{s:?}");
    prop_assert_ne!(s, base.as_str(), "a variant must differ from its base");
    let tail = format!("-{}", cleaned_suffix(suffix));
    prop_assert!(s.ends_with(&tail), "{s:?} must end with {tail:?}");
    prop_assert!(
        !s.starts_with('-') && (!s.contains("--") || base.as_str().contains("--")),
        "{s:?}"
    );
    Ok(())
}

/// Known-nasty `(base, suffix)` pairs for `with_suffix` and `free_variant`:
/// the flaky case from `tc_022_01_free_variant_returns_only_a_name_the_caller_accepted`
/// and the bases that end in the suffix at 31 and 32 bytes.
fn variant_corpus() -> Vec<(String, String)> {
    let b = |n: usize| "b".repeat(n);
    vec![
        ("aa0--a-0a-aa0aa0-a-0-a0-0-00a-2".to_owned(), "2".to_owned()),
        (format!("{}-2", "a".repeat(29)), "2".to_owned()),
        (format!("{}-2", "a".repeat(30)), "2".to_owned()),
        (format!("{}-1000", "a".repeat(26)), "1000".to_owned()),
        (format!("a-{}", b(30)), b(30)),
        (format!("a-{}", b(29)), b(29)),
        ("a".to_owned(), String::new()),
        ("all".to_owned(), "\u{130}".to_owned()),
    ]
}

/// Tracing: TC-022-01
/// ACs: FR-022-AC-1
///
/// The seed corpus of `variant_corpus`, always run: each pair through
/// `with_suffix`, and each base through `free_variant` with the base itself
/// and its first numbered variants refused.
#[test]
fn tc_022_01_the_variant_seed_corpus_yields_valid_different_names() {
    for (base, suffix) in variant_corpus() {
        check_with_suffix(&base, &suffix).unwrap_or_else(|e| panic!("{base:?} + {suffix:?}: {e}"));
        for widen in [None, Some(suffix.as_str())] {
            check_free_variant(&base, widen, 0)
                .unwrap_or_else(|e| panic!("{base:?} widened {widen:?}: {e}"));
        }
    }
    // A widening suffix that is also a numbered variant of the base: refuse the
    // widened name and `-2` to `-8`, so the loop reaches `-9`, which is the
    // widened name again and must not be asked a second time.
    let dup = format!("{}-0-zz", "a".repeat(27));
    check_free_variant(&dup, Some("0-9"), 8).unwrap_or_else(|e| panic!("{e}"));
}

/// `free_variant` over `candidate` (widened by `widen` when given), refusing
/// the first `refuse` names it asks about: the answer is never the candidate
/// and is the last name asked, and no name is asked twice.
fn check_free_variant(
    candidate: &str,
    widen: Option<&str>,
    refuse: u32,
) -> Result<(), TestCaseError> {
    let candidate = candidate
        .parse::<AgentName>()
        .unwrap_or_else(|_| slugify(candidate));
    let mut asked: Vec<AgentName> = Vec::new();
    let got = free_variant::<()>(&candidate, widen, |n| {
        asked.push(n.clone());
        Ok(asked.len() > usize::try_from(refuse).unwrap_or(usize::MAX))
    });
    let Ok(Some(got)) = got else {
        return Err(TestCaseError::fail("no free variant within the bound"));
    };
    prop_assert_eq!(asked.last(), Some(&got));
    prop_assert_ne!(&got, &candidate);
    let distinct: std::collections::HashSet<_> = asked.iter().collect();
    prop_assert_eq!(distinct.len(), asked.len(), "a name was asked twice");
    Ok(())
}

/// How many cases a fuzz-lane test runs: `PROPTEST_CASES` when set (see
/// `make fuzz`), else 20,000.
fn fuzz_config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(fuzz_config())]

    /// Tracing: TC-022-01, TC-022-10
    /// ACs: FR-022-AC-1, FR-022-AC-10
    ///
    /// Fuzz lane: `with_suffix` over arbitrary text, and over bases built to
    /// end in the very suffix being added (the shape that collided).
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
    fn fuzz_with_suffix_over_arbitrary_text(
        base in prop_oneof![".{0,80}", "[a-z][a-z0-9-]{25,31}"],
        suffix in prop_oneof![".{0,60}", "[a-z0-9-]{0,32}", "[0-9]{1,4}"],
        ends_in_suffix in any::<bool>(),
    ) {
        check_with_suffix(&base, &suffix)?;
        if ends_in_suffix {
            check_with_suffix(&format!("{base}-{suffix}"), &suffix)?;
        }
    }

    /// Tracing: TC-022-01
    /// ACs: FR-022-AC-1
    ///
    /// Fuzz lane: `free_variant`, with and without a widening suffix (the
    /// cross-machine path), refusing an arbitrary number of the names it asks
    /// about, never returns the candidate or a refused name, and asks about no
    /// name twice.
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
    fn fuzz_free_variant_over_arbitrary_candidates(
        candidate in prop_oneof![".{0,80}", "[a-z][a-z0-9-]{26,31}"],
        widen in proptest::option::of(prop_oneof![".{0,40}", "[a-z0-9-]{0,32}", "[0-9]-[0-9]"]),
        refuse in 0u32..998,
    ) {
        check_free_variant(&candidate, widen.as_deref(), refuse)?;
    }

    /// Tracing: TC-021-01
    /// ACs: FR-021-AC-1
    ///
    /// Fuzz lane: arbitrary bytes, decoded lossily (so invalid UTF-8 arrives
    /// as U+FFFD), up to 2 KiB, through `slugify`; every result is a valid bus
    /// name of at most 32 bytes that is not `all`, and a fixed point.
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
    fn fuzz_slugify_over_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let raw = String::from_utf8_lossy(&bytes);
        check_slug_is_a_valid_bus_name(&raw)?;
        let once = slugify(&raw);
        prop_assert_eq!(slugify(once.as_str()), once);
    }

    /// Tracing: TC-021-01
    /// ACs: FR-021-AC-1
    ///
    /// Fuzz lane, structure-aware: strings built from the fragments the
    /// function branches on (reserved word, dashes at the cap boundary,
    /// digits, underscores, multi-character lowercase expansions).
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
    fn fuzz_slugify_over_adversarial_fragments(
        parts in proptest::collection::vec(
            prop_oneof![
                Just("all".to_owned()), Just("-".to_owned()), Just("_".to_owned()),
                Just("\u{130}".to_owned()), Just("\u{df}".to_owned()), Just("0".to_owned()),
                "[a-z]{1,40}", ".{0,3}",
            ],
            0..24,
        ),
    ) {
        check_slug_is_a_valid_bus_name(&parts.concat())?;
    }
}
