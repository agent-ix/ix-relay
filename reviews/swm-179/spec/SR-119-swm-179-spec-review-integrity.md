---
id: SR-119
title: "Integrity analysis - FR-065 dashboard View selector and F1 help (SWM-179)"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay; spec/functional/FR-065-dashboard-view-selector-and-help.md; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md; spec/functional/FR-060-focused-scope-presentation.md; spec/functional/FR-064-named-focus-dashboard-reader.md"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-065", type: references }
---

## Summary

Ticket: SWM-179. Spec-only change at frozen head df58008 (base 86d94d7). Integrity analysis of the new FR-065 structure: consistency and conflict against FR-048, FR-059, FR-060 and FR-064, hidden assumptions (the multi-source probe), and atomicity of the labeling rules. The ticket's central worry, a department population mislabeled as the lead's focus or the reverse, is where the findings are: the lead-side label rule is wrong in three states, and the department-side rule names two sources where the code has three. 5 findings: 4 medium, 1 low.

## Verdict

**CONDITIONAL** - no `high` finding. The direction is right: FR-065 never calls a department population the lead's focus, adds no scope calculation, and keeps FR-060's labels. The medium defects are rules that, followed literally, would print a false caption or fail a conforming fixture.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | Tickets, Agents and Scope SHALL be named the lead's reported scope, which is false in whole-team fallback, ticket-only and no-report states where FR-060 requires the label never imply lead focus | spec/functional/FR-065-dashboard-view-selector-and-help.md:120 |
| FND-002 | medium | AC-7 labels Agents as the lead's reported scope, but Agents lists every reported assignment independent of scope (FR-048-AC-6), and in named mode Tickets and Scope follow the viewed focus (FR-064) | spec/functional/FR-065-dashboard-view-selector-and-help.md:156 |
| FND-003 | medium | The list of permitted fixture differences (header selector row, footer, labels) omits the identity row in department views, the Status purpose line and the replaced Health and Retro empty state that AC-6, AC-8 and AC-9 require | spec/functional/FR-065-dashboard-view-selector-and-help.md:159 |
| FND-004 | medium | The department selection is said to be explicit team and projects or the saved report's scope, but a lead-only startup derives the team from the lead's workspace, a third source the labels and the retro-config instruction do not cover | spec/functional/FR-065-dashboard-view-selector-and-help.md:122 |
| FND-005 | low | The header selector shows its opening key, while FR-048-AC-4 still says only footer hotkeys use the bright primary blue | spec/functional/FR-065-dashboard-view-selector-and-help.md:49 |

## Review Context

Reviewer model claude-sonnet-5-5; the reviewer's own harness session ID was not visible, so none is claimed; run id unavailable. Root-supplied marker metadata: repository visibility public, quoin 0.36.7, module spec-artifacts-process@v0.29.2. The PR number was not supplied; the marker carries the placeholder `unavailable`. Marker id=SR-119 method=spec-review/integrity lang=spec.

What was and was not run. I read the quoin 0.29.4 spec-integrity-analysis skill, FR-048, FR-059, FR-060 and FR-064 in full, FR-065 and the relay-tui source (monitor.rs, monitor_ui.rs, monitor_department.rs, monitor_main.rs) to check facts about the startup modes. Read, Glob, Grep, Edit and Write only; no quire or quoin command; nothing here is a validation result. The Jev client and criterion-strength analysis were not used. No AssuranceProfile exists under spec/.

Completeness step: FR-065 has a verification method for every behavior group through AC-1 to AC-10; it names no US or stakeholder requirement, the same as FR-060 and FR-064, and I did not raise it. Hidden-assumption probes: the multi-source lookup probe applies (the department selection comes from several sources) and produced FND-004; the external CLI, paginated API and auth probes do not apply (no new reads). Atomicity and testability: raised in SR-117 and SR-118; not repeated. The failure-domain pass is SR-121.

Checks that held: the three-way key retention in Precedence is consistent with FR-059 and FR-060 (the keys stay); the dropdown order matches the six views that exist; no FR-065 clause contradicts the FR-059 startup defaults (explicit unbound scope and saved reports start in Status, lead-only keeps Tickets); FR-065 does not alter FR-060's `c` footer text or FR-064's `f` behavior.

