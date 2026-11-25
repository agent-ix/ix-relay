---
id: SR-6110
title: "Gap analysis — ix-relay SWM-259 against FR-072"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-072-ordered-lane-assignment-list.md, spec/functional/FR-070-collector-source-health.md, crates/relay-core/src/assignment.rs, crates/relay-core/src/store/assignment.rs, crates/relay-core/src/store/lua/assign*.lua, crates/relay-core/src/store/moves.rs, crates/agent-msg/src/assign.rs, crates/relay-core/tests/assignment.rs, crates/agent-msg/tests/assign_cli.rs"
review_set: subset
---

# SR-6110: Gap analysis — ix-relay SWM-259 against FR-072

## Summary

Ticket: SWM-259. PR agent-ix/ix-relay#59 at `a7b60e3cc6af3a376c52797231e4221617b03472`. This is a planless gap audit; plan completion was not assessed.

Matrix: `quoin matrix --json` (quoin 0.36.10) shows FR-072-AC-1 to AC-11 all `tagged`: AC-1 and AC-7 on two tests each, the rest on one. Each tagged test was read and run against redis:7: relay-core `assignment` 12/12 and agent-msg `assign_cli` 1/1.

Reverse gaps: one. `Relay::assignment_probe_defects` is a public API with no caller outside its own test (FND-001). Every other new production path maps to FR-072.
Stubs: none.

## Verdict

**GAPS.** Two medium findings: AC-9's "probe defect is raised" is tagged as covered, but the FR-070 defect is never raised. AC-7's move behaviour is tested only on the success path, and the paths it does not test break the behaviour. One low: AC-1's case test covers writes only.

Clean: AC-1 to AC-6, AC-8, AC-10 and AC-11 are each asserted by their tagged tests as written: list order and refs, one transition per write, role and own-lane refusals with history unchanged, 51 tickets, overlong fields and unknown lanes refused, count and age bounds with truncation, byte-identical `DUMP` across reads, no work or focus keys, `order_id` stored exactly, and CLI writes needing the role while reads do not.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | FR-072-AC-9 is tagged as covered, but "the probe defect is raised" is checked only through the private `assign-defects` hash. FR-072 says the refusal SHALL raise the collector probe defect of FR-070, and nothing exports, alerts on or reports that hash. | crates/relay-core/tests/assignment.rs:409-456; crates/relay-core/src/store/assignment.rs:240-260 |
| FND-002 | medium | FR-072-AC-7 ("the list is readable under the new address") is tested only when the move succeeds. There is no test for a case-only rename, a target address that already holds a list (`-8`), or a follow error. In each of these cases the list stays under the old address, which breaks the behaviour "SHALL keep its list under the new address". | crates/relay-core/tests/assignment.rs:316-387; crates/relay-core/src/store/assignment.rs:207-211,402-419 |
| FND-003 | low | The FR-072-AC-1 case test `the_registry_spelling_of_an_address_names_the_list` checks only that a write with another spelling lands on the registry key. It never reads by the other spelling, and that read returns `null` (SR-6109 FND-004). | crates/relay-core/tests/assignment.rs:486-497 |

## Dispositions

Round 1. The fix head is `602d11a861f29d9be1765beac3e9c637a94bf644` (`a7b60e3..602d11a`). relay-core `assignment`: 19/19 pass. agent-msg `assign_cli` and doctor `tc_072_12` pass.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | still-open | `agent-msg doctor` now reports the refusal (`assignment_host_registry_stale`), so it is no longer silent. FR-072 still says the refusal "SHALL raise the collector probe defect of FR-070" (`ix_fleet_probe_ok`, the ProbeFailing alert), and nothing raises it. No relay-watcher ticket is filed (Linear SWM has none). To close: file the relay-watcher export ticket blocking FR-072-AC-9 (then this becomes deferred), or amend FR-072 to name the doctor warning as the defect. |
| FND-002 | fixed | AC-7 now has tests for a case-only rename (`a_case_only_rename_moves_the_list`), an occupied target, a store failure and a tab-name swap. The settlement gap is SR-6109 FND-009. |
| FND-003 | fixed | `any_spelling_of_an_address_reads_and_writes_the_one_list` reads and writes by another spelling. |

Round 2. The head is `79d1d6b9b0dcf06d8d6400fd62b1abb71964acdc`, rebased onto `c7b3c6a`. relay-core `assignment`: 20/20 pass.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | deferred | SWM-277 (relay-watcher exports FR-072 stale-host assignment refusals as the `probe_ok` defect; Backlog, project Fleet W1) blocks FR-072-AC-9 from counting as done. FR-072 now names the `assignment_host_registry_stale` doctor warning as the surfacing until SWM-277 lands. |
