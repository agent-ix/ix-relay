---
id: FR-064
title: "Present a session's named active focuses in the existing dashboard"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-060"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-059"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-061"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-062"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-063"
    type: "depends_on"
  - target: "ix://agent-ix/ix-board/FR-012"
    type: "depends_on"
---
# FR-064: Present a session's named active focuses in the existing dashboard

## Description

The existing `team-leader-dashboard` Scope view SHALL present a bound session's
named active focuses ([FR-062](./FR-062-session-active-focuses-and-adoption.md))
when the session is in named mode. It presents the result computed by ix-board
([FR-063](./FR-063-shared-focus-membership-and-summaries.md)) and adds no model,
calculation, classification or renderer. A user may view one focus alone or all
active focuses combined. Browsing never writes: the dashboard publishes no active
work and changes no definition, adoption or report.

When the session is not in named mode, the Scope view is the unchanged
[FR-060](./FR-060-focused-scope-presentation.md) legacy presentation, including
its whole-team fallback label and its first-delivery hierarchy, completed filter
and responsiveness behavior.

Named focus does not redefine the department views of
[FR-059](./FR-059-department-monitor-adapter.md) or their metrics: they never read
a named focus, apply no named filtering and are unchanged in both modes.

## Inputs

- The session's named state: mode, active focuses, adopted snapshots, latest
  revision, owner state, archived or removed state, read from relay records.
- The ix-board result for the adopted selections, its snapshot fetch time and
  source errors.
- The session's legacy scope, free-text focus and assignments, read as today.
- Terminal input events and source refreshes.

## Behavior

### Mode and source states

The dashboard reads the active record of the incarnation it is bound to
(FR-048) and never another.

- **Legacy mode**: the record has the mode `legacy`, or no record has ever been
  seen by this dashboard process. The Scope view follows FR-060 unchanged.
- **Named, active**: one or more active focuses. Rows come only from the adopted
  snapshots.
- **Named, none**: named mode with zero active focuses. The view shows "No active
  focus", no rows and no totals. It never shows the whole team and never shows the
  legacy scope's rows.
- **Named, unavailable**: the named record cannot be read or decoded. With no
  earlier result the view shows unavailable and the reason. After an earlier
  result it keeps the last good rows labeled last snapshot with the error. It
  SHALL NOT fall back to the legacy scope or the whole team, because absence of
  evidence is not "no named mode".
- **Named, record gone**: this dashboard process has read a `named` record for its
  bound incarnation and the record is now absent (superseded by a replacement
  process or reclaimed). The view keeps the last good rows labeled "named record
  removed" and does not fall back to the legacy scope or the whole team. A
  dashboard that never read a named record and finds none is in legacy mode.
- Unknown members of a stored record are ignored (FR-061 tolerance); an
  unsupported embedded selection version is shown as that focus unavailable.
- Each focus shows its state: current; update pending (adopted N, latest M);
  archived; unavailable (last adopted, currentness unknown); or removed. The
  state of the Linear snapshot (missing, stale, failed, last good) and any
  unresolved members are shown and are never rendered as healthy or zero.
- The owner state (live, absent, replaced, unknown) is shown in details only.
- Reads of the named record, the Linear snapshot and Herdr refresh independently
  on the existing intervals. Named records are read with the Redis report read
  and on manual refresh.

### Rows and navigation

- A named focus is a **heading row** with a distinct prefix (the bookmark glyph
  🔖), its name at the adopted revision, its team, its revision and its state. A
  heading is a library entry; it is not shown as a Linear initiative or project.
  A focus is never nested in another.
- Beneath a heading, each included initiative is a 🎯 row with its related
  projects as indented 📁 rows, and ticket inclusions appear as direct-ticket rows
  grouped under a "Direct tickets" row. A project not under a displayed
  initiative, the shared mark, the labels "(no selected initiative)" and
  "(initiative links unavailable)", ordering, and the omission rules follow FR-060
  inside each heading. An included initiative is always rendered, even when every
  project beneath it is hidden or excluded.
- The partial mark (with its reason) and the shared mark come from the shared
  result, on initiative rows and project rows alike. A project excluded by the
  focus is not shown. An empty focus shows its heading with "empty focus: nothing
  selected".
- Active focuses of different teams are separate groups, each with its team label.
  No row or total spans teams.
- The key `f` selects the viewed scope: combined (all active focuses), then each
  focus in order, then combined again. The choice is held only in the dashboard
  process, by focus ID, and is never stored. The scope line names the selection,
  the revision and state of each focus shown, and the source state. If the viewed
  focus leaves the active set, the choice returns to combined. If the active set
  becomes empty, the choice is cleared and the view shows "No active focus". A
  viewed focus that becomes archived, unavailable or removed stays selected and
  shows its last adopted snapshot, labeled with that state; it never shows an
  empty or current result.
