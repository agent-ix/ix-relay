//! Pane-tail classification (FR-069): what a lane's pane is waiting on, from the
//! last 80 lines (at most 16 KiB) of its output.
//!
//! The patterns are a typed catalogue, one entry per class and harness. A
//! classification names the class only: the matched text is never returned,
//! stored or exported.

use std::collections::HashMap;

/// The most lines of a pane read.
pub const TAIL_LINES: usize = 80;

/// The most bytes of a pane read.
pub const TAIL_BYTES: usize = 16 * 1024;

/// The harness a pane runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Harness {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
}

impl Harness {
    /// Both harnesses.
    #[cfg(test)]
    pub const ALL: [Harness; 2] = [Harness::Claude, Harness::Codex];

    /// The harness for a herdr or registration `kind`; anything but `codex` is
    /// read with the Claude catalogue.
    #[must_use]
    pub fn from_kind(kind: &str) -> Self {
        if kind.eq_ignore_ascii_case("codex") {
            Harness::Codex
        } else {
            Harness::Claude
        }
    }
}

/// What a pane tail shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneClass {
    /// A confirmation prompt awaiting input.
    Dialog,
    /// An authentication prompt.
    Login,
    /// A usage or rate limit notice.
    UsageLimit,
    /// A queued follow-up inputs notice.
    QueuedQuestion,
    /// A sleep or poll wait repeated over the tail.
    WaitLoop,
    /// None of the above.
    None,
}

impl PaneClass {
    /// Every class.
    #[cfg(test)]
    pub const ALL: [PaneClass; 6] = [
        PaneClass::Dialog,
        PaneClass::Login,
        PaneClass::UsageLimit,
        PaneClass::QueuedQuestion,
        PaneClass::WaitLoop,
        PaneClass::None,
    ];

    /// The `ix.fleet.lane.blocked` reason of this class; `none` has none.
    #[must_use]
    pub fn reason(self) -> Option<BlockedReason> {
        match self {
            PaneClass::Dialog => Some(BlockedReason::Dialog),
            PaneClass::Login => Some(BlockedReason::Login),
            PaneClass::UsageLimit => Some(BlockedReason::UsageLimit),
            PaneClass::QueuedQuestion => Some(BlockedReason::QueuedQuestion),
            PaneClass::WaitLoop => Some(BlockedReason::WaitLoop),
            PaneClass::None => None,
        }
    }
}

/// The values of the `reason` attribute of `ix.fleet.lane.blocked`: every pane
/// class and hook cause maps to exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockedReason {
    /// A `blocked` hook event with reason `permission`.
    Permission,
    /// Pane class `dialog`.
    Dialog,
    /// Pane class `login`.
    Login,
    /// Pane class `usage_limit`.
    UsageLimit,
    /// A `Stop` with `last_class=question`.
    Question,
    /// Pane class `queued_question`.
    QueuedQuestion,
    /// Pane class `wait_loop`.
    WaitLoop,
}

impl BlockedReason {
    /// Every reason.
    #[cfg(test)]
    pub const ALL: [BlockedReason; 7] = [
        BlockedReason::Permission,
        BlockedReason::Dialog,
        BlockedReason::Login,
        BlockedReason::UsageLimit,
        BlockedReason::Question,
        BlockedReason::QueuedQuestion,
        BlockedReason::WaitLoop,
    ];

    /// The attribute value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            BlockedReason::Permission => "permission",
            BlockedReason::Dialog => "dialog",
            BlockedReason::Login => "login",
            BlockedReason::UsageLimit => "usage_limit",
            BlockedReason::Question => "question",
            BlockedReason::QueuedQuestion => "queued_question",
            BlockedReason::WaitLoop => "wait_loop",
        }
    }
}

/// One catalogue entry: lines containing any needle (case-insensitive) put the
/// tail in `class` for `harness`.
struct Pattern {
    harness: Harness,
    class: PaneClass,
    needles: &'static [&'static str],
}

