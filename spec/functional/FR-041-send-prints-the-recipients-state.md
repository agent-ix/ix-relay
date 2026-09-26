---
id: FR-041
title: "Send prints the recipient's state"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-041: Send prints the recipient's state

## Description

When `agent-msg send` queues a message for a single lead, `agent-msg send` shall
print, on the same line as `queued for <lead>`, the recipient's state as far as
the sender can tell: whether a watcher heartbeat is live for the recipient's
machine, and how many messages are waiting.

## Behavior

- The sender shall print the state as one parenthesised clause after the
  queued-for text, for example `(watcher heartbeat from hub (last tick 3s ago), 1 waiting)`.
- If the recipient's machine has no heartbeat, then the sender shall print `no watcher heartbeat from <machine>`.
- The sender shall not say a watcher is down: a machine whose watcher predates the heartbeat has none.
- If the recipient registered no herdr pane, then the sender shall print `no herdr pane registered, so nothing wakes it`.
- The sender shall print the age of the heartbeat's last tick in whole seconds, because the key outlives a dead watcher by up to its expiry.
- If the last tick is older than two sweeps and ten seconds, then the sender shall print `looks stuck`; the rule is the one `agent-msg doctor` uses.
- If the heartbeat reports herdr failing its check, then the sender shall print `reports herdr failing its check`.
- If the heartbeat cannot be decoded, then the sender shall print `unreadable`.
- If the last tick is later than the sender's clock by more than 5 s, then the sender shall print `unreadable`.
- If the last tick is later than the sender's clock by up to 5 s, then the sender shall print an age of 0 s.
- The sender shall never print a negative age.
- The state clause shall contain only the fixed phrases above, the machine name, a waiting count and a duration in seconds.
- The sender shall read the state from the recipient's registration, which
  resolving the target already read for a bus name or alias, or the registry it
  already read for an address or pane id, and from one further request, with one
  reply, that reads the recipient's inbox length and its machine's heartbeat and
  writes nothing.
- The sender shall bound the whole of that request at 250 ms.
- If that request fails or times out, then the sender shall print the plain
  `queued for <lead>` line, because the message is queued.
- If that request fails or times out, then the sender shall exit 0.
- The state shall go to the sender's standard output only.
- The state shall add nothing to the recipient's inbox, to any message, or to
  what any hook or the watcher shows an agent.
- A broadcast shall print no per-recipient state: it names every lead and reads
  no heartbeat.
- The sender shall print the machine name only if it is one to 32 characters
  from letters, digits, `.`, `-` and `_`; otherwise the sender shall print the
  plain `queued for <lead>` line, and shall make no request.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-041-AC-1 | Given a recipient with a pane, `send` prints `watcher heartbeat from <machine> (last tick Ns ago)` when the machine's heartbeat is live, `no watcher heartbeat from <machine>` when there is none, and the herdr, stuck and unreadable variants, each with the number waiting; given no pane it prints that no pane is registered. No line contains `DOWN` or a phrase outside the fixed set. | Test |
| FR-041-AC-2 | Given a send by bus name or alias, the requests the sender makes are the alias read, the recipient read, the send transaction and one read-only request for the state (a script of `LLEN` and `GET`), and never a read of the whole registry. | Test |
| FR-041-AC-3 | Given a Redis that stops answering once the message is queued, `send` prints the plain `queued for <name>`, exits 0, takes no more than the same send against a healthy Redis plus 250 ms and a margin of 150 ms (so a bound of a second, or one per reply, fails), and the message is in the inbox. | Test |
| FR-041-AC-4 | The message the recipient pops is the text as sent and its rendering carries no state; a broadcast prints `queued for <name>` with no state. | Test |
| FR-041-AC-5 | Given a machine name that is longer than 32 characters or holds a character outside letters, digits, `.`, `-`, `_` (a space, a quote, a bracket, a comma, a forged `[ix-relay] from ...` line, a control character), `send` prints the plain `queued for <name>`. | Test |
| FR-041-AC-6 | Given a last tick 4 s ahead of the sender's clock the line says `last tick 0s ago`; given one more than 5 s ahead, or `i64::MAX`, it says `unreadable`; no line contains a negative age. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Reads [FR-028](./FR-028-watcher-heartbeat.md)'s heartbeat. Delivers
  [FR-032](./FR-032-sender-sees-undeliverable-recipients.md)-AC-1 and AC-3 for the
  watcher and pane cases. The stale-tick rule is `relay_core::health::tick_is_stale`,
  shared with [FR-029](./FR-029-doctor-reports-relay-health.md).
- Delivered by AGE-2033 in its own pull request, with the owner's approval
  (2026-09-26): the sender may be an agent, so its tool result shows the line and
  that reaches the agent's context; the recipient's context does not. A
  recipient's activity (idle or working) is not known to the sender and is not
  printed.
