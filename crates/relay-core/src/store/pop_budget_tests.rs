//! The budgeted pop bounds what a batch *renders to*, not what it is stored as
//! (FR-018-AC-4, NFR-002-M-2, NFR-002-M-3, AGE-2031). Redis-backed
//! (`IX_RELAY_TEST_URL`, set by `make test-redis`); those tests skip with a
//! message without it. The renderer is the real one: [`AddressBook::render`],
//! joined the way the hook and the watcher join it. These are unit tests
//! because `Message::rendered_len_max`, the oracle, is test-only.
//!
//! The generators are tight on purpose: one-character names, small timestamps
//! and a sender whose address is exactly [`MAX_LABEL_BYTES`] leave the weight no
//! slack to hide a shortfall in its fixed allowance or its per-character terms.

use std::{cell::Cell, num::NonZeroUsize, time::Duration};

use proptest::prelude::*;

use crate::{
    AddressBook, AgentName, Claim, MAX_LABEL_BYTES, Message, Place, Popper, Registration, Relay,
    Target,
};

/// The hook's byte budget (`agent-msg`'s `HOOK_BYTE_BUDGET`).
const HOOK_BUDGET: usize = 6 * 1024;
/// The watcher's prompt budget (`relay-watcher`'s `MAX_PROMPT_BYTES`).
const WATCHER_BUDGET: usize = 16 * 1024;

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn relay(prefix: &str) -> Option<Relay> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    let relay = Relay::connect(&url, Duration::from_secs(5)).expect("connect to test redis");
    Some(relay.with_prefix(prefix))
}

thread_local! {
    static CASE: Cell<u64> = const { Cell::new(0) };
}

/// A prefix no other case, test or run shares.
fn fresh_prefix(test: &str) -> String {
    let n = CASE.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    format!("test:{}:{test}-{n}:", std::process::id())
}

/// The senders of a batch, each registered with a workspace label of the given
/// length. `a`'s address is exactly [`MAX_LABEL_BYTES`] (`node-a:` + 87 + `:1`),
/// `b`'s is short, `c`'s is over the cap so it prints as its bus name.
const SENDERS: [(&str, usize); 3] = [("a", 87), ("b", 1), ("c", 300)];

fn book() -> AddressBook {
    let regs: Vec<Registration> = SENDERS
        .iter()
        .map(|(n, label)| {
            let pane: crate::PaneId = format!("w{}:p1", n).parse().unwrap();
            let session_id: crate::SessionId = format!("session-{n}").parse().unwrap();
            Registration {
                name: name(n),
                machine: "node-a".parse().unwrap(),
                kind: "claude".into(),
                session_id: Some(session_id.clone()),
                cwd: None,
                herdr_pane: Some(pane.clone()),
                place: Some(Place {
                    workspace: "w".repeat(*label),
                    tab: "1".into(),
                    herdr_ids: Some(crate::HerdrPlaceIds {
                        workspace: format!("workspace-{n}").parse().unwrap(),
                        tab: format!("workspace-{n}:t1").parse().unwrap(),
                        pane,
                        session_id,
                        verified_at_ms: 1,
                    }),
                }),
                last_seen_ms: 1,
            }
        })
        .collect();
    AddressBook::from_registrations(&regs)
}

/// The batch as the hook and the watcher print it.
fn printed(book: &AddressBook, batch: &[Message]) -> String {
    batch
        .iter()
        .map(|m| book.render(m))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text that stresses the expansion: line feeds, characters that render as the
/// 3-byte replacement, multi-byte text, quotes and backslashes.
fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            8 => Just("\n".to_owned()),
            8 => "[a-z ]{0,6}",
            2 => Just("é".to_owned()),
            2 => Just("\\".to_owned()),
            2 => Just("\"".to_owned()),
            2 => Just("\r".to_owned()),
            2 => Just("\x08".to_owned()),
            2 => Just("\x0c".to_owned()),
            2 => Just("\x1b".to_owned()),
            2 => Just("\x7f".to_owned()),
            2 => Just("\u{85}".to_owned()),
            2 => Just("\u{61c}".to_owned()),
            1 => Just("\u{2028}".to_owned()),
            1 => Just("\t".to_owned()),
            1 => any::<char>().prop_map(String::from),
        ],
        0..120,
    )
    .prop_map(|parts| parts.concat())
}

