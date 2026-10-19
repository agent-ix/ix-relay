---
id: FR-055
title: "The socket status file lists only registered leads whose process is alive"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-055: The socket status file lists only registered leads whose process is alive

## Description

`relay-watcher` shall keep a lead in its socket status file's resolved leads only while
the lead is registered on this machine and the Claude Code process it resolved is alive,
so that `agent-msg doctor` shows the leads the watcher can deliver to now.

## Background

A resolved-lead entry (the lead, the Claude Code pid, its socket and its version) was
added on each socket attempt and removed only when a later delivery to that lead failed.
A lead that was unregistered, pruned, or whose process exited and was never mailed again
stayed in the file for the life of the watcher and `doctor` kept showing it as resolved,
counting leads that no longer existed.

## Behavior

- When a tick starts, the watcher shall drop from the resolved leads every entry whose lead has
  no registration in the registry it read for the tick, whose registration is on another
  machine, or whose registration's `kind` is not `claude` ([FR-054](./FR-054-socket-delivery-is-for-claude-leads.md)).
- When a tick starts, the watcher shall drop every entry whose process is gone: the pid has no
  process, or the process there started at another time than the entry resolved
  ([FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md)'s liveness rule).
- The watcher shall drop an entry only for these reasons, and without delaying or failing
  any delivery.
- When the watcher dropped any entry, it shall write the status file.
- If the registry cannot be read in a tick, then the watcher shall drop no entry for
  registration reasons.
- Dropping an entry shall not drop a held batch, a pending decision, a dead-letter reason or a
  drift count ([FR-046](./FR-046-claude-code-formats-are-typed-and-drift-is-detected.md));
  those have their own rules.
- `agent-msg doctor` shall count a lead as resolved only if its entry is in the status file.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-055-AC-1 | Given a resolved lead that is then unregistered or pruned, the next tick leaves no entry for it and `doctor` does not count it. | Test |
| FR-055-AC-2 | Given a resolved lead whose Claude Code process exits (and one whose pid is reused by a process started at another time), the next tick drops its entry. | Test |
| FR-055-AC-3 | Given a resolved lead that is registered and whose process is alive, a tick keeps its entry; given a registry read that fails, no entry is dropped. | Test |
| FR-055-AC-4 | Given an entry dropped for a lead that has a held batch, a pending decision, a dead-letter reason or a drift count, those remain. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Related: [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md),
  [FR-054](./FR-054-socket-delivery-is-for-claude-leads.md),
  [FR-029](./FR-029-doctor-reports-relay-health.md).
