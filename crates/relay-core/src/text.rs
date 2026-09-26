//! The one definition of "text that is safe to show or type".
//!
//! Message bodies, escalation fields, address labels and the TUI all take text
//! from another lead. Each used to carry its own idea of which characters are
//! dangerous; this module is the single rule they share.

use std::borrow::Cow;

/// What [`sanitise`] puts in place of an unsafe character.
pub const REPLACEMENT: char = '\u{FFFD}';

/// Whether `c` must never reach a terminal, a pane or a signed relay line.
///
/// Unsafe are:
/// - every control character except `\n` and `\t` — C0 (including ESC, CR,
///   vertical tab and form feed), DEL and C1 (including NEL, U+0085);
/// - the Unicode line and paragraph separators U+2028 and U+2029, which some
///   viewers treat as line breaks;
/// - the bidirectional controls (U+061C, U+200E, U+200F, U+202A–U+202E,
///   U+2066–U+2069), which reorder how text *displays* without changing its
///   bytes;
/// - invisible format characters a reader (an LLM included) still sees but a
///   person does not: zero-width space U+200B, the word joiner and invisible
///   operators U+2060–U+2064, BOM/ZWNBSP U+FEFF, Mongolian vowel separator
///   U+180E, the interlinear annotation marks U+FFF9–U+FFFB, and the tag block
///   U+E0000–U+E007F ("ASCII smuggling"). ZWJ U+200D and ZWNJ U+200C stay
///   allowed (emoji sequences, Indic text), as do variation selectors and
///   U+FFFD.
///
/// `\n` and `\t` are the only exceptions: multi-line text is legitimate, and
/// the message renderer indents every continuation line itself.
#[must_use]
pub fn is_unsafe_char(c: char) -> bool {
    match c {
        '\n' | '\t' => false,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{2028}' | '\u{2029}' => true,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => true,
        '\u{200B}' | '\u{180E}' | '\u{FEFF}' => true,
        '\u{2060}'..='\u{2064}' | '\u{FFF9}'..='\u{FFFB}' | '\u{E0000}'..='\u{E007F}' => true,
        _ => c.is_control(),
    }
}

/// The first unsafe character in `text` and its byte offset, if any.
#[must_use]
pub fn first_unsafe(text: &str) -> Option<(usize, char)> {
    text.char_indices().find(|&(_, c)| is_unsafe_char(c))
}

/// `text` with every unsafe character (see [`is_unsafe_char`]) replaced by
/// [`REPLACEMENT`]. Borrows when there is nothing to replace.
///
/// Idempotent, and its output never contains an unsafe character: the
/// replacement is itself safe. Replacing rather than deleting keeps a removed
/// character from splicing its neighbours into something they were not.
#[must_use]
pub fn sanitise(text: &str) -> Cow<'_, str> {
    if first_unsafe(text).is_none() {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| if is_unsafe_char(c) { REPLACEMENT } else { c })
            .collect(),
    )
}

/// [`sanitise`] for a value that must stay on one line (a registration's
/// machine, kind, cwd or pane, an escalation's header fields): newlines and
/// tabs are replaced too.
#[must_use]
pub fn sanitise_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if is_unsafe_char(c) || c == '\n' || c == '\t' {
                REPLACEMENT
            } else {
                c
            }
        })
        .collect()
}

