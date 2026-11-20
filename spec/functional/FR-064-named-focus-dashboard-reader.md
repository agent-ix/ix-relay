---
id: FR-064
title: "Present a session's named active focuses in the existing dashboard"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-060"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-062"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-063"
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

## Inputs

- The session's named state: mode, active focuses, adopted snapshots, latest
  revision, owner state, archived or removed state, read from relay records.
- The ix-board result for the adopted selections, its snapshot fetch time and
  source errors.
- The session's legacy scope, free-text focus and assignments, read as today.
- Terminal input events and source refreshes.

## Behavior

### Mode and source states

- **Legacy mode**: the session has no named record. The Scope view follows FR-060
  unchanged.
- **Named, active**: one or more active focuses. Rows come only from the adopted
  snapshots.
- **Named, none**: named mode with zero active focuses. The view shows "No active
  focus", no rows and no totals. It never shows the whole team and never shows the
  legacy scope's rows.
- **Named, unavailable**: the named record cannot be read, decoded or has an
  unknown field. With no earlier result the view shows unavailable and the reason.
  After an earlier result it keeps the last good rows labeled last snapshot with
  the error. It SHALL NOT fall back to the legacy scope or the whole team, because
  absence of evidence is not "no named mode".
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
  result. A project excluded by the focus is not shown. An empty focus shows its
  heading with "empty focus: nothing selected".
- Active focuses of different teams are separate groups, each with its team label.
  No row or total spans teams.
- The key `f` selects the viewed scope: combined (all active focuses), then each
  focus in order, then combined again. The choice is held only in the dashboard
  process and is never stored. The scope line names the selection, the revision
  and state of each focus shown, and the source state.
- The combined view shows each focus's heading and rows and, per team group, one
  **union** line of done and total over the deduplicated leaves. Per-focus counts
  are never added; the union line is labeled as a union. Shared placements are
  marked. The individual view shows only that focus.
- Enter on a direct-ticket row, an initiative row or a project row opens the existing
  details; direct tickets drill down to their tickets and descendants. The Tickets
  and Agents views and the department views keep their keys and behavior and
  restrict their scope to the selected view's leaves in named mode; the existing
  `t`, `a`, `p`, `1`, `2`, `3`, `c`, `s`, `e`, `d`, `r`, `q` and `Esc` keys are
  unchanged.
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
  a toggle of `c`, or `f`. `f` selects within the one summary the shared interface
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
| FR-064-AC-1 | Given a session with no named record, the Scope view output is identical to the FR-060 fixtures, including whole-team fallback and ticket-only labels; the completed filter and the performance counters pass unchanged. | Test |
| FR-064-AC-2 | Given named mode with zero active focuses, the view shows "No active focus", no rows and no totals, with a legacy scope present and with none; given an active focus whose definition is empty, its heading and "empty focus" show. Neither shows the whole team. | Test |
| FR-064-AC-3 | Given an unreadable, undecodable or unknown-field named record, the view with no earlier result shows unavailable and the reason; with an earlier result it keeps the last good rows labeled last snapshot with the error; neither falls back to legacy scope or whole team. | Test |
| FR-064-AC-4 | Given two active focuses with an initiative, its projects, an excluded project and direct tickets, rows are heading, initiative, indented projects, then "Direct tickets"; the excluded project is absent; shared and partial marks and FR-060 labels appear; order is stable on repeat; the heading carries no Linear initiative or project count semantics. | Test |
| FR-064-AC-5 | Given three focuses with a shared project in one team and one focus of a second team, combined view shows each heading with its own counts, one labeled union line per team with each leaf once, the shared mark on every placement, no summed total and no cross-team total; `f` cycles to each focus alone and back to combined. | Test |
| FR-064-AC-6 | Given focus F adopted at 2 with latest 3, the view shows update pending (adopted 2, latest 3) with rows from revision 2; archived, unavailable (last adopted, currentness unknown), removed and renamed states show distinctly; owner states show in details and never as live without evidence. | Test |
| FR-064-AC-7 | Given a missing, stale or failed Linear snapshot or an unresolved member, the labels show and counts are unavailable or partial, never healthy or zero; the last good result and its fetch time are retained after a failed refresh. | Test |
| FR-064-AC-8 | Given a ticket-only focus, tickets appear as direct tickets with their project partial and drill down on Enter; the legacy "no initiative/project focus reported" label does not appear; open tickets and assignments never create rows. | Test |
| FR-064-AC-9 | Given named mode, the free-text focus is labeled "Now", a legacy scope is shown in details as not in use, assignments and agents are unchanged, and `t`, `a`, `p`, `1`, `2`, `3`, `c`, `s`, `e`, `d`, `r`, `q`, `Esc`, paging and arrows retain their actions. | Test |
| FR-064-AC-10 | Given a performance fixture of at least 4 initiatives, 34 projects, 2,000 leaves and three focuses sharing a project, with a source read pending, at least 200 scroll, selection, view, `f` and `c` events then quit are processed without waiting; counters show scroll, view, quit and idle draws change neither summary nor projection, `f` and `c` each add one projection build and no summary computation, and one snapshot or active-set change adds one of each; before and after results for the same fixture are recorded in the pull request and the cache stays bounded. | Test and Demonstration |
| FR-064-AC-11 | Navigation, `f`, `c`, view changes and quit produce no change to any relay record (store byte-identical), and no Linear write. | Test |
| FR-064-AC-12 | Given a relay with two leads owning Completion and Hardening focuses where one lead activates only Completion, that lead's dashboard shows Completion only; the other lead's dashboard is unaffected; an owner edit shows update pending until adopted. | Test and Demonstration |

## Dependencies

- [FR-062](./FR-062-session-active-focuses-and-adoption.md) supplies the named state.
  [FR-063](./FR-063-shared-focus-membership-and-summaries.md) supplies membership,
  exclusions, deduplication, marks and source failure. [FR-060](./FR-060-focused-scope-presentation.md)
  and [FR-048](./FR-048-session-work-monitor.md) are retained as stated;
  [FR-059](./FR-059-department-monitor-adapter.md) departments and explicit-scope
  startup have no named mode.
- Out of scope: department metrics and monitoring policy, a second tracker reader,
  speeding the full-team snapshot, and nested focuses.
