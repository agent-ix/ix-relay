---
id: SR-027
title: "Spec review (EARS) of ix-relay PR #33, AGE-2061: FR-046 and the FR-044 amendment"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay@5fe6eaa46c4b9dcd7f52f1c7506060266539c250; spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md (Description, Behavior, AC-1..AC-11), spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (amended ack-wait bullet, related links), spec/tests.md (TC-046 rows); checked against crates/claude-wire/src/*.rs and crates/relay-watcher/src/socket/deliverer.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-046"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
---
# SR-027: Spec review (EARS) of ix-relay PR #33, AGE-2061

## Summary

Ticket: AGE-2061. This checks the requirement statements FR-046 adds, and the FR-044
bullet it amends, against the EARS patterns. It also checks that each statement is true of
the code at 5fe6eaa. FR-046's Behavior bullets follow EARS well:

- ubiquitous: "`parse_control_line` shall map…", "Counters shall saturate…"
- event-driven: "When the watcher reads…", "When a transcript look finds…"
- state-driven, doctor: "…shall warn … when the reply-socket counters are not zero"

Each has one subject, one `shall`, and a testable object. The 11 ACs are Given/then and
each is traced to a TC-046 row. `quire validate` on FR-046 and FR-044 reports no errors.
The FR-044 amendment appends an exception clause to the ack-wait bullet. It is readable,
but it is an unmarked second condition inside an event-driven statement.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | The Description's opening sentence is stated as fact: "Every external interface the relay touches has a typed wire format, in its own crate or module, with a drift signal". It is not true. Spec 15.3 itself lists interfaces whose drift signal is "none": hook stdin, the Lua scripts, the transcript `customTitle` read and the `linear` CLI. Scope the sentence to Claude Code's cross-session formats, or word it as the rule being worked towards, so no reader takes it as a delivered property. | spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:13-15 |
| FND-002 | low | Drift bullet 1 and AC-3 say that a reply-socket line of "bytes that are not UTF-8" is counted as `Unrecognised`/`NotJson`. The code decodes the line lossily (`String::from_utf8_lossy`) and then parses it. A line that is valid JSON apart from an invalid byte inside a string still parses, and may become a `Receipt` (counted nowhere) or `Unrecognised`. Only lines that are not JSON after the lossy decode are `NotJson`. Reword it as "decoded lossily as UTF-8, then classified", or reject non-UTF-8 as `NotJson` before parsing, to match the claim. | spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:63-66, crates/relay-watcher/src/socket/deliverer.rs:1818-1828 |
| FND-003 | low | AC-5 and AC-1 are compound. AC-5 bundles five obligations: ambiguous, counted, next batch to the socket, the silent-receiver contrast, and `settle` never acking but acking the structured line. AC-1 bundles three: the dependency set, banned API names, and no relay crate. They are testable as written, but one failing clause fails the whole AC, and the matrix cannot say which. Split AC-5 at least, and add the post-lease obligation that SR-029 FND-001 asks for as its own AC. The FR-044 ack-wait bullet also carries its FR-046 exception as a parenthetical. State it as its own "If … then" bullet. | spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:94, spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:98, spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md:163-166 |

## Verdict

EARS conformance is good, with three low findings about wording, one of which is a
statement that is not quite true of the code (FND-002). Nothing here blocks merge on its
own. The behaviour AC-5 specifies is questioned on substance in SR-029 FND-001.
