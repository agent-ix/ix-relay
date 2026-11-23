---
id: SR-1859
title: "code-review review of SWM-180"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; spec/functional/FR-066-bounded-work-history.md; PR45 changed source"
review_set: subset
---

## Summary

FAIL: 1 finding(s) require disposition. Reviewed candidate0761f1baa421337a9419b8e9efc20988751c91dc against origin/main using the existing worktree. No applicable AssuranceProfile was found under spec/.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | Idle history reads return events older than the seven-day retention limit. | FR-066; crates/relay-core/src/store/work.rs:191-195 |

## Scope

- FR-066 (examined), spec/functional/FR-066-bounded-work-history.md: When a lead reports scope, assignment, removal or explicit pause/resume, the relay SHALL append an immutable session-qualified transition atomically with its current-state update.
- FR-066-AC-1 (examined), spec/functional/FR-066-bounded-work-history.md: Scope, short stage changes, activity corrections and removals appear in order with stable event IDs; a malformed history storage key prevents current-state mutation.
- FR-066-AC-2 (examined), spec/functional/FR-066-bounded-work-history.md: Repeated cursor reads return identical retained events without modifying current assignments or mail state; malformed cursors and oversize page requests fail.
- FR-066-AC-3 (examined), spec/functional/FR-066-bounded-work-history.md: Events preserve verified native session/process identity; focus text is absent from history and bootstrap snapshots. Replacement sessions retain their separate reporting incarnation.
- FR-066-AC-4 (examined), spec/functional/FR-066-bounded-work-history.md: More than 4,096 writes evict the oldest events; a cursor preceding retained data reports truncation and the retained lower bound.

## Evidence and verdict

The Rust lane reviewed every changed source file, Lua mutation/read boundaries, shared wire types, CLI routing, validation and real Redis tests. No production code was edited. Existing main TUI code is outside this PR review.

Age trimming occurs only in HISTORY at line39. work_events XRANGE at lines191-192 reads from the requested cursor without an age floor. The stream TTL is extended on every transition, so old entries can remain after a quiet interval even though available_from_ms advances to now minus seven days.

Focused reproduction used an isolated Redis Unix socket /tmp/swm180-review.sock. Seed a valid activity event at server-time minus eight days and another at minus one day; set the remaining stream TTL to six days. This is the state left after a write two days earlier trimmed entries at its then-current seven-day boundary. Run target/debug/agent-msg --url redis+unix:///tmp/swm180-review.sock --prefix age-review: work events --after 0-0 --limit 100. The result was observed_at_ms=1795416860184, available_from_ms=1794812060184, yet the first event had at_ms=1794725642000 and truncated=false. Reads must exclude expired rows and report consistent lower-bound/cursor metadata while preserving read-only semantics. Physical retention must also meet the specified maximum age, or the contract must explicitly describe lazy physical expiry with a strict readable horizon.

Five real Redis integration tests passed with IX_RELAY_TEST_URL set. FR066 grammar validation passed. Exact-head aggregate make ci was reported passed by the lead; this reviewer did not run the aggregate gate.

The source diff contains13 changed files. Review artifacts are reviewer custody files only; the lead must retain them under reviews/ when landing. GitHub metadata showed the reviewed head is MERGEABLE, which does not override these findings or the final gate.

## Dispositions

Round1, verified fix10999dacefe7eafe310e4d5765272a94735bf18f and final frozen candidate7316c0477386b26e22aa99a8fe6e110cd63163a2. Original findings above remain unchanged.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | work_events computes the Redis server-time cutoff before XRANGE, excludes expired rows from events and coverage, advances empty cursors and reports truncation. The idle regression passes; FR066 now explicitly distinguishes the strict readable seven-day horizon from bounded lazy physical cleanup and inactive-stream expiry. |

Latest scoped code verdict: PASS, no new findings. Seven real Redis work tests passed independently at10999da; final7316c04 changes only requirement wording and a source comment, so no repeated behavioral test was needed. The final requirement passes quire validation. Local git merge-tree --write-tree against origin/mainff618de succeeded without conflicts for7316c04, yielding tree0d5b33abfc69a8aa4a57e65ff3445ccc5466e285. No live GitHub API mergeability probe or aggregate gate was run. The lead owns the final gate and exact-head merge.
