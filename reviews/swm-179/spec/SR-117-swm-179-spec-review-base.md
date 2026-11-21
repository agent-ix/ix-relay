---
id: SR-117
title: "Spec review (base checklist) - FR-065 dashboard View selector and F1 help (SWM-179)"
type: SpecReview
analysis: base
scope: "agent-ix/ix-relay; spec/functional/FR-065-dashboard-view-selector-and-help.md; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-065", type: references }
---

## Summary

Ticket: SWM-179. Spec-only change at frozen head df58008 (base 86d94d7). Base review of the new FR-065 and its edits to FR-048 and FR-059 against the spec-review checklist (ID format, requirement quality, criterion coverage, cross-references, and the hash, digest and pin antipattern) and against the six ticket acceptance checks. The review set is `subset`: base plus the ears-conformance, integrity, dependency and failure-domain lenses, each written as its own artifact (SR-118 to SR-121). 6 findings: 2 medium, 4 low.

## Verdict

**CONDITIONAL** - no `high` finding. The capability is right-sized: FR-065 changes navigation and labels only and adds no data source, collector, metric, credential behavior or scope engine; the key choices `v` and `?` are accepted implementation choices and are not findings. The two medium defects are in criteria a test must satisfy: AC-4 cannot be satisfied together with AC-1 and the dropdown's own keys, and the evidence rule forbids shortening text that has no length bound.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-065-AC-4 sends Enter, j, k, arrows and paging with a popup open and requires the view unchanged, which contradicts the dropdown's own keys, and it does not say what `?`, `v` and F1 do inside the other popup | spec/functional/FR-065-dashboard-view-selector-and-help.md:153 |
| FND-002 | medium | Source, error and filter evidence SHALL NOT be shortened, but error text is unbounded, there is no row budget or minimum size, and lead views have no way to read the full text | spec/functional/FR-065-dashboard-view-selector-and-help.md:112 |
| FND-003 | low | The popup resize clause has no acceptance criterion | spec/functional/FR-065-dashboard-view-selector-and-help.md:99 |
| FND-004 | low | The conditional mouse clause is never true today, AC-4 tests only click-away, and nothing verifies the no-capture statement | spec/functional/FR-065-dashboard-view-selector-and-help.md:64 |
| FND-005 | low | FR-065-AC-6 omits the saved-report startup and the explicit-scope startup without a lead in the identity-row matrix | spec/functional/FR-065-dashboard-view-selector-and-help.md:155 |
| FND-006 | low | FR-065-AC-9 asserts no adapter call caused by the empty state, with no oracle for causation while background workers poll | spec/functional/FR-065-dashboard-view-selector-and-help.md:158 |

## Review Context

Reviewer model claude-sonnet-5-5; the reviewer's own harness session ID was not visible, so none is claimed; run id unavailable. Root-supplied marker metadata: repository visibility public, quoin 0.36.7, module spec-artifacts-process@v0.29.2. The PR number was not supplied; the marker carries the placeholder `unavailable` for the parent to replace. Marker id=SR-117 method=spec-review/base lang=spec.

What was and was not run. Read, Glob, Grep, Edit and Write only; no quire, quoin, git or tracker command. I made no validation claim. The base review is manual: I checked ID formats and uniqueness by reading (FR-065, FR-065-AC-1 to AC-10 are sequential and unique; Verification cells name a method only, no `TC-` id), link targets (FR-048, FR-059, FR-060 and FR-064 exist at the relative paths used; the frontmatter `ix://agent-ix/ix-relay/FR-0NN` targets exist), and the hash, digest and pin antipattern (a case-insensitive search of FR-065 for hash, digest, checksum, pin, manifest, ledger, provenance, attestation, receipt, relocation, inventory and revision matched only the word SHALL; no tool version is pinned in prose). `quire matrix` was not run, so trace-tag coverage of FR-065-AC-1 to AC-10 is not assessed; the author report already flags it for the parent. FR-065 carries no user-story or stakeholder-requirement link, the same as FR-060 and FR-064, so I did not raise it.

