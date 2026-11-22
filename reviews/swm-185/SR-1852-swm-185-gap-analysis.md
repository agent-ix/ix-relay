---
id: SR-1852
title: "Gap analysis - FR-048, FR-059, FR-061 and FR-064 criteria touched by the ix-board fcb0b358 consumer, spec criteria to tests to code (SWM-185)"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-064-named-focus-dashboard-reader.md; crates/relay-tui/src/monitor.rs; crates/relay-tui/src/monitor_ops.rs; crates/relay-tui/src/monitor_ops/quota.rs; crates/relay-tui/src/monitor_ui.rs; crates/agent-msg/src/focuses/resolve.rs; crates/agent-msg/src/focuses/tests.rs"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-048", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-059", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-061", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-064", type: references }
---

## Summary

Ticket: SWM-185. PR pending. Frozen source HEAD c97b4dd against base d5fa1c3. Planless, repository-driven gap analysis scoped to the criteria this change touches: FR-048-AC-5 and the paragraph added under it (line 43, which extends AC-5), FR-059-AC-5, FR-064-AC-10 and FR-061-AC-19. Plan completion was not assessed; this is a repository claim about those criteria and nothing more. FR-065 (view selector and F1 help) and the unchanged FR-048 criteria are not examined.

The change adds no criterion, no id and no relationship, and every behavior it adds has an owning criterion: the typed hold labels, timer and coalescing are FR-048-AC-5 with its added paragraph, the kept snapshot and its time are FR-059-AC-5 and FR-064-AC-10, and the new `Error` variant placement is FR-061-AC-19. The new tests carry `#[trace(...)]` attributes with existing ids only, with no docstring `Trace:` duplicates. No source or test stub backs a criterion. The audit found 3 gaps: one medium (the user-named case of an explicit backoff alongside other blockers is bound at no seam) and two low (three causes bound only at unit level, and the all-blockers obligation with no rendering-seam oracle). None is a missing feature.

## Verdict

**CONDITIONAL** - no `high` finding and no untagged criterion that I could establish (3 findings: 1 medium, 2 low). A planless CONDITIONAL asserts repository assurance for the listed criteria only (traceability, reverse gap, stubs) and nothing about plan completion. The defects in the code itself (SR-1851 FND-001, FND-002, FND-003) and the wording of the new paragraph (SR-1853, SR-1854) are not repeated here.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-048-AC-5: no test at any seam builds a decision whose primary state is an explicit server backoff together with another blocker or pending legacy reservations, the combination the contract names, and the cold-start backoff test pins the bare `Linear rate limited` as the expected detail | FR-048-AC-5, crates/relay-tui/src/monitor_ops.rs:974 |
| FND-002 | low | FR-048-AC-5: `RequestRejected`, `EvidenceUnavailable` without pending reservations and `RefusalUnexplained` are bound only by the unit mapping tests in quota.rs, so no test shows their label, remedy, absence of a countdown or absence of coalescing through `apply_tracker_refresh` or the screen | FR-048-AC-5, crates/relay-tui/src/monitor_ops/quota.rs:409 |
| FND-003 | low | FR-048-AC-5: the paragraph's obligation to report every blocker and the earliest reevaluation is asserted only on the stored `tracker.error` string, and the three UI tests assert labels, so nothing fails if the screen drops every blocker | FR-048-AC-5, crates/relay-tui/src/monitor_ui.rs:1438 |

## Coverage

