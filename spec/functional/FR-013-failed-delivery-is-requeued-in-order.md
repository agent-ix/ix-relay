---
id: FR-013
title: "A delivery that did not happen is requeued in order"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-013: A delivery that did not happen is requeued in order

## Description

If `relay-watcher` pops messages for a lead and herdr reports that no input was
sent to the pane, then the watcher shall return those messages to the front of
the lead's inbox in their original order, so a later pop returns them exactly as
if they had never been popped.

## Behavior

- The watcher shall pop through the atomic budgeted pop of
  [FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md) with a count limit
  of 50 and a byte budget of 16 KiB, so no message is popped that the watcher
  does not attempt to type in the same tick. The rendered-size bound on the
  prompt is [NFR-002](../non-functional/NFR-002-bounded-resources.md)'s.
- The watcher shall type a batch through a delivery transport that reports
  whether it is certain no input was sent; `herdr agent prompt` is the only
  transport today, and a transport is asked for nothing else the decision uses.
- If the transport is certain no input was sent (for herdr, a documented
  "no input was sent" outcome: `agent_blocked`, `agent_not_found`, or herdr
  failing to start), then the watcher shall requeue the batch at the front of
  the inbox in original order.
- If the delivery fails in any other way (a timeout, an unrecognised error, a
  bare non-zero exit), then the watcher shall notify the human operator, leaving the
  batch neither requeued nor acknowledged.
- The watcher shall return a batch through the in-flight record
  ([FR-020](./FR-020-hook-never-loses-popped-messages.md)), so the return is one
  atomic step that does not depend on the watcher still holding the messages.

A batch whose prompt failed ambiguously is not requeued because it might already
have been typed; it stays in flight and is delivered again, marked, when its
lease expires ([FR-020](./FR-020-hook-never-loses-popped-messages.md),
[FR-040](./FR-040-redelivery-is-marked-bounded-and-dead-lettered.md)), so the
worst outcome is a duplicate, not a loss.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-013-AC-1 | Every pop over any inbox contents, followed by a requeue of the popped messages, leaves the inbox contents equal to the contents before the pop. | Test |
| FR-013-AC-2 | Given herdr reports `agent_blocked` for the prompt, the inbox afterwards holds the batch ahead of any messages that were already queued behind it, in original order. | Test |
| FR-013-AC-3 | Given herdr times out on the prompt, the batch is not requeued and one notification naming the lead is raised. | Test |
| FR-013-AC-4 | Given a Redis timeout at any point during one watcher delivery, every message that was in the inbox before is afterwards either still in the inbox, in flight, or was handed to herdr's prompt. | Test |
| FR-013-AC-5 | Given a hook pop concurrent with a watcher delivery that does not fail, every message returned by the later of the two pops was queued after every message returned by the earlier one. | Test |
| FR-013-AC-6 | A herdr failure is classified "certainly not sent" exactly when herdr documents that no input was sent, and every other failure is ambiguous, whatever transport the watcher was built with. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md),
  [FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md)
- Delivered by AGE-2013.
- Constrained by [NFR-002](../non-functional/NFR-002-bounded-resources.md) and
  [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md).
- Gap closed by AGE-2026: the watcher pops with `pop_inbox_budget` and no
  longer requeues a tail. The budget counts rendered bytes since AGE-2031
  (NFR-002 watcher-prompt row). Messages lost after a
  pop (a crash before the prompt, an ambiguous prompt) were FR-020's, closed by
  AGE-2032: they stay in flight and return when the lease expires.
