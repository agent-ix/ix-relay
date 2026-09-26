---
id: FR-011
title: "Broadcast reaches every other lead once"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-011: Broadcast reaches every other lead once

## Description

When a lead sends a message to `all`, the store shall append exactly one copy to
the inbox of every registered lead other than the sender, and one entry to the
history log, in one atomic step.

## Behavior

- The store shall exclude the sender's own bus name from the recipients.
- The store shall report the recipients to the sender, sorted by bus name.
- When no other lead is registered, the store shall log the message and queue it
  nowhere, and `agent-msg send` shall report that it was logged only.
- The renderer shall mark a delivered broadcast with `(to all)` after the
  sender's label.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-011-AC-1 | For every set of registered leads containing the sender, a broadcast increases each other lead's inbox length by exactly one and leaves the sender's inbox length unchanged. | Test |
| FR-011-AC-2 | Given the sender is the only registered lead, the broadcast adds one log entry, queues nothing, and `agent-msg send` prints that it was logged only. | Test |
| FR-011-AC-3 | Given a broadcast from `qsl-lead` with text `hi`, the rendered line is `[ix-relay] from qsl-lead (to all): hi`. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Delivered by AGE-2010.
