---
id: FR-071
title: "An order has an ack deadline, expires and escalates"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-009"
    type: "implements"
---
# FR-071: An order has an ack deadline, expires and escalates

## Description

The relay SHALL give each order a deadline for the recipient's first sign of acting (the
`Acted` receipt of [FR-067](./FR-067-persistent-delivery-receipts.md)). When the deadline
passes without `Acted`, the relay SHALL record `AckExpired` for the order and raise an
escalation, so an order to a dead or unattended pane is noticed. An order is a message sent
as an order: explicitly with `agent-msg send --order`, or by the orchestrator's
`fleet assign`. A plain message has no deadline and no `AckExpired`.

## Behavior

- `agent-msg send --order` SHALL accept `--ack-deadline <age>` in the duration format of
  [FR-057](./FR-057-lane-focus-is-explicit-and-visible.md), from 10 seconds to 24 hours;
  the default for an order SHALL be 5 minutes, configurable by `[send] ack_deadline` under
  [FR-026](./FR-026-configuration-precedence.md). The command SHALL refuse, before enqueue, a value outside the range and
  `--ack-deadline` without `--order`.
- The deadline SHALL run from the `Delivered` receipt. An order that is never delivered,
  including one to a dead pane or to a recipient that is dead-lettered or undeliverable
  under [FR-031](./FR-031-dead-letters-instead-of-drops.md), SHALL expire after its
  deadline measured from enqueue.
- When the deadline passes before `Acted`, `relay-watcher` SHALL record one `AckExpired`
  receipt under the order's original identity and SHALL create one escalation under
  [FR-023](./FR-023-escalation-id-validity.md) naming the order identity, recipient
  address, sender address and the last receipt state, and no message text.
  The escalation has kind `order` and no ticket; it names the recipient lane address as its lead
  and the expected ticket when the order carries one. An escalation that cannot be filed
  is retried at most three times and then given up and counted, so it does not delay the
  escalation of other orders.
- `AckExpired` is distinct from an attempt lease timeout in FR-067; a lease timeout SHALL
  NOT start or stop an order deadline.
- An `Acted` receipt recorded before the deadline SHALL cancel the deadline. One recorded
  after `AckExpired` stays recorded and leaves the escalation in place.
- The relay SHALL measure the deadline with the relay server's (Redis) time, so that clock skew
  between hosts does not move it.
- The watcher SHALL export `ix.relay.orders.unacted` (orders past enqueue with no `Acted`
  and no `AckExpired`, by `recipient`, the lane address) and the
  `ix.relay.order.act_latency` histogram (seconds from `Delivered` to `Acted`), as in
  [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md).
- Deadline state SHALL be bounded by the receipt bounds of FR-067. The watcher SHALL scan
  for expired deadlines each tick without a key scan, and a watcher restart SHALL NOT
  create a second `AckExpired` or a second escalation for the same order.

### Expired orders are readable in one request

- The relay SHALL expose, read-only, the orders whose `AckExpired` it recorded and that have
  not been `Acted`, newest expiry first, without a key scan or a receipt-stream read:
  `Relay::expired_orders` and `agent-msg orders expired --json`. One reader script reads the
  index and the leased host lane lists of [FR-069](./FR-069-host-collector.md), which spell
  each recipient; an unreadable lane list leaves the stored spelling. Each item carries
  the order identity, the recipient lane address (in the spelling of the host lane lists when the
  lane is listed, else as stored at send time), the expected ticket, the enqueue time, the
  deadline, whether `Delivered` was recorded before expiry (a delivery after expiry does not
  count), the last receipt state before expiry and the escalation state (`pending`, `filed`,
  `failed`, or `unknown` for a value the reader does not know), and no message text.
- The relay SHALL record `Acted` on the order itself, so that no later receipt (acknowledged,
  timed out, requeued, redelivered, dead-lettered) puts an acted order back on the list.
- The call SHALL return at most 500 orders and examine at most 1,000 index rows, and SHALL set
  `truncated` when it stopped at either bound or when the index dropped live rows at its size
  bound. It SHALL return `skipped`, the count of rows it could not decode, and
  `indexed_since_ms`, the relay time the index began (an expiry recorded earlier, or by a
  `relay-watcher` that predates the index during a rolling upgrade, is not listed). It SHALL
  change nothing, and the script SHALL run with the Redis `no-writes` flag.
