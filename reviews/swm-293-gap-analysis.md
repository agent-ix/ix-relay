---
id: SR-6126
title: "Gap analysis — ix-relay SWM-293 lock-skip count against FR-068, FR-045 and FR-029"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-068-hook-activity-events.md, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md, spec/functional/FR-029-doctor-reports-relay-health.md, crates/agent-msg/src/activity_state.rs, crates/agent-msg/src/doctor/**, crates/relay-watcher/src/fleet/**, crates/relay-watcher/src/otlp/series.rs"
review_set: subset
relationships:
  - type: references
    target: "ix://agent-ix/ix-relay/FR-068"
  - type: references
    target: "ix://agent-ix/ix-relay/FR-045"
  - type: references
    target: "ix://agent-ix/ix-relay/FR-029"
---

# SR-6126: Gap analysis — ix-relay SWM-293 lock-skip count against FR-068, FR-045 and FR-029

## Summary

Ticket: SWM-293. PR agent-ix/ix-relay#67 at `710dc2208c7beb2c408b06596b1995d92911da5b`, scoped to `git diff origin/main...HEAD`. Criteria examined: FR-068-AC-14 and FR-068-AC-15 (new), the FR-068 lock-state statements, the FR-045 `ix.relay.hook.activity_skipped` catalogue row, and the FR-029 code catalogue with FR-029-AC-1, AC-6 and AC-7.

Matrix: `quire matrix --scope . --format json` (quire 0.52.1). FR-068-AC-15 is `tagged` with 4 binders (`concurrent_updates_apply_exactly_the_counted_ones_and_a_held_lock_drops_within_the_bound`, `lock_skips_from_separate_processes_add_up_in_the_sidecar`, `lock_skips_are_read_from_the_sidecar_with_or_without_the_state_file`, `tc_068_11_the_hook_counts_are_exported_by_reason_and_left_out_when_unreadable`); FR-068-AC-14 is tagged. FR-029 and most of FR-045 read as `untagged` to quire on origin/main as well (the repo binds those through `tc_NNN_*` names and `make check-matrix`); that is pre-existing and out of this diff's scope. `make check-matrix` passes.

The brief asked for a judgement of intent against code for AC-15, so a targeted semantic review of FR-068-AC-15 was done; the full semantic review was not run.

## Verdict

**CONDITIONAL**: every new criterion is tagged, but the doctor clause of AC-15 has no bound test, the FR-029 catalogue does not list the new code, and FR-068 does not state the cap or how the count clears.

## Findings

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-001 | medium | FR-029's code catalogue has no row for `activity_lock_skipped`. The PR adds the code to `Code`, to the pinned names and to `every_code_fires_with_its_documented_severity_a_cause_and_a_fix`, whose own doc says "a severity that changes is a change to the catalogue in FR-029", but FR-029 was not edited. FR-029-AC-6 and AC-7 are judged against that catalogue, so the code and its warn severity are unspecified there. The hook spool section of the table needs an `activity_lock_skipped` row with its severity and when it clears. | spec/functional/FR-029-doctor-reports-relay-health.md:106-109; crates/agent-msg/src/doctor/report.rs:129-130; crates/agent-msg/src/doctor/checks.rs:2777-2779 |
| FND-002 | medium | The doctor clause of FR-068-AC-15 ("`agent-msg doctor` warns while it is non-zero") has no test carrying the AC-15 tag. The check row sits in `every_code_fires_with_its_documented_severity_a_cause_and_a_fix`, which has no `#[trace]` at all, and the fact reader in `gather` that turns `spool/activity.lockskips` into `activity_lock_skips` is not exercised by any test: every test `Env` leaves the file unset, so a reader that always returned 0 would pass the suite. The four AC-15 binders cover only the hook and watcher clauses. | crates/agent-msg/src/doctor/facts.rs:399-404; crates/agent-msg/src/doctor/checks.rs:1254-1269, 2656 |
| FND-003 | medium | FR-068 states neither the 1 MiB cap nor whether the count ever resets, and the code does both things the statement does not allow for. "SHALL count each update dropped" is false past 1 048 576 bytes, where `note_lock_skip` stops writing; and "warn while it is non-zero" with a count that never decreases means a permanent warning after a single skip, which conflicts with FR-029-AC-1 (`--strict` exits 0 when there are no hook failures). The spec should state the bound, how the cap is reported, and how or when the warning clears. See SR-6125 FND-001 and FND-003 for the code side. | spec/functional/FR-068-hook-activity-events.md:104-111, 144; crates/agent-msg/src/activity_state.rs:29, 100-102 |
| FND-004 | low | AC-15 requires that "the count survives across separate hook processes", but `lock_skips_from_separate_processes_add_up_in_the_sidecar` uses two `ActivityState` handles in one test process. flock contention between two open file descriptions does hold in one process, so the lock part is exercised, but no test runs a second process (for example the `agent-msg` binary under `CARGO_BIN_EXE`) appending to the same sidecar, and the test name claims more than it does. | crates/agent-msg/src/activity_state.rs:342-357; spec/functional/FR-068-hook-activity-events.md:144 |

## Coverage

- FR-068-AC-14: tagged (1 binder), examined, clean for the changed text.
- FR-068-AC-15: tagged (4 binders); hook-side and watcher-side clauses bound; doctor clause unbound (FND-002); cross-process clause tested in-process only (FND-004).
- FR-045 `ix.relay.hook.activity_skipped` row: examined against `series.rs` and `Collector`; the `lock` reason and the "no lock skip is recorded" absence rule match the code.
- FR-029 catalogue: missing the new code (FND-001).
- Reverse gap: no new code without an owning requirement apart from the unspecified cap (FND-003).
- Stubs: none found in the diff.
- Semantic review: targeted to FR-068-AC-15 only.
- Plan completion: not assessed

## New findings (disposition pass 1)

Head `7a3f2260ebc6f5e6cf5b7ae7e39b1ec05d4a3a7c`, fix commit reviewed as `git diff 710dc22..7a3f226`.

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-005 | low | The amended FR-068-AC-15 includes "reports an unwritable spool directory", but the only test is the `every_code_fires_…` row that sets `activity_spool_unwritable = true` on the facts. The `spool_unwritable` reader in `gather` has no test against a real directory, which is why its permission-bit logic (SR-6125 FND-005) passes the suite. | crates/agent-msg/src/doctor/facts.rs:651-656; crates/agent-msg/src/doctor/checks.rs:2861-2863; spec/functional/FR-068-hook-activity-events.md:159 |

## Dispositions

| FND | outcome | reason |
| --- | ------- | ------ |
| FND-001 | fixed | FR-029's catalogue now has the "hook activity drops" rows for `activity_lock_skipped`, `activity_write_failed`, `activity_skips_unreadable` and `activity_spool_unwritable`, with severities, the 24 h rule, "at least" at the cap and the reset. |
| FND-002 | fixed | `every_code_fires_with_its_documented_severity_a_cause_and_a_fix` now carries `#[trace("FR-068-AC-15")]`, and `tc_068_15_doctor_reads_the_skip_sidecars_and_warns_only_on_recent_growth` runs the real doctor with real sidecar files through `gather`. |
| FND-003 | fixed | FR-068 now states the 1,048,576 bound, "at least" reporting, the saturated gauge, reset by deletion, the 24 h warning window and the unwritable-spool report. |
| FND-004 | fixed | `separate_hook_processes_count_lock_skips_and_write_failures_apart` in `tests/hook_activity.rs` spawns real hook processes against a held lock; the in-process test is renamed `two_handles_on_one_state_file_share_the_lock_skip_count`. |
| FND-005 | fixed | Round 2 (head `822f03c5ad45cb8a7552ff3e6b9092cc6e72a71b`): `a_read_only_spool_directory_is_unwritable_and_a_writable_one_is_not`, tagged FR-068-AC-15, runs the real `spool_unwritable` reader against a real temp directory (writable, missing, 0555), skipping only the read-only half when run as root. |
