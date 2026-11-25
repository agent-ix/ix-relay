---
id: SR-110
title: "Code review (Rust lane) - FR-060 focused Scope implementation (SWM-176)"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-tui/src/{monitor.rs,monitor_main.rs,monitor_ops.rs,monitor_scope.rs,monitor_tracker.rs,monitor_ui.rs,monitor_views.rs,monitor_scope_tests.rs,monitor_scale_tests.rs}; Cargo.toml; Cargo.lock"
review_set: subset
---

## Summary

Ticket: SWM-176. PR ix-relay#40 at frozen c41932e442465c64775f7d9566b85543995088d2 (base 8dd31a8), Rust lane of code-review over the production and test diff. The design is sound: one cached shared summary keyed on source revisions, a separate projection keyed on summary generation and the toggle, filtering decided only from the SDK category, and no per-draw summarize call. No blocking correctness defect was found in the cache, hierarchy, ordering, key handling or replacement fences. Two medium findings concern required Scope text that is clipped at the 80x24 target, and four low findings concern evidence quality.

## Verdict

**CONDITIONAL** - no high finding. Two medium findings (hidden-count text and footer mode text clipped at 80 columns and untested there) and four low findings, all fixable in the same PR without a new layer.

Latest verdict after disposition round 1 (head 2bdd2ed39567c86367f234168b4c680e881dace1): **CONDITIONAL** - all six original findings are fixed; one new low finding (FND-007) is open.

Latest verdict after disposition round 2 (head 7ae6348891a141b2be699141a14bbbc27297b85f): **CLEAR** - every finding (FND-001 to FND-006 and FND-007) has a latest outcome of fixed; no open finding. The whole-repository legacy matrix gap outside this diff remains incomplete and no whole-repository assurance PASS is claimed.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | The hidden-project count, the progress-includes-hidden statement and the filter-unavailable text are the last spans of a wrapped summary whose height is clamped to 4 rows by a char-count estimate, so they can be clipped silently at 80x24; no test renders a hidden or unavailable state at 80x24 | crates/relay-tui/src/monitor_ui.rs:111 |
| FND-002 | medium | Footer text "c completed filter unavailable" makes the control line 87 columns, so it is clipped at 80x24; and any source error replaces the whole control line, dropping the c key and mode while rows are retained; tests only render the footer at 120 columns | crates/relay-tui/src/monitor_ui.rs:332 |
| FND-003 | low | The after benchmark draws once before starting its timer (warm-up includes the cold summary and projection build for the first size), whereas the before result counted 435 draws including the cold one, so the 434 vs 435 difference is not like for like and is not labeled | crates/relay-tui/src/monitor_scale_tests.rs:178 |
| FND-004 | low | The pending-read check asserts only that the worker channel stays empty while handle_key and draw run; neither touches the worker, and the real run loop is not exercised, so the assertion cannot fail and does not demonstrate non-blocking input | crates/relay-tui/src/monitor_scale_tests.rs:190 |
| FND-005 | low | The selection-follows-placement test calls restore_selection and remember_selection directly with hand-set builds instead of pressing c or applying a shrinking refresh through render; and the remembered key survives a view switch that resets the table, so a rebuild while away jumps selection to a stale placement | crates/relay-tui/src/monitor_scope_tests.rs:457 |
| FND-006 | low | completion_supplied is true when any displayed project has a non-Unknown category, so a scope whose projects all carry Unknown shows the filter as unavailable, and a mixed scope says hiding completed with no statement that Unknown-category projects are retained | crates/relay-tui/src/monitor_tracker.rs:315 |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-007 | low | The fix moves the Scope filter statement onto its own summary row and shifts the expected y of an existing FR-048/FR-059 Scope fixture by one for Scope rows; FR-060 Precedence permits Scope fixture changes only for rows, labels and the key, so either the precedence text or the layout needs to account for the added row | crates/relay-tui/src/monitor_ui.rs:924 |

## Dispositions

