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
scopes. It reads the shared model adapted by
[FR-059](./FR-059-department-monitor-adapter.md) and adds no model, renderer,
metric, classification or readiness calculation. Tickets remain drill-down
detail; they never define focus.

### Precedence

This requirement refines the Scope view of
[FR-048](./FR-048-session-work-monitor.md) (AC-7, AC-9). FR-048 and FR-059 are
not edited. For the Scope view only, FR-060 takes precedence over the following
statements, and only to the extent listed:

- FR-048-AC-7 "one terminal row per scoped initiative and project": a project
  has one row per displayed parent placement (see Hierarchy), and completed or
  canceled projects are hidden by default.
- FR-059-AC-4 and FR-059-AC-7 "existing Scope fixtures retain their outputs":
  Scope fixtures change only for the rows, labels and key above.

Everything else in FR-048-AC-7 is retained: the `p` and `t` keys, the blank row
below Focus, the Initiative/project column heading, no table title, the 🎯 and 📁
prefixes, no extra type or count-label row, the listed count columns, and the
details content. The Tickets and Agents views, the department views and the
existing keys are unchanged. FR-060 adds no heading, group or separator rows.

## Inputs

- The lead's reported scope (initiative, project, ticket and team selectors)
  and the shared ix-board scoped result for it.
- From that result: per-row progress, the project-to-initiative relationships,
  and the project status category (see Classification).
- Terminal input events and source, report or scope refreshes.

## Behavior

### Focus modes

The three modes are mutually exclusive and are selected only from the reported
scope:

- **Focused**: the report names at least one initiative or project. The scope
  line says the rows are the lead's reported focus.
- **Whole-team fallback**: the report names no initiative, project or ticket,
  or no report exists. The scope line says plainly that no focus is reported and
  the rows are the whole team. It never says or implies lead focus.
- **Ticket-only**: the report names tickets but no initiative or project. It uses
  the fallback label ("no initiative/project focus reported"), and the tickets
  narrow the shared result as FR-048-AC-9 states.

Focus is never inferred from open tickets, assignments or activity.

### Hierarchy

- In Focused mode each reported initiative is a 🎯 row with its related projects
  as 📁 rows indented beneath it, using the relationships carried by the shared
  model. Parent matching uses the initiative's stable tracker id where the model
  supplies it. Matching by name is allowed only when the name is unique among the
  initiatives in the result; otherwise the project's links are treated as
  unavailable.
- A project related to more than one displayed initiative has one row under each
  and carries a "shared" marker on every placement. Its progress is the single
  shared-model value for that project, and no total is formed over rows.
- After all initiative rows, each remaining project is a top-level 📁 row with an
  inline label in its name cell:
  - "(no selected initiative)" when the model reports its initiative links
    complete and none is displayed;
  - "(initiative links unavailable)" when the model reports the links missing,
    partial or ambiguous. Such a project is never labeled unassigned.
- Order is deterministic: initiatives and projects follow the shared-model result
  order. Where the model supplies none, they sort by name (byte order) then
  tracker id. A project under an initiative keeps its relative model order.
  Hiding or showing completed projects removes or adds rows without reordering
  the others.
- In Whole-team fallback and Ticket-only modes, an initiative with no displayed
  project row is omitted. A reported initiative in Focused mode is always rendered,
  including when every project beneath it is hidden.
- When Focused mode names projects only and all are hidden, the Scope view has no
  rows and shows a message line that names the number hidden and the toggle key.

### Classification

- The completed/canceled decision SHALL come only from the project status
  category supplied by the shared model; ix-board owns it. The view SHALL NOT
  decide it from the status display string, from done/total counts or from any
  local list of status names, because tracker status names can be renamed or
  added.
- The pinned ix-board revision (`8381a601`) does not supply that category.
  `Project.status` is a string: `Started` or `Paused`, otherwise the tracker's
  display name (ix-board `src/model.rs:152-153`; `src/linear.rs:1361-1369`).
  The category enum (`LinearProjectStatusType`, `src/linear.rs:1610-1617`) is
  private to ix-board and is discarded for statuses other than Started. Relay
  carries only the string (`ScopeProgress.status: Option<String>`,
  `crates/relay-tui/src/monitor_tracker.rs:19-20`).
