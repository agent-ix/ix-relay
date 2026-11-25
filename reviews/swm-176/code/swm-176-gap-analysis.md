---
id: SR-111
title: "Gap analysis - FR-060 focused Scope implementation (SWM-176)"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-060-focused-scope-presentation.md (AC-1..AC-9); spec/functional/FR-048-session-work-monitor.md (AC-7, AC-9); spec/functional/FR-059-department-monitor-adapter.md; crates/relay-tui/src/monitor_*.rs"
review_set: subset
---

## Summary

Ticket: SWM-176. PR ix-relay#40 at frozen c41932e442465c64775f7d9566b85543995088d2 (base 8dd31a8). Gap analysis (quoin:gap-analysis, planless) of the changed production code against FR-060-AC-1..AC-9 and the underlying FR-048 and FR-059. All nine FR-060 acceptance criteria are bound by Rust trace attributes on real tests, and every new production behavior has an owning FR-060 requirement. Three low gaps remain in how the evidence reaches the behavior.

Plan completion: not assessed

## Verdict

**CONDITIONAL** - no untagged FR-060 criterion and no high finding; three low findings. This is a verdict on the changed requirements only. It is not a whole-repository assurance PASS: the repository-wide quire matrix still reports unrelated untagged criteria and trace-marker warnings in legacy files, which are baseline gaps outside this diff and are not attributed to it.

Latest verdict after disposition round 1 (head 2bdd2ed39567c86367f234168b4c680e881dace1): **CLEAR for the changed requirements** - FND-001 accepted-no-change with the limit documented, FND-002 and FND-003 fixed; no open finding. The whole-repository legacy matrix gap remains incomplete outside this diff and no whole-repository assurance PASS is claimed.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | low | The ix-board model carries no initiative-link completeness marker, so the implementation hard-codes links_complete false; the complete-links label "(no selected initiative)" of FR-060-AC-1 is unreachable from a real snapshot and is verified only on a hand-built DashboardView, and a project-only focus labels every project "(initiative links unavailable)" | crates/relay-tui/src/monitor_tracker.rs:360 |
| FND-002 | low | FR-060-AC-7 requires a stale model to label the hidden count stale, but the only source-state test drives a failed refresh; the age-based staleness path and the ended-session path are not exercised with a hidden count | crates/relay-tui/src/monitor_scope_tests.rs:583 |
| FND-003 | low | The Scope states required at the 80x24 target (hidden count, filter unavailable, last snapshot with error, footer mode) are asserted only at 100 to 130 columns; the 80x24 render test asserts rows and labels with no hidden or unavailable state | crates/relay-tui/src/monitor_scope_tests.rs:196 |

## Dispositions

Round 1 against frozen head 2bdd2ed39567c86367f234168b4c680e881dace1. Original findings above are unchanged. The reviewer ran no command; pass counts are parent-reported (67 passed, 1 ignored, 0 failed).

| FND | outcome | reason |
|---|---|---|
| FND-001 | accepted-no-change | The limit is real and unchanged: the SDK has no link-completeness marker and none was assumed. implementation/code-fix1/known-limits.md records it, names the real-summary tests that show "(initiative links unavailable)" and the hand-built fixture that shows the complete-links branch, and says only progress() needs to change when the SDK supplies a marker. No false unassigned label, API or pin was added. Root should carry the limit into the PR description so it is not read as AC-1 demonstrated on live data. |
| FND-002 | fixed | an_aged_snapshot_and_an_ended_session_label_the_count_at_80x24 ages fetched_at by two hours and sets session_ended, each with a real hidden count, and asserts the count is followed by the [last snapshot] label while a fresh app shows no label. error_with_last_snapshot_keeps_rows_counts_footer_mode_and_the_error_at_80x24 covers the failed-refresh path with the count. |
| FND-003 | fixed | Six new tests render at 80x24: mixed hidden and unknown, all-Unknown (filter unavailable, nothing hidden), error with last snapshot, aged and ended, and long stage names. assert_scope_essentials checks the blank row below Focus, the Initiative/project heading and the footer key and mode; assert_count_statements checks the hidden count, the includes-hidden statement and the unknown-retained statement. |

## Coverage

Plan completion: not assessed

