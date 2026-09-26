---
id: US-007
title: "Never reach the wrong lead after a move or rename"
type: US
relationships:
  - target: "ix://agent-ix/ix-relay/StR-003"
    type: "traces_to"
---
# US-007: Never reach the wrong lead after a move or rename

## Story

**As a** team lead sending to another lead by its address
**I want** a lead to stay the same lead on the bus when its pane moves or its tab is renamed, and a send to its old address to be refused with its new one
**So that** I never deliver to whoever now sits at the old place, and I learn where the lead went.

## Context

Addresses are built from herdr labels, which change: tabs are renamed, panes are
moved, unlabelled tabs renumber. The lead's harness session id does not change
across any of these. The operator decided on 2026-09-25 (relayed by the AGE-2029
coordinator) that an old address gets no grace window.

## Traceability (Informative)

Drives [FR-033](../functional/FR-033-registration-keyed-to-session-and-herdr-ids.md)
to [FR-035](../functional/FR-035-old-address-is-refused-with-the-new-one.md).
