---
id: US-003
title: "Be reachable without configuring a name"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-001"
    type: "traces_to"
---
# US-003: Be reachable without configuring a name

## Story

**As a** team lead started in a herdr pane with no bus name configured
**I want** to be given a stable name from where I sit
**So that** other leads can message me from my first turn, and my name never silently takes over another lead's.

## Context

Leads are started by hand and by tooling; setting a per-session environment
variable is easy to forget. herdr labels can be any text, and two panes on
different machines can sit at identically labelled places.

## Traceability (Informative)

Drives [FR-021](../functional/FR-021-derived-names-are-valid.md) and
[FR-022](../functional/FR-022-name-collisions-resolve-to-a-free-name.md).
