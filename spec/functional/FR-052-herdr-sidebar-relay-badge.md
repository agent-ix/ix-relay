---
id: FR-052
title: "Optional Herdr sidebar relay badge"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "traces_to"
  - target: "ix://agent-ix/ix-relay/FR-049"
    type: "depends_on"
---
# FR-052: Optional Herdr sidebar relay badge

## Description

The local relay watcher SHALL publish a display-only `ix_relay` pane metadata
token for each registered lead whose exact Herdr session still matches its
registration. Users MAY add `$ix_relay` to an optional Herdr agent sidebar row.
The token SHALL expire unless refreshed and SHALL NOT replace or modify
Herdr's native agent activity state. The default tab bar and popup remain
available without sidebar configuration.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-052-AC-1 | Verified registered panes receive connected, degraded, or unavailable state and a pending count; unreadable counts are explicitly unknown. Non-leads and unverified/replaced sessions receive no metadata. | Unit tests |
| FR-052-AC-2 | Pending count observes inbox length without popping, acknowledging, or moving messages in-flight. | Redis integration tests |
| FR-052-AC-3 | Metadata is display-only, has a bounded TTL, and stops appearing after the watcher stops refreshing it. Native idle/working/blocked state is unchanged. | Unit and Herdr command tests |
| FR-052-AC-4 | Publication is best-effort and bounded, and does not prevent the watcher from servicing leads when Herdr is slow or unavailable. | Unit tests |
| FR-052-AC-5 | Documentation shows an optional sidebar row using `$ix_relay` while retaining the normal `state_icon` agent row. | Documentation review |