- Reconciliation: no matrix of mine; the parent's official Quire matrix (matrix.json, exit 0 at 14545) was read as supplied data. I searched it for the names of the tests this change adds and found them bound: `a_reserve_hold_with_zero_local_usage_is_neither_a_rate_limit_nor_a_local_hold`, `without_a_snapshot_the_error_carries_the_typed_decision`, `each_decision_state_maps_to_one_cause_and_only_backoff_is_a_rate_limit`, `a_reserve_hold_is_labelled_as_a_hold_and_never_as_a_rate_limit`, `quarantined_quota_evidence_shows_no_countdown_and_is_not_a_rate_limit`, `a_server_backoff_decision_alone_carries_the_rate_limited_label`, `an_untyped_outcome_stays_unknown_unless_typed_freshness_reports_a_failure` and `a_typed_outcome_decides_before_the_freshness_fields`. `spans_round_up_to_whole_seconds_and_never_go_negative` has no `#[trace]` and is a helper test with no owning criterion; I raise nothing for it. The matrix also lists `unmarked-trace-id` entries for FR-048-AC-5 at monitor_ops.rs:1052 and 1114, older docstring citations this diff does not touch. The FR-048-AC-5 row of the matrix carries the criterion text without the added paragraph; the paragraph's obligations are therefore bound to the criterion only by the tests' tags.
- Plan completion: not assessed
- Criteria examined: FR-048-AC-5 and its added paragraph, FR-059-AC-5, FR-064-AC-10, FR-061-AC-19. Other criteria of these requirements and the legacy rows of the full-repository matrix are not assessed and are not clean assurance.
- Evidence: no run evidence of my own. The parent reports fmt exit 0, clippy exit 0, focused tests exit 0 (517 passed, 12 ignored, 26 summaries) and the matrix exit 0 at this source; I ran none of these and claim no fresh green. The typed refusals in the tests are fixture decisions returned by a fake `WorkspaceSource` through a real `ReadService` over a private cache; they are not produced by a live `QuotaGate` and ledger, which is upstream's coverage and is not assessed here.
- Untraced behaviors / stubs: inventoried 10 behaviors (typed outcome capture, outcome-first classification, typed-freshness fallback for `Unknown`, the closed `Cause`, decision wording with every blocker, reevaluation shown relative to the attempt, waiting versus coalescing causes, `TrackerStatus` and its label, the agent-msg placement, the pin); untraced at their integration seam: 3 (explicit backoff with other blockers or pending reservations, FND-001; three causes through the dashboard, FND-002; blockers on screen, FND-003); source stubs 0; test stubs 0. No `TODO`, `FIXME`, `todo!`, `unimplemented!` or new `#[ignore]` in the delta.
- Semantic review: not run as the separate opt-in pass. The per-criterion judgement below is the reviewer's reading of the tests.
- Upstream: the accepted SDK at fcb0b358 was read only where the consumer touches it; an exhaustive upstream audit and the SDK's own quota tests were not assessed.

### Per-criterion assessment

| AC | Evidence assessed | Strength |
|---|---|---|
| FR-048-AC-5 | Real `ReadService` tests for reserve, local window, backoff, quarantine, simultaneous blockers, lock refusal, cache reuse, recovery and the cold start; the 30-minute countdown asserted equal to `now + TRACKER_REFRESH_INTERVAL` while the decision reports 12m 03s; `rate` absent from every non-backoff wording | partial: strong for the kept-snapshot holds named in the tests, weak for the combinations in FND-001 and FND-002 and for the screen in FND-003 (defects: SR-1851 FND-001, FND-002, FND-003) |
| FR-059-AC-5 | Kept snapshot retains `fetched_at`, shows the typed notice, raw cause absent, recovery clears the notice and the status | strong |
| FR-064-AC-10 | UI tests keep `Tasks: 1/2 done` and the last-snapshot label beside each hold; a failed read never shows healthy | strong |
| FR-061-AC-19 | Table test `each_sdk_failure_is_placed_under_one_of_the_contract_reasons` gains the `QuotaAdmissionRefused` row, which fails if the arm moves from `failed`; the 300 s, deadline and no-stale tests are untouched | strong |

### Bindings checked

The bindings listed in the comment payload were read in the test bodies. None is tagged to the wrong criterion. `without_a_snapshot_the_error_carries_the_typed_decision` is tagged FR-048-AC-5 and FR-059-AC-5; it exercises a read with no snapshot, which FR-064-AC-10 describes more closely than FR-059-AC-5, and I judge the tag acceptable because FR-048-AC-5 owns the behavior.

## Finding details

### FND-001 (medium) No oracle for an explicit backoff beside other blockers