/// The catalogue. Order is precedence: a limit or login notice outranks a
/// dialog, which outranks a queued notice. `wait_loop` is structural, below.
const CATALOGUE: &[Pattern] = &[
    Pattern {
        harness: Harness::Claude,
        class: PaneClass::UsageLimit,
        needles: &[
            "usage limit reached",
            "5-hour limit reached",
            "you've hit your limit",
            "rate limit reached",
        ],
    },
    Pattern {
        harness: Harness::Codex,
        class: PaneClass::UsageLimit,
        needles: &[
            "you've hit your usage limit",
            "usage limit. upgrade",
            "try again at",
        ],
    },
    Pattern {
        harness: Harness::Claude,
        class: PaneClass::Login,
        needles: &[
            "please run /login",
            "invalid api key",
            "oauth token has expired",
            "not logged in",
        ],
    },
    Pattern {
        harness: Harness::Codex,
        class: PaneClass::Login,
        needles: &["sign in with chatgpt", "run `codex login`", "not signed in"],
    },
    Pattern {
        harness: Harness::Claude,
        class: PaneClass::Dialog,
        needles: &[
            "do you want to proceed?",
            "do you want to make this edit",
            "(y/n)",
        ],
    },
    Pattern {
        harness: Harness::Codex,
        class: PaneClass::Dialog,
        needles: &[
            "would you like to run the following command",
            "yes, proceed",
            "(y/n)",
        ],
    },
    Pattern {
        harness: Harness::Claude,
        class: PaneClass::QueuedQuestion,
        needles: &[
            "queued follow-up inputs",
            "press up to edit queued messages",
        ],
    },
    Pattern {
        harness: Harness::Codex,
        class: PaneClass::QueuedQuestion,
        needles: &["queued follow-up inputs"],
    },
];

/// Words that make a repeated line a wait.
const WAIT_WORDS: &[&str] = &["sleep ", "sleeping", "waiting", "polling", "checking again"];

/// How many times the same wait line must repeat to be a loop.
const WAIT_REPEATS: usize = 3;

/// The last [`TAIL_LINES`] lines of `text`, and of those the last [`TAIL_BYTES`]
/// bytes: all a classification ever reads.
#[must_use]
pub fn bound_tail(text: &str) -> &str {
    let mut start = 0;
    let mut seen = 0;
    for (index, byte) in text.bytes().enumerate().rev() {
        if byte == b'\n' && index + 1 != text.len() {
            seen += 1;
            if seen == TAIL_LINES {
                start = index + 1;
                break;
            }
        }
    }
    let mut tail = &text[start..];
    if tail.len() > TAIL_BYTES {
        let mut cut = tail.len() - TAIL_BYTES;
        while !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail = &tail[cut..];
    }
    tail
}