## Examined Scope

```yaml
scope:
  - {id: "FR-065", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The existing `team-leader-dashboard` SHALL offer one View selector that lists all six existing views (Tickets, Agents, Scope, Status, Health, Retro) and one F1 help popup, identical in every view."}
  - {id: "FR-065-Precedence", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "those fixtures change only in the header selector row, the footer and the Status/Health/Retro labels stated here."}
  - {id: "FR-065-View-selector", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Every view SHALL show, in its header, the same selector labelled `View` with the current view name and the key that opens it."}
  - {id: "FR-065-Header-footer", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The identity row SHALL show the same monitored lead identity, binding state and activity markers in all six views."}
  - {id: "FR-065-Scope-labels", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Tickets, Agents and Scope SHALL name their population as the lead's reported scope, using the FR-060 labels unchanged."}
  - {id: "FR-065-Empty-Health-Retro", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "For a live startup, it names `--retro-config <file>` whose team and project populations match the selectors"}
  - {id: "FR-065-AC-6", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "The identity row (lead identity, binding state, activity markers, or the explicit no-lead and binding-diagnostic text) is identical across all six views"}
  - {id: "FR-065-AC-7", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "A fixture where the department team/projects and the lead's reported scope differ renders Status, Health and Retro labeled as department scope"}
  - {id: "FR-065-AC-8", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Status shows the specified purpose line at 80x24 and 120x40 without hiding source or error evidence."}
  - {id: "FR-065-AC-9", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Health and Retro fixtures with no configuration, with a saved report lacking the fields, with a failed source and with a stale retained model"}
  - {id: "FR-065-AC-10", path: "spec/functional/FR-065-dashboard-view-selector-and-help.md", role: examined, excerpt: "Existing FR-048-AC-1 binding, FR-059 source-failure, replacement-fence, quit, terminal-restoration and FR-060 scroll/selection/toggle fixtures pass with only the header, footer and label differences stated above."}
  - {id: "FR-048-AC-4", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: "Only footer hotkeys use the bright primary blue; their descriptions and separators are muted."}
  - {id: "FR-048-AC-6", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: "The a key selects a scrollable Agents view that lists every reported assignment even when scope or Linear is unavailable."}
  - {id: "FR-059-Startup", path: "spec/functional/FR-059-department-monitor-adapter.md", role: context_only, excerpt: "Explicit unbound scope and saved reports SHALL start in Status. Existing lead-only startup SHALL retain its Tickets default and `t`, `a`, `p` navigation."}
  - {id: "FR-059-AC-4", path: "spec/functional/FR-059-department-monitor-adapter.md", role: examined, excerpt: "Existing compact Tickets/Agents/Scope fixtures and their keys retain their outputs, except for the header selector, footer and labels changed by FR-065."}
  - {id: "FR-060-Focus-modes", path: "spec/functional/FR-060-focused-scope-presentation.md", role: context_only, excerpt: "**Whole-team fallback**: the report names no initiative, project or ticket, or no report exists. The scope line says plainly that no focus is reported and the rows are the whole team. It never says or implies lead focus."}
  - {id: "FR-064-Rows-and-navigation", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: context_only, excerpt: "The Tickets view uses the leaves of the viewed selection as its scope. The Agents view keeps FR-048-AC-6: it lists every reported assignment."}
```

## Finding details

### FND-001 (medium) The lead-side label rule is false when there is no lead report

Lines 120-121: "Tickets, Agents and Scope SHALL name their population as the lead's reported scope, using the FR-060 labels unchanged." FR-060 defines three modes. In Whole-team fallback ("the report names no initiative, project or ticket, or no report exists") the scope line "says plainly that no focus is reported ... It never says or implies lead focus"; Ticket-only uses the same fallback label. FR-065 makes the generic caption "the lead's reported scope" the rule and the FR-060 labels the qualifier, so in a fallback state the new rule asks for a caption FR-060 forbids, and AC-7 asserts the caption in those views. The same holds in a startup with no lead report at all: explicit team or project scope, or a saved report, where FR-065 itself says an unavailable view stays selectable, so Tickets, Agents and Scope exist but no lead has reported anything. The ticket's acceptance 4 is precisely that no population is mislabeled.

