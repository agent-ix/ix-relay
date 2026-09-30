---
id: SR-046
title: "Session monitor implementation gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@016ea95d2b4eb08b7222c440d1a2b46a6897c4fa; changed Cargo manifests/lock, README.md, crates/agent-msg/src/{main,spool,work}.rs, crates/relay-core/src/{error,lib,store,work}.rs, crates/relay-core/src/store/{keys,work}.rs, crates/relay-core/tests/work.rs, crates/relay-ops/src/{errors,lib,work}.rs, crates/relay-ops/tests/work.rs, crates/relay-tui/src/monitor*.rs, spec/functional/FR-048-session-work-monitor.md"
review_set: subset
---
## Summary

Mapped FR-048 to behavior and evidence across report storage, measured binding,
authoritative scope calculation and terminal presentation. No source or test stubs
are present in the change. The report commands own no message delivery or tracker
mutation; runtime reads and Linear reads run off the terminal input thread.

## Verdict

**PASS** — no missing implementation obligations in the reviewed change.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Criterion evidence

- AC-1: address ambiguity, native-session disagreement, profile-value refusal,
  harness mismatch and process/session replacement; Redis machine/session/PID
  isolation and display-label rename preserve the correct record.
- AC-2: real concurrent agent writers on one ticket, independent scope/focus,
  removal isolation, 128-agent limit before and after reserved fields, unsafe
  text refusal and server-side read bounds. Store keys prove no inbox writes.
- AC-3: ix-board owns selector unions, descendants and leaf exclusions; the
  monitor renders 1/2 for a completed parent with one completed child and one
  Review leaf, and keeps unreported focus and assignments unknown.
- AC-4: 69x15 details preserve full title, priority and blockers; 12 reported
  internal agents scroll; pane activity refuses the wrong native session and
  stage completion leaves the Linear Review stage intact.
- AC-5: independent report polling succeeds while the Herdr read is blocked;
  source errors retain labeled last snapshots; manual-refresh tests measure the
  force flag and ix-board's injected clock measures forced-fetch cache age.

The installed quire lacks `matrix`; source criterion bindings remain explicit,
and existing repository matrix recount is retained as its separate gate.
