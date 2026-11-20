---
id: FR-063
title: "Shared ix-board membership, exclusions and partial summaries for named focuses"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-061"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-062"
    type: "depends_on"
---
# FR-063: Shared ix-board membership, exclusions and partial summaries for named focuses

## Description

The membership of a named focus, its exclusions, leaf deduplication, progress,
readiness and the failure state of each source SHALL be computed by the shared
ix-board library from the current workspace snapshot. ix-relay SHALL pass the
adopted selections to it and present the result. ix-relay SHALL NOT contain a
second membership, progress, readiness or classification engine, and SHALL NOT
read Linear by another path to build one.

This requirement states the behavior relay needs from that shared interface and
the boundary between the two. The interface does not exist in the pinned
revision; the section "Required shared interface" lists what is missing.

## Inputs

- One selection per active focus: its team, its inclusion and exclusion members
  (initiatives, projects, milestones with their projects, tickets), taken from the
  session's adopted snapshot ([FR-062](./FR-062-session-active-focuses-and-adoption.md)).
- One ix-board workspace snapshot with its fetch time, and the ix-board error
  classification of a failed read.
- A request for one focus individually or for the combination of all active
  focuses.

## Outputs

- For each focus, ordered rows: initiative rows with their projects, direct-ticket
  rows, and per-row done, total, remaining by stage, ready and blocked counts.
- Marks: **shared** (a row appears in more than one active focus or under more
  than one initiative) and **partial** (the row's counts cover only part of the
  Linear object, with the reason: a milestone-only or ticket-only inclusion, or an
  exclusion).
- The union of leaf tasks for the combined request, per team group, with a single
  count of each leaf.
- Typed reference states for each member: resolved, missing, moved to another team,
  and any ix-board classification of a snapshot that is partial, stale or failed.

## Behavior

### Membership

- An inclusion of an initiative selects the initiative's **current** projects at
  the snapshot. The selector identity (the initiative ID) is frozen in the revision;
  the projects beneath it are recomputed on every snapshot. A project added to or
  removed from the initiative changes membership on the next refresh with no new
  revision.
- An inclusion of a project selects the project's leaf tasks of the focus's team.
  A milestone selects only its leaf tasks, and requires its owning project to match
  the project recorded with it. A ticket selects itself and its descendants, using
  the existing scope semantics.
- A focus selects only leaf tasks of its own team. Tasks of another team that
  belong to a selected initiative or project are not counted, and a ticket member of
  another team is unresolved. This restates the existing team boundary of
  [FR-048](./FR-048-session-work-monitor.md) AC-9 for named focuses.
- The leaf tasks of a focus are the union of the leaf tasks of its inclusions minus
  the union of the leaf tasks of its exclusions. Exclusion is evaluated by closure:
  excluding an initiative removes the leaves of its current projects; excluding a
  project removes its leaves; excluding a milestone removes its leaves; excluding a
  ticket removes it and its descendants. Within one focus, an exclusion wins over
  every inclusion of the same leaf.
- Exclusions apply only inside the focus that lists them. Another active focus
  that includes the same leaf keeps it.
- An empty inclusion list gives no leaves and no rows. It is reported as an
  empty focus, never as the whole team. Zero active focuses gives the combined
  result "no active focus" with no rows and no totals.
- Progress counts the non-canceled, non-duplicate leaf tasks with the existing
  meaning of done, remaining by stage, ready and blocked. Parent tickets do not
  inflate totals. A project row's counts are restricted to the focus's leaves and
  are marked partial when exclusions or milestone-only or ticket-only inclusion
  leave some of the project outside the focus.
- Tickets and activity never create focus. Open tickets, assignments and agent
  reports never add a leaf to a focus.

### Combination and placement

- The combined result for several focuses of one team is the union of their leaf
  sets. Each leaf is counted once. Per-focus rows keep their own counts and the
  relay SHALL NOT add them. A row that appears in more than one focus, or a project
  beneath more than one displayed initiative, carries the shared mark in every
  placement and each placement shows the same value for the same scope of leaves.
- Active focuses of different teams are separate groups. No total, union or
  readiness is formed across teams.
- Order is the shared model's order, then name (byte order), then ID. The same
  input gives the same order.

### Source states

- A missing, stale, partial or failed snapshot, an unresolved member, or an unknown
  project status SHALL be reported as such, with counts marked unavailable or
  partial. Unknown is never reported as healthy, complete or zero.
- After a failed refresh the last good result is retained with its original fetch
  time and the error. A result computed for one revision is never presented as
  another.
- Computation is repeatable from the same snapshot and selections, and its inputs
  are recorded as a key of focus ID, adopted revision and snapshot fetch time.

### Required shared interface