- The index holds an expiry for 8 days (the order record's lifetime) and at most 10,000
  entries, the oldest dropped first. These bounds are the index's own; the 7-day and
  10,000-event bounds of FR-067 apply to the receipt stream.

### All retained orders are readable in a bounded window

- `Relay::orders` and `agent-msg orders --since <time> --until <time> --limit <n> [--cursor <json>] --json`
  SHALL expose all indexed order records, regardless of whether they are open, acted,
  expired, evicted or in another stored status. `--since` and `--until` are inclusive
  Unix-millisecond bounds on `enqueued_at`; `since` SHALL be no later than `until`, and
  `limit` SHALL be between 1 and 500. Results SHALL be newest first by the stable tuple
  `(enqueued_at, order_id)`, descending.
- Each row SHALL contain `order_id`, `sender`, `recipient` (the stored lane address),
  `receipt_target` (the stored message target), `expect_ticket`, `ack_ms`, `deadline_at`,
  `enqueued_at`, `delivered_at`, `acted_at`, and `state`. `deadline_at` SHALL be computed
  by the relay as `delivered_at + ack_ms` when delivered, otherwise `enqueued_at + ack_ms`;
  absent optional values SHALL be JSON `null`. `state` SHALL be the explicit stored order
  status, including `open`, `acted`, `expired`, `evicted`, or any other status value, with
  unknown values preserved verbatim. Rows SHALL contain no message body or credentials.
- The response SHALL contain `orders`, `truncated`, `next_cursor`, `window`, `limit`, and
  `oldest_available_at`. `window` SHALL identify both inclusive enqueue-time bounds and the
  descending order. `oldest_available_at` SHALL identify the oldest enqueue time for which
  this index can provide complete data, accounting for the index start and the eight-day
  order-record retention; it SHALL be `null` until the first order is indexed. An order's
  absence before that time is outside retained coverage.
- `next_cursor`, when non-null, SHALL be the JSON tuple `{"enqueued_at": <Unix ms>,
  "order_id": <id>}` for the last returned order. A continuation SHALL return only tuples
  strictly older than that tuple, including all rows at the same millisecond through the
  order-id tie-breaker. `truncated` SHALL be true exactly when another matching row is
  available and SHALL then have a usable `next_cursor`; otherwise it SHALL be false and
  `next_cursor` SHALL be null.
- The order index SHALL cover the same eight-day lifetime as the order records and SHALL be
  populated when each order is enqueued. Reads SHALL use a bounded, single read-only Redis
  script, SHALL NOT scan keys, and SHALL change nothing on the bus.

### Order acceptance is confirmed by the consumer

- `Acted` shows that the recipient session acted after delivery; it is not proof that the
  order was accepted. An order MAY carry `--expect-ticket <id>` (at most 64 bytes), which
  the relay SHALL store with the order and return in the receipt export.
- The relay SHALL expose each lane's focus ticket from its newest hook-reported branch or
  cwd (`ix.fleet.lane.focus_info` in FR-045 and the last-activity record of
  [FR-068](./FR-068-hook-activity-events.md)), so that a consumer can compare it with the
  expected ticket.
- Confirming acceptance (the W2 consumer contract) belongs to the consumer: ix-board
  `fleet assign` SHALL treat an order as accepted only when the lane's focus ticket matches
  the assigned ticket, and the relay SHALL NOT decide acceptance.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-071-AC-1 | Given `send --order` with no deadline option, the deadline is 5 minutes; with `--ack-deadline 90s` it is 90 seconds; with `2s` and `25h`, and with `--ack-deadline` without `--order`, the send is refused and nothing is enqueued. | Test |
| FR-071-AC-2 | Given a plain message to a pane that never acts, no deadline, no `AckExpired` and no escalation exist for it at any time. | Test |
| FR-071-AC-3 | Given an order delivered to a pane whose session never emits a hook event, on an injected clock one tick after the deadline records exactly one `AckExpired` receipt and one escalation naming the order identity and addresses and holding no message text. | Test |
| FR-071-AC-4 | Given an order to a dead pane that is never delivered, `AckExpired` is recorded after the deadline measured from enqueue, as in the fault-injection run. | Test |
| FR-071-AC-5 | Given `Acted` before the deadline, no `AckExpired` and no escalation exist; given `Acted` after `AckExpired`, both receipts exist and the escalation remains. | Test |
| FR-071-AC-6 | Given an attempt lease timeout and requeue within the deadline, the order's deadline is unchanged and no `AckExpired` is recorded. | Test |
| FR-071-AC-7 | Given the watcher restarted after the deadline and again after `AckExpired`, exactly one `AckExpired` and one escalation exist. | Test |
| FR-071-AC-8 | A push shows `ix.relay.orders.unacted` equal to the count of orders without `Acted` or `AckExpired`, labelled by `recipient`, falling when one is acted; the act latency histogram records `Delivered` to `Acted` seconds. | Test |
| FR-071-AC-9 | A tick that scans deadlines makes no key scan and adds at most one pipelined request. | Test |
| FR-071-AC-10 | Given an order sent with `--expect-ticket`, the receipt export returns the expected ticket, and `focus_info` for the recipient lane shows the ticket parsed from its newest hook-reported branch; the relay records no acceptance of its own. | Test |
| FR-071-AC-11 | Given delivered and never-delivered orders that expired, one acted after expiry, one with a failed escalation and one filed, `expired_orders` and `agent-msg orders expired --json` return every un-acted expired order with its fields and escalation state in one request and exclude the acted one, including after an acknowledgement, lease timeout, requeue, redelivery or dead letter that follows `Acted`; given more than 500, exactly 500 come back with `truncated` set; a row that cannot be decoded is counted in `skipped`, `indexed_since_ms` names the index start, and more than 1,000 index rows or a size-bound trim set `truncated`; a `DUMP` of every relay key before and after the call is identical. | Test |
| FR-071-AC-12 | Given orders in every stored status and both delivered and never-delivered orders in the bounded enqueue-time window, `agent-msg orders` returns only the requested range, newest first by `(enqueued_at, order_id)`, with all required fields and relay-computed deadlines; same-millisecond pages continue strictly after the cursor tuple, `truncated` and `next_cursor` obey their invariant, retention metadata identifies the oldest complete coverage, bodies and credentials are absent, and a `DUMP` of every relay key before and after the read is identical. | Test |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-067](./FR-067-persistent-delivery-receipts.md),
  [FR-068](./FR-068-hook-activity-events.md), [FR-072](./FR-072-ordered-lane-assignment-list.md),
  [FR-024](./FR-024-escalation-status-transitions.md),
  [FR-025](./FR-025-escalation-text-safety.md), [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md).
- Acceptance by focus match is owned by ix-board#86 FR-035 (`fleet assign`).
- The escalation ladder beyond the first escalation (lead, orchestrator, Discord) and the
  `fleet assign` consumer are owned outside this repository (ix-board) and consume this
  escalation and these receipts.