Matrix verification: quire matrix --scope . --format json --strict, written to scratch, reports FR-060-AC-1 through FR-060-AC-9 each bound by at least one test carrying a Rust trace attribute (AC-8 by the ignored scale scenario monitor_scale_tests.rs:143 and by the non-ignored counter test monitor_scope_tests.rs:616, so AC-8 is not tagged only by an ignored test). The command exits 1 for the whole repository because of untagged criteria and trace-marker warnings in files this diff does not touch. FR-048-AC-7 and FR-048-AC-9 fixtures are unchanged by the diff except for the added status_category fixture fields. The disposition round was read-only and did not rerun the matrix; the fix adds tests carrying #[trace] for AC-4, AC-5, AC-6, AC-7 and AC-8 (production_event_loop_never_waits_for_a_pending_tracker_read, not ignored), so AC-8 now has a non-ignored production-loop test as well.

Reverse gap: new production items are Source.revision, App.show_completed, App.scope, ScopeState, ScopeProjection, FocusMode, the c key, the footer mode text and InitiativeRef; each is owned by FR-060 (Hierarchy, Completed projects, Responsiveness). The pub counters on ScopeState exist for FR-060-AC-8. The ix-board bump to d6e28f4 (0.6.0) is the prerequisite FR-060 names as a dependency. Added by the fix: unknown_projects and the unknown-retained statement (FR-060 Completed projects: unknown is retained), KeySource, Workers and event_loop (testability seam of the existing run loop, FR-060 Responsiveness and AC-8). No unspecified behavior was found. The SR-110 FND-007 layout question touches FR-060 Precedence.

Stubs and hollow evidence: none after the fix; the weak pending-read oracle recorded in SR-110 FND-004 is replaced by a loop-driven one. Semantic review was not run.

Not examined: live Linear data, live pane and agent state, and the release benchmark logs themselves; the benchmark numbers are root-owned and no latency or live CPU claim is made here.

## Examined Scope

```yaml
scope:
  - {id: "FR-060-AC-1", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "A shared-model fixture with two initiatives, a project under both, a project under one, and a project with complete links and no displayed initiative renders initiative rows with their projects nested and in model order."}
  - {id: "FR-060-AC-2", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "A fixture with missing, partial and ambiguous initiative links, and one with duplicate initiative names, labels the affected projects \"(initiative links unavailable)\" and never \"(no selected initiative)\" or unassigned."}
  - {id: "FR-060-AC-3", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "Three reported-scope fixtures select exactly one mode each: initiative/project named is Focused, nothing named is Whole-team fallback with its plain no-focus label, and tickets only is Ticket-only with the fallback label."}
  - {id: "FR-060-AC-4", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "With a fixture whose model supplies status categories for active, completed, canceled, unknown and active-with-all-tasks-done projects, only completed and canceled are hidden by default."}
  - {id: "FR-060-AC-5", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "A project hidden under two initiatives is counted once in the hidden-project count."}
  - {id: "FR-060-AC-6", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "With a fixture that lacks the status category, no project is hidden even when its display name is \"Completed\" or \"Canceled\", the scope line says the completed filter is unavailable, no hidden count is offered"}
  - {id: "FR-060-AC-7", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "After a successful model then a failed refresh, the rows and hidden count of the last snapshot remain with the last-snapshot and error labels, and a stale model labels the count stale."}
  - {id: "FR-060-AC-8", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "A bounded optimized-build scenario uses a fixture with at least 4 initiatives, 34 projects, one shared project and 2,000 leaf tasks, with no truncation of rows."}
  - {id: "FR-060-AC-9", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "Changing the source model, saved report, reported scope or toggle updates the displayed rows and counts."}
  - {id: "FR-048-AC-7", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: "Scope view with a blank row below Focus, an Initiative/project column heading, no redundant table title, and one terminal row per scoped initiative and project, prefixed by 🎯 and 📁"}
  - {id: "FR-048-AC-9", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: "Initiative, project and ticket selectors narrow that team boundary."}
  - {id: "FR-059-AC-4", path: "spec/functional/FR-059-department-monitor-adapter.md", role: context_only, excerpt: "TestBackend fixtures at 80x24 and 120x40 switch views with 1/2/3"}
```
