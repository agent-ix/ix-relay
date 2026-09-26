---
id: US-004
title: "Escalate a decision to the operator"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-002"
    type: "traces_to"
---
# US-004: Escalate a decision to the operator

## Story

**As a** team lead facing a decision that is genuinely the operator's to make
**I want** to file it with my context, the options I see and my recommendation, and keep working until it is decided
**So that** the operator can decide it once, in their own time, and I receive that one decision.

## Context

The operator reads escalations in a terminal UI and may open an investigation
session in the lead's working directory. Escalation text is written by an agent
and could contain anything.

## Traceability (Informative)

Drives [FR-023](../functional/FR-023-escalation-id-validity.md) to
[FR-025](../functional/FR-025-escalation-text-safety.md).
