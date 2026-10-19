---
id: FR-031
title: "Undeliverable messages become dead letters with a typed reason"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-031: Undeliverable messages become dead letters with a typed reason

## Description

If ix-relay would otherwise drop a queued or popped message, then ix-relay shall
move the message to the bus's dead-letter list with its recipient, the time and a
reason from a fixed set, instead of dropping it.

## Behavior

- The reason shall be one of `undecodable` (an inbox entry that does not decode),
  `prompt-ambiguous` (a watcher prompt that may or may not have been typed),
  `requeue-failed` (a requeue that Redis refused), and `recipient-pruned` (queued
  messages of a registration being pruned).
- A dead letter shall keep the original stored entry byte for byte.
- `agent-msg dead-letters` shall list dead letters oldest first, with their
  text neutralised as in [FR-016](./FR-016-render-neutralises-control-characters.md).
- The dead-letter list shall keep at least the most recent 10,000 entries and
  every entry younger than 30 days, whichever is more.
- The watcher shall not prune a registration whose inbox holds messages without
  first dead-lettering them with reason `recipient-pruned`, or, when a new session
  holds the registration's address, handing them to that session
  ([FR-053](./FR-053-stranded-mail-is-forwarded-or-dead-lettered.md), which also
  states when the watcher does either).

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-031-AC-1 | Given an inbox whose second entry is not valid message JSON, after the pop the dead-letter list holds that entry byte for byte with reason `undecodable`. | Test |
| FR-031-AC-2 | Given herdr times out on a watcher prompt, the dead-letter list holds each message of the batch with reason `prompt-ambiguous`. | Test |
| FR-031-AC-3 | Given the requeue after an `agent_blocked` prompt fails, the dead-letter list holds each message of the batch with reason `requeue-failed`. | Test |
| FR-031-AC-4 | Every message removed from an inbox, for any sequence of sends, pops, prompt outcomes and prunes, is afterwards either delivered to herdr or the hook's stdout, back in an inbox, or in the dead-letter list. | Test |
| FR-031-AC-5 | Given a dead letter whose text contains ESC, `agent-msg dead-letters` prints it without ESC. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- The dead-letter list of [FR-040](./FR-040-redelivery-is-marked-bounded-and-dead-lettered.md)
  (`deadletter:<lead>`, AGE-2032) holds only entries whose redeliveries ran out. It
  has no reason field and no `agent-msg dead-letters` command; AGE-2033 decides
  whether to merge the two lists.
- Related: [FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md), [FR-013](./FR-013-failed-delivery-is-requeued-in-order.md), [FR-020](./FR-020-hook-never-loses-popped-messages.md) (AGE-2032).
- **Gap on `main`** (AGE-2033): not built. FR-031-AC-1 to FR-031-AC-5 are
  expected to fail on `main`.
