---
id: SR-121
title: "Failure-domain analysis - FR-065 dashboard View selector and F1 help (SWM-179)"
type: SpecReview
analysis: failure-domain
scope: "agent-ix/ix-relay; spec/functional/FR-065-dashboard-view-selector-and-help.md; spec/functional/FR-059-department-monitor-adapter.md; spec/functional/FR-060-focused-scope-presentation.md; crates/relay-tui/src/monitor_main.rs; crates/relay-tui/src/monitor_department.rs"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-065", type: references }
---

## Summary

Ticket: SWM-179. Spec-only change at frozen head df58008 (base 86d94d7). Failure-domain analysis of FR-065: popup modality against source failures, the not-configured, pending, failed and stale states of Health and Retro, and state integrity while a popup is open. The ticket's sharpest requirement here is that missing, failed and stale stay distinct and that critical evidence stays visible. 4 findings: 2 medium, 2 low.

## Verdict

**CONDITIONAL** - no `high` finding. The design is read-only and the popups add no side effect, so purity and termination are clean. The gaps are two places where a conforming implementation can still hide a failure from the user (evidence under a full-height popup) or print a not-configured state as a failure.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | Help may fill the terminal while evidence must stay outside any popup and new errors appear behind it, with no bound on popup extent | spec/functional/FR-065-dashboard-view-selector-and-help.md:154 |
| FND-002 | medium | Not-configured, failed and stale must be distinct, but the current code carries not-configured as a failed read and AC-9 would pass with the failure row still shown | spec/functional/FR-065-dashboard-view-selector-and-help.md:140 |
| FND-003 | low | A configured Health or Retro source awaiting its first read has no state of its own and would read as not configured or unavailable | spec/functional/FR-065-dashboard-view-selector-and-help.md:133 |
| FND-004 | low | Closing a popup is said to leave selection and scroll as before, but refreshes continue behind it and may clamp them | spec/functional/FR-065-dashboard-view-selector-and-help.md:97 |

## Review Context

Reviewer model claude-sonnet-5-5; the reviewer's own harness session ID was not visible, so none is claimed; run id unavailable. Root-supplied marker metadata: repository visibility public, quoin 0.36.7, module spec-artifacts-process@v0.29.2. The PR number was not supplied; the marker carries the placeholder `unavailable`. Marker id=SR-121 method=spec-review/failure-domain lang=spec.

What was and was not run. I read the quoin 0.29.4 spec-failure-domain-analysis skill, FR-065, FR-059 and FR-060, and the relay-tui files named in scope. Read, Glob, Grep, Edit and Write only; no command run; nothing here is a validation result. The Jev client and criterion-strength analysis were not used. No AssuranceProfile exists under spec/.

Checklist results with no finding:

- Extension points: none. The dropdown and the help popup take no callbacks, hooks or plugins.
- Entity identity: the six views are a fixed set keyed by name, the dropdown order is fixed, "at most one popup" makes the popup identity a two-valued state, and selecting the current view is defined as a no-op. No uniqueness key is missing. The identity-row matrix gap is a coverage defect, raised in SR-117 FND-005.
- Evaluation purity: help and the dropdown read state and write none; the spec forbids data sources, collectors and scope calculation, and AC-7 and AC-9 verify it.
- Topological robustness: no graph or recursion. The dropdown and help scroll are bounded by six entries and a fixed key list, so termination holds. Behavior below 80x24 is the only size case left undefined, and it is raised in SR-117 FND-002 with the evidence rule.
- Failure behavior of the read path: popups do not pause or retry reads (lines 99-101), so a blocked source read cannot make quit unresponsive through them; quit while a popup is open is by Escape then Escape, which the spec states.

## Examined Scope

