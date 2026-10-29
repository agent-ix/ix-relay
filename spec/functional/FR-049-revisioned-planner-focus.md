---
id: FR-049
title: "Publish revisioned planner focus"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
---
# FR-049: Publish revisioned planner focus

## Description

The relay SHALL publish planned focus separately from session activity reports. When a planner publishes a changed plan, the relay SHALL create an immutable revision owned by that verified planner session incarnation.

## Behavior

Planned focus contains an explicit Linear team key, a summary, tracker selectors, ordered priority goals and their monitoring thresholds. Each goal names its own project/milestone/issue selectors, a minimum worker count of at least one, a positive no-progress interval and a positive observation window. Thresholds for spend and net ticket growth are nonnegative. Human assignees are not inferred from a shared account.

Project and milestone selectors resolve to stable Linear IDs through ix-board. Milestones are qualified by their owning project; duplicate names across projects require qualification. A plan selects one of whole-team, explicit nonempty selectors, or explicit empty/paused scope. Empty input never implicitly means whole-team. Milestones narrow the selected projects. A milestone without its project, a foreign-team selection or a goal outside the plan's selected scope is rejected. Priority goals may overlap; counts use the union of leaf issue IDs rather than summed group totals. A revision declares an available-worker budget; the sum of goal minimums cannot exceed it. Paused scope has no active goals. A stale or incomplete tracker snapshot cannot validate a new selection; publication reports the missing source rather than silently retaining unresolved selectors.

Publication uses an expected prior revision (zero for first publication). The relay SHALL reject a mismatched expected revision without changing the plan. Publication verifies that the publisher is not a follower in the same atomic role check used by follow registration. A successful publication increments a per-incarnation unsigned revision number exactly once and returns the resulting revision. Repeating a publication with its old expected revision returns a conflict; clients reread before deciding to republish. Overflow is rejected. A no-op publication with the matching expected revision returns the existing revision. Scope/activity timestamps are not plan revisions.

A revision includes the verified WorkBinding, normalized selectors, team, goals, available-worker budget, thresholds, publication time and schema version. Resolved IDs freeze selector identity, not the issue population: current scope membership is recomputed from current tracker facts, while historical metrics require contemporaneous membership evidence. The relay SHALL validate the complete revision before atomically publishing its current pointer. The current pointer and revisions expire after 30 days without successful publication; readers label missing data unavailable. Identical publication renews retention without changing the revision. A lead stores its adopted snapshot in its own report, so revision expiry cannot silently change adopted work.

Limits per revision: 64 KiB encoded JSON, 128 selectors total across scope and goals, 32 goals, 2,048 UTF-8 bytes for the summary and 256 bytes per goal name. Counts and revisions use checked integer arithmetic. Invalid input, unknown schema versions, malformed stored data and exceeded limits return typed failures without mutation. Existing work scope, work focus, assignments and inbox records retain their semantics.

Proposed CLI contract (to be implemented in SWM-128): `agent-msg work plan publish --file <json> --expected-revision <n>` and `agent-msg work plan show`. Existing `--lead` and `--session` select and verify the publisher; omitted lead means the verified calling pane. Authorization remains the existing trusted relay/operator model; following is not message authorization.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-049-AC-1 | Given first publication with expected revision 0, the stored revision is 1; a changed publication expecting 1 creates 2 while revision 1 remains unchanged during retention. | Test |
| FR-049-AC-2 | Given two changed publishers both expecting revision 1, exactly one creates revision 2 and the other receives a typed conflict; there is no mixed payload. | Test |
| FR-049-AC-3 | Given identical normalized content and matching expected revision, publication returns the existing revision and preserves concurrent activity and agent reports. | Test |
| FR-049-AC-4 | Given project P with milestones Layer 4 and Layer 5, a Layer 4 plan includes only P/Layer 4 leaf tasks; a same-named milestone in project Q is not selected. | Test |
| FR-049-AC-5 | Given whole-team, explicit selectors and paused scope, reads distinguish all three; absent scope, invalid project/milestone pairing and foreign-team selectors are rejected. | Test |
| FR-049-AC-6 | Given overlapping goals, shared leaf tasks count once in the plan total; a goal outside scope, active goals on paused scope or summed minimums above the declared worker budget are rejected. | Test |
| FR-049-AC-7 | Given boundary-sized valid input it is accepted; one extra selector, goal or byte is rejected atomically, as are negative/zero invalid thresholds and revision overflow. | Test |
| FR-049-AC-8 | Given an expired publisher record or unsupported schema version, reads return unavailable or schema failure rather than an empty or healthy plan; adopted snapshots remain readable. | Test |

## Dependencies

[FR-048](./FR-048-session-work-monitor.md) owns session isolation. ix-board owns selector resolution and leaf-task membership. SWM-128 implements this contract; no tracker mutations are introduced.