- Therefore this requirement depends on a shared-model release that exposes the
  project status category and a relay dependency update to it, both outside this
  specification change. Until the category is available the view SHALL treat the
  classification of every project as unknown: it hides none, shows
  "completed filter unavailable: status category not supplied" in the scope line,
  and does not offer a hidden count.
- A project with an unrecognized or absent category is unknown and is retained.

### Completed projects

- When the category is available, the view SHALL by default hide each project
  whose category is completed or canceled, and retain every other and unknown
  project, including an active project whose current tasks are all done.
- The key `c` (no modifier) toggles showing completed and canceled projects in
  the Scope view. It collides with no existing monitor key (`q`, `Esc`, `r`, `d`,
  `Enter`, `s`, `e`, `t`, `a`, `p`, `1`, `2`, `3`, `j`, `k`, arrows, `PageUp`,
  `PageDown`). It has no effect in other views. The footer in the Scope view
  names the key and the current mode ("hiding completed" or "showing completed").
  While the status category is unavailable (see Classification) nothing is
  hidden, so the footer SHALL instead show "completed filter unavailable" and
  SHALL NOT show "hiding completed". The process mode flag and the key remain
  valid and take effect when the category becomes available.
- The mode is held for the life of the dashboard process, starts as hidden, and
  survives view changes and refreshes. It is not written to any report, tracker,
  configuration or file.
- The scope line states the number of hidden projects. The unit is distinct
  projects, not placements: a shared project hidden under two initiatives counts
  once. When initiatives are omitted, it states that number too.
- Hiding changes no shared-model progress. The view states that initiative
  progress includes the hidden projects whenever any are hidden, and forms no
  new total or completion value.
- After a toggle or refresh changes the rows, selection stays on the same
  project placement when it remains and otherwise clamps; scroll clamps.

### Source states

- With no model yet, the Scope view shows unavailable and the source reason, no
  rows, and no hidden count (never zero). The toggle key remains accepted and its
  mode is retained.
- After a failed refresh the last successful snapshot, its rows and its hidden
  count are retained with the existing last-snapshot and error labels. A stale
  source labels the count stale in the same way as the rows.
- Missing, stale and error states remain visible as FR-048 and FR-059 require.

### Responsiveness and refresh

- Scroll, selection, view switching, the toggle and quit SHALL NOT wait on source
  collection.
- Two results are distinguished. The **shared summary** is the ix-board scoped
  result computed from the snapshot, report and team. The **projection** is the
  hierarchy, labels, order and visibility derived from the summary and the toggle.
  The shared summary is invalidated only by a change to the snapshot, the report
  or the scope; the projection is invalidated by a summary change or a toggle
  change. Scroll, selection, view switches, quit and idle draws invalidate neither.
  The toggle SHALL NOT recompute an unchanged shared summary.
- The existing frame render computes the shared summary on every draw. An
  implementation may reuse the shared result between draws and SHALL NOT alter the
  shared formulas to do so.
- A change to the source, saved report, reported scope or toggle SHALL refresh the
  displayed rows and counts.
- The live QSL observation (about 2,500 leaf tasks across 34 projects and four
  initiatives) is a reference size, not a limit: the view SHALL NOT truncate or cap
  rows or projects. A debug build's CPU use (95.5% observed) is not the release
  baseline; an optimized full-scope run was observed at 52.9% CPU and is recorded
  as a before measurement, not as a threshold.
- Native session, process and replacement binding checks, existing view keys,
  responsive quit and terminal restoration SHALL remain unchanged.

## Acceptance Criteria

