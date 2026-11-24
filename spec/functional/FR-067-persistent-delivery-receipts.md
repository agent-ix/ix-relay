---
id: FR-067
title: "Retain bounded delivery receipts without message content"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-067: Retain bounded delivery receipts without message content

## Description

The relay SHALL retain bounded lifecycle receipts so a monitoring consumer can
distinguish queued messages, observed transport delivery, and committed delivery
acknowledgement without reading message content.

## Inputs

- Successful enqueue, delivery attempts, transport observations, acknowledgement,
  requeue, lease expiry and dead-letter transitions.
- An export request identifying a time interval and an optional continuation cursor.

## Outputs

- An ordered, paginated receipt export with coverage and source-availability metadata.
- Explicit receipt-recording failures without a fabricated successful outcome.

## Behavior

- When a send successfully enqueues a message, the store SHALL assign that original
  message a stable identity distinct from every other successful enqueue, including
  an enqueue with byte-identical message content.
- The relay SHALL preserve the original identity through claims, requeues,
  redelivery, dead-lettering and watcher or consumer restart for as long as that
  message remains queued, in flight or dead-lettered.
- For each delivery attempt, the relay SHALL assign an attempt identity associated
  with the original message identity.
- Each retained receipt SHALL contain a unique event identity, the original message
  identity, the applicable attempt identity, the recorded outcome, the source-native
  sender and target identities, the event time and the receipt observation time in
  Unix milliseconds.
- The relay SHALL mark an event time unknown when its source cannot establish it.
- The export SHALL preserve recorded event order independently of wall-clock ties
  or backwards clock movement.
- When an enqueue commits, the store SHALL commit its queued receipt in the same
  atomic operation as the inbox and message-history entries required by
  [FR-010](./FR-010-send-queues-one-copy-to-the-addressed-lead.md).
- When a transport reports delivery success, the relay SHALL record a delivered
  receipt naming that observation's evidence kind,
  including whether socket delivery remains unconfirmed.
- When the store commits a valid acknowledgement, the store SHALL commit the
  acknowledged receipt atomically with removal of that attempt's in-flight state.
- If an acknowledgement is stale, repeated or belongs to another attempt, then the
  store SHALL NOT create an acknowledged receipt for the current attempt.
- The relay SHALL interpret delivered as the transport observation and acknowledged
  as the relay's committed settlement, without asserting that a human or agent has
  read or understood the message. Only `Acted` (below) asserts activity after delivery:
  that the recipient session acted after delivery. `Acted` does not show that the
  session accepted the message as an order or took it up; order acceptance is confirmed
  by the consumer under [FR-071](./FR-071-order-ack-deadline-and-timeout.md).
- When the recipient lead's session writes its first `prompt`, `pre_tool` or `post_tool`
  activity event under [FR-068](./FR-068-hook-activity-events.md) after a message's
  `Delivered` receipt, the relay SHALL record one `Acted` receipt for that message under
  its original identity, atomically with that event write, naming the event's session.
  One event SHALL mark `Acted` every Delivered, not-yet-Acted message of that lead whose
  `Delivered` precedes the event, at most 50 per event.
- The relay SHALL decide that an event follows `Delivered` by the relay server's (Redis)
  time recorded at each write, not by the host-reported event time, so that clock skew
  between hosts cannot create or hide an `Acted`.
- The relay SHALL NOT record `Acted` from an event of another session, an event written
  before the `Delivered` receipt, a `session_start`, `stop`, `idle`, `blocked` or
  `session_end` event, or for a message that was never `Delivered`. A retried event SHALL
  NOT create a second `Acted`.
- An activity event written by the hook call that delivered a message SHALL NOT record `Acted`
  for it or for any message: only an event of a later hook call of the session counts. A plain
  inbox read of a message SHALL record `Delivered` (evidence `inbox_read`) beside `Removed`.
- The relay-watcher SHALL record the order-level `AckExpired` receipt of
  [FR-071](./FR-071-order-ack-deadline-and-timeout.md) under the same identity and
  retention bounds as other receipts, and the export SHALL report `Acted` and `AckExpired`
  in the same ordered, paginated form. `AckExpired` is distinct from the attempt-level
  timeout outcome above.
- If delivery is failed, ambiguous, held or timed out, then the relay SHALL NOT
  record delivered or acknowledged solely because the send was queued or a lease
  elapsed.
- When an attempt times out, is requeued, is redelivered or is dead-lettered, the
  relay SHALL record that distinct outcome under the same original identity with
  the applicable attempt identity and source-reported reason.
- When the same transition observation or settlement is retried, the relay SHALL
  retain the same event identity rather than count it as another transition.
- The receipt store SHALL retain at most 10,000 receipt events, evicting the oldest
  events when that bound is reached.
- The export SHALL exclude receipt events older than seven days at the read time.
- Receipt eviction SHALL NOT remove identity metadata still needed by a queued,
  in-flight or dead-lettered message.
- The relay SHALL export receipts through the existing message-history JSON command
  family, using an explicit receipt selection that does not change the ordinary
  message-history result or append requeues as new sent messages under
  [FR-038](./FR-038-watcher-wakes-when-a-message-is-sent.md).
- The export SHALL accept a bounded page size of at most 1,000 events and return a
  continuation cursor when more retained events match the requested interval.
- The export SHALL identify the requested interval, the covered interval, the
  instrumentation start, the retention boundary, and whether further pages remain.
