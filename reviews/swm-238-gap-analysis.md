---
id: SR-6109
title: "Gap analysis — ix-relay SWM-238 against FR-069, FR-070, FR-045"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-069-host-collector.md, spec/functional/FR-070-collector-source-health.md, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md, crates/host-probe/**, crates/relay-watcher/src/fleet/**"
review_set: subset
---

# SR-6109: Gap analysis — ix-relay SWM-238 against FR-069, FR-070, FR-045

## Summary

Ticket: SWM-238. PR agent-ix/ix-relay#58 at `a4caeb0ff9bd213c8025134c375872459760c88b`. This is a planless audit, scoped to the criteria this diff adds or touches. The computed matrix (`quire matrix --scope . --format json`) was read at the head.

- FR-069-AC-17, AC-18, AC-19 and AC-20 (new) are tagged by `tc_069_17`, `tc_069_18`, `tc_069_19` and `tc_069_20`, and each test asserts the AC text.
- FR-069-AC-1 to AC-4 kept their tagged tests through the move into `crates/host-probe/src/host.rs`. The matrix resolves them at the new paths.
- FR-070-AC-1 (`tc_070_01`, changed for the Linux-only `pressure` default) is still tagged and asserts the new default set.
- Stubs and coverage inflation: none found in the moved or new code. The new readers are real and tested from fixtures.
- Reverse gap: the moved `compilers_are_listed_from_a_proc_tree_with_their_cwd` test carried no trace tag before the move and still carries none. That predates this PR and is out of scope.
- The remaining untagged criteria repo-wide (for example FR-069-AC-15, Demonstration) predate this diff and are out of scope.

## Verdict

**CONDITIONAL.** Every criterion this diff adds is tagged and tested. There is one low trace finding and one low ticket-scope finding.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | `tc_069_20_a_host_without_psi_reports_pressure_unavailable_and_an_unreadable_one_failed` is traced to FR-070-AC-7 as well. That AC is about `temperature` only ("a profile that omits `temperature` ... `probe_ok` for `temperature` is 0"), and the test exercises `pressure`. The binding inflates FR-070-AC-7, which `tc_070_07` already covers. Drop the FR-070-AC-7 tag, or extend FR-070-AC-7 to name `pressure`. | crates/relay-watcher/src/fleet/mod.rs:1211-1213 |
| FND-002 | low | SWM-238's acceptance criteria also say "The matching _lead-bin shell scripts are replaced" and "Usable by ix-build". The PR does not replace any script, and the crate does not yet hold the probes ix-build/locked-build needs (SR-6108 FND-002). Neither the PR nor the ticket names the ticket that owns these. SWM-218 (retire _lead-bin monitoring scripts) may cover the scripts. The ticket should not be closed Done on this PR alone without a stated hand-off. | crates/host-probe/src/lib.rs:1-17 |

## Coverage

- Criteria examined: FR-069-AC-1, AC-2, AC-3, AC-4, AC-17, AC-18, AC-19, AC-20; FR-070-AC-1, FR-070-AC-7. All tagged.
- Semantic review (intent to test to code): not run as a separate opt-in pass. The tests for the new ACs were read against the AC text by hand.
- Plan completion: not assessed

## Dispositions

Round 1, fix head `0fb73d636d2f908dfb1efece9063a092309a321a`.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `tc_069_20` now carries only `#[trace("FR-069-AC-20")]`. FR-070-AC-7 stays tagged by `tc_070_07`. |
| FND-002 | deferred | The PR body states that SWM-238 stays open for script retirement and ix-build use. The locked-build probes and reading model are tracked in SWM-275. |
