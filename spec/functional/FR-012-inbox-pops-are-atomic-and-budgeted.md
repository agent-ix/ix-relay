---
id: FR-012
title: "Inbox pops are atomic and budgeted"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-012: Inbox pops are atomic and budgeted

## Description

When a consumer pops a lead's inbox, the store shall take and return the oldest
pending messages in one atomic Redis operation, so that no two pops ever return
the same message and nothing leaves the inbox that the pop does not return. What
a pop takes is recorded as in flight in the same operation
([FR-020](./FR-020-hook-never-loses-popped-messages.md)); only the interactive
`agent-msg inbox` read removes messages outright.

## Behavior

- The store shall return popped messages oldest first.
- When a budgeted pop is requested with a count limit, the store shall take at
  most that many entries.
- When a budgeted pop is requested with a byte budget, the store shall take the
  longest run of oldest entries whose summed weight is within the budget, and at
  least the oldest entry even if it alone exceeds the budget. An entry's weight is
  never less than the bytes it prints as (its signature with the longest shown
  sender label, its text with each line feed indented, and the byte joining it to
  the next), so the byte budget bounds the printed batch, not the stored one.
- The store shall report, with a budgeted pop, how many messages remain in the
  inbox immediately after it.
- If a popped entry fails to decode, then the store shall report it as rejected
  and shall still return every entry of the same pop that decoded.
- If the pop operation fails, then the store shall leave the inbox unchanged.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-012-AC-1 | Every budgeted pop over any inbox contents, count limit and byte budget returns messages that, followed by the inbox contents afterwards, equal the inbox contents before, in order. | Test |
| FR-012-AC-2 | Every budgeted pop over any non-empty inbox returns at least one message, at most the count limit, and a total printed length within the byte budget whenever it returns more than one. | Test |
| FR-012-AC-3 | Every budgeted pop over any inbox contents reports a remaining count equal to the inbox length immediately after the pop. | Test |
| FR-012-AC-4 | Given N concurrent consumers popping one inbox of M messages until it is empty, every message is returned exactly once in total. | Test |
| FR-012-AC-5 | Given a batch whose second entry is not valid message JSON, the pop returns the first and third messages and reports one rejected entry. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Delivered by AGE-2010 and AGE-2013; the move to the in-flight record by AGE-2032.
