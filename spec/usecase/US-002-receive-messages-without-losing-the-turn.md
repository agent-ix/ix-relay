---
id: US-002
title: "Receive messages without losing the turn"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-001"
    type: "traces_to"
---
# US-002: Receive messages without losing the turn

## Story

**As a** team lead working through a task
**I want** messages sent to me to arrive while I work or wake me when I am idle, each once and clearly signed
**So that** I act on what other leads tell me promptly, and a relay problem never costs me my turn or my work.

## Context

A lead may be mid-turn, idle, starting, or waiting on an approval dialog. Its
harness runs hooks around tool calls and at the end of a turn; an idle lead can
only be reached by typing into its pane. Text arriving this way reads as if the
user typed it, so it must be unmistakably signed and unable to impersonate
another sender.

## Acceptance Examples (Illustrative)

### US-002-EX-1: Mid-turn delivery

- **Given** a working lead with one queued message
- **When** it finishes its next tool call
- **Then** the message appears in its context, signed with the sender

### US-002-EX-2: Relay down

- **Given** the bus is unreachable
- **When** the lead's hooks run
- **Then** the lead's turn continues exactly as without ix-relay

## Traceability (Informative)

Drives [FR-006](../functional/FR-006-registration-records-place.md) to
[FR-020](../functional/FR-020-hook-never-loses-popped-messages.md),
[FR-040](../functional/FR-040-redelivery-is-marked-bounded-and-dead-lettered.md), and
[FR-038](../functional/FR-038-watcher-wakes-when-a-message-is-sent.md).
