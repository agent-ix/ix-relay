---
id: SR-1872
title: "Gap analysis — hook activity event emission (PR 54)"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay PR54 head b558490; FR-068, FR-017-AC-8, NFR-001; crates/agent-msg, crates/relay-core"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-068", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-017", type: references }
  - { target: "ix://agent-ix/ix-relay/NFR-001", type: references }
---

# SR-1872: Gap analysis — hook activity event emission (PR 54)

## Summary

Ticket: SWM-202. This is a planless repository audit of PR 54 at head b558490, covering FR-068, FR-017-AC-8 and the new NFR-001 rows. The matrix was computed with `quire matrix --scope . --format json` (cli 0.52.1, engine 0.62.2). Reverse-gap and stub checks covered the diff's source files. Each test tagged to an FR-068 criterion was read against its criterion.

The out-of-scope items were not counted as findings:

- FR-068-AC-13, the plugin registration, deferred until after the dashboard lane.
- The `ix.relay.hook.activity_skipped` export in FR-068-AC-11, owned by SWM-205.

## Verdict

**FAIL**, because of one high finding and untagged criteria.

What holds:

- Every criterion from FR-068-AC-1 to AC-12, and FR-017-AC-8, has at least one test that names it.
- AC-1 to AC-7, AC-9, AC-10 and AC-12 are asserted by tests that exercise the real code. AC-7 is checked against real Redis.
- No stub, `todo!` or unreachable placeholder stands behind a tagged criterion.
- No new code lacks an owning requirement. The only addition beyond FR-068 is `activity_events`/`activity_len`/`activity_last`, read helpers that the tests and the FR-069 consumer use.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | The tests for FR-068-AC-8 and AC-11 pass, but both criteria are false. AC-8 says "Redis accepting connections and never answering … records one `activity` hook failure". AC-11 says "with Redis blackholed, after one failed write the next 5 seconds of tool-call hooks add no write time and are counted as skipped". Both tests inject latency or a fault only into the fake bus's `Activity` op, after connect, probe and derive have already succeeded. Against a real Redis that accepts and never answers, the call fails at `derive`, leaves a `derive`/`timeout` record and never records `activity`, writes or reads no backoff, and pays the full timeout on every call (measured: about 320 ms per `PreToolUse`, about 920 ms per `SessionEnd`). The tests check a different scenario from the one the criteria state. | crates/agent-msg/src/hook_run/tests.rs:2659-2725, crates/agent-msg/src/hook_run.rs:727 |
| FND-002 | medium | The computed matrix binds no test to FR-068-AC-1 to AC-12 or to FR-017-AC-8. All 13 criteria show as unbacked, with reason `unmarked-trace-id`. The new tests carry `/// Trace: FR-068-AC-n` doc comments, which the repo's traceability model does not bind. The bound convention here is the `#[trace("FR-…")]` attribute, for example `relay-core/src/test_support.rs:147`, and the repo has 332 such bindings. Until the tags change, the matrix reports this feature as untested. | crates/agent-msg/src/activity.rs:208, crates/agent-msg/src/activity_state.rs:133, crates/agent-msg/src/hook_run/tests.rs:2569, crates/relay-core/src/activity.rs:235, crates/relay-core/tests/activity.rs:46 |
| FND-003 | medium | The three NFR-001 rows this feature adds have no benchmark: NFR-001-M-7 (`PreToolUse` with Redis blackholed, at most 400 ms), M-8 (`Notification` and `SessionEnd` blackholed, at most 1000 ms) and M-9 (`PreToolUse` during a failure backoff, at most 400 ms). `tests/hook_budget.rs` still covers only `UserPromptSubmit`, `PostToolUse` and `Stop`. M-9 cannot pass in a real backoff until SR-1871 FND-001 is fixed, because a blackholed Redis never starts the backoff. | crates/agent-msg/tests/hook_budget.rs |
| FND-004 | low | No round-trip pin covers the new activity-only paths. FR-068 says the event write adds "at most one round trip to the hook's existing request count". `tests/round_trips.rs` pins only the existing `UserPromptSubmit` and `PostToolUse` paths, now one higher each. `PreToolUse`, `Notification` and `SessionEnd` (probe plus `EVAL`) have no pin, so a later sequential read on those per-tool-call paths would not fail any test. | crates/agent-msg/tests/round_trips.rs:320-377 |

## Coverage

- FR-068: 12 criteria are in scope (AC-13 is deferred). 12 have at least one named test. 0 are bound in the computed matrix (FND-002). 2 have tests that do not match the criterion (AC-8 and AC-11, FND-001).
- FR-017-AC-8: named by one test, which asserts the criterion, but it is not bound in the matrix (FND-002).
- NFR-001: M-7, M-8 and M-9 have no benchmark (FND-003).
- Reverse gap: none.
- Stubs: none.
- Semantic review: AC-to-test intent was read for every FR-068 tagged test as part of this review. A separate fan-out semantic pass was not run.
- Plan completion: not assessed
