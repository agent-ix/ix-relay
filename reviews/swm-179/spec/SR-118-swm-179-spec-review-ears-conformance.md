---
id: SR-118
title: "EARS conformance analysis - FR-065 dashboard View selector and F1 help (SWM-179)"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay; spec/functional/FR-065-dashboard-view-selector-and-help.md; spec/functional/FR-059-department-monitor-adapter.md"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-065", type: references }
---

## Summary

Ticket: SWM-179. Spec-only change at frozen head df58008 (base 86d94d7). EARS requirement-grammar analysis of the new and edited requirement statements: the FR-065 Description and its 21 Behavior bullets, and the one edited FR-059 presentation bullet. Fifteen of the 21 Behavior bullets carry a SHALL; six carry none, and they include the modal-isolation rule and the empty-state side-effect rule, the two statements the ticket cares most about. 4 findings: 4 medium.

## Verdict

**CONDITIONAL** - no `high` finding, because no statement is ambiguous about what gets built; the defects are missing SHALL and non-singular bullets, which degrade traceability to FR-065-AC-1 to AC-10. Most FR-065 bullets use the repo's own idiom (a key as grammatical subject, as in FR-059 and FR-060) and I raised none of those.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | The modal-isolation rule has four sentences, no SHALL and no named system subject, and mixes an invariant, an input rule and an exception | spec/functional/FR-065-dashboard-view-selector-and-help.md:93 |
| FND-002 | medium | The empty-state side-effect rule has no SHALL and packs three negative obligations into one statement | spec/functional/FR-065-dashboard-view-selector-and-help.md:143 |
| FND-003 | medium | The dropdown key bullet carries one SHALL NOT clause and five unmarked behaviors, with no system subject | spec/functional/FR-065-dashboard-view-selector-and-help.md:57 |
| FND-004 | medium | The department scope-label bullet packs four SHALL and SHALL NOT obligations, including an undefined assert, into one statement | spec/functional/FR-065-dashboard-view-selector-and-help.md:122 |

## Review Context

Reviewer model claude-sonnet-5-5; the reviewer's own harness session ID was not visible, so none is claimed; run id unavailable. Root-supplied marker metadata: repository visibility public, quoin 0.36.7, module spec-artifacts-process@v0.29.2. The PR number was not supplied; the marker carries the placeholder `unavailable`. Marker id=SR-118 method=spec-review/ears-conformance lang=spec.

What was and was not run. The skill's step 1 is the engine check `quire validate --scope . "spec/**/*.md" --summary`. I had no shell and did not run it, so I have no `[ears:<check>]` warnings and no GrammarSummary and claim none. Every finding below is a manual reading of the five engine checks (non-singular, vague-response, missing-subject, non-canonical-trigger, unclassifiable) and of the semantic checks (keyword versus intent, concrete response, right pattern). The parent should compare the engine output with these four rows when it runs the validation. I read the quoin 0.29.4 spec-ears-analysis skill, the brief and the frozen FR-065 file.

Examined with no finding: the Description (one SHALL, one subject, the rest are non-normative scope statements); "Every view SHALL show..." and the `v` open rule; the F1 and `?` open rule; "Help SHALL list" and "Help SHALL mark"; the footer and identity bullets (each carries more than one SHALL, but each obligation is on one subject and maps to one AC, so I did not raise them); "Status SHALL show" the purpose line; the mouse bullet (three SHALL, one per click, conditional on an If trigger; the verification problem is in SR-117 FND-004); "the list SHALL scroll". The edited FR-059 presentation bullet adds a subordinate ownership clause ("which also owns help and the department scope labels") that is non-normative and sits inside a SHALL sentence; the bullet keeps the same three SHALL it had before the edit. The FR-048-AC-4 edit is an acceptance-criterion cell, outside this lens. Repo idiom, not raised: statements whose subject is a key ("The key `v` (no modifier) SHALL open...", FR-059 "The keys `1`, `2` and `3` SHALL select...").

The Jev client and criterion-strength analysis were not used. No AssuranceProfile exists under spec/; none applies.

## Examined Scope

