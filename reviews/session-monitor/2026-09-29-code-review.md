---
id: SR-045
title: "Code and Rust review of session work reporting and monitor"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@b49515127f3346a891bdb0d23aced39a184ac7da; changed Cargo manifests/lock, README.md, crates/agent-msg/{Cargo.toml,src/main.rs,src/spool.rs,src/work.rs}, crates/relay-core/{src/error.rs,src/lib.rs,src/store.rs,src/store/keys.rs,src/store/work.rs,src/work.rs,tests/work.rs}, crates/relay-ops/{Cargo.toml,src/errors.rs,src/lib.rs,src/work.rs,tests/work.rs}, crates/relay-tui/src/monitor*.rs, spec/functional/FR-048-session-work-monitor.md; diff against 5169352"
review_set: subset
---
## Summary

Reviewed the full change for repository Rust idioms, measured identity, bounded
report storage, concurrent updates, worker lifetimes, UI source honesty and
behavioral evidence. CI configuration and gate breadth are unchanged. ix-board
is consumed by merged git revision; no source or schema is copied across repos.

## Verdict

**CONDITIONAL** — identity and removal checks require fixes before handoff.
Format and workspace Clippy passed. The real Redis test lane passed the new
store tests, but the compact rendering test exposed the priority formatting
finding below and prevented the full gate completing.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | Scanning every argv pair for `resume` can mistake a profile value for a subcommand, e.g. `codex --profile resume exec task` binds session `exec`. The Herdr native-session record also needs its harness kind checked before it becomes identity evidence. Restrict process evidence to the measured direct resume form and refuse a conflicting harness. | crates/relay-ops/src/work.rs:126 |
| FND-002 | medium | Removal validates input but does not compare stored binding: the same machine/session/PID with a different pane or harness can delete an assignment that reads and updates would refuse. Apply the same atomic incarnation guard to removal. | crates/relay-core/src/store/work.rs:90 |
| FND-003 | low | Priority Display prints `2 (High)` while the compact acceptance test requires `Priority: High`; use the authoritative label method instead. | crates/relay-tui/src/monitor_ui.rs:438 |

## Review coverage

Atomic fields preserve simultaneous agents; scope progress and queues come from
the same authoritative snapshot. Sources retain explicit errors and actual fetch
age. Internal assignments are labeled reported; pane observations require a
native session match. Bounded channels retain updates and shutdown drops receivers
to release blocked sends. Blocking readers detach deliberately and stop after
timeout; they do not hold the terminal thread. No new unsafe or stubs were found.
The installed quire lacks `matrix`; criterion bindings and spec validation are
checked without claiming that unavailable command passed.

## Dispositions

| Finding | Outcome | Evidence |
| --- | --- | --- |
| FND-001 | fixed de47b6f06475f10091883c6970a6b6b037ba4aef | Profile-value refusal and mismatched harness tests pass; binding still resolves the measured direct resume form. |
| FND-002 | fixed 06343de0478dcc35e3c6019f01aa6ec05bc518f6 | Removal shares the atomic identity guard with updates; a mismatched pane cannot delete either agent. |
| FND-003 | fixed 016ea95d2b4eb08b7222c440d1a2b46a6897c4fa | Detail rendering uses ProjectPriority.label; the 69x15 acceptance test passes. |

## Final verdict

**PASS** — all findings fixed. Dispositions verified against
016ea95d2b4eb08b7222c440d1a2b46a6897c4fa; original finding rows and reviewed SHA
are retained. The final full gate result is recorded in the PR after completion.
