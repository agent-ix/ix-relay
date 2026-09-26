---
id: SR-043
title: "Spec review (integrity) of ix-relay PR #36, AGE-2063: NFR-001 stdin bound, FR-030 stdin_timeout, README"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay@08cdcad91dfd7e43a2970f8a2717c124f1212190; spec/non-functional/NFR-001-hook-time-budget.md (Statement, Scope, Rationale, Measurement and Evaluation, Dependencies), spec/functional/FR-030-hook-failures-are-spooled-and-reported.md (decode codes, budget stage, skipped-pop bullets), spec/tests.md (TC-N001-M-05 row), README.md (hook paragraph); checked against: crates/agent-msg/src/hook_run.rs, crates/agent-msg/src/stdin.rs, crates/agent-msg/src/budget.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/NFR-001"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-030"
    type: "reviews"
---
# SR-043: Spec review (integrity) of ix-relay PR #36

## Summary

Ticket: AGE-2063. The review checks the edited NFR-001 and FR-030 text for consistency with
itself, with the rest of each document, with the README and with the code at 08cdcad.
`quire validate --scope . spec/non-functional/NFR-001-hook-time-budget.md
spec/functional/FR-030-hook-failures-are-spooled-and-reported.md` exits 0 on both. FR-030
carries two EARS warnings at line 41 (`unclassifiable`, `missing-subject`). They are
identical on main, so they come from before this PR.
`make check-matrix` passes.

What matches the code: FR-030's new `stdin_timeout` definition ("300 ms without new bytes,
or 900 ms from the start of the call"), give or take one 10 ms poll (SR-040 FND-002).
NFR-001's Scope bullet and its "counted from the same start" rule also match. The PR adds no
compatibility layer and touches nothing under `.github`.

## Verdict

**PASS WITH FINDINGS.** NFR-001's first normative clause is now false for `UserPromptSubmit`,
and the document states nothing about the case that makes it false (FND-001).

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | NFR-001's Statement opens: "The hook handler shall finish every call, from reading stdin to exit, within its event's total budget — 300 ms for `UserPromptSubmit`". The stdin clause the PR adds lets any call, `UserPromptSubmit` included, read for up to 900 ms before the event is known. Measured against the real binary, a late `UserPromptSubmit` ran 817 ms (`budget/budget_exhausted`) and a slow drip ran 918 ms (`stdin_timeout`), both against a 300 ms budget and a 400 ms threshold. The spec now contradicts itself, and the code follows the stdin clause. Nothing covers the new outcome that a late `UserPromptSubmit` payload gets `budget/budget_exhausted`: FR-030 lists stage `budget` only for refused herdr calls and skipped pops, and it says a skipped `UserPromptSubmit` pop is not recorded. Parts of NFR-001 are stale: "Why 300 ms before the payload …" and "The cost is a `SessionStart` whose payload takes longer than that: it is recorded" no longer hold for a payload that keeps arriving. The M&E table has no row for the new 900 ms bound. The README still gives `UserPromptSubmit` as 300 ms. Fix: in the Statement, make the stdin bound an explicit exception to the per-event budget, and say what a late payload records per event. Add an M&E row (slow drip: 900 ms target, 1000 ms threshold). Refresh the Dependencies rationale and the README sentence. | spec/non-functional/NFR-001-hook-time-budget.md:16-28, spec/non-functional/NFR-001-hook-time-budget.md:91, spec/non-functional/NFR-001-hook-time-budget.md:110-120, spec/functional/FR-030-hook-failures-are-spooled-and-reported.md:34-38, README.md:145 |