Ticket acceptance (request.md) checked against FR-065: check 1 (selector in all six views, keyboard and conditional mouse) maps to View selector bullets and AC-1, AC-2, AC-4; check 2 (F1 help, modal isolation) to Help popup, Modal isolation, AC-3, AC-4; check 3 (compact footer, evidence outside popup) to the footer bullets and AC-5 (see FND-002); check 4 (stable identity, department versus lead scope, Status purpose) to AC-6, AC-7, AC-8 (labeling defects are in SR-119); check 5 (empty Health and Retro) to AC-9 (see FND-006 and SR-121); check 6 (fixtures at 80x24 and wider) to AC-1 to AC-10. Scope boundary: nothing in FR-065 adds telemetry, a collector, a scope or data engine, an auth path or named-focus implementation; the Out of scope bullet matches the ticket's list.

The Jev client and criterion-strength analysis were not used and no availability is claimed. No AssuranceProfile exists under spec/; none applies. Analyses not run in this review set: evidence, risk-complexity and scope-boundary (not selected by the brief), and app-review (the skill reviews a `component_type: application` spec.md against the spec-editor-ui; this repo is `rust-cli` and the diff does not touch spec/spec.md).

## Examined Scope

```yaml
scope:
  - {id: "FR-065", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The existing `team-leader-dashboard` SHALL offer one View selector that lists all six existing views (Tickets, Agents, Scope, Status, Health, Retro) and one F1 help popup, identical in every view."}
  - {id: "FR-065-Precedence", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "For presentation of the view switch only, this requirement takes precedence over the following statements, and only to the extent listed:"}
  - {id: "FR-065-View-selector", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Every view SHALL show, in its header, the same selector labelled `View` with the current view name and the key that opens it."}
  - {id: "FR-065-Mouse", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "This requirement adds no mouse capture mode. If the existing terminal lifecycle delivers mouse events, a click on the selector SHALL open the dropdown"}
  - {id: "FR-065-Help", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Help SHALL list, with the views where each applies:"}
  - {id: "FR-065-Modal-isolation", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Source polling, refresh results and the terminal lifecycle continue while a popup is open."}
  - {id: "FR-065-Evidence", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Source, stale, error, last-snapshot, rate-limit countdown, binding and filter evidence SHALL be shown outside any popup, on their own row or rows"}
  - {id: "FR-065-Scope-labels", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Tickets, Agents and Scope SHALL name their population as the lead's reported scope, using the FR-060 labels unchanged."}
  - {id: "FR-065-Empty-Health-Retro", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "When no retro configuration or saved report supplies Health or Retro fields, the view SHALL show one concise state, not a table of placeholders"}
  - {id: "FR-065-AC-1", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "In each of Tickets, Agents, Scope, Status, Health and Retro, at 80x24 and 120x40, the header shows the same `View` selector; `v` opens a dropdown listing all six views with their keys"}
  - {id: "FR-065-AC-2", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Escape or `v` with the dropdown open closes it and leaves view, selection, scroll and details unchanged, and does not quit; Escape with no popup open still quits."}
  - {id: "FR-065-AC-3", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "F1 and `?` open help in all six views; F1 and Escape close it without quitting or state change."}
  - {id: "FR-065-AC-4", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "With a popup open, a fixture sending every existing key (`q`, `r`, `d`, `Enter`, `s`, `e`, `t`, `a`, `p`, `1`, `2`, `3`, `c`, `f`, `j`, `k`, arrows, PageUp, PageDown) leaves table selection, scroll, details, view, refresh flag, status toggle and filter mode unchanged"}
  - {id: "FR-065-AC-5", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The footer in all six views contains `v View` and `F1 Help` and no other shortcut list except the Scope filter key and mode required by FR-060, and no persistent tab strip appears."}
  - {id: "FR-065-AC-6", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The identity row (lead identity, binding state, activity markers, or the explicit no-lead and binding-diagnostic text) is identical across all six views"}
  - {id: "FR-065-AC-7", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "A fixture where the department team/projects and the lead's reported scope differ renders Status, Health and Retro labeled as department scope"}
  - {id: "FR-065-AC-8", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Status shows the specified purpose line at 80x24 and 120x40 without hiding source or error evidence."}
  - {id: "FR-065-AC-9", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Adapters for credentials, network, Herdr and Redis record no call caused by the empty state or by opening either popup."}
  - {id: "FR-065-AC-10", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Existing FR-048-AC-1 binding, FR-059 source-failure, replacement-fence, quit, terminal-restoration and FR-060 scroll/selection/toggle fixtures pass with only the header, footer and label differences stated above."}
  - {id: "FR-065-Dependencies", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Out of scope: metric formulas, new telemetry, credential or setup changes, named-focus activation or adoption, changes to tracker initiatives, source timing, a new renderer or framework, dependency repins and any other service."}
  - {id: "FR-048-AC-4", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: "The current view is named by the View selector of [FR-065](./FR-065-dashboard-view-selector-and-help.md), with no persistent tab strip."}
  - {id: "FR-059-Presentation", path: "spec/functional/FR-059-department-monitor-adapter.md", role: examined, excerpt: "The keys `1`, `2` and `3` SHALL select Status, Health and Retro respectively, as compatibility shortcuts for the View selector of [FR-065](./FR-065-dashboard-view-selector-and-help.md)"}
  - {id: "FR-059-AC-4", path: "spec/functional/FR-059-department-monitor-adapter.md", role: examined, excerpt: "Existing compact Tickets/Agents/Scope fixtures and their keys retain their outputs, except for the header selector, footer and labels changed by FR-065."}
  - {id: "FR-060", path: "spec/functional/FR-060-focused-scope-presentation.md", role: context_only, excerpt: "The existing `team-leader-dashboard` Scope view SHALL present the lead's explicitly reported initiative and project selection as a parent-child hierarchy"}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: context_only, excerpt: "The existing `team-leader-dashboard` Scope view SHALL present a bound session's named active focuses"}
```

