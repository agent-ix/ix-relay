---
id: SR-020
title: "Spec review (EARS) of ix-relay PR #32, AGE-2059: FR-030 herdr-failure clause and AC-8"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay@13ed67c84a9f7e14c42cff0fe07b96849de6eada; spec/functional/FR-030-hook-failures-are-spooled-and-reported.md (new Behavior bullet, FR-030-AC-8), spec/tests.md (TC-030-08 row)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-030"
    type: "reviews"
---
# SR-020: Spec review (EARS) of ix-relay PR #32

## Summary

Ticket: AGE-2059. The review covers the one Behaviour bullet the PR adds to FR-030, the
amended FR-030-AC-8 and the edited TC-030-08 row. The new bullet is a well-formed
unwanted-behaviour statement ("If ... then the handler shall ..."). It has a subject, a
testable response and an explicit negative ("shall not treat the pane as a session that is
not a lead"). Both of these pass: `quire validate --scope . 'spec/**/*.md'` exits 0, and
`make check-matrix` passes. The PR body leaves the NFR-001-M-5 idle-progress stdin deadline
as an explicit owner decision, and no spec text changes it.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | FR-030-AC-8 is now a three-clause compound AC: closed stderr, panic, and herdr failure. Its new clause ends "leaves one `budget` or `derive` record" without the condition that picks which one, so a test that accepts either record satisfies the AC as written. The Behaviour bullet states the condition, and the deterministic test asserts it, but the AC should too. Split the herdr clause into its own AC (for example FR-030-AC-10) with a TC row of its own. State both outcomes in it: `budget`/`budget_exhausted` when a herdr call was refused for want of budget, otherwise `derive`/`herdr_failed`. | spec/functional/FR-030-hook-failures-are-spooled-and-reported.md:84, spec/tests.md:224 |
| FND-002 | low | The bullet says `budget` applies "when the time budget was spent before herdr could be asked". The code reports `budget` when any herdr call was refused for want of budget (`BoundedHerdr::budget_was_spent`). That includes a refusal after earlier herdr calls were made and failed, as its own doc comment says ("or while it was"). Reword to "when a herdr call was refused because the time budget was spent", so the spec matches the code. | spec/functional/FR-030-hook-failures-are-spooled-and-reported.md:32-36, crates/agent-msg/src/identity.rs:370-373 |

## Verdict

**PASS WITH FINDINGS.** The new requirement text is EARS-conformant and testable. Both
findings are wording and traceability fixes, and neither blocks.
