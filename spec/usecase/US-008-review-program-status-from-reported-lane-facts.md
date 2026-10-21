---
id: US-008
title: "Review program status from reported lane facts"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-001"
    type: "traces_to"
---
# US-008: Review program status from reported lane facts

## Story

**As an** operator coordinating work across team-lead lanes
**I want** to collect each lane's stated work and blocker status, see its declared focus, and inspect explicitly recorded server capacity
**So that** I can review progress and allocation using attributable reports and operator-entered facts.

## Context

Open tickets and server labels alone do not show whether a lane is actively
working, what it is waiting for, or what capacity an operator has allocated to a
server. The review must distinguish a lead's report from inferred tracker or
machine facts.

## Acceptance Example

**Given** two registered lanes and a status request sent to both, with one lane
having an explicitly set focus and its server having operator-recorded capacity,
**when** one lane submits its fixed-form reply and the operator reads the partial
collection, **then** the JSON identifies the request, contains the submitted
reply and lists the other lane as a nonresponder; `who --focus` shows only the
explicitly reported focus, and `who --servers` shows only the recorded capacity.

## Traceability (Informative)

Drives [FR-056](../functional/FR-056-repeatable-program-review-status.md),
[FR-057](../functional/FR-057-lane-focus-is-explicit-and-visible.md) and
[FR-058](../functional/FR-058-explicit-server-capacity.md).
