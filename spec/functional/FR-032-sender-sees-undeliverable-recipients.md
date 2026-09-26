---
id: FR-032
title: "The sender sees when a recipient cannot currently be reached"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-032: The sender sees when a recipient cannot currently be reached

## Description

When `agent-msg send` queues a message for a lead that the relay cannot
currently deliver to or wake, `agent-msg send` shall print, after the queued-for
line, the recipient's state and the reason, and shall still exit 0 because the
message is queued.

## Behavior

- The relay shall classify a recipient as undeliverable when the watcher has
  found that the recipient's pane no longer hosts the registered session.
- The relay shall classify a recipient as not wakeable when no watcher
  heartbeat is live for the recipient's machine or the recipient's registration
  has no pane: a working recipient still receives through its own hooks, but an
  idle one waits until its next turn.
- The watcher shall record, on a registration whose pane no longer hosts its
  session, that it is undeliverable and since when; the next registration of
  that lead clears it.
- `agent-msg who` shall show each undeliverable lead's state and reason.
- For a broadcast, `agent-msg send` shall print one state line per undeliverable
  recipient.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-032-AC-1 | Given the recipient's machine has no live watcher heartbeat, `agent-msg send` prints `no watcher heartbeat from <machine>` on the queued line ([FR-041](./FR-041-send-prints-the-recipients-state.md)), exits 0, and the message is in the recipient's inbox. | Test |
| FR-032-AC-2 | Given the watcher found the recipient's pane hosting a different session, `agent-msg send` prints a line naming the recipient as undeliverable because its pane was replaced. | Test |
| FR-032-AC-3 | Given a recipient with a live watcher and a pane, `agent-msg send` prints `watcher heartbeat from <machine>` with its last tick. | Test |
| FR-032-AC-4 | Given an undeliverable lead registers again from a live pane, `agent-msg who` no longer shows it as undeliverable. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- FR-032-AC-1 and AC-3 are delivered by [FR-041](./FR-041-send-prints-the-recipients-state.md).
  **Gap on `main`** (AGE-2033): FR-032-AC-2 is met only in its watcher and `who` halves. Built
  by [FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md)'s watcher pass: the
  watcher records, in the key `<prefix>health:replaced:<lead>` (one per lead), that a
  registration's pane hosts another session and since when, and `agent-msg who` prints
  `UNDELIVERABLE: pane <id> now hosts another session (since <time>)` for it. The record
  is bound to the registration it describes (its pane and `last_seen_ms`), so the next
  registration, or a relink, ends it without anyone deleting it (FR-032-AC-4). The
  key expires 7 days after it was written. The record is not a member of `Registration`.