Round 1 against frozen head 2bdd2ed39567c86367f234168b4c680e881dace1 (coder fix of c41932e). Original findings above are unchanged. Parent-run evidence cited: 67 passed, 1 ignored, 0 failed (code-fix1-tests2.log); clippy all targets 0 (code-fix1-clippy2.log); formatter exit 0; optimized build exit 0; the reviewer ran nothing.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed | monitor_ui.rs:111 builds the summary as scope line, then a separate filter line (hidden count, includes-hidden, omitted, unknown retained, last-snapshot label), then the stage list, so the stage list can no longer push the statements out; scope_height now uses wrapped_rows (word-wrap aware) bounded by the rows the other panels leave instead of a clamp of 4. Tests at 80x24: hidden_count_and_mixed_unknown_statements_fit_80x24, many_long_stage_names_cannot_push_the_essential_text_out_at_80x24 (eight long stage names plus error). |
| FND-002 | fixed | footer_controls sizes from real width: the c key and mode join the general controls only when they fit, otherwise take their own line, and in the Scope view the error line sits above the controls instead of replacing them. 80x24 oracles assert "c completed filter unavailable", "c hiding completed" and "c showing completed" in the error, aged and ended states. |
| FND-003 | fixed | The scale scenario builds a fresh app per size and starts the timer before the first key and draw, asserts the cold draw is exactly one summary and one projection build, and reports cold, steady and total. Parent run: 435 draws per size like the before's 435; 80x24 cold 6.9 / steady 74.9 / total 81.8 ms, 200x60 cold 6.2 / steady 123.9 / total 130.1 ms. Release test-render timings only, not live CPU or an SLO. |
| FND-004 | fixed | monitor_main.rs now has event_loop, KeySource and Workers, used by run (TerminalKeys) and by production_event_loop_never_waits_for_a_pending_tracker_read, which spawns the real TrackerWorker with a read held pending, runs about 200 scripted keys plus a manual r through the production loop, releases the read only at the end, and fails if a 60 s watchdog had to release it. A loop that waited on collection would trip the watchdog assertion. Counter deltas (2 summaries, 2 plus toggles builds) are asserted. |
| FND-005 | fixed | pressing_c_and_shrinking_refreshes_keep_the_selected_placement_through_render drives c, j and apply_tracker_refresh through handle_key and render and asserts the selected row text and table index, including clamp on removing the selected placement; forget_selection is called from select_view and the e reset, and a_view_switch_reset_is_not_undone_by_the_remembered_placement asserts row 0 after a rebuild while away. Both reset paths were checked (monitor.rs:240 and 289). |
| FND-006 | fixed | ScopeProjection.unknown_projects counts distinct Unknown-category projects while the category is supplied, and filter_text states "N project(s) with unknown status retained" (monitor_scope.rs:109,133,278); tests assert the mixed statement at 80x24, a shared Unknown project counted once, and 0 while unsupplied. A scope whose displayed projects are all Unknown still reads as filter unavailable: this is truthful, since no displayed project carries a category, and is asserted deliberately. |

Round 2 against frozen head 7ae6348891a141b2be699141a14bbbc27297b85f (only monitor_ui.rs changed from 2bdd2ed). Earlier rows above are unchanged. The reviewer ran nothing; parent-reported: formatter 0, focused tests 67 passed 1 ignored 0 failed, clippy all targets 0, optimized build 0, scale run 435 draws per size (80x24 cold 6.5 / steady 70.5 / total 76.9 ms; 200x60 cold 6.0 / steady 115.6 / total 121.5 ms; release TestBackend render and keys, not live CPU or an SLO).

| FND | outcome | reason |
|---|---|---|
| FND-007 | fixed | The dedicated summary row is gone. monitor_ui.rs:106-122 places the filter statement, with its [last snapshot] label, on the existing Tasks line between the done count and the stage list, so the summary is again the scope line and the Tasks line. The y adjustment in the shared Scope fixture is removed; its Tasks row now lists "Tasks: ", the count, " done · ", FILTER_UNAVAILABLE, " · ", "1", " Review" with their colors, so no count, color or tab assertion was dropped, and the other views keep the original pieces. The statements still precede the variable-length stage list and scope_height still uses wrapped_rows, so the 80x24 essential-text oracles still hold by construction. No FR-060 spec edit is needed because no row is added. |

## Review Context