/// [`sanitise`], then every `\n` followed by four spaces: text made safe to
/// print after a `[ix-relay]` signature or any other line prefix, because no
/// continuation line can begin with anything but spaces. Message rendering and
/// the `log` listing both use this, so there is one way to show a sender's text.
#[must_use]
pub fn neutralise(text: &str) -> String {
    sanitise(text).replace('\n', "\n    ")
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn newline_and_tab_are_the_only_safe_controls() {
        assert!(!is_unsafe_char('\n') && !is_unsafe_char('\t'));
        for c in [
            '\x1b', '\r', '\x0b', '\x0c', '\x7f', '\u{85}', '\u{2028}', '\u{202e}',
        ] {
            assert!(is_unsafe_char(c), "{c:?} must be unsafe");
        }
        assert!(!is_unsafe_char('a') && !is_unsafe_char('é') && !is_unsafe_char(REPLACEMENT));
    }

    /// Each invisible range, by its edges, plus what must stay allowed.
    #[test]
    fn the_invisible_format_ranges_are_unsafe_and_joiners_are_not() {
        let unsafe_edges = [
            ('\u{200B}', '\u{200B}', "zero-width space"),
            (
                '\u{2060}',
                '\u{2064}',
                "word joiner and invisible operators",
            ),
            ('\u{FEFF}', '\u{FEFF}', "BOM"),
            ('\u{180E}', '\u{180E}', "Mongolian vowel separator"),
            ('\u{FFF9}', '\u{FFFB}', "interlinear annotation"),
            ('\u{E0000}', '\u{E007F}', "tag block"),
        ];
        for (lo, hi, what) in unsafe_edges {
            assert!(is_unsafe_char(lo) && is_unsafe_char(hi), "{what}");
        }
        // Allowed: joiners, variation selectors, U+FFFD, and neighbours just
        // outside each range (U+2065 is unassigned, U+E0080 is past the tags).
        for c in [
            '\u{200C}',
            '\u{200D}',
            '\u{FE0F}',
            '\u{FE00}',
            REPLACEMENT,
            '\u{2059}',
            '\u{2065}',
            '\u{E0080}',
            '\u{FFF8}',
            '\u{FFFC}',
            '\u{200A}',
            '\u{1F1E6}',
        ] {
            assert!(!is_unsafe_char(c), "{c:?} must stay allowed");
        }
        assert_eq!(
            sanitise("a\u{E0041}\u{200B}b\u{200D}c"),
            "a\u{FFFD}\u{FFFD}b\u{200D}c"
        );
    }

    #[test]
    fn first_unsafe_reports_the_codepoint_and_byte_offset() {
        assert_eq!(first_unsafe("é\x1b[0m"), Some((2, '\x1b')));
        assert_eq!(first_unsafe("plain\ntext\t"), None);
    }

    /// Arbitrary text, weighted towards the characters the rule cares about.
    fn adversarial() -> impl Strategy<Value = String> {
        let nasty = prop::sample::select(vec![
            '\x1b',
            '\r',
            '\n',
            '\t',
            '\x0b',
            '\x0c',
            '\x7f',
            '\u{85}',
            '\u{2028}',
            '\u{2029}',
            '\u{202e}',
            '\u{2066}',
            '\u{200f}',
            '\u{200b}',
            '\u{2060}',
            '\u{feff}',
            '\u{180e}',
            '\u{fff9}',
            '\u{e0041}',
            '\u{200d}',
            '[',
            ']',
        ]);
        prop::collection::vec(prop_oneof![any::<char>(), nasty], 0..64)
            .prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        #[test]
        fn sanitise_is_idempotent(s in adversarial()) {
            let once = sanitise(&s).into_owned();
            let twice = sanitise(&once).into_owned();
            prop_assert_eq!(once, twice);
        }

        #[test]
        fn sanitise_output_never_contains_an_unsafe_char(s in adversarial()) {
            prop_assert!(first_unsafe(&sanitise(&s)).is_none());
        }

        #[test]
        fn sanitise_line_output_has_no_unsafe_char_and_no_line_break(s in adversarial()) {
            let line = sanitise_line(&s);
            prop_assert!(first_unsafe(&line).is_none());
            prop_assert!(!line.contains(['\n', '\t']));
            prop_assert_eq!(sanitise_line(&line), line);
        }

        #[test]
        fn sanitise_leaves_safe_text_untouched(s in "[a-zA-Z0-9 \n\t.,:;!?-]{0,64}") {
            prop_assert!(matches!(sanitise(&s), Cow::Borrowed(_)));
        }
    }
}
