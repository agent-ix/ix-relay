---
id: FR-060
title: Present the lead's focused initiative and project scope
type: FR
relationships:
  - target: ix://agent-ix/ix-relay/FR-048
    type: constrains
  - target: ix://agent-ix/ix-relay/FR-059
    type: depends_on
---
# FR-060: Present the lead's focused initiative and project scope

## Description

The existing `team-leader-dashboard` Scope view SHALL present the lead's
explicitly reported initiative and project selection as a parent-child
hierarchy, hide finished projects by default, and stay responsive on large
scopes. This refines the Scope view of [FR-048](./FR-048-session-work-monitor.md)
(AC-7, AC-9) and reads the shared model adapted by
[FR-059](./FR-059-department-monitor-adapter.md). It adds no model, renderer,
metric or readiness calculation. Tickets remain drill-down detail; they never
define focus.

## Inputs

- The lead's reported scope (initiative, project and team selectors) and the
  shared ix-board scoped model for it, including authoritative tracker
  initiative-to-project relationships and project status.
- Terminal input events and source, report or scope refreshes.

## Behavior

### Focus and hierarchy

- With a reported initiative/project scope, Scope SHALL list each scoped
  initiative with its related projects nested beneath it, using only the
  tracker relationships carried by the shared model.
- A project related to more than one displayed initiative SHALL appear under
  each and be marked shared. A selected project with no displayed parent
  initiative SHALL appear in a labeled project-only group rather than under an
  invented initiative.
- Row progress SHALL be the shared-model value for that initiative or project.
  The view SHALL NOT add a total over initiatives or projects, and shared
  projects SHALL NOT be counted a second time.
- With no reported scope, Scope SHALL label the whole-team fallback plainly as
  not being the lead's focus. It SHALL NOT infer focus from open tickets,
  assignments or activity, and the later rules apply to it unchanged. In that
  fallback, initiative groups with no displayed project and no open work SHALL
  NOT be rendered as headings. A reported initiative is always rendered.

### Completed projects

- By default Scope SHALL hide a project whose authoritative tracker status is
  completed or canceled. It SHALL retain projects with any other or unknown
  status, including an active project whose current tasks are all done. It
  SHALL NOT decide visibility from done/total counts.
- A single key shown in the footer SHALL toggle showing completed and canceled
  projects. The view SHALL state the current mode and the number of hidden
  projects. Toggling SHALL NOT write tracker state, reports or scope.
- Hiding rows SHALL NOT change the shared-model progress of any displayed row
  or initiative, and SHALL NOT produce new totals. The view SHALL say that
  initiative progress includes hidden projects when any are hidden.
- Selection and scroll SHALL be clamped after a toggle or refresh changes the
  row count.

### Responsiveness and refresh

- Scrolling, selection, view switching and quit SHALL NOT wait on source
  collection or on rebuilding the Scope hierarchy for each input event.
  Hierarchy and visibility results SHALL be recomputed only when the shared
  model, reported scope or the completed-project toggle changes.
- A change to the source, saved report or reported scope SHALL refresh the
  displayed rows. Missing, stale and error states SHALL remain visible, with
  the last successful snapshot retained as FR-048 and FR-059 require.
- Native session, process and replacement binding checks, existing view keys,
  responsive quit and terminal restoration SHALL remain unchanged.

## Acceptance Criteria

| ID | Criteria | Verification |
|---|---|---|
| FR-060-AC-1 | A shared-model fixture with initiatives, projects, a project under two initiatives and a project with no selected initiative renders each project beneath its tracker-related initiative, marks the shared project in each place, and shows the other project in a labeled project-only group. Every row shows the model's counts. No total is added, and the shared project adds nothing twice. | Test |
| FR-060-AC-2 | With no reported scope, Scope shows a plain whole-team-fallback label and does not call the rows the lead's focus. Open tickets and assignments never create or change the focus. Initiative groups with no displayed project and no open work are not rendered; a reported initiative with no displayed project is. | Test |
| FR-060-AC-3 | Fixtures with active, completed, canceled and unknown-status projects, and an active project with all current tasks done, hide only the completed and canceled projects by default. The toggle key, current mode and hidden count are visible in the footer or header, and toggling shows then hides the same rows without any write. | Test |
| FR-060-AC-4 | With completed projects hidden and then shown, every remaining row and every initiative row has identical shared-model progress, and the view states that hidden projects are included. Hiding creates no new total or completion value. Selection and scroll clamp after the toggle and after a refresh that shrinks the rows. | Test |
| FR-060-AC-5 | A representative large-scope fixture at least as large as the observed QSL case (4 initiatives, 34 projects) with many tickets is exercised by an optimized (release-profile) build or test run. Repeated scroll, selection, view-switch and quit events complete without waiting for the source or rebuilding the hierarchy; an instrumented counter shows no hierarchy rebuild per event. Before and after results for the same scope are recorded in the pull request, and the debug-preview CPU observation is not accepted as the measurement. No fixed time limit is set. | Test and Demonstration |
| FR-060-AC-6 | Changing the source model, saved report, reported scope or toggle updates the displayed rows. Missing, stale and error fixtures remain visible, and the last successful snapshot is retained after a failed refresh. Existing FR-048-AC-1 binding, FR-059 replacement-fence, quit and terminal-restoration fixtures still pass unchanged. | Test |

## Dependencies

- [FR-048](./FR-048-session-work-monitor.md), the Scope view and scope selection
  this requirement constrains; [FR-059](./FR-059-department-monitor-adapter.md),
  shared-model adaptation and source lifecycle; ix-board, which owns the scoped
  model, relationships, status and progress.
- Out of scope: shared planner publication and follower adoption, new metrics or
  readiness arithmetic, a second model or renderer, credential or token access,
  dependency changes, and trace repairs for existing requirements.
