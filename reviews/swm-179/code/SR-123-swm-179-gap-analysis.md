---
id: SR-123
title: "Gap analysis - FR-065 View selector and F1 help, spec criteria to tests to code (SWM-179)"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-065-dashboard-view-selector-and-help.md; crates/relay-tui/src/monitor.rs; crates/relay-tui/src/monitor_nav.rs; crates/relay-tui/src/monitor_ui.rs; crates/relay-tui/src/monitor_department.rs; crates/relay-tui/src/monitor_main.rs"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-065", type: references }
---

## Summary

Ticket: SWM-179. PR ix-relay#44 at frozen head 0f602f65ca06a90390b01aecff326e4d2b0887db. Planless, repository-driven gap analysis scoped to FR-065 only: its 13 acceptance criteria against the tagged tests and the source under crates/relay-tui/src. Plan completion was not assessed; this is a repository claim about FR-065 and nothing more. FR-064 (`f`, named mode), the named activation tickets 129 to 131 and the unconfigured historical metrics tickets 132, 133 and 167 are later work and no finding asks for them.

AC-1 to AC-12 each carry at least one `#[trace]` on a running test and AC-13 is the expected Inspection criterion without a symbol. No source or test stub backs a criterion. The audit found 7 gaps: 4 medium and 3 low. They are untested branches of stated criteria (AC-3 applicability cells, AC-5 evidence conditions, AC-6 identity cases), one behavior no criterion owns (a fifth Health and Retro message), and three smaller assertion gaps. None is a missing feature.

## Verdict

**CONDITIONAL** - no untagged criterion and no `high` finding; 4 medium and 3 low findings. A planless CONDITIONAL asserts repository assurance for FR-065 only (traceability, reverse gap, stubs) and nothing about plan completion. The separate test-oracle findings in SR-122 (FND-004, FND-005, FND-006, FND-007) are not repeated here.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-065-AC-3: the applicability test asserts 10 of the 24 key-by-view cells it names, and the row-selection and paging row of the Help table is asserted in no view | FR-065-AC-3, crates/relay-tui/src/monitor_nav.rs:511 |
| FND-002 | medium | FR-065-AC-5: the 80x24 evidence fixture has no stale source and no active filter, and the overflow, truncation marker and below-80x24 checks run on lead views only | FR-065-AC-5, crates/relay-tui/src/monitor_ui.rs:1773 |
| FND-003 | medium | FR-065-AC-6: the identity-row test covers 3 of the 5 listed cases and 120x40 only; the saved-report startup and explicit scope with no lead and no diagnostic are never rendered | FR-065-AC-6, crates/relay-tui/src/monitor_ui.rs:1821 |
| FND-004 | medium | Reverse gap: a fifth Health and Retro message (configuration supplied, first read succeeded, fields absent) has no owning requirement and no test | crates/relay-tui/src/monitor_department.rs:627 |
| FND-005 | low | FR-065-AC-1: scope preserved is never asserted, and Health and Retro are not selected through the dropdown with no data | FR-065-AC-1, crates/relay-tui/src/monitor_nav.rs:368 |
| FND-006 | low | FR-065-AC-8: the clause that every other Status row is unchanged by the purpose line has no assertion | FR-065-AC-8, crates/relay-tui/src/monitor_department.rs:1298 |
| FND-007 | low | FR-065-AC-12: no test pins the colours of the new rate-limit and stale evidence rows, which changed from the single error colour and are not in the Permitted list | FR-065-AC-12, crates/relay-tui/src/monitor_ui.rs:493 |

## Coverage

