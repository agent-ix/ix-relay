---
id: SR-092
title: "Integrity analysis - FR-060 focused Scope presentation (SWM-176)"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay; spec/functional/FR-060-focused-scope-presentation.md; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md"
review_set: subset
---

## Summary

Ticket: SWM-176. PR ix-relay#39 at frozen 11c226b3a71e94741e62b45cce0fcc41eb0e2580. Integrity analysis of FR-060: completeness, consistency, atomicity and hidden-assumption probes. `quire validate --scope .` on the FR exits 0 and the computed matrix mints exactly FR-060-AC-1 through FR-060-AC-6.

## Verdict

**CONDITIONAL** - three medium/low ambiguities (hidden-count unit, toggle key and scope, ordering). No ID, cross-reference or relationship defect: each AC has a Verification method and Dependencies agree with the frontmatter edges (constrains FR-048, depends_on FR-059). The FR-048/FR-059 precedence contradiction is recorded once, in SR-091.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | Hidden-project count unit undefined when a shared project appears under several initiatives | spec/functional/FR-060-focused-scope-presentation.md:84 |
| FND-002 | low | Toggle key unnamed; scope and persistence across views unstated | spec/functional/FR-060-focused-scope-presentation.md:84 |
| FND-003 | low | Row ordering for initiatives, projects and project-only group unspecified | spec/functional/FR-060-focused-scope-presentation.md:82 |

## Review Context

Reviewer model claude-sonnet-5-5; run id unavailable from this harness. Linear marker id=SR-092 method=spec-review/integrity lang=spec. Review set is the proportional subset chosen by the dispatch: code-review, integrity, scope-boundary, failure-domain. Other quoin analyses (ears, evidence, risk-complexity, criterion-strength, object, architecture) were not run for this one six-AC FR. No AssuranceProfile applies. Read-only apart from evidence artifacts and Linear comments.

## Examined Scope

