---
id: SR-6119
title: "Gap analysis — ix-relay SWM-276 against FR-029, FR-053, FR-057, FR-068, NFR-001"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/{FR-029-doctor-reports-relay-health,FR-053-stranded-mail-is-forwarded-or-dead-lettered,FR-057-lane-focus-is-explicit-and-visible,FR-068-hook-activity-events}.md, spec/non-functional/NFR-001-hook-time-budget.md, crates/agent-msg/src/{activity_state,doctor/facts}.rs, crates/agent-msg/tests/**"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-029"
    type: "references"
  - target: "ix://agent-ix/ix-relay/FR-068"
    type: "references"
---

# SR-6119: Gap analysis — ix-relay SWM-276 against FR-029, FR-053, FR-057, FR-068, NFR-001

## Summary

Ticket: SWM-276. PR agent-ix/ix-relay#64 at `a83dc87cf548c7da39828c5695b77f7a4403eb75`, scoped to `git diff origin/main...HEAD`. The matrix was computed with `quire matrix --scope . --format json` (quoin 0.36.10, spec-artifacts-process v0.29.2). The criteria examined are those whose tests changed: FR-029-AC-3 (tc_029_03), FR-068-AC-11 (activity_state concurrency test), FR-053-AC-2 and FR-053-AC-12 (tc_053_02, tc_053_12), FR-057-AC-4 and FR-057-AC-5 (tc_057), and NFR-001-M-5 (hook_stdin). A targeted semantic check of test intent against AC text was run for these, because the brief asked for one. No spec text changed.

## Verdict

**FAIL.** One high finding (FND-001): the production doctor gains behaviour that no requirement owns, an env-var multiplier on its probe deadlines. That behaviour contradicts FR-029's "finish within 5 s however … herdr and the service manager are failing" whenever it is set above 2. FND-002 (medium): the ticket's own list includes a fourth flake (`address::unknown_containers_report_live_addresses`, seen in the #58 gate) that this PR neither fixes nor defers, and the ticket's acceptance box asks for 5/5 green `make ci` under load, while 3/3 was reported. FND-003 (low): the activity-state concurrency test is tagged FR-068-AC-11, but it tests lock concurrency (no lost update), which no FR-068 AC states.

Semantic checks that were clean:

- FR-057-AC-4/5: the new tc_057 wait plus `1s` matches "strictly greater than the threshold". The strict boundary cases (30,000 vs 30,001 ms) are not this test's job.
- FR-053-AC-2/12: the assertions on the notice content and the 6 KiB bound are unchanged. Only the hook invocation is wrapped (its retry gating is SR-6118 FND-002).
- NFR-001-M-5: `spawn_closed` keeps the "closed empty pipe → one malformed record, one ship attempt" outcome. The benchmark-lane wall-clock tests are untouched.
- No stubs, `todo!()` or placeholder returns were added.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | Reverse gap: `IX_RELAY_DOCTOR_PROBE_SCALE` (production doctor deadline multiplier) has no owning requirement. Set above 2, it lets `doctor` exceed FR-029's 5 s bound | crates/agent-msg/src/doctor/facts.rs:67-86 |
| FND-002 | medium | Ticket scope: the `address::unknown_containers_report_live_addresses` flake listed on SWM-276 is not addressed or deferred, and the "make ci green 5/5 under load" acceptance box is met only 3/3 | crates/agent-msg/tests/address.rs |
| FND-003 | low | Trace: the concurrent-update/lock test is tagged FR-068-AC-11 (backoff and skip counting under a blackholed Redis), but it asserts no lost update under file-lock contention, which is stated only in the activity_state module doc | crates/agent-msg/src/activity_state.rs:234 |

## Coverage

- FR-029-AC-3: examined. The matrix reads it as untagged because doctor.rs uses the doc-comment `ACs:` form that the declared grammar does not read. That is pre-existing repo-wide (431 untagged, including FR-053, FR-057 and NFR-001-M-5) and not introduced by this diff.
- FR-068-AC-11: tagged (10 binders); examined. See FND-003.
- FR-053-AC-2, FR-053-AC-12, FR-057-AC-4, FR-057-AC-5, NFR-001-M-5: examined (doc-comment traces, same pre-existing matrix note).
- Reverse gap: one item (FND-001).
- Semantic review: targeted to the changed tests (above), not repository-wide.
- Plan completion: not assessed