The paragraph says a held read reports every blocker and the earliest reevaluation the service gave, and the user's contract names the quarantine with an actual explicit backoff. Every fixture decision in the delta has one hold or a primary `EvidenceUnavailable`: `reserve_held`, `local_held`, `server_backoff` (one blocker), `quarantined` and `simultaneous` (primary `EvidenceUnavailable`, three blockers, four pending reservations). The SDK derives the primary state by precedence, so a backoff outranks every other blocker (quota_decision.rs:27-36), and quarantined reservations add an `EvidenceUnavailable` blocker (quota.rs:770-775). No test therefore builds a decision whose primary state is `ServerBackoff` and which also holds another blocker or `legacy_pending_reservations > 0`, at the `Notice` level, through the worker, or on screen, so the wording, the remedy and the all-blockers obligation for that decision are bound by nothing. The one cold-start backoff assertion (monitor_ops.rs:974-977) expects `Some("Linear rate limited")` as the entire detail, which is the loss SR-1851 FND-001 reports, so that test cannot fail when the contract is met. Request: add a fixture whose primary state is a backoff and which carries another blocker and pending reservations, and assert its label, countdown, every blocker, the earliest reevaluation and the reconcile action on the kept-snapshot path and the cold path; replace the cold-start assertion with the decision's detail.

### FND-002 (low) Three causes are bound only at unit level

`each_decision_state_maps_to_one_cause_and_only_backoff_is_a_rate_limit` (quota.rs:409), `only_waiting_holds_coalesce_and_only_backoff_is_labelled_rate_limited` and `a_nonquota_failure_names_no_cause` bind the `Cause` mapping, `waits()`, the absence of `rate` and `429`, and the neutral failure wording for every variant. The dashboard-level tests (the `Scenario` enum at monitor_ops.rs:440 and the three UI tests) never use `RequestRejected`, `EvidenceUnavailable` with no pending reservations, or an `Available` decision on a refusal. For those three, `tracker_status`, the evidence label (`Tracker request cannot be admitted`, `Tracker quota evidence unavailable`, `Tracker quota refused`), the remedy line, the missing countdown and the missing coalescing in the worker are asserted nowhere, though FR-048-AC-5's added paragraph names `request not admissible` and `quota evidence unavailable` as labels the monitor SHALL show. Request: add `Scenario` variants and one parameterized dashboard assertion for these three, using the existing fixtures and the closed-enum pattern.

### FND-003 (low) The all-blockers obligation has no rendering-seam oracle

The detail with every blocker is asserted on `app.tracker.error` in monitor_ops.rs (for example the `hold: ...` strings of `simultaneous_blockers_all_reach_the_dashboard_under_one_primary_summary`). The three UI tests (monitor_ui.rs:1438, 1456, 1474) render at 100 columns and assert the label, `next attempt in`, `last snapshot`, `Tasks: 1/2 done` and the absence of rate-limit words. Because the lead views show the detail only as the tail of one truncated row, the unit assertions hold while no blocker is visible, so a rendering regression and the current behavior (SR-1851 FND-002) both pass. This is the missing oracle for that defect, a different unit from the code path there. Request: when SR-1851 FND-002 is fixed, add a rendering test over a multi-blocker decision at a realistic width that asserts a blocker and the earliest reevaluation are on screen.

## Verdict after disposition pass 1

Not run at the review pass. Dispositions follow for this same reviewer session after the fix round.

## Dispositions (round 1, all 3)

Disposition pass 1 at frozen source f4cfaad (atop c97b4dd, base d5fa1c3), PR 47, by the same reviewer, judged from the current source, tests and spec in the worktree; fix1-ready.md, fix1-compile-ready.md and fix1-fixture-ready.md were read as claims and the source as fact. I ran no command and no matrix. Parent-supplied facts relied on, none re-run by me: fmt exit 0; four consumer crates all-targets clippy 84052 exit 0; final TUI all-targets clippy 81901 exit 0; TUI tests 48195 exit 0 (45 inbox and 142 dashboard, 187 passed, 1 existing ignored); the final computed matrix (fix1-final-matrix.json) exit 0 with FR-048-AC-5, FR-059-AC-5, FR-064-AC-10 and FR-065-AC-10 tagged, and whole-repository legacy coverage not claimed clean; Quire validate of FR-048 and FR-065 exit 0 with only the two legacy line 16 EARS warnings. An earlier direct TUI run 83387 had two fixture failures, since fixed in test code with the assertions kept, and an earlier combined run 97349 stopped on two unchanged doctor deadline failures, with the isolated doctor run 28442 exit 0. The initial c97b4dd combined run (517 passed, 12 ignored) is initial evidence only and no new full combined pass is claimed.

