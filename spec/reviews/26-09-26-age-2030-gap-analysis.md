---
id: SR-007
title: "Gap analysis of FR-044 (socket delivery) at ix-relay#26 head 36c2c46"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@36c2c46626cdf092005a5a05bd525f7a9780e29b; spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md, spec/tests.md (FR-044 and TC-044-01..09 rows), crates/relay-watcher/src/socket/**, crates/relay-watcher/src/{main,watch,episodes,supervise,deliver}.rs, crates/relay-core/src/{socket_status,config}.rs, crates/relay-core/src/store/{inflight.rs,lua/renew.lua,lua/deadletter.lua}, crates/relay-core/tests/inflight_model.rs, crates/relay-watcher/tests/startup.rs, crates/agent-msg/src/doctor/**"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/TM-001"
    type: "references"
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
---
# SR-007: Gap analysis of FR-044 at ix-relay#26 head 36c2c46

## Summary

Ticket: AGE-2030. PR: ix-relay#26 at 36c2c46. This is a planless audit, scoped to FR-044
and the code this PR adds or changes. It covers matrix traceability, reverse gaps and
stubs, and whether the round-3 behavior statements in FR-044 have backing tests.

`quire coverage --scope . --json` cannot read this repo's trace grammar: it reports 1
backed row of 585 across the whole repo. That predates this PR (the repo keeps its own
`make check-matrix`). Matrix verification therefore fell back to a grep index. Every test
name listed in the TC-044-01..09 rows exists: 95 of 95, and no `tc_044_*` test is
unlisted. `make check-matrix`: 299 rows, OK. Two tests carry wrong or missing tracking tags.
No stubs: every new function has a real body, and no test only asserts constants, except
`tc_044_06_the_default_ack_wait_fits_in_the_default_tick_budget`, which also pins the
production `SocketConfig` and so is not a tautology. No reverse gap: the new code (budget,
settlement retry, grace, adoption checks, `stop_on`) is all owned by FR-044 statements.

Earlier rounds (1-3, posted on the PR before this skill) are re-verified in SR-006. For
this audit, the round-3 fix table's claim that "tests use the production constants" holds:
`Home::production()` is `SocketConfig::with_dirs` (tests.rs:2487), used by the AC-6
production tests.

## Verdict

CONDITIONAL. There are only low-severity gaps. The medium code defects are in SR-006.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | Two tracking-tag defects. `tc_044_04_a_message_that_outlived_its_wait_is_acked_when_the_transcript_shows_it` is listed under TC-044-04 in spec/tests.md, but its doc tag reads `Tracing: TC-044-05` / `ACs: FR-044-AC-5`. `tc_044_05_a_delivered_receipt_alone_does_not_ack_before_the_grace_ends` has no `Tracing:` or `ACs:` tag at all. | crates/relay-watcher/src/socket/tests.rs:1361-1367, crates/relay-watcher/src/socket/tests.rs:3109-3110, spec/tests.md |
| FND-002 | low | FR-044 statements with no backing test at this head. (a) "in a 0700 directory": no test checks the directory mode, and it is not true for an existing dir (SR-006 FND-008). (b) "none starves" and "each later tick reaches the next": backed only by a two-tick check (tests.rs:2661-2665), not by a bound on how many ticks the last lead waits (SR-006 FND-006). (c) "shall not dead-letter an approved message as never approved": tested within one watcher's life only, not across a restart (SR-006 FND-004). (d) "at most five failed tries … then a `settlement_failed` note": the give-up is unit-tested (tests.rs:3041), but what happens to the batch afterwards is untested and unspecified (SR-006 FND-003). | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md, crates/relay-watcher/src/socket/tests.rs |
| FND-003 | low | AC-4 has no test for a transcript that already exists and receives the line before the watch's baseline is taken. `FakeClaude` polls `accept` every 5 ms, and most tests start with no transcript file (so `Watch::from_start` applies), which is why the round-3 baseline regression (SR-006 FND-001) passes every test. Add a receiver that records as soon as it reads, with a pre-existing transcript. | crates/relay-watcher/src/socket/tests.rs:162-203, crates/relay-watcher/src/socket/deliverer.rs:725-728 |

## Coverage

- Matrix rows in scope: FR-044 (9 rows), TC-044-01..09 (9 rows). All 18 rows name tests that
  exist: 95 distinct `tc_044_*` names, all defined.
- Tagged-test index (grep fallback, since `quire coverage` backs 1/585 repo-wide, which
  predates this PR): 93 of 95 tests carry a matching `Tracing: TC-044-NN` tag; 1 has a
  mismatched tag and 1 has none (FND-001).
- `make check-matrix`: 299 rows, OK.
- Reverse gaps: none found in the changed code.
- Stubs and coverage inflation: none found.
- Semantic review (intent↔test↔code): not run as the opt-in lane. The reviewer brief
  directed attacks on the round-3 fixes instead, and those results are in SR-006.
- Plan completion: not assessed.

## New findings (disposition pass 3)

Checked at 1df698cd1ba5ffd5919c1d6d2b606d70ae7f8180 (squashed and rebased onto main 2e4bc1d). A
grep index over the whole crate tree: 114 distinct `tc_044_*` tests defined; every `tc_044_*`
name listed in the FR-044 and TC-044 rows is defined; `make check-matrix`: 322 rows, OK; no
duplicate TC or AC row next to the FR-045 rows.

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-004 | low | One tagged FR-044 test is in no matrix row. `tc_044_09_an_open_status_directory_is_untrusted_in_the_facts` (added in the last fix round, 12df7e7) carries `Tracing: TC-044-09` / `ACs: FR-044-AC-9`, but neither the FR-044-AC-9 row nor the TC-044-09 row of spec/tests.md lists it, so the matrix undercounts AC-9's evidence and `make check-matrix` does not notice. Add it to both rows. | crates/agent-msg/src/doctor/facts.rs:841-847, spec/tests.md (FR-044-AC-9 and TC-044-09 rows) |

## Dispositions

Re-checked at e002abfb15b106f4ff82424782f44aa3ccf46cee (fix commit e002abf). A grep index over
the whole crate tree: 106 FR-044 tests, every one carries `Tracing: TC-044-NN` and
`ACs: FR-044-AC-NN` matching its row, every `tc_044_*` name listed in spec/tests.md is defined
and every defined one is listed in its own row; `make check-matrix`: 306 rows, OK.

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | e002abf: `tc_044_04_a_message_that_outlived_its_wait_is_acked_when_the_transcript_shows_it` now tagged `Tracing: TC-044-04` / `ACs: FR-044-AC-4`; the grace test was renamed `tc_044_12_a_delivered_receipt_alone_does_not_ack_before_the_grace_ends` and tagged TC-044-12 / FR-044-AC-12. |
| FND-002 | fixed | e002abf: (a) `an_existing_open_status_directory_is_made_private_and_an_untrusted_one_is_not_read` checks the dir mode from a pre-existing 0755 dir; (b) `tc_044_14_a_healthy_lead_behind_twenty_wedged_ones_is_served_within_one_round_of_ticks` bounds the wait at 21 ticks; (c) `tc_044_12_an_approved_batch_adopted_after_a_restart_continues_its_grace` covers the restart; (d) the give-up is gone and `tc_044_11_a_dead_letter_write_that_fails_for_ever_never_re_posts_the_message` tests the batch over 20 ticks. All pass on Redis. |
| FND-003 | fixed | e002abf: `tc_044_04_a_receiver_that_records_at_once_into_an_existing_transcript_is_never_missed` (pre-existing transcript, 0/20/200 extra project dirs, 200 rounds). Mutation-checked by the reviewer: moving the watch after the write fails it. |