The pinned ix-board revision `8381a601` provides `FocusSelection` and
`ScopeSelection` (inclusion lists only), `FocusRecord` and `FocusMember` (four
member kinds, a schema version, no team, no exclusions), and `ScopeSummary` (leaf
IDs, done and remaining for one selection). It treats an empty selector set with a
team as the whole team. The relay needs the following, to be provided by the
ix-board owner; this requirement does not claim they exist:

1. Exclusion lists for the same four member kinds, in `FocusSelection` and the
   scope selection, and in `FocusRecord`, with a schema version change.
2. An entry point that treats an explicitly empty named selection as empty, not as
   the whole team, distinct from the whole-team selection.
3. A milestone member qualified by its owning project, validated on resolution.
4. A multi-selection entry point returning, per selection and for the combination
   by team group: the leaf set, the union of leaf IDs, per-initiative and
   per-project partial summaries with ready and blocked counts and the shared and
   partial marks above, in deterministic order.
5. Typed per-member resolution and reference states (resolved, missing, wrong team,
   mismatched milestone), and the existing failure classification.
6. The project status category already required by
   [FR-060](./FR-060-focused-scope-presentation.md).

The relay may keep a bounded cache of results (see
[FR-064](./FR-064-named-focus-dashboard-reader.md)). It SHALL NOT edit the shared
library's selection, summary or classification rules.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-063-AC-1 | Given a fixture with initiative I containing projects P1 and P2, a focus including I and excluding P2 selects only P1's leaves. A leaf in P1 that the focus also excludes by ticket is absent, and a leaf in P2 included by ticket in the same focus is still absent (exclusion wins). | Test |
| FR-063-AC-2 | Given focuses A (includes P) and B (includes P, excludes ticket T in P), A keeps T and B does not; the combined result includes T once. | Test |
| FR-063-AC-3 | Given initiative I with project P1 and a later snapshot where project P2 joins I, the same revision of a focus including I selects P1, then P1 and P2, with no new revision. | Test |
| FR-063-AC-4 | Given project P with milestones M4 and M5, a focus including P/M4 counts only M4's leaves and marks the P row partial; a milestone ID paired with the wrong project is unresolved; a same-named milestone of another project is not selected. | Test |
| FR-063-AC-5 | Given a focus of team X that includes an initiative containing tasks of teams X and Y, only X's leaves are counted; a ticket of team Y is unresolved. | Test |
| FR-063-AC-6 | Given an empty inclusion list, the result is an empty focus with no rows and zero leaves, not the whole team; with zero active focuses, the combined result is "no active focus". | Test |
| FR-063-AC-7 | Given a ticket-only focus with a ticket whose parent project is not included, the ticket and its descendants appear as direct-ticket rows with their project partial, and the focus never turns into a project or team focus. | Test |
| FR-063-AC-8 | Given two focuses sharing project P and one sharing a leaf under different initiatives, the combined result counts each leaf once; each per-focus row keeps its own counts; P's row carries the shared mark in every placement; no summed total exists in any output. | Test |
| FR-063-AC-9 | Given active focuses of teams X and Y, results form separate team groups with no cross-team total. | Test |
| FR-063-AC-10 | Given a stale, partial or failed snapshot, an unresolved member, and an unknown project category, the result labels each, counts are unavailable or partial, and none is reported as healthy, complete or zero; after a failed refresh the last good result keeps its original fetch time with the error. | Test |
| FR-063-AC-11 | Given the same snapshot and selections, repeated computation returns identical order and counts; an open ticket, an assignment or a work report never adds a leaf. | Test |
| FR-063-AC-12 | Inspection of the relay confirms it contains no membership, exclusion, deduplication, progress or readiness calculation for named focuses and no Linear reader beyond the shared library's read service. | Inspection |
| FR-063-AC-13 | Given initiative I with projects P1 and P2 and a later snapshot where P1 leaves I, the same revision of a focus including I selects P1 and P2, then P2 only, with no new revision. | Test |

## Dependencies

- [FR-061](./FR-061-named-focus-definitions.md) stores the selections and
  [FR-062](./FR-062-session-active-focuses-and-adoption.md) pins the adopted
  revision. [FR-048](./FR-048-session-work-monitor.md) AC-3 and AC-9 define the
  existing progress meaning and team boundary for legacy scope, which this
  requirement does not change.
- ix-board owns the interface in "Required shared interface". The relay waits for
  that accepted contract and, once it exists, one dependency update; it does not
  pin intermediate revisions. The project status category is tracked separately
  (see [FR-060](./FR-060-focused-scope-presentation.md)) and blocks only the hiding
  of completed projects.
- Out of scope: the whole-team slow snapshot read, a second raw tracker reader,
  department metrics, spend and drift measures, and any Linear write.
