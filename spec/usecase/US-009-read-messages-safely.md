---
id: US-009
title: "Read and reply to relay messages safely"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-001"
    type: "traces_to"
---
# US-009: Read and reply to relay messages safely

## Story

**As a** team lead working in a Herdr pane
**I want** to read recent and pending relay messages in a formatted view and
reply as my verified lead
**So that** I can act on team messages without losing pending mail or confusing
message text for commands.

## Context

History and pending preview are read-only. A reply is an explicit send action
that uses the same session-qualified identity rule as address resolution.
Message bodies remain data even when they contain text that looks like a
command or another relay signature.

## Traceability (Informative)

Drives [FR-051](../functional/FR-051-formatted-message-history-and-reply.md).
