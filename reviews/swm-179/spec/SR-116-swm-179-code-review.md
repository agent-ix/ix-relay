---
id: SR-116
title: "Code review (spec lane) - FR-065 dashboard View selector and F1 help (SWM-179)"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; spec/functional/FR-065-dashboard-view-selector-and-help.md; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md; crates/relay-tui/src/monitor.rs; crates/relay-tui/src/monitor_ui.rs; crates/relay-tui/src/monitor_department.rs; crates/relay-tui/src/monitor_main.rs"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-065", type: references }
---

## Summary

Ticket: SWM-179. Spec-only change at frozen head df58008 (base 86d94d7), branch codex/swm-179-dashboard-navigation. Code-review method, lang spec: the diff is Markdown only (new FR-065, edits to FR-048 and FR-059), so there is no Rust lane and no gap analysis. The review applied the language-independent sections of code-review to the spec: Duplication, Unasked-for Ceremony and File Tracking, Integrity, and Spec-Code Faithfulness (every factual claim FR-065 makes about the current dashboard was checked against crates/relay-tui/src). 2 findings: 1 medium, 1 low.

## Verdict

**CONDITIONAL** - no `high` finding. FR-065 adds no hash, digest, pin, manifest, ledger or file-tracking ceremony, vendors nothing from another repo, and its claims about the current code that I checked are true (`v`, F1 and `?` are unbound; Escape quits; there is no mouse capture; an error replaces the controls outside Scope; the tab strips exist). Two defects remain: Help is required to state "applicable views" but the spec never says which, and the code shows the answer is not "every view" for `s`, `e` and Left/Right; and the key inventory is restated a third and fourth time.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-065-AC-3 requires Help to list each key with its applicable views, but no applicability is stated, and the code shows `s`, `e` and Left/Right are not global | spec/functional/FR-065-dashboard-view-selector-and-help.md:152 |
| FND-002 | low | FR-065-AC-4 restates the full key inventory that FR-060-AC-4 and FR-064-AC-12 already enumerate, a second and third home for one list | spec/functional/FR-065-dashboard-view-selector-and-help.md:153 |

## Review Context

Reviewer model claude-sonnet-5-5; the reviewer's own harness session ID was not visible, so none is claimed; run id unavailable. Root-supplied marker metadata: repository visibility public, quoin 0.36.7, module spec-artifacts-process@v0.29.2. The PR number was not supplied to the reviewer; the marker carries the placeholder `unavailable` for the parent to replace.

What was and was not run. I read the installed dev-team reviewer skill (0.4.1), dev-tools code-review (0.2.0), the quoin 0.29.4 spec-review, spec-ears, spec-integrity, spec-dependency, spec-failure-domain and spec-app-review skills, the brief, the ticket acceptance text (request.md), the author report (spec-ready.md), the actual diff (spec-review.diff), the full FR-048, FR-059, FR-060 and FR-064 texts, the frozen worktree copy of FR-065, and the relay-tui source files named in scope. I had Read, Glob, Grep, Edit and Write only. I ran no command: no quire, quoin, git, cargo or tracker call. Nothing here is a validation result; the parent runs `quire validate` at an explicit path. No Rust lane, no rust-review, no gap analysis (MD-only diff). The Jev client and criterion-strength analysis were not used and no availability is claimed. No AssuranceProfile exists under spec/ (searched by file name and by the `type: AssuranceProfile` line); none applies.

Checks that produced no finding (examined, held):

- Duplication across repos and vendoring: none. FR-065 links FR-048, FR-059, FR-060 and FR-064 by relative path and restates no upstream type or constant. It adds no `ix://` target outside ix-relay.
- Unasked-for ceremony (P0): none. A case-insensitive search of FR-065 for hash, digest, checksum, pin, manifest, ledger, provenance, attestation, receipt, relocation, inventory and revision matched only the word SHALL.
- Spec-code faithfulness, claims that hold: `v` collides with no key in `handle_key` (monitor.rs:233-278); Escape and `q` set `should_quit` (monitor.rs:233); the terminal setup enables raw mode and the alternate screen only, with no mouse capture (monitor_main.rs:164-167), so the conditional mouse clause is correctly conditional; outside Scope an error replaces the controls row (monitor_ui.rs:377-383), as the author report says; the tab strips FR-065 removes are at monitor_ui.rs:226-250 and monitor_department.rs:486-495; the quoted FR-048-AC-4 and FR-059-AC-4 phrases in Precedence match the base text.
- The FR-048-AC-4 edit is applied cleanly and FR-048, FR-059 now point at FR-065.
- The `f` key does not exist in the source (no `'f'` arm in `handle_key`); FR-065 correctly makes it conditional, and the verification gap this causes is raised as a dependency finding in SR-120, not here.

