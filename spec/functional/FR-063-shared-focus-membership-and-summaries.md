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
  - target: "ix://agent-ix/ix-relay/FR-060"
    type: "depends_on"
  - target: "ix://agent-ix/ix-board/FR-010"
    type: "depends_on"
  - target: "ix://agent-ix/ix-board/FR-012"
    type: "depends_on"
  - target: "ix://agent-ix/ix-board/FR-016"
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

This requirement states the behavior the relay needs from the shared library and
checks at its boundary. It does not define the library's types or API: the
ix-board owner defines them in the ix-board repository, and this requirement
neither restates nor duplicates that contract. The owning contracts are
[ix-board FR-010](ix://agent-ix/ix-board/FR-010) (membership, team, leaf and
qualified selector rules), [ix-board FR-012](ix://agent-ix/ix-board/FR-012)
(per-focus and combined summaries, partial and shared facts, readiness and
blocker integration) and [ix-board FR-016](ix://agent-ix/ix-board/FR-016) (the
versioned focus record). They reuse ix-board's existing expansion, readiness and
blocker-closure behavior.

It also adds the option `--members` to `agent-msg focuses show` of
[FR-061](./FR-061-named-focus-definitions.md).

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
  Linear object). A partial mark carries its reason, one or more of: a
  milestone-only or ticket-only inclusion, an exclusion, or the team boundary
  (leaf tasks of another team were left out). Initiative rows and project rows
  carry the mark by the same rule.
- The union of leaf tasks for the combined request, per team group, with a single
  count of each leaf.
- Typed reference states for each member: resolved, missing, moved to another team,
  and any ix-board classification of a snapshot that is partial, stale or failed.
- For `agent-msg focuses show <focus> --members`, the same rows and states for the
  one definition's current revision, computed on demand and never stored.

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
  inflate totals. A project row's or an initiative row's counts are restricted to
  the focus's leaves and are marked partial, with the reason, when exclusions,
  milestone-only or ticket-only inclusion, or the team boundary leave some of the
  Linear object outside the focus.
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

### What the relay needs from the shared library

The relay consumes the library; it does not specify it. Some of this exists in
ix-board today (qualified milestone parsing and partial-project summaries). The
rest (stored qualification, exclusions, an explicit empty named selection distinct
from the whole-team meaning of a team alone, and the batch result) is a **future
ix-board extension** tracked by SWM-178, an implementation prerequisite of the
relay work; the project status category is already available. Conditions the
relay checks at its boundary, and treats as the library's obligations:

- One computation per refresh returns the per-focus results and the per-team-group
  union for all active selections, in deterministic order, so that an individual
  view and the combined view come from one result.
- An explicitly empty named selection yields an empty result, distinct from the
  whole-team selection that legacy scope uses.
- Exclusions, milestone qualification by project, dynamic initiative membership,
  the team boundary and the marks above are evaluated by the library, and a member
  it cannot resolve is reported with a typed state.
- A snapshot read can be bounded by a deadline and returns a typed timeout, so a
  command or a refresh never waits unboundedly and a timeout publishes nothing.
  This is a need handed to the ix-board owner; the relay defines no SDK API for it.
- The project status category required by
  [FR-060](./FR-060-focused-scope-presentation.md) is already available.

The relay may keep a bounded cache of results (see
[FR-064](./FR-064-named-focus-dashboard-reader.md)). It SHALL NOT edit the shared
library's selection, summary or classification rules, and consumes one accepted
library revision through one dependency update.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-063-AC-1 | A focus including initiative I (projects P1 and P2) and excluding P2 selects only P1's leaves; a P1 leaf that the focus also excludes by ticket is absent; a P2 leaf included by ticket in the same focus is still absent, because exclusion wins. | Test |
| FR-063-AC-2 | Of focuses A (includes P) and B (includes P, excludes ticket T in P), A keeps T and B does not, and the combined result includes T once. | Test |
| FR-063-AC-3 | With initiative I holding P1 and a later snapshot in which P2 joins I, the same revision of a focus including I selects P1 and, after the refresh, P1 and P2, with no new revision. | Test |
| FR-063-AC-4 | With initiative I holding P1 and P2 and a later snapshot in which P1 leaves I, the same revision of a focus including I selects P1 and P2 and, after the refresh, P2 only, with no new revision. | Test |
| FR-063-AC-5 | With project P holding milestones M4 and M5, a focus including P/M4 counts only M4's leaves and marks the P row partial with the milestone reason; a milestone ID paired with the wrong project is unresolved; a same-named milestone of another project is not selected. | Test |
| FR-063-AC-6 | A focus of team X including an initiative that contains tasks of teams X and Y counts only X's leaves, marks both the initiative row and each project row that lost other-team tasks partial with the team-boundary reason, and treats a ticket of team Y as unresolved. | Test |
| FR-063-AC-7 | A focus excluding one project of an initiative marks the initiative row partial with the exclusion reason, and the initiative's counts equal the leaves of its remaining projects only. | Test |
| FR-063-AC-8 | An empty inclusion list gives an empty focus with no rows and zero leaves, not the whole team; zero active focuses gives the combined result "no active focus". | Test |
| FR-063-AC-9 | A ticket-only focus whose ticket's parent project is not included shows the ticket and its descendants as direct-ticket rows with their project partial, and the focus never becomes a project or team focus. | Test |
| FR-063-AC-10 | With two focuses sharing project P and one leaf shared under different initiatives, the combined result counts each leaf once, each per-focus row keeps its own counts, P's row carries the shared mark in every placement, and no output holds a summed total. | Test |
| FR-063-AC-11 | With active focuses of teams X and Y, results form separate team groups with no cross-team total. | Test |
| FR-063-AC-12 | A stale, partial or failed snapshot, an unresolved member and an unknown project category are each labeled, counts are unavailable or partial, and none is reported as healthy, complete or zero. | Test |
| FR-063-AC-13 | After a failed refresh the last good result keeps its original fetch time and the error. | Test |
| FR-063-AC-14 | The same snapshot and selections give identical order and counts on repeat, and an open ticket, an assignment or a work report never adds a leaf. | Test |
| FR-063-AC-15 | `agent-msg focuses show <focus> --members` prints the rows and states of the current revision from one snapshot read bounded as in FR-061, stores nothing, and with a failed read prints the definition with `source_unavailable` and its reason and exit status 1. | Test |
| FR-063-AC-16 | Inspection of the relay confirms it contains no membership, exclusion, deduplication, progress or readiness calculation for named focuses, no Linear reader beyond the shared library's read service, and no restatement of the library's selection types. | Inspection |

## Dependencies

- [FR-061](./FR-061-named-focus-definitions.md) stores the selections and
  [FR-062](./FR-062-session-active-focuses-and-adoption.md) holds the adopted
  snapshot. [FR-048](./FR-048-session-work-monitor.md) AC-3 and AC-9 define the
  existing progress meaning and team boundary for legacy scope, which this
  requirement does not change.
- The shared ix-board library owns the interface the relay consumes (ix-board
  FR-010, FR-012 and FR-016 above). The extension is specified by the single
  ix-board ticket SWM-178, which follows the accepted relay contract and is an
  explicit implementation prerequisite of the relay adapter work. The relay then
  makes one dependency update and does not pin intermediate revisions. A
  deadline-bounded read is a separate need handed to the ix-board owner.
- Out of scope: the whole-team slow snapshot read, a second raw tracker reader,
  department metrics, spend and drift measures, and any Linear write.