| ID | Criteria | Verification |
|---|---|---|
| FR-060-AC-1 | A shared-model fixture with two initiatives, a project under both, a project under one, and a project with complete links and no displayed initiative renders initiative rows with their projects nested and in model order. The shared project has a row and a shared marker under each, and the other project is labeled "(no selected initiative)" after the initiatives. Every row shows the model's counts, no total is formed, and no heading, group or separator row appears. Retained FR-048-AC-7 items (blank row below Focus, column heading, 🎯/📁 prefixes, no title, `t` returns to Tickets) are asserted. The same input renders the same order on repeat. | Test |
| FR-060-AC-2 | A fixture with missing, partial and ambiguous initiative links, and one with duplicate initiative names, labels the affected projects "(initiative links unavailable)" and never "(no selected initiative)" or unassigned. Parent matching uses the stable id where supplied. | Test |
| FR-060-AC-3 | Three reported-scope fixtures select exactly one mode each: initiative/project named is Focused, nothing named is Whole-team fallback with its plain no-focus label, and tickets only is Ticket-only with the fallback label. Open tickets and assignments never create focus. In fallback, an initiative with no displayed project is omitted; in Focused mode a reported initiative whose projects are all hidden is rendered, and a projects-only focus with all hidden shows no rows plus the message line with the count and key. | Test |
| FR-060-AC-4 | With a fixture whose model supplies status categories for active, completed, canceled, unknown and active-with-all-tasks-done projects, only completed and canceled are hidden by default. Key `c` toggles and re-toggles to the same rows with no write, the footer shows the key and mode in the Scope view only, mode survives view changes and refresh, and every existing key (`q`, `Esc`, `r`, `d`, `Enter`, `s`, `e`, `t`, `a`, `p`, `1`, `2`, `3`, `j`, `k`, arrows, paging) still performs its prior action. Selection stays on the same placement or clamps after a toggle or a shrinking refresh. | Test |
| FR-060-AC-5 | A project hidden under two initiatives is counted once in the hidden-project count. Every remaining row and initiative has the same shared-model progress with completed projects hidden and shown, the view says initiative progress includes hidden projects, and no new total appears. Hiding or showing preserves the order of the other rows. | Test |
| FR-060-AC-6 | With a fixture that lacks the status category, as the pinned ix-board `8381a601` does, no project is hidden even when its display name is "Completed" or "Canceled", the scope line says the completed filter is unavailable, no hidden count is offered, and the footer shows "completed filter unavailable" and never "hiding completed". Pressing `c` in that state changes the retained mode flag without hiding or showing rows, and the flag applies once a fixture supplies the category. A project with an unrecognized category is retained. No local status-name or done/total rule exists. | Test and Inspection |
| FR-060-AC-7 | With no model, the Scope view shows unavailable with the source reason, no rows and no zero count, and `c` keeps its mode. After a successful model then a failed refresh, the rows and hidden count of the last snapshot remain with the last-snapshot and error labels, and a stale model labels the count stale. | Test |
| FR-060-AC-8 | A bounded optimized-build scenario uses a fixture with at least 4 initiatives, 34 projects, one shared project and 2,000 leaf tasks, with no truncation of rows. While a source read is held pending, a scripted sequence of at least 200 scroll, selection, view-switch and `c` events and then quit is processed without waiting for that read. Two counters, one for shared-summary computation and one for projection build, show these exact deltas: scroll, selection, view-switch, quit and idle draws change neither; one `c` toggle adds one projection build and no summary computation; one snapshot, report or scope change adds one summary computation and one projection build. Before (current main) and after results for the same fixture, including wall time for the sequence and CPU, are recorded in the pull request; the debug-build observation is not used and no fixed time limit applies. | Test and Demonstration |
| FR-060-AC-9 | Changing the source model, saved report, reported scope or toggle updates the displayed rows and counts. Existing FR-048-AC-1 binding, FR-059 replacement-fence, quit and terminal-restoration fixtures still pass unchanged. | Test |

## Dependencies

- [FR-048](./FR-048-session-work-monitor.md), the Scope view and scope selection
  this requirement constrains; [FR-059](./FR-059-department-monitor-adapter.md),
  shared-model adaptation and source lifecycle.
- ix-board owns the scoped result, the relationships and their completeness, row
  order, progress, and the project status category. The category is not in the
  pinned revision `8381a601`. A shared-model release that exposes it, and a relay
  dependency update, are prerequisites for the hide behavior and are not part of
  this specification change. Other behavior in this requirement does not wait on
  them.
- Out of scope: shared planner publication and follower adoption, new metrics or
  readiness arithmetic, a second model or renderer, credential or token access,
  dependency changes in this change, and trace repairs for existing requirements.
