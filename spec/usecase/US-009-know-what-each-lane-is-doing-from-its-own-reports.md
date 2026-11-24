---
id: US-009
title: "Know what each lane is doing from its own reports"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-001"
    type: "traces_to"
---
# US-009: Know what each lane is doing from its own reports

## Story

**As an** operator or orchestrating agent responsible for keeping every host and lane busy
**I want** each lane to report its own activity, blocked state and receipt of orders, and each host to report measured load and its own probe health
**So that** idle lanes, blocked lanes, unacted orders and failing probes are detected from evidence within minutes, not discovered hours later by polling or by reading pane text.

## Context

Probing lanes from outside (ssh, shell scripts, byte-limited transcript reads) failed
silently: a broken probe read as "host unreachable" for hours, an order sent to a dead
pane was never noticed, and "all hosts in target" was claimed without measurement.
The relay already sees every lane's hook events and every order's delivery, so it is
the natural source of ground truth. A failed source must be visible as a failure, never
as absence or as a healthy reading.

## Acceptance Criteria

- A lane's most recent activity, and why it is blocked or idle, is available without probing the lane.
- Host CPU, memory and compiler load are measured on the host itself.
- An order that was delivered but not acted on, or never acknowledged, is visible and escalates.
- A collector source that cannot be read is reported as failed.

## Traced Requirements

- [FR-068](../functional/FR-068-hook-activity-events.md)
- [FR-069](../functional/FR-069-host-collector.md)
- [FR-070](../functional/FR-070-collector-source-health.md)
- [FR-071](../functional/FR-071-order-ack-deadline-and-timeout.md)
- [FR-072](../functional/FR-072-ordered-lane-assignment-list.md)
- [FR-067](../functional/FR-067-persistent-delivery-receipts.md) (Acted receipt state)
- [FR-045](../functional/FR-045-watcher-pushes-metrics-over-otlp.md) (fleet series)