```yaml
scope:
  - {id: "FR-065-Help", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Help SHALL mark the keys that apply in the current view. If help is taller than the terminal, Up, Down, PageUp and PageDown scroll it; every line is reachable at 80x24."}
  - {id: "FR-065-Dropdown-size", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "If the terminal is too small for the whole dropdown, the list SHALL scroll to keep the highlight visible."}
  - {id: "FR-065-Modal-isolation", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Source polling, refresh results and the terminal lifecycle continue while a popup is open."}
  - {id: "FR-065-Popup-close", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Closing a popup by Escape leaves the underlying view, selection, scroll, details and filter state as they were before it opened."}
  - {id: "FR-065-Evidence", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Source, stale, error, last-snapshot, rate-limit countdown, binding and filter evidence SHALL be shown outside any popup, on their own row or rows"}
  - {id: "FR-065-Empty-Health-Retro", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "When no retro configuration or saved report supplies Health or Retro fields, the view SHALL show one concise state, not a table of placeholders"}
  - {id: "FR-065-Not-configured-failed-stale", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Not configured, failed and stale remain distinct."}
  - {id: "FR-065-AC-2", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Escape or `v` with the dropdown open closes it and leaves view, selection, scroll and details unchanged, and does not quit; Escape with no popup open still quits."}
  - {id: "FR-065-AC-5", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "At 80x24 with a source error, a stale source, a rate-limit countdown and an active filter present, all of that evidence remains on screen outside the popup and `v View` and `F1 Help` remain visible."}
  - {id: "FR-065-AC-9", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Health and Retro fixtures with no configuration, with a saved report lacking the fields, with a failed source and with a stale retained model"}
  - {id: "FR-065-AC-10", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "An open popup while a source refresh completes or fails keeps the popup open and updates the evidence behind it."}
  - {id: "FR-059-Presentation", path: "spec/functional/FR-059-department-monitor-adapter.md", role: context_only, excerpt: "When a department view is selected, the monitor SHALL preserve scope and reset row selection and scroll, with selection clamped after a refresh shrinks rows."}
  - {id: "FR-059-Failure", path: "spec/functional/FR-059-department-monitor-adapter.md", role: context_only, excerpt: "If refresh fails, then the monitor SHALL retain the last successful model with its original evidence/window timestamps and a visible last-snapshot error."}
  - {id: "FR-060-Completed-projects", path: "spec/functional/FR-060-focused-scope-presentation.md", role: context_only, excerpt: "After a toggle or refresh changes the rows, selection stays on the same project placement when it remains and otherwise clamps; scroll clamps."}
```

## Finding details

### FND-001 (medium) Evidence outside the popup, with no limit on how much the popup covers

AC-5 requires that at 80x24 "all of that evidence remains on screen outside the popup", and the Evidence bullet requires it "outside any popup". Help, though, is allowed to be taller than the terminal and scroll ("every line is reachable at 80x24", lines 87-89), so it can occupy the whole screen, and the dropdown scrolls when it is too small (lines 68-69). The spec never bounds the popup's extent or says which rows a popup must leave uncovered. The Modal isolation bullet then says new data and errors "appear behind it and, if applicable, in the footer", where "if applicable" is undefined, and AC-10 verifies only that the popup "updates the evidence behind it". A source failure, a rate-limit countdown or a replaced session (the identity row status, "session ended/replaced · restart to rebind") that arrives while help is open at 80x24 can be completely hidden, which is the failure the ticket's acceptance 3 forbids. AC-5 does not say a popup is open in its fixture at all.

Fix: reserve rows. State that a popup SHALL leave the identity row, the evidence rows and the footer visible and scroll inside the remaining area (or that evidence is drawn above the popup), delete "if applicable", and make AC-5 and AC-10 assert the evidence rows with help open at 80x24 while a refresh fails and while the session is replaced.

### FND-002 (medium) Not configured is carried as a failure today and AC-9 would not catch it staying one

Lines 140-142 require "Not configured, failed and stale remain distinct" and that none appears as zero, green or complete. In the code, when neither `--report` nor `--retro-config` is given, startup calls `app.department.failure("Department health/retro unavailable: supply --retro-config ...")` (monitor_main.rs:148-150), so not-configured is stored as a failed read. `diagnostic_rows` turns any department error into a "Department" source-failure row (monitor_department.rs:426-452), the renderer prints that row in the red diagnostics region of every department view (monitor_department.rs:603-632), and the `e` list shows it as an unavailable source; a test pins the text (monitor_department.rs:1035). AC-9's not-configured fixture asserts "the not-configured requirement text" and "no zero, green or complete value", not the absence of a failure row. An implementation can add the new empty-state text and keep the `failure(...)` call, pass AC-9, and still present the not-configured state as a failed source in Status, Health and Retro, which is the conflation the ticket's acceptance 5 names.

Fix: say in the bullet that the not-configured state is not a source failure: it SHALL NOT appear in the source-failure rows, the `e` diagnostics or error styling, and it SHALL NOT make a normal quit nonzero; and add that assertion to the AC-9 no-configuration and saved-report-without-fields fixtures.

### FND-003 (low) A configured source awaiting its first read has no state

When `--retro-config` is supplied, startup spawns the department worker (monitor_main.rs:135-141, 151-153), and until its first result `department.value` and `department.error` are both empty. The renderer then falls back to "Unavailable: no shared department report" (monitor_department.rs:581-589). FR-065's empty-state rule is for "no retro configuration or saved report supplies" the fields, and its distinct set is not configured, failed and stale; a configured source that has not yet answered is none of them. Implemented as written, the pending window shows either the not-configured instruction (false, the user did supply a config) or the old unspecific text. Fix: add a fourth state, waiting for the first read, that names the configuration being read and is distinct from the other three, with an AC-9 fixture before the first result.