- If any requested history is unavailable because instrumentation had not begun,
  retention evicted events, a cursor was evicted, recording failed or the source
  cannot be read, then the export SHALL report incomplete or unavailable coverage
  with that reason rather than claim a complete empty result.
- If a delivery observation cannot be persisted, then the relay SHALL surface the
  recording failure and preserve unavailable coverage for the affected interval
  after recovery rather than invent the missing receipt.
- The export SHALL perform no inbox pop, acknowledgement, requeue, retention trim,
  receipt mutation or delivery attempt.
- The export SHALL exclude message bodies, transcript content and credentials.
- The receipt collector SHALL NOT infer correction, status, order or planner-audit
  classifications from message text.
- Receipt collection SHALL preserve the existing delivery, lease, recovery,
  dead-letter and acknowledgement decisions, including the at-most-once plain-inbox
  path described by [FR-020](./FR-020-hook-never-loses-popped-messages.md).
- If receipt persistence fails during a transition, then the relay SHALL preserve
  the original message under the existing recovery rules without silently losing
  it or creating a second original enqueue.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-067-AC-1 | In an isolated Redis fixture, enqueue, deliver and acknowledge one message, then export its receipts. Queued, delivered and acknowledged events have distinct stable event identities in recorded order, one original identity, the applicable attempt identity, source-native sender/target, event/observation times and delivery evidence kind. A queued-only message has no delivered or acknowledged event. | Test |
| FR-067-AC-2 | Enqueue two byte-identical messages, expire the first attempt of one, restart the watcher or consumer, redeliver and acknowledge it. The two originals remain distinct; the recovered message retains its original identity with distinct attempt identities and timeout/redelivery outcomes. Repeating the same export or settlement does not create or count another event. | Test |
| FR-067-AC-3 | With equal or backwards source times, export order remains stable. An unavailable source event time is explicitly unknown. A held or ambiguous transport observation produces no fabricated delivery; socket delivery acknowledged as unconfirmed retains that evidence kind and never claims recipient reading or action. | Test |
| FR-067-AC-4 | Drop the connection around enqueue and acknowledgement transactions. After recovery, each committed enqueue has its queued receipt, and each committed valid acknowledgement has its acknowledged receipt; an uncommitted or stale acknowledgement does not manufacture a success. Inject a delivery-observation recording failure and verify that the failure and incomplete interval remain visible after recovery while existing recovery retains the original message. | Test |
| FR-067-AC-5 | Advance the fixture clock beyond seven days and separately append 10,001 events. Retained history obeys both bounds; evicted history and an evicted continuation cursor report incomplete coverage with a reason. A message still waiting across receipt expiry retains its original identity on later delivery. | Test |
| FR-067-AC-6 | Request an interval before instrumentation, an interval with a known recording gap, a source that cannot be read, and a covered interval with no events. Only the last is complete empty history. A request with more than 1,000 matching events returns bounded pages and continuation metadata without silently dropping the remainder. | Test |
| FR-067-AC-7 | Snapshot inbox, in-flight and receipt state, export all pages twice and compare state byte for byte. Reads leave all three unchanged, including events past their retention age that have not yet been trimmed by a writer. Exported data contains none of a seeded secret message body, transcript or credential value and no prose-derived semantic classification. | Test |
| FR-067-AC-8 | Exercise successful delivery, definite failure, ambiguous failure, lease recovery, exhausted redelivery, stale acknowledgement and a plain-inbox pop with receipt collection enabled. Existing ordering, markings and recovery outcomes remain unchanged; ordinary message-history JSON still contains each original send once and no requeue entry. | Test |
| FR-067-AC-9 | Given a Delivered message and then, in the recipient's session, a `prompt`, `pre_tool` or `post_tool` event, the export shows one `Acted` receipt after `Delivered` under the same original identity, the event's session, and no message content; the acknowledgement state is unchanged. | Test |
| FR-067-AC-10 | Given events of another session, an event written before `Delivered` by relay server time, `stop`, `idle`, `blocked`, `session_start` and `session_end` events, and a message never Delivered, no `Acted` is recorded; a retried event adds no second `Acted`; one event marks at most 50 Delivered messages. | Test |
| FR-067-AC-11 | Given an ack deadline passing without `Acted`, the export shows an `AckExpired` receipt under the original identity, distinct in outcome from an attempt lease timeout, in recorded order with the other receipts. | Test |
| FR-067-AC-12 | Given a recipient host whose clock is 10 minutes behind, and another 10 minutes ahead of the relay server, an event written after `Delivered` records `Acted` and an event written before it does not, by relay server time. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md).
- **Extended by**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md) with `Acted` ([FR-068](./FR-068-hook-activity-events.md)) and `AckExpired` ([FR-071](./FR-071-order-ack-deadline-and-timeout.md)).
- Existing outcome semantics: [FR-010](./FR-010-send-queues-one-copy-to-the-addressed-lead.md),
  [FR-020](./FR-020-hook-never-loses-popped-messages.md),
  [FR-038](./FR-038-watcher-wakes-when-a-message-is-sent.md),
  [FR-040](./FR-040-redelivery-is-marked-bounded-and-dead-lettered.md) and
  [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md).
- Consumer: department monitoring receipt evidence; popup/reply UI, semantic message
  classification and retrospective reconstruction are outside this requirement.