/// Classifies a pane `tail` read from a `harness` pane. Only the bounded tail is
/// read, and only the class is returned.
#[must_use]
pub fn classify(tail: &str, harness: Harness) -> PaneClass {
    let tail = bound_tail(tail).to_lowercase();
    for pattern in CATALOGUE.iter().filter(|p| p.harness == harness) {
        if pattern.needles.iter().any(|n| tail.contains(n)) {
            return pattern.class;
        }
    }
    let mut repeats: HashMap<String, usize> = HashMap::new();
    for line in tail.lines() {
        if WAIT_WORDS.iter().any(|w| line.contains(w)) {
            let shape: String = line
                .trim()
                .chars()
                .map(|c| if c.is_ascii_digit() { '#' } else { c })
                .collect();
            let n = repeats.entry(shape).or_insert(0);
            *n += 1;
            if *n >= WAIT_REPEATS {
                return PaneClass::WaitLoop;
            }
        }
    }
    PaneClass::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;

    /// The fixture tail of each class for each harness.
    fn fixture(harness: Harness, class: PaneClass) -> String {
        let body = match (harness, class) {
            (Harness::Claude, PaneClass::Dialog) => "Do you want to proceed?\n❯ 1. Yes\n  2. No",
            (Harness::Codex, PaneClass::Dialog) => {
                "Would you like to run the following command?\n  Yes, proceed (y)"
            }
            (Harness::Claude, PaneClass::Login) => "Invalid API key · Please run /login",
            (Harness::Codex, PaneClass::Login) => "Not signed in. Run `codex login`",
            (Harness::Claude, PaneClass::UsageLimit) => "5-hour limit reached ∙ resets 3pm",
            (Harness::Codex, PaneClass::UsageLimit) => {
                "You've hit your usage limit. Upgrade or try again at 3pm"
            }
            (_, PaneClass::QueuedQuestion) => "Queued follow-up inputs\n  1. continue",
            (_, PaneClass::WaitLoop) => {
                "sleep 30 # wait for ci\nsleep 30 # wait for ci\nsleep 30 # wait for ci"
            }
            (_, _) => "compiling relay-core v0.3.4\nfinished dev profile",
        };
        format!("earlier output\n{body}\n")
    }

    #[trace("FR-069-AC-7")]
    #[test]
    fn tc_069_07_each_fixture_classifies_to_its_class_for_both_harnesses() {
        for harness in Harness::ALL {
            for class in PaneClass::ALL {
                assert_eq!(
                    classify(&fixture(harness, class), harness),
                    class,
                    "{harness:?} {class:?}"
                );
            }
        }
    }

    #[trace("FR-069-AC-7")]
    #[test]
    fn tc_069_07_a_tail_over_the_bound_is_read_only_to_the_bound() {
        // A dialog marker beyond 80 lines, then plain output: not seen.
        let mut text = String::from("Do you want to proceed?\n");
        for i in 0..TAIL_LINES {
            text.push_str(&format!("line {i}\n"));
        }
        assert_eq!(classify(&text, Harness::Claude), PaneClass::None);
        // Beyond 16 KiB inside the last 80 lines: not seen either.
        let long = "x".repeat(TAIL_BYTES);
        let text = format!("Do you want to proceed?\n{long}\nlast\n");
        assert_eq!(classify(&text, Harness::Claude), PaneClass::None);
        assert!(bound_tail(&text).len() <= TAIL_BYTES);
        // Inside the bound it is seen.
        let text = format!("{long}\nDo you want to proceed?\nlast\n");
        assert_eq!(classify(&text, Harness::Claude), PaneClass::Dialog);
        // A multi-byte character at the cut does not panic.
        let wide = "é".repeat(TAIL_BYTES);
        assert!(bound_tail(&wide).len() <= TAIL_BYTES);
    }

    #[trace("FR-069-AC-12")]
    #[test]
    fn tc_069_12_every_class_has_a_reason_and_every_reason_is_reachable() {
        for class in PaneClass::ALL {
            match class {
                PaneClass::None => assert_eq!(class.reason(), None),
                other => assert!(other.reason().is_some(), "{other:?} has no reason"),
            }
        }
        let from_classes: Vec<_> = PaneClass::ALL.iter().filter_map(|c| c.reason()).collect();
        for reason in BlockedReason::ALL {
            let hook_cause = matches!(reason, BlockedReason::Permission | BlockedReason::Question);
            assert!(
                hook_cause || from_classes.contains(&reason),
                "{} is neither a class nor a hook cause",
                reason.name()
            );
        }
    }

    #[test]
    fn the_catalogue_has_an_entry_for_every_pattern_class_and_harness() {
        for harness in Harness::ALL {
            for class in [
                PaneClass::Dialog,
                PaneClass::Login,
                PaneClass::UsageLimit,
                PaneClass::QueuedQuestion,
            ] {
                assert!(
                    CATALOGUE
                        .iter()
                        .any(|p| p.harness == harness && p.class == class),
                    "{harness:?} {class:?}"
                );
            }
        }
        assert_eq!(Harness::from_kind("codex"), Harness::Codex);
        assert_eq!(Harness::from_kind("claude"), Harness::Claude);
    }
}