- The combined view shows each focus's heading and rows and, per team group, one
  **union** line of done and total over the deduplicated leaves. Per-focus counts
  are never added; the union line is labeled as a union. Shared placements are
  marked. The individual view shows only that focus.
- Enter on a direct-ticket row, an initiative row or a project row opens the existing
  details; direct tickets drill down to their tickets and descendants.
- The Tickets view uses the leaves of the viewed selection as its scope. The
  Agents view keeps FR-048-AC-6: it lists every reported assignment. The
  department views (`1`, `2`, `3`) are unchanged and read no named focus. The
  existing `t`, `a`, `p`, `1`, `2`, `3`, `c`, `s`, `e`, `d`, `r`, `q` and `Esc` keys
  are unchanged.
- The session's free-text focus is labeled **Now** in the named view (FR-048
  free-text focus). A legacy reported scope that exists is shown in details as
  "legacy reported scope, not in use while named focuses are set", with its
  selector counts. Assignments and agents are unchanged.

### Responsiveness, cache and refresh

- Scroll, selection, view switching, `f`, `c` and quit SHALL NOT wait on a source
  read or a computation. A change of the active set, an adoption, a pending
  revision or a snapshot starts the recomputation off the input path; the rows
  already shown stay, labeled updating, until the new result is ready; the newest
  request wins and an older one is dropped.
- As in FR-060, the **shared summary** is invalidated only by a snapshot change,
  an adopted-snapshot or active-set change; the **projection** by a summary change,
  a toggle of `c`, or `f`. `f` selects within the one summary the shared library
  returns for all focuses and the combination, so it adds one projection build and
  no summary computation.
- The cache holds at most the current and the last good summary and their
  projections. It does not grow with navigation, and it is cleared when the dashboard
  exits. Its key is the focus IDs, adopted revisions and the snapshot fetch time.
- The completed-project behavior of FR-060 applies to named mode: the `c` toggle,
  the "completed filter unavailable" state while the status category is absent, and
  the hidden-project count in distinct projects. This waits on the same category as
  FR-060 and adds no other prerequisite.
- The relay SHALL NOT truncate rows of a large scope. The performance scenario of
  FR-060 (at least 4 initiatives, 34 projects, 2,000 leaves, a pending source read,
  at least 200 events, counters and recorded before and after results) is retained
  and repeated in named mode with at least three active focuses sharing a project.

### Reconciliation with FR-060

FR-060 is not otherwise edited. For the Scope view in named mode only, this
requirement takes precedence over the following statements of FR-060, and only to
the extent listed. In legacy mode nothing below applies.

- "Focus modes" (Focused, Whole-team fallback, Ticket-only): these three modes are
  determined only from a legacy reported scope. Named mode has its own states
  above. The whole-team fallback label is never used in named mode, and no named
  state is displayed as a fallback.
- "Tickets never define focus": this keeps its meaning that open tickets,
  assignments and activity never create focus. A ticket that the adopted
  definition explicitly includes is part of that definition and is shown as a direct
  ticket; it is not displayed with the legacy label "no initiative/project focus
  reported" and does not narrow a whole-team result.
- "FR-060 adds no heading, group or separator rows": named mode adds the focus
  heading, the "Direct tickets" row and the union line described above, and no
  other.
- "Hierarchy" and the completed, source-state and responsiveness sections apply
  inside each heading as described.
- A project that an exclusion removes is not shown and is not counted as hidden
  completed.