## Finding details

### FND-001 (medium) AC-4 contradicts the dropdown and leaves the other popup's own keys undefined

AC-4 is the isolation criterion: with a popup open, "every existing key (`q`, `r`, `d`, `Enter`, `s`, `e`, `t`, `a`, `p`, `1`, `2`, `3`, `c`, `f`, `j`, `k`, arrows, PageUp, PageDown)" must leave table selection, scroll, details, view, refresh flag, status toggle and filter mode unchanged. But the dropdown owns exactly Up, Down, `j`, `k` and Enter (lines 57-60 and AC-1): `j` then Enter selects another view and changes the view; Enter on the current view closes the popup. The help popup owns Up, Down, PageUp and PageDown (lines 87-89), which scroll help, not the table. Three consequences for a test author. (1) Read literally, the dropdown rows of the sweep fail on a conforming implementation. (2) Order matters: Enter, Escape, `v` and F1 close a popup, after which every later key in the sweep reaches the table, so a single fixture "sending every key" stops testing isolation midway; the AC does not say a fresh popup per key. (3) Three cases are undefined: `?` while help is open (the text says only F1 and Escape close it; "a key that would open the other popup is ignored" does not cover the same popup), `v` while help is open, and PageUp or PageDown inside the dropdown. A test author must invent each.

Fix: replace the sweep with a per-popup table of owned keys and their effects (dropdown: Up, Down, `j`, `k`, Enter, Escape, `v`; help: Up, Down, PageUp, PageDown, Escape, F1, and say what `?` does), and require the sweep over every other key with a fresh popup per key and the stated unchanged set.

### FND-002 (medium) The evidence rule forbids shortening text that has no length bound