## Examined Scope

```yaml
scope:
  - {id: "FR-065", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The existing `team-leader-dashboard` SHALL offer one View selector that lists all six existing views (Tickets, Agents, Scope, Status, Health, Retro) and one F1 help popup, identical in every view."}
  - {id: "FR-065-Precedence", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "For presentation of the view switch only, this requirement takes precedence over the following statements, and only to the extent listed:"}
  - {id: "FR-065-Help", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Help SHALL list, with the views where each applies:"}
  - {id: "FR-065-AC-3", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "F1 and `?` open help in all six views; F1 and Escape close it without quitting or state change."}
  - {id: "FR-065-AC-4", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "With a popup open, a fixture sending every existing key (`q`, `r`, `d`, `Enter`, `s`, `e`, `t`, `a`, `p`, `1`, `2`, `3`, `c`, `f`, `j`, `k`, arrows, PageUp, PageDown) leaves table selection, scroll, details, view, refresh flag, status toggle and filter mode unchanged"}
  - {id: "FR-065-Dependencies", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Out of scope: metric formulas, new telemetry, credential or setup changes, named-focus activation or adoption, changes to tracker initiatives, source timing, a new renderer or framework, dependency repins and any other service."}
  - {id: "FR-048-AC-4", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: "The current view is named by the View selector of [FR-065](./FR-065-dashboard-view-selector-and-help.md), with no persistent tab strip."}
  - {id: "FR-048-Dependencies", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: "[FR-065](./FR-065-dashboard-view-selector-and-help.md) adds the View selector and F1 help. The `t`, `a` and `p` keys remain compatibility shortcuts."}
  - {id: "FR-059-Presentation", path: "spec/functional/FR-059-department-monitor-adapter.md", role: examined, excerpt: "The keys `1`, `2` and `3` SHALL select Status, Health and Retro respectively, as compatibility shortcuts for the View selector of [FR-065](./FR-065-dashboard-view-selector-and-help.md), which also owns help and the department scope labels."}
  - {id: "FR-059-AC-4", path: "spec/functional/FR-059-department-monitor-adapter.md", role: examined, excerpt: "Existing compact Tickets/Agents/Scope fixtures and their keys retain their outputs, except for the header selector, footer and labels changed by FR-065."}
  - {id: "FR-059-Dependencies", path: "spec/functional/FR-059-department-monitor-adapter.md", role: examined, excerpt: "[FR-065](./FR-065-dashboard-view-selector-and-help.md), the View selector, F1 help and department scope labels over these views."}
  - {id: "crates/relay-tui/src/monitor.rs", path: "crates/relay-tui/src/monitor.rs", role: examined, excerpt: "KeyCode::Char('e') => {"}
  - {id: "crates/relay-tui/src/monitor_ui.rs", path: "crates/relay-tui/src/monitor_ui.rs", role: examined, excerpt: "fn footer_controls("}
  - {id: "crates/relay-tui/src/monitor_department.rs", path: "crates/relay-tui/src/monitor_department.rs", role: examined, excerpt: "let rows = if app.show_diagnostics {"}
  - {id: "crates/relay-tui/src/monitor_main.rs", path: "crates/relay-tui/src/monitor_main.rs", role: examined, excerpt: "enable_raw_mode()?;"}
  - {id: "FR-060-AC-4", path: "spec/functional/FR-060-focused-scope-presentation.md", role: context_only, excerpt: "every existing key (`q`, `Esc`, `r`, `d`, `Enter`, `s`, `e`, `t`, `a`, `p`, `1`, `2`, `3`, `j`, `k`, arrows, paging) still performs its prior action."}
  - {id: "FR-064-AC-12", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: context_only, excerpt: "and `t`, `a`, `p`, `1`, `2`, `3`, `c`, `s`, `e`, `d`, `r`, `q`, `Esc`, paging and arrows keep their actions."}
```

## Finding details

### FND-001 (medium) Help applicability is required but never stated, and the code shows it is not uniform

Rule: code-review Spec-Code Faithfulness (does the described behavior match the implemented one) and Code-Test Alignment (an AC needs an oracle). FR-065-AC-3 requires Help to list "every key in the Behavior list with applicable views" and to mark "the current view's applicable keys". The Help bullet (lines 75-86) lists the keys but gives views only for `c` (Scope) and `f` (named focuses) and for Left/Right ("department details"). It says nothing about `s` and `e`, and the AC supplies no matrix, so a test has no expected set to assert against.