fn message() -> impl Strategy<Value = Message> {
    (
        0..SENDERS.len(),
        prop_oneof![Just(Target::All), Just(Target::Agent(name("r")))],
        text(),
        // Mostly small, so a forwarded message's time is short; sometimes any
        // clock at all, so it is as long (or as unprintable) as it can be.
        prop_oneof![4 => 0i64..10, 1 => any::<i64>()],
        // A moved message (FR-053) carries a notice on its first line.
        any::<bool>(),
    )
        .prop_map(|(from, to, text, sent_at_ms, forwarded)| Message {
            from: name(SENDERS[from].0),
            to,
            text,
            sent_at_ms,
            forwarded,
        })
}

/// `messages` written to `r`'s inbox exactly as they are (no send-time
/// checks, so hostile text reaches the pop too), then popped once.
fn pop_once(r: &mut Relay, messages: &[Message], budget: usize) -> crate::BudgetPopResult {
    r.requeue_front(&name("r"), messages).unwrap();
    pop_acked(r, budget)
}

/// One pop as a hook does it, acknowledged so nothing stays in flight.
fn pop_acked(r: &mut Relay, budget: usize) -> crate::BudgetPopResult {
    let popped = r
        .pop_inbox_budget(
            &name("r"),
            NonZeroUsize::new(64).unwrap(),
            budget,
            Claim::new(Popper::Hook),
        )
        .unwrap();
    if let Some(batch) = popped.batch {
        r.ack(&name("r"), batch, crate::Popper::Hook).unwrap();
    }
    popped
}

/// The pop's weight is never below `rendered_len_max`: with a budget one byte
/// short of two messages' worst-case sizes, exactly one is taken.
fn takes_exactly_one_of(r: &mut Relay, first: &Message, second: &Message) -> Result<(), String> {
    let budget = first.rendered_len_max() + second.rendered_len_max() - 1;
    let popped = pop_once(r, &[first.clone(), second.clone()], budget);
    if popped.messages.len() == 1 && popped.pending == 1 {
        Ok(())
    } else {
        Err(format!(
            "budget {budget} took {} of 2 messages",
            popped.messages.len()
        ))
    }
}

proptest! {
    /// Tracing: TC-018-04, TC-N002-M-02, TC-N002-M-03
    /// ACs: FR-018-AC-4
    ///
    /// For any batch of messages, any sender labels and either budget, the
    /// popped batch prints to at most the budget unless it is one message, the
    /// first, which is always taken. Nothing is lost: what is not popped is
    /// still queued, in order.
    #[test]
    fn tc_018_04_a_popped_batch_never_prints_past_the_budget(
        batch in prop::collection::vec(message(), 1..40),
        budget in prop_oneof![
            Just(HOOK_BUDGET),
            Just(WATCHER_BUDGET),
            0usize..3_000,
        ],
    ) {
        let Some(mut r) = relay(&fresh_prefix("pop-budget")) else { return Ok(()); };
        let popped = pop_once(&mut r, &batch, budget);
        prop_assert!(popped.rejected.is_empty());
        prop_assert!(!popped.messages.is_empty(), "the first message is always taken");
        prop_assert_eq!(&popped.messages[..], &batch[..popped.messages.len()]);
        prop_assert_eq!(popped.pending, batch.len() - popped.messages.len());
        let text = printed(&book(), &popped.messages);
        if popped.messages.len() > 1 {
            prop_assert!(
                text.len() <= budget,
                "{} printed bytes for {} messages, budget {}",
                text.len(), popped.messages.len(), budget
            );
        }
    }

    /// Tracing: TC-018-04, TC-020-09, TC-N002-M-03
    /// ACs: FR-018-AC-4, FR-020-AC-9
    ///
    /// The sharp oracle: a budget one byte short of two messages' worst-case
    /// sizes takes exactly one of them, for any two messages. It fails if the
    /// weight of either is ever below `rendered_len_max`.
    #[test]
    fn tc_018_04_the_pop_weight_is_never_below_the_rendered_size(
        first in message(),
        second in message(),
    ) {
        let Some(mut r) = relay(&fresh_prefix("pop-weight")) else { return Ok(()); };
        prop_assert_eq!(takes_exactly_one_of(&mut r, &first, &second), Ok(()));
    }

    /// Tracing: TC-N002-M-03, TC-020-09, TC-040-05, TC-053-12
    /// ACs: FR-018-AC-4, FR-020-AC-9, FR-040-AC-5, FR-053-AC-12
    ///
    /// The Rust size of a message is never below what it prints to, under any
    /// address book, with the byte that joins it to the next counted.
    #[test]
    fn rendered_len_max_bounds_what_render_prints(m in message()) {
        let shown = book().render(&m);
        prop_assert!(shown.len() < m.rendered_len_max(), "{shown:?}");
        // FR-040-AC-5: the redelivery marker is inside the weight too.
        let marked = book().render_marked(&m, true);
        prop_assert!(marked.len() < m.rendered_len_max(), "{marked:?}");
        let widest = m.render_from_marked(&"x".repeat(MAX_LABEL_BYTES), true);
        if m.forwarded {
            // The notice is at its longest for the widest time only: still bounded.
            prop_assert!(widest.len() < m.rendered_len_max(), "{widest:?}");
        } else {
            prop_assert_eq!(widest.len() + 1, m.rendered_len_max());
        }
    }
}

