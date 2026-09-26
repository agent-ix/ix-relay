---
id: FR-040
title: "A redelivered message is marked, bounded and dead-lettered"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-040: A redelivered message is marked, bounded and dead-lettered

## Description

When the store returns a message that was popped before and never acknowledged,
the recipient shall be shown that it is a redelivery, and after a fixed number of
redeliveries the store shall move the message to the lead's dead-letter list
instead of delivering it again.

## Behavior

- The store shall carry, with each entry, how many times it went back to the inbox
  because its lease expired (as trailing spaces after its JSON, which every reader
  ignores), so that a byte-identical entry sent afresh starts at zero and no side
  record of counts exists to leak.
- A message popped with a count above zero shall render as
  `[ix-relay] from <sender> (redelivered): <text>`; the stored message and its
  wire form shall not change.
- The rendered-size weight of every message shall include the marker, so the byte
  budgets hold whether or not a message is redelivered.
- If an expired entry has already been redelivered 3 times, then the store shall
  move it to `deadletter:<lead>` byte for byte instead of returning it to the inbox.
- The dead-letter list shall keep the newest 1,000 entries per lead.
- If a reap drops older dead letters to keep the list at that size, then it shall
  report how many, and the watcher shall log the number.
- `agent-msg inbox --dead` shall print a lead's dead letters oldest first, with
  their text neutralised as in [FR-016](./FR-016-render-neutralises-control-characters.md),
  and shall not remove them.
- Entries that do not decode are acknowledged with their batch and reported, as
  before; they are dead-lettered only when they are never acknowledged.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-040-AC-1 | Given a message whose batch expired once, the next pop returns it and the hook output shows `(redelivered)` in its signature; a message never redelivered shows no marker. | Test |
| FR-040-AC-2 | Given a message popped and never acknowledged, it is delivered at most 4 times in all, and afterwards it is in the dead-letter list, byte for byte, and not in the inbox or in flight. | Test |
| FR-040-AC-3 | Given a message acknowledged after a redelivery, no key holds a count for it, and a later identical entry is delivered unmarked. | Test |
| FR-040-AC-4 | Given dead letters, `agent-msg inbox --dead` prints them oldest first without ESC and leaves them in the list; the list never exceeds 1,000 entries. | Test |
| FR-040-AC-5 | The marker adds no more bytes to a rendered message than the weight of the pop allows for. | Test |
| FR-040-AC-6 | Given a redelivered entry and a byte-identical entry sent afresh, the fresh one is not marked and is delivered four times before it is dead-lettered. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md),
  [FR-020](./FR-020-hook-never-loses-popped-messages.md)
- Delivered by AGE-2032. The dead-letter reasons and the `agent-msg dead-letters`
  command of [FR-031](./FR-031-dead-letters-instead-of-drops.md) (AGE-2033) are not
  built here; this list holds only entries whose redeliveries are exhausted.
