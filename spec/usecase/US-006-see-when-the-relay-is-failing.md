---
id: US-006
title: "See when the relay is failing"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-003"
    type: "traces_to"
---
# US-006: See when the relay is failing

## Story

**As an** operator responsible for the swarm, or a lead sending a message
**I want** to be told when a watcher is down, a hook is failing, a message could not be delivered, or herdr has changed under the relay
**So that** a message that did not arrive is a known, recoverable event rather than a silence I discover hours later.

## Context

Hooks exit 0 on every failure so an agent's turn is never broken, which also
means nobody sees the failure. The watcher runs as a background service that may
not be installed. Messages can be popped and then lost, and herdr's CLI output
can change between versions.

## Traceability (Informative)

Drives [FR-028](../functional/FR-028-watcher-heartbeat.md) to
[FR-032](../functional/FR-032-sender-sees-undeliverable-recipients.md),
[FR-036](../functional/FR-036-herdr-version-and-shape-check.md),
[FR-037](../functional/FR-037-escalation-retention.md) and
[FR-039](../functional/FR-039-read-only-commands-retry-a-timed-out-read.md) and
[FR-045](../functional/FR-045-watcher-pushes-metrics-over-otlp.md).