```yaml
scope:
  - {id: "FR-060", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "The existing `team-leader-dashboard` Scope view SHALL present the lead's\nexplicitly reported initiative and project selection as a parent-child\nhierarchy, hide finished projects by default, and stay responsive on large\nscopes. This refines the Scope view of [FR-048](./FR-048-session-work-monitor.md)\n(AC-7, AC-9) and reads the shared model adapted by\n[FR-059](./FR-059-department-monitor-adapter.md). It adds no model, renderer,\nmetric or readiness calculation. Tickets remain drill-down detail; they never\ndefine focus."}
  - {id: "FR-060", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "- With no reported scope, Scope SHALL label the whole-team fallback plainly as\n  not being the lead's focus. It SHALL NOT infer focus from open tickets,\n  assignments or activity, and the later rules apply to it unchanged. In that\n  fallback, initiative groups with no displayed project and no open work SHALL\n  NOT be rendered as headings. A reported initiative is always rendered."}
  - {id: "FR-060", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "- By default Scope SHALL hide a project whose authoritative tracker status is\n  completed or canceled. It SHALL retain projects with any other or unknown\n  status, including an active project whose current tasks are all done. It\n  SHALL NOT decide visibility from done/total counts."}
  - {id: "FR-060", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "- [FR-048](./FR-048-session-work-monitor.md), the Scope view and scope selection\n  this requirement constrains; [FR-059](./FR-059-department-monitor-adapter.md),\n  shared-model adaptation and source lifecycle; ix-board, which owns the scoped\n  model, relationships, status and progress."}
  - {id: "FR-060-AC-1", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "A shared-model fixture with initiatives, projects, a project under two initiatives and a project with no selected initiative renders each project beneath its tracker-related initiative, marks the shared project in each place, and shows the other project in a labeled project-only group. Every row shows the model's counts. No total is added, and the shared project adds nothing twice."}
  - {id: "FR-060-AC-2", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "With no reported scope, Scope shows a plain whole-team-fallback label and does not call the rows the lead's focus. Open tickets and assignments never create or change the focus. Initiative groups with no displayed project and no open work are not rendered; a reported initiative with no displayed project is."}
  - {id: "FR-060-AC-3", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "Fixtures with active, completed, canceled and unknown-status projects, and an active project with all current tasks done, hide only the completed and canceled projects by default. The toggle key, current mode and hidden count are visible in the footer or header, and toggling shows then hides the same rows without any write."}
  - {id: "FR-060-AC-4", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "With completed projects hidden and then shown, every remaining row and every initiative row has identical shared-model progress, and the view states that hidden projects are included. Hiding creates no new total or completion value. Selection and scroll clamp after the toggle and after a refresh that shrinks the rows."}
  - {id: "FR-060-AC-5", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "A representative large-scope fixture at least as large as the observed QSL case (4 initiatives, 34 projects) with many tickets is exercised by an optimized (release-profile) build or test run. Repeated scroll, selection, view-switch and quit events complete without waiting for the source or rebuilding the hierarchy; an instrumented counter shows no hierarchy rebuild per event. Before and after results for the same scope are recorded in the pull request, and the debug-preview CPU observation is not accepted as the measurement. No fixed time limit is set."}
  - {id: "FR-060-AC-6", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "Changing the source model, saved report, reported scope or toggle updates the displayed rows. Missing, stale and error fixtures remain visible, and the last successful snapshot is retained after a failed refresh. Existing FR-048-AC-1 binding, FR-059 replacement-fence, quit and terminal-restoration fixtures still pass unchanged."}
  - {id: "FR-048-AC-7", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: "The p key selects a scrollable Scope view with a blank row below Focus, an Initiative/project column heading, no redundant table title, and one terminal row per scoped initiative and project, prefixed by 🎯 and 📁 respectively, with no extra type or repeated count-label row, showing completed/total leaf tasks, remaining tasks, ready tasks, and unfinished tasks with unfinished or unknown prerequisites. Details show project tracker status and remaining workflow stages. Initiative counts are restricted to scope; overlapping groups are not summed. The t key restores Tickets."}
  - {id: "FR-048-AC-9", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: "Scope defaults to the bound workspace team key, with an explicit --team override. Initiative, project and ticket selectors narrow that team boundary. Foreign-team tasks never inflate counts; unprojected tasks belong to whole-team scope. Project focus does not display associated initiatives as initiative focus. Explicit --all-team clears narrower selectors without removing agent reports; the command refuses conflicting or absent selections."}
  - {id: "FR-059-AC-4", path: "spec/functional/FR-059-department-monitor-adapter.md", role: context_only, excerpt: "TestBackend fixtures at 80x24 and 120x40 switch views with 1/2/3, retain scope, reset selection/scroll, reach the final row and clamp selection after shorter data. Details expose evidence and owner/action; long text and control characters cannot hide diagnostics or inject terminal controls. Existing compact Tickets/Agents/Scope fixtures and their keys retain their outputs."}
  - {id: "FR-059-AC-7", path: "spec/functional/FR-059-department-monitor-adapter.md", role: context_only, excerpt: "Bound-session replacement preserves the explicit restart requirement and prevents replacement activity/report attribution while department data remains navigable. Existing FR-048 reporting isolation and Tickets/Agents/Scope behavior remain unchanged; department reads produce no work-report, registration, tracker or retro-store write."}
```

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-004 | low | Footer must always show mode 'hiding completed' although with no status category nothing is hidden | spec/functional/FR-060-focused-scope-presentation.md:123 |
| FND-005 | low | AC-8 'exactly one per input change' on a combined summary-and-hierarchy counter is ambiguous for the toggle | spec/functional/FR-060-focused-scope-presentation.md:179 |

## Dispositions

Round 1 against frozen head 38323b9676bd067826bbb8f9cb2a0b3c9d5aca9e. Original findings above are unchanged. FND-004 and FND-005 are new this round and have no outcome yet.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed | Hidden count unit is distinct projects, not placements (FR-060 lines 131-133); FR-060-AC-5 asserts a project hidden under two initiatives counts once. |
| FND-002 | fixed | Key `c` is named, collision-checked against the existing key list, Scope-view only, process-lifetime and unpersisted (lines 123-130); FR-060-AC-4 asserts all existing keys still act. |
| FND-003 | fixed | Deterministic order is stated (lines 85-89) and FR-060-AC-1 asserts it, including stability across hide and show (AC-5). |

## Dispositions (round 2)

Round 2 against frozen head 7771d0b8ba02c947ea09fe53c9f9ad77f39ed5df. Findings and round 1 rows above are unchanged. FND-001, FND-002 and FND-003 stay fixed (latest row, round 1) and were re-checked against the current text with no regression.

| FND | outcome | reason |
|---|---|---|
| FND-004 | fixed | The Completed projects section now requires the footer to show 'completed filter unavailable' and never 'hiding completed' while the status category is unavailable, with the mode flag and key retained for when it becomes available. FR-060-AC-6 asserts the footer text, that `c` changes only the retained flag without altering rows, and that the flag applies once a fixture supplies the category. |
| FND-005 | fixed | The Responsiveness section separates the shared summary (invalidated only by snapshot, report or scope) from the projection (summary or toggle), and says the toggle SHALL NOT recompute an unchanged summary. FR-060-AC-8 uses two counters with exact deltas: nothing for scroll, selection, view-switch, quit and idle draws; one projection build and no summary computation per `c`; one of each per snapshot, report or scope change. |
