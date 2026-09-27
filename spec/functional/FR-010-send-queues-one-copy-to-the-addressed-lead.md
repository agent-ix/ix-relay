---
id: FR-010
title: "Send queues one copy for the addressed lead only"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-010: Send queues one copy for the addressed lead only

## Description

When a lead sends a message to a single-lead target, the store shall append
exactly one copy of the message to the inbox of that target's canonical bus name
and one entry to the history log, in one atomic step, and shall append nothing to
any other inbox.

## Behavior

- The store shall resolve an alias target to its canonical name before queueing.
- If the canonical name is not registered, then the store shall refuse the send
  with an unknown-agent error and shall queue and log nothing.
- If the message text is longer than 4096 bytes, then the store shall refuse the
  send with a message-too-large error before queueing or logging anything.
- The store shall sign each queued message with the sender's bus name and the
  sender's clock in Unix milliseconds.
- The history log shall retain approximately the most recent 10,000 messages.
- When the target is a bus name or an alias, the sender shall resolve it by
  reading the alias table and that one lead's record only, never the whole
  registry, and shall confirm the send with the lead's bus name; a pane id or a
  `server:workspace[:tab[:pane]]` address, which need the registry to resolve, read it
  once and confirm with the shown address.
- Combined with atomic pops ([FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md)),
  each sent message is delivered at most once to its recipient.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-010-AC-1 | Every send from any registered sender to any registered lead X increases X's inbox length by exactly one and leaves every other inbox length unchanged. | Test |
| FR-010-AC-2 | Given alias `old` pointing at registered `new`, sending to `old` queues the message in `new`'s inbox and no inbox named `old` exists afterwards. | Test |
| FR-010-AC-3 | Given an unregistered target name, the send returns an unknown-agent error and no inbox or log entry is added. | Test |
| FR-010-AC-4 | Given text of 4097 bytes, the send returns a message-too-large error and no inbox or log entry is added; given exactly 4096 bytes, the send succeeds. | Test |
| FR-010-AC-5 | Given a Redis connection dropped during the send's transaction, either both the inbox entry and the log entry exist afterwards or neither does. | Test |
| FR-010-AC-6 | For every interleaving of one send with concurrent pops of the recipient's inbox, the message is returned by at most one pop. | Test |
| FR-010-AC-7 | Given a send by bus name, and one by alias, the requests the sender makes never include a read of the whole registry, and the confirmation begins `queued for <bus name>` (followed by the recipient's state, [FR-041](./FR-041-send-prints-the-recipients-state.md), read in one further pipelined request); given a send by address, the registry is read and the confirmation names the shown address. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Delivered by AGE-2010.
- Constrained by [NFR-002](../non-functional/NFR-002-bounded-resources.md).
