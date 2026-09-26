---
id: FR-034
title: "Renames and moves are recorded as events naming the exact ids"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-007"
    type: "implements"
---
# FR-034: Renames and moves are recorded as events naming the exact ids

## Description

When the watcher changes a registration's place because a herdr workspace or tab
was renamed or a pane moved, the watcher shall record an event naming the lead's
bus name, the herdr workspace, tab and pane ids involved, the old address, the
new address and the time.

## Behavior

- The event log shall keep every event younger than 7 days and at most 10,000
  events.
- The watcher shall record one event per registration per change, not one per
  poll.
- When ix-relay prints an event, the printer shall neutralise its text as in
  [FR-016](./FR-016-render-neutralises-control-characters.md).

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-034-AC-1 | Given tab `code` of workspace `EA` on `hub` is renamed `build`, one event names the lead, the tab's herdr id, `hub:EA:code` and `hub:EA:build`. | Test |
| FR-034-AC-2 | Given a pane moves from tab `code` to tab `plan`, one event names the pane id, the old and the new address. | Test |
| FR-034-AC-3 | Given no label or pane changes across ten polls, no event is recorded. | Test |
| FR-034-AC-4 | Given an event older than 7 days and fewer than 10,000 newer events, the event is no longer returned. | Test |

## Dependencies

- **Upstream**: [US-007](../usecase/US-007-never-reach-the-wrong-lead-after-a-move.md)
- **Gap on `main`** (AGE-2033): partly built by
  [FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md): one event per relink
  and per place change, none for an unchanged poll (FR-034-AC-2 and AC-3). Not built:
  the event does not carry herdr's workspace and tab ids, because registrations do not
  record them (FR-034-AC-1), and the stream is capped at about 1,000 entries with no age
  bound, not 7 days and 10,000 (FR-034-AC-4).