### FND-004 (low) Unchanged state versus refresh behind a popup

Lines 97-98: closing a popup by Escape "leaves the underlying view, selection, scroll, details and filter state as they were before it opened". Lines 99-101 say refresh results continue behind the popup, and FR-059 and FR-060 require selection and scroll to clamp when a refresh changes the rows. If rows shrink while a popup is open, the state after closing cannot be "as they were before it opened". AC-2 and AC-4 are static fixtures and would not show it, and AC-10 does not assert the selection after a refresh behind a popup. Fix: say the popup and its keys change none of that state, and that refresh-driven clamping applies as with no popup; assert the clamped selection in AC-10.

## Verdict after disposition pass 1

Not run. Dispositions are reserved for this same session after the fix round.

## Dispositions

Disposition pass 1 (round 1), same reviewer session as round 0. Fix source frozen at 6b48064 (base 86d94d7), spec-only. Original findings, severities, excerpts and the round-0 verdict above are unchanged. spec-fix1-ready.md was treated as data; each outcome was checked against the current FR-065 and against the code facts I read in round 0 (monitor_main.rs:148-150, monitor_department.rs:426-452, 581-589, 603-632). I ran no command. The four-row evidence budget is an accepted root implementation choice.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed | Modal isolation now says "A popup SHALL draw only within the work area. It SHALL NOT cover the header rows, the evidence rows or the controls footer row", and Terms define the work area as excluding the identity row with the selector, the scope label, the evidence rows and the footer row. The "behind it, if applicable" wording is removed. AC-5 asserts the evidence labels and footer hints at 80x24; AC-11 asserts a failing refresh, a rate-limit countdown and a session replacement all appear in the evidence rows and footer with help open at 80x24, with the popup inside the work area. |
| FND-002 | fixed | Empty Health and Retro now say the Not configured and Waiting states "SHALL NOT be stored, labeled or styled as a source failure", SHALL NOT appear in the source-failure rows or the `e` source list, SHALL NOT be described as an authentication failure and "SHALL NOT by themselves affect the exit status"; FR-059's rules for real collection failures are unchanged. AC-9 asserts the same for the no-configuration, saved-report and waiting fixtures. This closes the gap that let the existing `department.failure(...)` storage pass. Implementation must change that storage and its pinned test text; that is an implementation consequence, not a spec gap. |
| FND-003 | fixed | Health and Retro now choose one of four states: Not configured, Waiting (a configuration was supplied and its first read has not returned, with a message that names the retro configuration), Failed (classified reason, no retained model) and Stale (retained values with the stale or last-snapshot label). AC-9 has a fixture for configuration supplied before its first read. |
| FND-004 | fixed | Closing a popup now restores state "except for changes from refresh results or terminal resize received while it was open", which apply the existing FR-059 and FR-060 clamping as if no popup were open. AC-11 asserts that a refresh that shrinks rows while a popup is open clamps the selection and that closing leaves exactly that state. |

## Current verdict after disposition pass 1

**CONDITIONAL** for the failure-domain analysis. All 4 original findings (2 medium, 2 low) are fixed. One new low finding (FND-005, below) is a regression I found in the new text: it is open and needs a one-sentence fix before this method is PASS. It is not high, so the method is not a FAIL.

## New findings (disposition pass 1)

### Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-005 | low | The new bullet "SHALL NOT start collection" while a popup is open or Health or Retro is Not configured or Waiting contradicts the retained rule that source polling continues while a popup is open | spec/functional/FR-065-dashboard-view-selector-and-help.md:256 |

### FND-005 (low) The no-collection rule contradicts the polling rule

The Modal isolation bullet at lines 158-159 says "Source polling, refresh results and the terminal lifecycle SHALL continue while a popup is open". The Empty Health and Retro bullet at lines 256-257 says "While Health or Retro shows Not configured or Waiting, and while either popup is open, the dashboard SHALL NOT start collection." Read literally, scheduled polling (the FR-048 Linear, Herdr and Redis timers, and the department worker's own reads while Waiting) is both required to continue and forbidden to start while a popup is open or Health or Retro shows Not configured, and in the Status view the tracker reads that feed it must go on in the Not configured state. AC-10 avoids the conflict by holding the worker clocks and comparing counts, so the test is sound; the normative text is not. This is the same "caused by" ambiguity as SR-117 FND-006, reintroduced as an absolute in the fix. Evidence: lines 158-159 and 256-257 of the fixed FR-065.

Fix: say that the empty state and opening or closing a popup SHALL NOT cause or trigger a collection, authentication attempt or write, and that scheduled polling is unchanged (FR-048 and FR-059). This is a wording edit to one bullet; no new requirement.
