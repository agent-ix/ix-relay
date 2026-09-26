---
id: US-001
title: "Message a lead by where it sits"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-001"
    type: "traces_to"
---
# US-001: Message a lead by where it sits

## Story

**As a** team lead that needs to tell another lead something
**I want** to name the recipient the way the operator sees it — machine, workspace and, when needed, tab — or by its bus name
**So that** I can reach the right lead without first looking up an internal identifier, and am told plainly when my description fits more than one lead or none.

## Context

herdr shows each pane by machine, workspace label and tab label. Bus names are
slugs and pane ids repeat across machines, so neither is how a person thinks of a
lead. Two panes can sit in one tab, and one workspace label can exist on several
machines.

## Acceptance Examples (Illustrative)

### US-001-EX-1: Unique workspace

- **Given** one lead in workspace `research` on `node-a`
- **When** a lead sends to `node-a:research`
- **Then** that lead receives it

### US-001-EX-2: Two tabs, no tab given

- **Given** leads in tabs `code` and `plan` of workspace `EA` on `hub`
- **When** a lead sends to `hub:EA`
- **Then** nothing is sent and both `hub:EA:code` and `hub:EA:plan` are listed

## Traceability (Informative)

Drives [FR-001](../functional/FR-001-address-grammar-and-cleaning.md) to
[FR-005](../functional/FR-005-shown-address-rule.md).