Lines 112-116: source, stale, error, last-snapshot, countdown, binding and filter evidence "SHALL NOT be shortened, hidden or replaced", while the terminal may be narrow and "v View and F1 Help are never dropped". Today the opposite holds: the footer error line is cut to the terminal width (monitor_ui.rs:359-364, `display_text(errors, width)` over a join of up to four source errors), the department failure rows are one unwrapped line per failing source (monitor_department.rs:603-632) and the department source panel is a fixed two rows (monitor_department.rs:458-466). Tracker, Herdr and Redis error text is unbounded, so "not shortened" is unsatisfiable on any row budget, and the spec states none: no rows per evidence kind, no order when 80x24 runs out (the work table keeps `Min(3)`), and no minimum size (the two fixed hints alone are 16 columns wide, so "never dropped" is false below that). In the lead views there is also no way to read a long error in full, because `e` has no visible effect there (SR-116 FND-001). AC-5 uses a short fixture and so passes while the requirement stays unsatisfiable.

Fix: state a bound the implementation can meet: each evidence kind keeps a classified label that is never truncated (source, state, reason class, countdown, filter), long detail wraps to a stated row budget and ends in a visible truncation marker, and the full text is reachable from the existing details or diagnostic path; name 80x24 as the minimum supported size and say that below it text is truncated without a panic; add an AC-5 fixture with a long multi-source error.

### FND-003 (low) The resize clause has no criterion

Lines 99-101 require that "Terminal resize re-lays the popup out and keeps it open". No AC exercises it: AC-1, AC-2 and AC-3 fix the sizes at 80x24 and 120x40, and AC-10 covers only a refresh completing while a popup is open. A resize from 120x40 to 80x24 with the dropdown open is the case that makes the highlight-visible and scroll clauses matter. Fix: add one resize step to AC-2 and AC-3.

### FND-004 (low) The mouse clause is conditional, unverifiable today and its negative has no check

Lines 64-67 add no capture mode and specify click behavior "if the existing terminal lifecycle delivers mouse events". The terminal setup enables raw mode and the alternate screen only (monitor_main.rs:164-167), so the condition is false in the code under review, and AC-4's mouse sentence ("clicks outside the dropdown cancel it") can never run; click-to-open and click-to-select have no criterion at all. The part that is verifiable today, that the change enables no mouse capture, is only stated, and AC-4's Verification is `Test`. Fix: add an Inspection to AC-4 that the terminal setup is unchanged and no capture is enabled, and say that the click clauses are verified only when a later change delivers mouse events.

### FND-005 (low) AC-6 leaves two startup modes out of the identity-row matrix

AC-6 lists a verified lead, a missing or ambiguous lead with explicit scope, and a replaced session. A saved-report startup has no lead by construction (`--report` conflicts with `--lead` and `--session`, FR-059 and monitor_main.rs:41), and the department header today shows only "Department · {lead_name}" with no binding state or activity marker (monitor_department.rs:477-485). FR-065 requires the same identity row in all six views, including the three department views, so the saved-report case is the one where the Status, Health and Retro identity row has no lead to show and the "says so plainly" clause is the only content. Fix: add the saved-report startup, and the explicit-scope startup with no `--lead`, to the AC-6 list.

### FND-006 (low) AC-9's no-call assertion has no oracle for causation

AC-9 requires that the credential, network, Herdr and Redis adapters "record no call caused by the empty state or by opening either popup". In a live-startup fixture those workers poll on their own timers (FR-048: Linear every 30 minutes, Herdr and Redis reads every minute), so an absolute zero fails and "caused by" has no definition a test can check. The offline case is clean (FR-059-AC-2 already uses fail-if-called adapters for a saved report). Fix: for the live case assert that the adapter call counts are equal before and after the interaction with the worker clocks held, and for the saved-report case reuse the fail-if-called adapters.

## Verdict after disposition pass 1

Not run. Dispositions are reserved for this same session after the fix round.
