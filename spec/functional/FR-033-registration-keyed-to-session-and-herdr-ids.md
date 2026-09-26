---
id: FR-033
title: "A registration is keyed to its session and records herdr's stable ids"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-007"
    type: "implements"
---
# FR-033: A registration is keyed to its session and records herdr's stable ids

## Description

When a lead registers from a herdr pane, the store shall record, in addition to
the labels of [FR-006](./FR-006-registration-records-place.md), herdr's stable
workspace id, tab id and pane id and the harness session id, and shall hold at
most one registration per harness session id.

## Behavior

- When the watcher sees a registered session id in a pane other than the
  registered one (a pane moved to another tab or workspace), the watcher shall
  update that registration's herdr ids and place within one place-refresh
  period, keeping its bus name.
- If a new registration carries a session id already held by a registration
  under another bus name, then the store shall replace the older registration,
  keeping the older bus name as an alias of the newer one.
- The store shall never re-link a registration by labels alone.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-033-AC-1 | Given a lead registered from pane `wB:p3` whose pane is moved to another tab, within 30 s plus one poll interval its registration records the new tab id and place and the same bus name. | Test |
| FR-033-AC-2 | For every sequence of registrations, no two registrations in the registry carry the same harness session id. | Test |
| FR-033-AC-3 | Given two panes with identical workspace and tab labels on one machine, a registration moves between them only when the registered session id is seen in the other pane. | Test |
| FR-033-AC-4 | Given a registration with no session id, the watcher never re-links it to another pane. | Test |

## Dependencies

- **Upstream**: [US-007](../usecase/US-007-never-reach-the-wrong-lead-after-a-move.md)
- **Gap on `main`** (AGE-2033): partly built by
  [FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md). The watcher follows a
  session to the pane that hosts it and never by labels alone, and never relinks a
  registration with no session id (FR-033-AC-3 and AC-4). Not built: a registration
  records the pane and the place, not herdr's workspace and tab ids, so FR-033-AC-1's
  tab id is not recorded, and nothing enforces one registration per session id
  (FR-033-AC-2).