The source answers the question, and the answer is not "every view". `s` toggles `app.show_status` (monitor.rs:236) and that flag is read only by the lead-view footer (monitor_ui.rs:336-341); the department renderer (monitor_department.rs:456-639) never reads it, so `s` has no visible effect in Status, Health or Retro. `e` toggles `app.show_diagnostics` and also resets the table and scroll (monitor.rs:237-242); only the department renderer reads the flag (monitor_department.rs:542, 583), so in Tickets, Agents and Scope `e` shows nothing and silently clears the selection. Left and Right change `detail_scroll` in every view (monitor.rs:252-257) but only the department detail pane scrolls by it (monitor_department.rs:598). FR-065 gets Left/Right right ("department details") and leaves `s` and `e` as global-sounding lines under "diagnostics".

Risk: an implementer either advertises `s` and `e` as global (the help then lies in the views where they do nothing, and `e` advertises a key that resets the selection) or reverse-engineers the matrix from the code. AC-3 cannot fail on the part the user asked for ("applicable context" in ticket acceptance 2).

Fix: state applicability in the Help bullet and AC-3: `s` Tickets, Agents, Scope; `e` Status, Health, Retro; Left/Right Status, Health, Retro with details open; `c` Scope; `d` and Enter all views; and make AC-3 assert the marked set per view from that table. Do not change `e` or `s` behavior in this ticket; the finding is only that Help must not claim more than the code does.

### FND-002 (low) The key inventory is restated again

Rule: code-review Duplication inside one repo, applied mechanically: does the repo now hold a second statement of something defined elsewhere. Search done: the key names `PageUp`, `PageDown` and the `t`, `a`, `p`, `1`, `2`, `3` list across spec/functional. The same inventory already exists in FR-060 Behavior (lines 124-126, the "collides with no existing monitor key" list), FR-060-AC-4 (line 185), FR-064 Behavior (lines 123-124) and FR-064-AC-12 (line 195). FR-065 adds it to the Help bullet (lines 75-86) and spells it a third time in AC-4 (line 153), now with `c` and `f` appended. The action each key performs stays owned by FR-048, FR-059 and FR-060; the repeated lists are what drift when a key is added or removed. FR-065 is itself a key addition, and FR-060 line 124 and FR-064 line 123 do not mention `v`, F1 or `?`.

Severity is low, not higher, because the lists state which keys exist, not what they do, the repo already does this in two requirements, and the Help contents need some home. Confidence is low: another reviewer could accept AC-4's list as fixture data.

Fix: write AC-4 as "every key defined by FR-048, FR-059 and FR-060, and by FR-064 where named focuses are active, plus F1 and `?` and `v`", keep the literal key table in the test fixture, and let the Help bullet name keys by owner. This also removes the `f` problem raised in SR-120.

## Verdict after disposition pass 1

Not run. Dispositions are reserved for this same session after the fix round.

## Dispositions

Disposition pass 1 (round 1), same reviewer session as round 0. Fix source frozen at 6b48064 (base 86d94d7), spec-only. Original findings, severities, excerpts and the round-0 verdict above are unchanged. The author's spec-fix1-ready.md was treated as data; every outcome below was checked against the current FR-065 and the FR-048, FR-059, FR-060 and FR-064 clauses. I ran no command (Read, Glob, Grep, Edit, Write only); the parent's Quire runs are the validation evidence.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed | FR-065 Help now carries a table naming each key's owner and views: `s` Tickets, Agents, Scope; `e` Status, Health, Retro; Left/Right Status, Health, Retro with details open; `c` Scope; `d`, Enter, `r`, navigation, `q` and Escape all views. It adds "Help SHALL NOT mark a key as applicable in a view where it has no visible effect" and "This requirement SHALL NOT change what `s`, `e`, Left or Right do in any view", matching the code reading (monitor.rs:236-257, monitor_ui.rs:336, monitor_department.rs:542,598). FR-065-AC-3 asserts that each view's marked keys equal that view's table column. |
| FND-002 | fixed | The Help table names each key once, by owning requirement, and FR-065-AC-4 now draws its sweep from "the keys defined by FR-048, FR-059 and FR-060 plus F1, `?` and `v`", with no literal list in the AC. `f` is no longer carried (see SR-120 FND-003). The older FR-060 and FR-064 lists remain as they were and are not duplicated further. |

## Current verdict after disposition pass 1

**PASS** for the code-review method. Both original findings (1 medium, 1 low) are fixed; none is open, deferred or rejected. I found no regression of the kind this method covers: the new Help applicability claims match the source I read, and FR-065 still adds no hash, digest, pin or ceremony and no cross-repo copy. A separate low regression in the popup rules is recorded in SR-121 and does not change this method's verdict. This is a source and text reading, not a validation; the Quire results come from the parent.

## New findings (disposition pass 1)

None for this method.
