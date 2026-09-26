---
id: SR-041
title: "Gap analysis of ix-relay PR #36 (AGE-2063) against NFR-001-M-5 and FR-030"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@08cdcad91dfd7e43a2970f8a2717c124f1212190; spec/non-functional/NFR-001-hook-time-budget.md (Statement, Scope, Measurement row M-5, Dependencies), spec/functional/FR-030-hook-failures-are-spooled-and-reported.md (decode codes), spec/tests.md (TC-N001-M-05 row), README.md, crates/agent-msg/src/stdin.rs, crates/agent-msg/src/budget.rs, crates/agent-msg/src/hook_run.rs, crates/agent-msg/src/hook_run/tests.rs, crates/agent-msg/tests/hook_stdin.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/NFR-001"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-030"
    type: "reviews"
---
# SR-041: Gap analysis of ix-relay PR #36 (AGE-2063)

## Summary

Ticket: AGE-2063. PR: ix-relay#36, head 08cdcad. This is a planless gap analysis. It follows
NFR-001-M-5 and FR-030's `stdin_timeout` code from the spec, to the TC-N001-M-05 matrix row,
to the tagged tests, to the code. Plan completion: not assessed.

Here is what the ticket asks for, re-measured against the code rather than taken from the
ticket text. The idle window is implemented (`stdin::collect`, `IDLE_WINDOW`) and restarts
on every chunk. The overall bound is implemented (`STDIN_TOTAL`, 900 ms, counted from
`started`). The ticket text says the read is "still capped by the event's overall budget".
The code caps it at 900 ms instead, because the event is unknown until the payload is
decoded (see SR-040). NFR-001, FR-030, README and tests.md are updated. FR-017 cites no
stdin deadline, so it needs no change.

The four new unit tests carry `Tracing: TC-N001-M-05` / `ACs: NFR-001-M-5` tags and are
named in the matrix row. None is a stub. `make check-matrix` passes (345 rows, unchanged,
because the PR edits an existing row).

Real-binary check, reproduced in this review: every drip pattern on every event exits 0
with exactly one record of the right kind (details in SR-040).

## Verdict

**PASS WITH FINDINGS.** The idle window is traced and tested. The 900 ms cap, which is the
bound that replaces the old 300 ms guarantee, is traced to TC-N001-M-05 but is not actually
tested at any level. So is the new outcome that a late `UserPromptSubmit` payload now
produces (FND-001).

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | NFR-001-M-5's new overall bound ("at most 900 ms from the start of the call in all") has no test that fails when it is violated. Setting `STDIN_TOTAL` to 1 h, or back to 300 ms (the pre-PR behaviour, which undoes the ticket at the wiring), passes the default lane and the `--ignored` benchmark lane. The `hook_run` tests drive `SlowSource`, which ignores the read budget, so `STDIN_TOTAL` is never exercised. Also untested is the outcome that is new with this PR: a `UserPromptSubmit` payload that completes after 300 ms is read and then recorded as `budget/budget_exhausted` (measured at 817 ms, no connection, one record) instead of `decode/stdin_timeout`. Fix: add a `hook_run` or binary case each for a payload finishing between 300 and 900 ms (`PostToolUse` served; `UserPromptSubmit` gives one `budget_exhausted` record and pops nothing), and a timed assertion for the cap (SR-040 FND-001). | crates/agent-msg/src/hook_run.rs:116, crates/agent-msg/src/hook_run.rs:719, crates/agent-msg/src/stdin.rs:342-351, spec/tests.md:366 |
