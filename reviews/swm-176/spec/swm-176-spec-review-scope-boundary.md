---
id: SR-093
title: "Scope, boundary and dependency analysis - FR-060 (SWM-176)"
type: SpecReview
analysis: scope-boundary
scope: "agent-ix/ix-relay; spec/functional/FR-060-focused-scope-presentation.md; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md"
review_set: subset
---

## Summary

Ticket: SWM-176. PR ix-relay#39 at frozen 11c226b3a71e94741e62b45cce0fcc41eb0e2580. Boundary and dependency analysis of FR-060 against FR-048, FR-059 and ix-board ownership. A separate dependency-analysis artifact is not produced: the only new relationship edges (constrains FR-048, depends_on FR-059) were checked here and are correct.

## Verdict

**CONDITIONAL** - one medium ownership and dependency gap. Boundaries otherwise hold: FR-060 adds no model, renderer, metric or readiness arithmetic, and planner publication, follower adoption, credentials, dependency changes and trace repairs are excluded, matching the SWM-176 not-in-scope list.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | Ownership of completed/canceled classification and model-availability blocker not allocated | spec/functional/FR-060-focused-scope-presentation.md:91 |

## Review Context

Reviewer model claude-sonnet-5-5; run id unavailable from this harness. Linear marker id=SR-093 method=spec-review/scope-boundary lang=spec. Source read only for evidence (crates/relay-tui/src/monitor_tracker.rs, monitor_department.rs); nothing built or run. The pinned ix-board rev is in Cargo.toml and its model source was not fetched or built, so what it supplies for status category is unverified here. No AssuranceProfile applies.

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

## Dispositions

Round 1 against frozen head 38323b9676bd067826bbb8f9cb2a0b3c9d5aca9e. The original finding above is unchanged.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed | Classification section assigns the completed/canceled decision to the ix-board status category only, forbids display-string, name-list and done/total rules, records that the pinned 8381a601 does not supply the category, and declares a shared-model release plus relay dependency update as explicit prerequisites. Until then every project is unknown, nothing is hidden and the filter is labeled unavailable (AC-6). No API is invented and no API implementation is demanded of this spec; other behavior does not wait on it. The root-verified public model evidence agrees (Project.status is a string, no public category). |

## Round 2 check

Round 2 against frozen head 7771d0b8ba02c947ea09fe53c9f9ad77f39ed5df. Every finding in this file has a latest outcome of fixed (round 1). The round 2 fix text was re-read for regressions against this file's findings and none was found, so no new Dispositions rows are added. Original findings and round 1 rows are unchanged.
