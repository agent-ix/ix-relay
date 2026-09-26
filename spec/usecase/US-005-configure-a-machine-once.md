---
id: US-005
title: "Configure a machine once"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-002"
    type: "traces_to"
---
# US-005: Configure a machine once

## Story

**As an** operator who owns a machine in the swarm
**I want** one settings file that every ix-relay tool on that machine reads the same way, overridable per invocation
**So that** I fix the bus address or machine name once, and a mistake in that file tells me where it is without breaking any lead's turn.

## Traceability (Informative)

Drives [FR-026](../functional/FR-026-configuration-precedence.md) and
[FR-027](../functional/FR-027-malformed-configuration-per-command.md).