Fix: state the lead-side label per mode: Focused is the lead's reported focus; fallback and ticket-only use the FR-060 label verbatim; with no lead report (explicit scope or saved-report startup) say there is no lead report; named mode follows FR-064. AC-7 then asserts one caption per mode, not one caption for three views.

### FND-002 (medium) Agents is not scope-bound, and named mode changes what Tickets and Scope show

AC-7 requires "Tickets, Agents and Scope labeled as the lead's reported scope". FR-048-AC-6 makes the Agents view list "every reported assignment even when scope or Linear is unavailable": its population is the reported assignments, not a scope, so the label misattributes in the direction opposite to the one FR-065 guards against. FR-064, already in the spec tree, says that in named mode the Tickets view "uses the leaves of the viewed selection as its scope", Scope shows the named focuses, and a legacy reported scope is shown in details as "not in use while named focuses are set". In that mode the lead's reported scope is not the population of Tickets or Scope. FR-065 cites FR-064 only for the `f` key.

Fix: label Agents by what it lists (reported assignments, all of them); defer the Tickets and Scope caption to FR-064 in named mode; AC-7 asserts those three captions.

### FND-003 (medium) The permitted-difference list is narrower than the requirements that change the fixtures

Precedence (lines 36-38) lets the FR-059-AC-4 fixtures change "only in the header selector row, the footer and the Status/Health/Retro labels"; AC-10 says "only the header, footer and label differences stated above". But FR-065 also requires: an identity row with binding state and activity markers in the department views (AC-6; today the department header is "Department · {lead_name}" and the scope text, monitor_department.rs:477-485); the Status purpose line (AC-8); and a single empty state in place of the current Health and Retro table plus "Unavailable: no shared department report" (AC-9; monitor_department.rs:581-589); and the relocated evidence rows. A conforming implementation fails FR-059-AC-4 and FR-065-AC-10 read literally, since those differences are not among the "stated above". Either the department fixtures are retained, and AC-6, AC-8 and AC-9 cannot pass, or they are not, and the Precedence list is wrong. FR-060's own sentence that Scope fixtures "change only for the rows, labels and key above" is the dependency half of this conflict and is raised in SR-120 FND-001.

Fix: list every permitted difference once (selector row, identity row in department views, footer, evidence rows, scope labels, Status purpose line, Health and Retro empty states) and have Precedence and AC-10 refer to that list.

### FND-004 (medium) The department selection has a third source that the labels do not cover

Lines 122-123 name the department population as "the explicit team and projects, or the saved report's scope". The code derives a third one: `department_team()` returns the explicit `--team`, and otherwise, when no project is given, the team from the bound lead's workspace (monitor.rs:146-156). A lead-only startup, the one FR-059 says keeps the Tickets default and the one a user runs beside a coding pane, reaches Status, Health and Retro by `1`, `2` and `3` with that derived team and every project. In that startup the "explicit team and projects" label is false, and the department population is the lead's own team, the case where it is easiest to confuse with the lead's focus. The empty-state instruction ("`--retro-config <file>` whose team and project populations match the selectors") also has no selectors to match.

Fix: add the derived source to the label rule (name it as derived from the lead's workspace, all projects) and say what the retro-config instruction says when no explicit selector was given. Add a lead-only fixture to AC-7.

### FND-005 (low) The selector key and the retained blue-hotkeys sentence

The View selector shows "the key that opens it" in the header (line 49-50). FR-048-AC-4, edited by this diff, still says "Only footer hotkeys use the bright primary blue; their descriptions and separators are muted." If the selector key is styled as the existing tab keys are (PRIMARY, monitor_ui.rs:238-241, already a mismatch with that sentence), the retained text is false; if it is not, the spec does not say how it is styled. Fix: say in FR-065 how the selector key is styled, and amend the FR-048-AC-4 sentence to "footer and selector hotkeys" or state that the selector key is not blue.

## Verdict after disposition pass 1

Not run. Dispositions are reserved for this same session after the fix round.