- Reconciliation: quire matrix run by the root (fix1-matrix.json, exit 0, reported by the root as FR-065 AC-1 to AC-12 tagged and AC-13 method-without-symbol) - no run evidence read. I did not run quire or quoin. I read the saved matrix with pattern searches because it is a single JSON line; the binders and `status` fields I extracted agree with the root's report. I cross-checked tag presence by reading every `#[trace("FR-065-AC-n")]` attribute in the diff, which agrees. This is not an engine figure of my own, and a tag proves traceability, not that the test checks the criterion. The saved stderr carries `trace-id-outside-marker` warnings (21 on FR-065 symbols, see SR-122 FND-010) and legacy warnings on other files; whole-repository legacy warning coverage is unknown and no clean-repository assurance is claimed.
- Plan completion: not assessed
- Criteria (FR-065, root's matrix, not recomputed by me): tagged 12 · untagged 0 · tagged-by-ignored-test 0 · method-without-symbol 1 (AC-13, Inspection, performed below). Other requirements and the legacy rows of the full-repository matrix are not assessed and are not clean assurance.
- Evidence: no run evidence (quoin matrix not run). The root reports clippy (`-p relay-tui --all-targets -D warnings`) exit 0, tests 45 inbox and 86 dashboard passed with 1 existing ignored scenario, fmt exit 0; I ran none of these and claim no fresh green.
- Untraced behaviors / stubs: inventoried 16 FR-065 behaviors (the `v` key, F1 and `?`, dropdown keys, help keys, footer row, selector row, evidence rows, identity row, lead population labels, department selection label, fixed relation text, Status purpose line, the four spec empty states, the dropdown current marker, the help applicable marker, popup isolation); untraced 1 (FND-004); source stubs 0; test stubs 0. No `TODO`, `FIXME`, `todo!`, `unimplemented!` or new `#[ignore]` in the changed files.
- Semantic review: not run as the separate opt-in pass. The per-criterion judgement below is the reviewer's reading of the tests, not that step.
- AC-13 inspection: terminal setup (`enable_raw_mode`, `EnterAlternateScreen`, guard, `restore_terminal`) is the same text as before in monitor_main.rs:164-171 and 419-422; the diff to monitor_main.rs touches only the module list, the department input lines and tests; a search for `MouseCapture`, `EnableMouse`, `DisableMouse` and `Event::Mouse` under crates/relay-tui/src finds nothing. AC-13 holds.

### Per-criterion assessment (tag present on AC-1 to AC-12)

| AC | Evidence assessed | Strength |
|---|---|---|
| 1 | dropdown from each of six views at both sizes, key list, highlight on current, Down/Up/j/k and Enter equal to the `2` key, current view a no-op | strong; scope preserved and no-data selection not asserted (FND-005) |
| 2 | Escape and `v` cancel keep view, selection, details, scroll, filter, no quit; clamp at both ends; Escape with no popup quits; short work area and resize | strong |
| 3 | open and close by F1, `?`, Escape; applicable keys; scroll and resize | partial (FND-001; reachability oracle in SR-122 FND-007) |
| 4 | 24 keys by both popups, 8 state fields, `q` does not quit | strong |
| 5 | footer and selector in six views at both sizes, no tab strip; four-label fixture, truncation marker, `+2 more`, sub-80 on lead views | partial (FND-002) |
| 6 | verified, replaced and ambiguous-lead identity rows identical across six views | partial (FND-003) |
| 7 | lead-view labels (focused, whole team, ticket-only, unbound, Agents) and department labels (explicit, derived, saved) with forbidden words | strong on coverage; fixed text oracle in SR-122 FND-005; named mode needs FR-064 and is not asserted |
| 8 | purpose line at both sizes with evidence rows, absent from Health and Retro | partial (FND-006) |
| 9 | five states per view, one message each, no failure row, no authentication word, no zero or green | strong; the derived-label and fifth-state branches are untested (SR-122 FND-002, FND-004 here) |
| 10 | saved report and live fixtures run the real `event_loop` | saved effective through the error oracle; live oracle cannot fail (SR-122 FND-004, FND-006) |
| 11 | help at 80x24 stays inside the work area, failed refresh, rate limit and replaced session visible, shrinking refresh clamps selection | strong |
| 12 | key hint, selector, footer, dropdown and help colours in six views; department evidence colour policy with colour on and off | partial (FND-007) |
| 13 | Inspection | held (see above) |

## Finding details

### FND-001 (medium) FR-065-AC-3: key-by-view applicability is largely unasserted

AC-3 requires that for every view the keys marked applicable equal exactly that view's column of the Help table, with `s`, `e`, Left/Right and `c` marked only where the table says. `help_marks_exactly_the_keys_that_apply_in_each_view` (monitor_nav.rs:511-549) asserts ten cells of the 24 that `s`, `e`, `c` and Left/Right make across six views: `s` in Tickets, Agents, Scope and Status; `e` in Health and Tickets; `c` in Scope and Retro; Left/Right in Status and Scope. Not asserted: `s` in Health and Retro; `e` in Agents, Scope, Status and Retro; `c` in Tickets, Agents, Status and Health; Left/Right in Tickets, Agents, Health and Retro. The always-list covers six rows but not `Up, Down, j, k, PageUp, PageDown`, so moving that row to a subset of views passes. The closing assertion compares the count of check marks with `applicable_keys(view).len()`, two readings of the same `HELP` table, so it cannot catch a wrong table. Opening help with `?` is exercised from the default view only, though it shares one match arm with F1. Risk: a wrong `views` slice for `s` or `e` in `HELP` keeps the suite green and misleads the user, which is the point of marking applicability. Test request: write the Help table's views column as a literal 11-by-6 expectation in the test and compare it with `applicable_keys(view)` for every view.

### FND-002 (medium) FR-065-AC-5: evidence conditions and department-view overflow are not exercised

AC-5 names, at 80x24, a source error, a stale source, a rate-limit countdown, a replaced session and an active filter, all labels intact in at most four evidence rows with the footer hints present; a long multi-source fixture with a truncation marker, a not-shown count and, in Status, Health and Retro, the full text through `e` then `d`; and a below-80x24 fixture without panic. `evidence_rows_keep_labels_intact_and_bound_themselves_at_80x24` (monitor_ui.rs:1773-1816) uses replaced session, Herdr failure, Reports failure and a rate limit (four labels, all four rows), then adds Scope failed for `+2 more sources not shown`. No test asserts the new `Tracker stale` evidence row (monitor_ui.rs:397; a grep for the label finds only the definition), `Department evidence · last snapshot` is asserted only alone in the AC-9 fixture and never beside other evidence, and no fixture frame holds an active Scope filter state; the `…` marker, `+N more` and the sub-80 sizes (40x10 and 60x16) run on the lead views only, so `monitor_department::render` at overflow and below 80x24 is never driven. The `e` then `d` path in department views is covered only by the existing FR-059 test, which has three sources and no overflow. Risk: department layout changes that hide a label when four or more sources fail pass the suite. Test request: add the stale and filter conditions to the lead fixture, and one department fixture with five failing or stale sources at 80x24 asserting labels, the marker, the count, `e` then `d`, and a 60x16 draw.

### FND-003 (medium) FR-065-AC-6: two of the five identity cases are never rendered

The AC lists a verified lead; a missing or ambiguous lead with explicit scope; a replaced session; explicit scope with no `--lead`; and a saved-report startup. `identity_row_is_the_same_in_all_six_views` (monitor_ui.rs:1821-1848) builds three apps: verified, replaced, and unbound with an ambiguous-lead runtime error. It draws only at 120x40 and compares row 0 across the six views. Not covered: explicit scope with no lead and no diagnostic (the row reads `identity unknown · no lead bound` with nothing after it) and the saved-report startup (`department_input = Saved`, no binding, `lead_name` default). Both go through the same `identity_line`, so the risk is small, but the saved-report startup is the one startup where no binding or runtime source exists at all. Test request: add those two apps to the loop and also compare at 80x24.

### FND-004 (medium) Reverse gap: a fifth Health and Retro message has no owner

FR-065 Empty Health and Retro defines four states: Not configured, Waiting, Failed and Stale. `empty_message` (monitor_department.rs:627-633) adds a fifth user-visible message for the live case where a configuration was supplied, the first read succeeded and the report has no Health classes or no Retro measurements and loops: `The retained report supplies no {name} fields; nothing is shown.` The spec does not define that state, does not say whether it carries a stale label when `department.error` is set with a retained report that has no fields, and no test reaches the branch (the AC-9 stale fixture has fields). It is truthful and does not show a value, so it is not wrong, but it is a rule a second implementer would write differently. This review authors no requirement. Fix: either state it in FR-065 (a spec decision) and add a fixture, or fold it into one of the four states and test that.

### FND-005 (low) FR-065-AC-1: two clauses have no assertion

AC-1 says the target view renders with selection and scroll reset and scope preserved, and that Health and Retro are selectable with no data. The test (monitor_nav.rs:368-...) compares the view, table selection, detail scroll and row count with the `2` key, which proves reset; it asserts nothing on `team`, `projects`, `scope`, `show_completed` or `department_input` before and after, so "scope preserved" is untested. It also uses `monitor_ui::tests::app()`, which carries a tracker snapshot and a report, so selecting Health or Retro through the dropdown with no data is only reached by the compatibility keys `2` and `3` in the monitor_main fixtures. Both clauses hold by construction today (`select_view` is shared), so the risk is low. Test request: assert the preserved fields in the existing comparison, and add one dropdown selection of Retro from an `App::default()`.

### FND-006 (low) FR-065-AC-8: unchanged rows are not asserted

AC-8's second sentence says every Status row other than the purpose line is unchanged by it for a shared fixture. `status_states_its_purpose_without_hiding_evidence_or_rows` (monitor_department.rs:1298-1318) asserts the line, two evidence labels and `Bound activity`. It never compares the rest of the Status rows with and without the line. `status_rows` is not touched by the diff, so the clause holds by inspection. Test request: render the Status body rows from `status_rows` and the drawn body, and assert the rows equal the pre-change expectation for the fixture.

### FND-007 (low) FR-065-AC-12: new evidence colours are unpinned and outside the list

AC-12 requires existing fixtures to differ only by the Permitted presentation differences. Before this change the lead views drew every error as one line in the error colour, including the rate-limit countdown. `evidence_lines` now colours severity 1 and 2 rows with `WARNING` (monitor_ui.rs:493-497), so the rate-limit and `Tracker stale` rows are amber and plain facts are muted. The Permitted list names the placement and bounding of the evidence rows, not their colour, and the only colour assertion (monitor_ui.rs:1103-1105) covers the failure row. Whether severity colours are an accepted part of an evidence row is a spec question, so confidence is low. Fix: either assert the colours of the rate-limit and stale rows and note them in FR-065, or keep the error colour for rate-limit rows as before.

## Verdict after disposition pass 1

Not run. Dispositions are reserved for this same session after the fix round.

## Dispositions (round 1, all 7)

Disposition pass 1 at frozen fix 7584112, judged from the current tests and source, not from the author notes, against the current FR-065 (all 13 AC ids unchanged by the developer-directed copy delta). I ran no command and claim no test green; each outcome is conditional on the root's pending test run.

| FND | Outcome | Reason |
|---|---|---|
| FND-001 | fixed | `APPLIES` (monitor_nav.rs:508-520) writes the Help table's views as a literal 11-by-6 grid. `help_marks_exactly_the_keys_that_apply_in_each_view` (524) compares `applicable_keys(view)` with it for all six views, which makes 66 cells including the row-selection row, and checks each drawn row's mark. `help_rows_state_views_only_where_a_key_does_not_apply_everywhere` (548) asserts the eleven row texts. |
| FND-002 | fixed | `rate_limit_replaced_session_stale_source_and_active_filter_fit_at_80x24` (monitor_scope_tests.rs:1045) holds a rate limit, a replaced session, a stale source and a real active filter at 80x24 with `Tracker stale` asserted; `department_overflow_keeps_labels_marker_count_and_full_text_through_e_then_d` (monitor_department.rs:1274) drives department overflow, `e` then `d` and the small sizes. The AC text lists all five conditions at once, and four evidence rows cannot hold five labels plus the footer, so two fixtures is the reasonable reading; I accept the split. |
| FND-003 | fixed | `identity_row_is_the_same_in_all_six_views` (monitor_ui.rs:1859) now covers verified, replaced, ambiguous, explicit scope with no lead and saved-report startup at 80x24 and 120x40 with exact prefixes. |
| FND-004 | fixed | FR-065 now owns the fifth state as **Fields absent** (spec lines 248-252) with AC-9 and AC-10 wording; `empty_message` has distinct saved and configured texts and the AC-9 test covers all six states plus the lead-derived case. The new spec text is reviewed in SR-124. |
| FND-005 | fixed | The AC-1 test (monitor_nav.rs:339) asserts `team`, `projects`, `show_completed` and `department_input` are preserved and selects Retro and Health from `App::default()`, which shows "Not configured". |
| FND-006 | fixed | `the_purpose_line_leaves_every_other_status_row_as_it_was` (monitor_department.rs:1408) asserts the eight other rows as literals. |
| FND-007 | fixed | FR-065 Permitted item 4 now allows warning-palette stale and rate-limit rows and AC-12 says "pinned by test"; `rate_limit_and_stale_rows_use_the_existing_warning_palette` (monitor_ui.rs:2093) pins WARNING for them and the error colour for Herdr failed. |

## New findings (disposition pass 1)

None. The Fields absent state, its spec text and tests were checked against each other in the dispositions of FND-004 and in SR-124; the one defect found in the spec copy is recorded there.

## Current verdict (after disposition pass 1)

**PASS** for the 7 gap findings: all fixed on the source as read, no new gap. The coverage reading remains conditional on the root's pending results: the TUI tests have not executed (the earlier wrapper was cancelled with exit 143 before compilation, which is not a failure), and the root's code-fix1 matrix reports FR-065 AC-1 to AC-12 tagged, AC-13 Inspection and zero FR-065 warnings, as supplied and not recomputed by me. I claim no run of my own.