```yaml
scope:
  - {id: "FR-065", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The existing `team-leader-dashboard` SHALL offer one View selector that lists all six existing views (Tickets, Agents, Scope, Status, Health, Retro) and one F1 help popup, identical in every view."}
  - {id: "FR-065-View-selector", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Every view SHALL show, in its header, the same selector labelled `View` with the current view name and the key that opens it."}
  - {id: "FR-065-Dropdown-keys", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "While the dropdown is open, Up, Down, `j` and `k` move the highlight (clamped, no wrap); Enter selects the highlighted view and closes the dropdown; Escape and `v` close it."}
  - {id: "FR-065-Mouse", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "This requirement adds no mouse capture mode. If the existing terminal lifecycle delivers mouse events, a click on the selector SHALL open the dropdown"}
  - {id: "FR-065-Help", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "F1, or `?`, SHALL open a help popup from every view. F1 and Escape close it without quitting and without changing any view state."}
  - {id: "FR-065-Modal-isolation", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "At most one popup is open. While one is open, all key and mouse input is consumed by it."}
  - {id: "FR-065-Header-footer", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The normal footer SHALL be one compact row that always shows `v View` and `F1 Help` in every view, and `q Quit` when width allows."}
  - {id: "FR-065-Scope-labels", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Status, Health and Retro SHALL name their population as the department selection: the explicit team and projects, or the saved report's scope."}
  - {id: "FR-065-Empty-Health-Retro", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The empty state starts no collection, retries no authentication and writes nothing."}
  - {id: "FR-059-Presentation", path: "spec/functional/FR-059-department-monitor-adapter.md", role: examined, excerpt: "The keys `1`, `2` and `3` SHALL select Status, Health and Retro respectively, as compatibility shortcuts for the View selector of [FR-065](./FR-065-dashboard-view-selector-and-help.md), which also owns help and the department scope labels."}
```

## Finding details

### FND-001 (medium) Modal isolation is stated without SHALL and without a subject

Lines 93-96 are the isolation requirement the ticket asks for ("Modal input cannot leak into underlying tables or quit accidentally"). They are four sentences with no SHALL: an invariant ("At most one popup is open"), an input rule ("all key and mouse input is consumed by it"), a negative rule over six targets ("No key reaches the table, details, refresh, source toggles, view keys, the Scope filter or quit") and an exception ("A key that would open the other popup is ignored, except that F1 closes the help popup"). The engine skips a statement with no SHALL, so the most safety-relevant bullet is outside the grammar check, has no system subject, and maps to AC-4 only by inference. The fix is mechanical: "The dashboard SHALL allow at most one popup open." and "While a popup is open, the dashboard SHALL deliver every key and mouse event to that popup only." with the six-target list and the exception as separate statements, one per AC row.

### FND-002 (medium) The empty-state side-effect rule is unmarked and compound

Lines 143-144: "The empty state starts no collection, retries no authentication and writes nothing." No SHALL, and three obligations (no collection, no auth retry, no write) in one sentence. It is also the statement the ticket calls out (acceptance 5: "No ... auth retries or automatic collectors"), and its subject, "the empty state", is a display state, not a system. Written as "While Health or Retro shows the not-configured state, the dashboard SHALL NOT start collection, SHALL NOT retry authentication and SHALL NOT write." it is still compound; split into three statements so AC-9 can assert each adapter separately. The bullet before it ("Not configured, failed and stale remain distinct") has the same defect and is covered by SR-121 FND-002.

### FND-003 (medium) The dropdown behavior bullet is mostly unmarked and non-singular

Lines 57-60 open with a correct state-driven trigger ("While the dropdown is open"), then give several behaviors: Up, Down, `j`, `k` move the highlight; the highlight is clamped; there is no wrap; Enter selects and closes; Escape and `v` close. Only the last sentence carries SHALL NOT, and it carries two ("SHALL NOT quit and SHALL NOT change the view, selection, scroll or details state"). The grammatical subject is a key list, not the dashboard. Nine observable behaviors (move, clamp, no wrap, select, close on Enter, close on Escape, close on `v`, no quit, no state change) share one statement and map to AC-1 and AC-2 only by reading. Split into one statement per behavior with the dashboard as subject.

### FND-004 (medium) The department scope-label bullet is non-singular and its last clauses are undefined

Lines 122-127 carry four obligations: name the population (SHALL), state it may differ from the lead's focus (SHALL), never call it the lead's focus (SHALL NOT), and neither "compute or assert that the two are equal or narrower" (SHALL NOT). The fourth is the weakest: "assert" is not observable by a test (the dashboard prints one fixed label, so it never asserts equality), and "No scope is calculated for this label" is a fifth statement with no modal verb. AC-7 verifies the first three by fixture and the fourth only by an Inspection of the diff. Split the bullet and replace "compute or assert" with an observable form, for example "the label text SHALL NOT contain the words equal, same, narrower or subset", or drop the clause and keep the Inspection as the verification of "adds no scope calculation".

## Verdict after disposition pass 1

Not run. Dispositions are reserved for this same session after the fix round.