| FND | Outcome | Reason |
|---|---|---|
| FND-001 | fixed | The combination the finding names is now built and asserted at three seams, and the bare cold-start assertion is gone. `fixture::mixed_backoff` is a decision whose primary state is a server backoff over an evidence blocker and a local hold, and `a_backoff_with_pending_reservations_keeps_the_backoff_every_blocker_and_the_action` (quota.rs:658) asserts its label, every blocker and the reconcile action. The kept-snapshot path is `a_backoff_with_quarantine_shows_every_decision_fact_and_the_reconcile_action` (monitor_ops.rs:1014) through a real `ReadService` over a private cache, and the cold path is `a_cold_backoff_with_quarantine_carries_the_same_facts_as_a_warm_one` (monitor_ops.rs:1042), which asserts the SDK still returns the legacy error, the captured outcome is `QuotaRefused`, and the cold facts and actions equal the warm ones. The screen is `a_backoff_with_quarantine_keeps_the_countdown_and_the_reconcile_action_in_view` (monitor_ui.rs:1534). All four carry `#[trace]` to FR-048-AC-5 and the screen test also to FR-064-AC-10. |
| FND-002 | fixed | `refusals_that_do_not_wait_show_their_label_and_no_countdown` (monitor_ui.rs:1499) is one parameterized dashboard test over `rejected`, `evidence_only` and `unexplained`. For each it asserts the label (`Tracker request cannot be admitted`, `Tracker quota evidence unavailable`, `Tracker quota refused`), the `Tracker action` row for the first two and its absence for the third, `Tasks: 1/2 done`, and the absence of `next attempt`, `rate-limited`, `rate limited` and `429`. The worker side is bound by `a_cold_refusal_keeps_its_decision_and_its_waiting_rule` (a quarantine has no retry time and is not protected, monitor_ops.rs:1085) and `an_unexplained_refusal_does_not_wait_and_names_the_source_action` (monitor_ops.rs:1203); that a request not admissible does not coalesce stays bound at unit level by `only_waiting_holds_coalesce_and_only_backoff_is_labelled_rate_limited` over every cause, which is what the finding already accepted for the mapping. |
| FND-003 | fixed | The oracle is at the rendering seam. `the_inspector_reaches_every_blocker_and_action_by_scrolling_and_closing_restores_scope` (monitor_nav.rs:909, tagged FR-048-AC-5 and FR-065-AC-11) renders the inspector at 100x14 and asserts, on screen, the summary and the action on the first frame and then the attempt, the pending reservations, the earliest reevaluation and each hold, including the wrapped last one, by scrolling, so a dropped blocker fails the frame. `a_backoff_with_quarantine_keeps_the_countdown_and_the_reconcile_action_in_view` and the `Tracker action` rows cover the lead-view evidence row. The old failure mode (unit assertions green while no blocker is visible) is closed. |

## New findings (disposition pass 1)

None. The criteria in scope are still FR-048-AC-5, FR-059-AC-5, FR-064-AC-10 and FR-061-AC-19 for this artifact, with the inspector tests adding FR-065-AC-3, AC-4 and AC-11 tags that I read in the test bodies; none is tagged to the wrong criterion. I found no new stub, `todo!`, `unimplemented!` or `#[ignore]` in the changed tests and no behavior without an owning criterion: the inspector and actions are owned by the added FR-048 text and the four FR-065 edits, which add no id.

## Review context (disposition pass 1, appended)

Same disposition session as SR-1851, at the same source and with the same parent-supplied facts. Correction to the earlier context: the original pass used the older cached quoin 0.29.4 and dev-tools 0.2.0. This pass read the current catalog files, all readable, under /Users/peter/.codex/plugins/cache/agent-ix/: quoin/0.31.1/skills/gap-analysis/SKILL.md with references/step-3-matrix-verification.md and step-4-underspecified-code.md; dev-team/0.4.1/skills/reviewer/SKILL.md; dev-tools/0.3.2/skills/code-review/SKILL.md, rust-review/SKILL.md and rust-style/SKILL.md with references/testing.md. Not opened: gap-analysis steps 1, 2, 5 and 6 (no plan, no scope-expansion or report step applies to a planless disposition). Nothing in the original findings depends on the older copies. The original text of this artifact above is unchanged.

## Current verdict (after disposition pass 1)

**PASS** - all 3 findings are fixed on the source as read, no new finding was found, and the parent's computed matrix reports the touched criteria tagged. Plan completion was not assessed and whole-repository legacy matrix gaps are not claimed clean. This is a reading of source and the parent-supplied results above; I ran no gate and no matrix. The overall PR is not yet clear: SR-1853 has one new low finding.