Reviewer model claude-sonnet-5-5; run id unavailable from this harness. Linear comment markers id=SR-110 method=code-review lang=rust on SWM-176. Rust lane run against repository conventions in CLAUDE.md (workspace lints missing_docs, unwrap_used, expect_used, allow_attributes_without_reason). No AssuranceProfile applies. Diff reviewed as 8dd31a8..c41932e (11 files, 1,669 insertions); the local main ref is not the stated base. Per root instruction no Cargo build, test, clippy or fmt was run by the reviewer; the coder and root results (58 passed, 1 ignored, clippy and fmt clean) are recorded by root and are not re-asserted here. The ix-board SDK source at d6e28f4 is not in the local cargo checkouts; the SDK shape was read from the saved accepted-model copy at .dev-dept-recovery/swm-176/implementation/accepted-sdk-model.rs (Project.status_category, ProjectStatusCategory::completion, ProjectCompletion, Initiative id and name, no initiative-link completeness marker). Checks that produced no finding: no summarize call remains in the draw path (monitor_ui.rs imports it only in tests); cache keys use Source.revision, not tick, selection or draw; failure keeps value and revision so rows and hidden count are retained; the only Scope-key change is c with an empty modifier set and the replacement-fence and quit paths are unchanged; the one lint allow carries a reason and satisfies allow_attributes_without_reason; ordering falls back to name then id as FR-060 requires; hiding counts distinct projects not placements; the SDK category is the only classifier, with no status-string or done/total decision. Staged under the evidence directory for root custody at reviews/swm-176/code/.

Disposition rounds 1 and 2 were read-only reviews of the fixed source diffs and current files (Read, Glob, Grep, Write only). Behavior was judged from code and test text; every pass, exit and timing figure is parent-reported, not reviewer-run. The draw error is wrapped with io::Error::other in event_loop; no caller inspects the error kind, so no finding. Round 2 re-read the current monitor_ui.rs against the fix2 diff and found no regression and no new defect.

## Examined Scope

```yaml
scope:
  - {id: "crates/relay-tui/src/monitor_scope.rs", path: "crates/relay-tui/src/monitor_scope.rs", role: examined, excerpt: "pub fn project(view: &DashboardView, scope: Option<&WorkScope>, show_completed: bool) -> ScopeProjection"}
  - {id: "crates/relay-tui/src/monitor_tracker.rs", path: "crates/relay-tui/src/monitor_tracker.rs", role: examined, excerpt: "project_progress.completion = project.completion();"}
  - {id: "crates/relay-tui/src/monitor_ui.rs", path: "crates/relay-tui/src/monitor_ui.rs", role: examined, excerpt: "app.scope.refresh(&app.tracker, &app.report, team.as_deref(), app.show_completed);"}
  - {id: "crates/relay-tui/src/monitor_views.rs", path: "crates/relay-tui/src/monitor_views.rs", role: examined, excerpt: "app.scope.restore_selection(&mut app.table, projection);"}
  - {id: "crates/relay-tui/src/monitor.rs", path: "crates/relay-tui/src/monitor.rs", role: examined, excerpt: "KeyCode::Char('c') if app.view == View::Scope && key.modifiers.is_empty() => {"}
  - {id: "crates/relay-tui/src/monitor_ops.rs", path: "crates/relay-tui/src/monitor_ops.rs", role: examined, excerpt: "pub(crate) fn spawn_with_read("}
  - {id: "crates/relay-tui/src/monitor_main.rs", path: "crates/relay-tui/src/monitor_main.rs", role: examined, excerpt: "mod monitor_scope;"}
  - {id: "crates/relay-tui/src/monitor_scope_tests.rs", path: "crates/relay-tui/src/monitor_scope_tests.rs", role: examined, excerpt: "fn selection_follows_the_same_placement_or_clamps_after_rows_change()"}
  - {id: "crates/relay-tui/src/monitor_scale_tests.rs", path: "crates/relay-tui/src/monitor_scale_tests.rs", role: examined, excerpt: "fn large_scope_event_sequence_with_pending_source_read()"}
  - {id: "Cargo.toml", path: "Cargo.toml", role: examined, excerpt: "ix-board = { git = \"https://github.com/agent-ix/ix-board\", rev = \"d6e28f40ddb71b05e0cb3e9607e799c6399aabc6\" }"}
  - {id: "Cargo.lock", path: "Cargo.lock", role: examined, excerpt: "ix-board 0.6.0 git d6e28f40ddb71b05e0cb3e9607e799c6399aabc6"}
```
