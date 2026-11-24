---
id: SR-6119
title: "Gap analysis — ix-relay SWM-277 FR-072-AC-9 probe_ok export"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-072-ordered-lane-assignment-list.md, spec/functional/FR-070-collector-source-health.md, spec/functional/FR-069-host-collector.md, crates/relay-watcher/src/fleet/mod.rs, crates/relay-watcher/src/main.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-072"
    type: "references"
  - target: "ix://agent-ix/ix-relay/FR-070"
    type: "references"
---

# SR-6119: Gap analysis — ix-relay SWM-277 FR-072-AC-9 probe_ok export

## Summary

Ticket: SWM-277. PR agent-ix/ix-relay#63 at `2f01a976793638a58321bd2f848ed76bfa3d1231`, planless audit of the diff `origin/main...HEAD` against FR-072-AC-9, FR-070 and FR-069. Matrix computed with `quire matrix --scope . --format json` (quoin 0.36.10).

FR-072-AC-9 is tagged by three tests: `relay-core/tests/assignment.rs` `a_stale_host_refuses_raises_the_defect_and_a_refresh_restores_the_write` (refusal, list unchanged, defect stored, write succeeds after refresh), `agent-msg` doctor `checks.rs:3269` (doctor warning), and the new `relay-watcher` `tc_072_09` (export 0, then 1 after republish). Together they cover every clause of the amended AC; the new test passes against redis:7. Every FR-070 and FR-072 criterion is tagged. No stub, no `todo!`/`unimplemented!`, no ignored test standing behind the new tag.

## Verdict

**FAIL.** One high finding: the new exported source contradicts FR-070's closed source list (the same defect as SR-6118 FND-001, recorded here as a spec↔code gap). Otherwise the chain spec ↔ matrix ↔ test ↔ code holds for FR-072-AC-9.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | Code exports `ix_fleet_probe_ok{source="host_registry_stale"}`, but FR-070 says the sources SHALL be exactly eleven named values and gives no rule for a per-defective-host source outside the host profile; the spec does not own this behavior | spec/functional/FR-070-collector-source-health.md:22 |
| FND-002 | low | The "failed defect read keeps the last known defects" behavior in `refresh_fleet` has no owning requirement text and no test | crates/relay-watcher/src/main.rs:433 |

## Coverage

- FR-072-AC-9: tagged (3 tests), new watcher clause exercised by `tc_072_09`.
- FR-070-AC-1..12: tagged; unchanged by this diff. FR-070-AC-1 ("none for a source the profile declares unavailable") is unaffected because the new series is outside `Source::ALL`.
- FR-069: unchanged by this diff; FR-069-AC-15 is untagged on the computed matrix because it is verified by Demonstration; pre-existing and outside the diff, not counted.
- Reverse gap: the stale-host series loop in `Collector::fill` and `set_stale_hosts` are owned by FR-072 (behavior bullet 3); no other unowned code added.
- Semantic review: skipped (not requested).

Plan completion: not assessed
