---
id: SR-042
title: "Spec review (EARS) of ix-relay PR #36, AGE-2063: NFR-001 stdin clause and FR-030 stdin_timeout"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay@08cdcad91dfd7e43a2970f8a2717c124f1212190; spec/non-functional/NFR-001-hook-time-budget.md (Statement, Scope reading bullet), spec/functional/FR-030-hook-failures-are-spooled-and-reported.md (first Behavior bullet, decode codes), spec/tests.md (TC-N001-M-05 row)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/NFR-001"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-030"
    type: "reviews"
---
# SR-042: Spec review (EARS) of ix-relay PR #36

## Summary

Ticket: AGE-2063. The review covers the requirement text the PR edits: the stdin clause of
NFR-001's Statement, the reading bullet in NFR-001's Scope, and the `stdin_timeout`
definition in FR-030. `quire validate` passes on both files.

The FR-030 edit keeps its own form. It defines a code inside an existing ubiquitous
"shall carry" statement, and both limits (300 ms idle, 900 ms total) are quantified and
testable. The NFR-001 Scope bullet is a clear "ends at the first of" list with measurable
terms.

## Verdict

**PASS WITH FINDINGS.** The NFR-001 Statement's stdin clause is testable, but it is buried
in a compound statement (FND-001, low). Its consistency problem is recorded in SR-043.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | NFR-001's Statement is a single sentence chain with several `shall` responses: the per-event budgets, the pop window, the stdin wait, and the timeout outcome. The PR adds a state-driven condition ("while the hook has no complete hook payload"), two bounds and an outcome, most of them inside parentheticals. The result is not single-clause, and the new bound cannot be cited on its own. Split it into a separate state-driven statement, for example: "While the hook has no complete hook payload, the handler shall end the read after 300 ms without new bytes or 900 ms from the start of the call, whichever comes first, and shall then record one `decode`/`stdin_timeout` failure, register and pop nothing, and exit 0." | spec/non-functional/NFR-001-hook-time-budget.md:16-28 |
