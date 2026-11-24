---
id: FR-071
title: "An order has an ack deadline, times out and escalates"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-009"
    type: "implements"
---
# FR-071: An order has an ack deadline, times out and escalates

## Description

The relay SHALL give each order a deadline for the recipient's first sign of acting
(the `Acted` receipt of [FR-067](./FR-067-persistent-delivery-receipts.md)). When the
deadline passes without `Acted`, the relay SHALL record `TimedOut` for the order and raise
an escalation, so an order to a dead or unattended pane is noticed. An order is a message
sent with a deadline; ordinary messages are unchanged.

## Behavior

- `agent-msg send` SHALL accept an ack deadline per order in the same duration format as
  [FR-057](./FR-057-lane-focus-is-explicit-and-visible.md), from 10 seconds to 24 hours;
  the default SHALL be 5 minutes, configurable by `[send] ack_deadline` under
  [FR-026](./FR-026-configuration-precedence.md). A value outside the range SHALL be
  refused before enqueue.
- The deadline SHALL run from the `Delivered` receipt, or from enqueue when the order is
  never delivered. A message to a recipient that is dead-lettered or undeliverable under
  [FR-031](./FR-031-dead-letters-instead-of-drops.md) SHALL time out from enqueue.
- When the deadline passes before `Acted`, `relay-watcher` SHALL record one `TimedOut`
  receipt under the order's original identity and SHALL create one escalation under
  [FR-023](./FR-023-escalation-id-validity.md) naming the order identity, recipient
  address, sender address and the last receipt state, and no message text.
- The `TimedOut` receipt is distinct from an attempt lease timeout in FR-067; a lease
  timeout SHALL NOT start or stop an order deadline.
- An `Acted` receipt recorded before the deadline SHALL cancel the deadline, and one
  recorded after `TimedOut` stays recorded and leaves the escalation in place.
- The watcher SHALL export `ix_relay_orders_unacted` (orders past `Delivered` or enqueue
  with no `Acted` and no `TimedOut`, by `recipient` bounded as in
  [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md)) and the
  `ix.relay.order.act_latency` histogram (seconds from `Delivered` to `Acted`).
- Deadline state SHALL be bounded by the receipt bounds of FR-067. The watcher SHALL scan
  for expired deadlines each tick without a key scan, and a watcher restart SHALL NOT
  create a second `TimedOut` or a second escalation for the same order.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-071-AC-1 | Given a send with no deadline option, the order's deadline is 5 minutes; with `--ack-deadline 90s` it is 90 seconds; with `2s` and `25h` the send is refused and nothing is enqueued. | Test |
| FR-071-AC-2 | Given an order delivered to a pane whose session never emits a hook event, on an injected clock one tick after the deadline records exactly one `TimedOut` receipt and one escalation naming the order identity and addresses and holding no message text. | Test |
| FR-071-AC-3 | Given an order to a dead pane that is never delivered, `TimedOut` is recorded from enqueue time after the deadline (the fault-injection run). | Test |
| FR-071-AC-4 | Given `Acted` before the deadline, no `TimedOut` and no escalation exist; given `Acted` after `TimedOut`, both receipts exist and the escalation remains. | Test |
| FR-071-AC-5 | Given an attempt lease timeout and requeue within the deadline, the order's deadline is unchanged and no `TimedOut` is recorded. | Test |
| FR-071-AC-6 | Given the watcher restarted after the deadline and again after `TimedOut`, exactly one `TimedOut` and one escalation exist. | Test |
| FR-071-AC-7 | A push shows `ix_relay_orders_unacted` equal to the count of orders without `Acted` or `TimedOut`, falling when one is acted; the act latency histogram records `Delivered` to `Acted` seconds. | Test |
| FR-071-AC-8 | A tick that scans deadlines makes no key scan and adds at most one pipelined request. | Test |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-067](./FR-067-persistent-delivery-receipts.md),
  [FR-068](./FR-068-hook-activity-events.md), [FR-024](./FR-024-escalation-status-transitions.md),
  [FR-025](./FR-025-escalation-text-safety.md), [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md).
- The escalation ladder beyond the first escalation (lead, orchestrator, Discord) is owned
  outside this repository and consumes this escalation.