Unknown, stale and last-good labels, and the guarantee that unknown is retained and
never healthy, are not weakened.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-064-AC-1 | A session with no named record shows Scope view output identical to the FR-060 fixtures, including whole-team fallback and ticket-only labels, with the completed filter and the performance counters passing unchanged. | Test |
| FR-064-AC-2 | Named mode with zero active focuses shows "No active focus", no rows and no totals, with a legacy scope present and with none; an active focus whose definition is empty shows its heading and "empty focus"; neither shows the whole team. | Test |
| FR-064-AC-3 | An unreadable or undecodable named record shows unavailable and the reason when no earlier result exists, and keeps the last good rows labeled last snapshot with the error after an earlier result; neither falls back to legacy scope or whole team. | Test |
| FR-064-AC-4 | A named record that has been read by this dashboard and is later absent keeps the last good rows labeled "named record removed" with no fallback to legacy scope or whole team; a record with mode `legacy` and a dashboard that never saw a named record show the FR-060 legacy view. | Test |
| FR-064-AC-5 | A named record with an unknown member is displayed normally; a focus whose embedded selection version is unsupported shows as unavailable while other focuses still show. | Test |
| FR-064-AC-6 | Two active focuses with an initiative, its projects, an excluded project and direct tickets show rows in the order heading, initiative, indented projects, then "Direct tickets"; the excluded project is absent; shared and partial marks and FR-060 labels appear on initiative and project rows; order is stable on repeat; the heading carries no Linear initiative or project count semantics. | Test |
| FR-064-AC-7 | Three focuses with a shared project in one team plus one focus of a second team show in the combined view each heading with its own counts, one labeled union line per team with each leaf once, the shared mark on every placement, no summed total and no cross-team total; `f` cycles to each focus alone and back to combined. | Test |
| FR-064-AC-8 | The viewed focus being deactivated returns the choice to combined; the active set becoming empty clears the choice and shows "No active focus"; the viewed focus becoming archived, unavailable or removed stays selected and shows its last adopted snapshot labeled with that state. | Test |
| FR-064-AC-9 | Focus F adopted at 2 with latest 3 shows update pending (adopted 2, latest 3) with rows from revision 2; archived, unavailable (last adopted, currentness unknown), removed and renamed states show distinctly; owner states show in details and never as live without evidence. | Test |
| FR-064-AC-10 | A missing, stale or failed Linear snapshot or an unresolved member shows its label with counts unavailable or partial, never healthy or zero, and the last good result and its fetch time are retained after a failed refresh. | Test |
| FR-064-AC-11 | A ticket-only focus shows its tickets as direct tickets with their project partial and drilling down on Enter; the legacy "no initiative/project focus reported" label does not appear; open tickets and assignments never create rows. | Test |
| FR-064-AC-12 | In named mode the free-text focus is labeled "Now", a legacy scope is shown in details as not in use, assignments and agents are unchanged, and `t`, `a`, `p`, `1`, `2`, `3`, `c`, `s`, `e`, `d`, `r`, `q`, `Esc`, paging and arrows keep their actions. | Test |
| FR-064-AC-13 | The department views `1`, `2` and `3` render byte-identical output for the same department fixtures in legacy mode and in named mode, and read no named record; the Agents view lists every reported assignment in named mode. | Test |
| FR-064-AC-14 | Over a fixture of at least 4 initiatives, 34 projects, 2,000 leaves and three focuses sharing a project, with a source read pending, at least 200 scroll, selection, view, `f` and `c` events, followed by quit, are processed without waiting on the source, and quit returns promptly. | Test |
| FR-064-AC-15 | In that fixture the counters show that scroll, view, quit and idle draws change neither the summary nor the projection, that `f` and `c` each add one projection build and no summary computation, and that one snapshot change or one active-set change adds one summary and one projection. | Test |
| FR-064-AC-16 | In that fixture the cache holds at most the current and the last good summary and their projections after 200 navigation events and is empty after the dashboard exits. | Test |
| FR-064-AC-17 | The pull request records the before and after results of the named-mode performance fixture next to the FR-060 results for the same fixture. | Demonstration |
| FR-064-AC-18 | Navigation, `f`, `c`, view changes and quit leave every relay record byte-identical and make no Linear write. | Test |
| FR-064-AC-19 | On a relay with two leads owning Completion and Hardening focuses where one lead's session activates only Completion, that session's dashboard shows Completion only, the other lead's dashboard is unaffected, and an owner edit shows update pending until adopted. | Test |
| FR-064-AC-20 | The two-lead scenario of FR-064-AC-19 runs in the visible Herdr panes, with one lead subscribing to one focus, an owner edit, a session restart and a source outage, and is recorded in the pull request. | Demonstration |

## Dependencies

- [FR-061](./FR-061-named-focus-definitions.md) supplies the owner state and
  definition tolerance, and [FR-062](./FR-062-session-active-focuses-and-adoption.md)
  supplies the named state. [FR-063](./FR-063-shared-focus-membership-and-summaries.md)
  supplies membership, exclusions, deduplication, marks and source failure.
  [FR-060](./FR-060-focused-scope-presentation.md) and
  [FR-048](./FR-048-session-work-monitor.md) are retained as stated;
  [FR-059](./FR-059-department-monitor-adapter.md) departments and explicit-scope
  startup have no named mode and are unchanged.
- [FR-065](./FR-065-dashboard-view-selector-and-help.md) adds the View selector, F1
  help, footer and population labels, and lists the presentation differences it
  allows in the FR-064 statements and fixtures that say "unchanged" or "identical".
  Keys and data are unchanged. When this requirement delivers the `f` key, it also
  adds `f` to the FR-065 Help table (Scope view, named mode only) and to the FR-065
  popup key sweep, and verifies both here; FR-065 does not wait on it.
- Out of scope: department metrics and monitoring policy, a second tracker reader,
  speeding the full-team snapshot, and nested focuses.
