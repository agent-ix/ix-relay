---
id: NFR-002
title: "Bounded sizes, retention and waits"
type: NFR
quality_attribute: reliability
relationships:
  - target: "ix://agent-ix/ix-relay/FR-010"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-013"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-018"
    type: "constrains"
---
# NFR-002: Bounded sizes, retention and waits

## Statement

The ix-relay tools shall keep every message, every delivery, every stored history
and every wait on an external process or on Redis within a fixed bound, so that no
input and no misbehaving dependency makes a tool grow or wait without limit.

## Scope

- Message size, hook output size, watcher prompt size, history retention, cache
  key lifetime, Redis call timeouts and herdr call timeouts.
- The watcher's OTLP push ([FR-045](../functional/FR-045-watcher-pushes-metrics-over-otlp.md)):
  one exporter thread (none when no endpoint is set) and no listener; a queue of at
  most 2 snapshots (the oldest dropped when full) plus the one being sent, each at most
  6,000 points and a 4 MiB request; a connect timeout of 2 s, a 5 s limit on the whole
  request and a 64 KiB response cap; capped backoff from 1 s to 5 min; a push interval
  of 5 to 3,600 s; 2,000 panes, 50 leads and 64 hook failure pairs as series bounds.
  Once per push (30 s by default) it costs the tick two pipelined store requests after
  the tick's deliveries: the local leads' queue lengths, over their own keys with no key
  scan, and at most the newest 1,000 hook health entries. A collector that never
  answers holds the exporter thread only, never delays a tick, and is shown in the
  heartbeat as `stuck` after 30 s.

## Rationale

The bus is shared by every lead on three machines through one Redis instance.
An unbounded message, history or wait turns one misbehaving participant into an
outage for all of them.

## Measurement and Evaluation

| Metric | Target | Threshold | Method |
|--------|--------|-----------|--------|
| Accepted message text size | 4096 bytes | 4096 bytes | unit-testing |
| Hook output text size | 6 KiB | 6 KiB plus one rendered message plus the pending line | property-based-testing |
| Watcher prompt text size | 16 KiB | 16 KiB plus one rendered message | property-based-testing |
| History log length after 20,000 sends | 10,000 entries | 11,000 entries | integration-testing |
| Lifetime of a session-name or suppression cache key | 30 days | 30 days | integration-testing |
| Interactive command Redis wait | 3 s | 3.1 s | fault-injection |
| Any single herdr call, including one writing more output than a pipe buffer holds | its per-call timeout | its per-call timeout plus 100 ms | fault-injection |
| Transcript scan time and memory for a file of size S | linear in S | linear in S | performance-benchmarking |

## Verification

Tests assert each size bound at its boundary and a benchmark checks the scan's
growth; a herdr stub that writes more than a pipe buffer before exiting checks
that no herdr call waits past its timeout.

## Dependencies

- **Upstream**: [FR-010](../functional/FR-010-send-queues-one-copy-to-the-addressed-lead.md),
  [FR-013](../functional/FR-013-failed-delivery-is-requeued-in-order.md),
  [FR-018](../functional/FR-018-hook-delivers-per-event.md)
- Delivered by AGE-2010, AGE-2013 and AGE-2018.
- `herdr-cli` drains a child's output on threads (AGE-2026), closing the
  herdr-call row's gap.
- Closed by AGE-2031 for the hook-output and watcher-prompt rows: the pop counts
  printed bytes ([FR-018](../functional/FR-018-hook-delivers-per-event.md)).