/// The address the tightest sender has: exactly the cap.
#[test]
fn the_widest_sender_label_is_exactly_the_cap() {
    let book = book();
    assert_eq!(book.label(&name("a")).len(), MAX_LABEL_BYTES);
    assert_eq!(
        book.label(&name("c")),
        "unverified",
        "over the cap cannot become an alias"
    );
}

/// Tracing: TC-018-04
/// ACs: FR-018-AC-4
///
/// One fixed case per character class whose printed form is longer than its
/// stored form, each as 1000 characters, so a missing term in the weight shows
/// as a shortfall of hundreds of bytes: line feed (2 stored, 5 printed), the
/// other escaped controls `\r`, `\x08`, `\x0c` (2, 3), DEL (1, 3), a C1 control
/// (2, 3) and U+061C (2, 3).
#[test]
fn tc_018_04_each_expanding_character_class_is_weighed_at_its_printed_size() {
    let Some(mut r) = relay(&fresh_prefix("pop-classes")) else {
        return;
    };
    for c in ["\n", "\r", "\x08", "\x0c", "\x7f", "\u{85}", "\u{61c}"] {
        let m = Message {
            from: name("a"),
            to: Target::Agent(name("r")),
            text: c.repeat(1000),
            sent_at_ms: 0,
            forwarded: false,
        };
        assert_eq!(takes_exactly_one_of(&mut r, &m, &m), Ok(()), "{c:?}");
        // What was not taken is still queued: drain for the next class.
        pop_acked(&mut r, usize::MAX);
    }
}

/// Tracing: TC-018-04
/// ACs: FR-018-AC-4
///
/// The case from AGE-2031: a message that is 2 stored bytes per line feed and 5
/// printed. Three of them are 6,000 stored bytes and printed 15,000: past a 6 KiB
/// budget when counted by stored length, so the pop takes one.
#[test]
fn tc_018_04_newline_heavy_messages_are_counted_as_printed() {
    let Some(mut r) = relay(&fresh_prefix("pop-budget-newlines")) else {
        return;
    };
    let heavy = Message {
        from: name("a"),
        to: Target::Agent(name("r")),
        text: "\n".repeat(1000),
        sent_at_ms: 1,
        forwarded: false,
    };
    let popped = pop_once(&mut r, &vec![heavy; 3], HOOK_BUDGET);
    assert_eq!(popped.messages.len(), 1, "5,000 printed bytes each");
    assert_eq!(popped.pending, 2);
}
