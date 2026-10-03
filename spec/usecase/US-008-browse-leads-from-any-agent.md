---
id: US-008
title: "Browse relay leads from any coding agent"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-001"
    type: "traces_to"
  - target: "ix://agent-ix/ix-relay/StR-003"
    type: "traces_to"
---
# US-008: Browse relay leads from any coding agent

## Story

**As a** team lead working in a Herdr pane
**I want** a shared status indicator and searchable directory of other leads
**So that** I can see whether the relay is healthy and find the correct verified
address without switching coding agents or risking a message to the wrong pane.

## Context

Herdr hosts Claude Code, Codex, OpenCode, and other coding agents in the same
pane model. Relay discovery and health therefore belong in one Herdr plugin,
while identity and delivery continue to come from ix-relay.

## Traceability (Informative)

Drives [FR-049](../functional/FR-049-herdr-relay-status.md) and
[FR-050](../functional/FR-050-herdr-online-lead-directory.md).
