---
id: FR-049
title: "Shared Herdr relay status"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "traces_to"
  - target: "ix://agent-ix/ix-relay/FR-028"
    type: "depends_on"
---
# FR-049: Shared Herdr relay status

## Description

Herdr SHALL expose relay connectivity and local watcher state through a plugin
available to every coding agent hosted by Herdr. The status surface SHALL make
registration identity explicit and SHALL not begin an agent turn.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-049-AC-1 | A read-only status query reports Redis reachability, watcher freshness/health, and whether the current pane's live Herdr session exactly matches a relay registration. Missing, conflicting, or unavailable identity is unknown, never inferred from an agent name. | Unit tests |
| FR-049-AC-2 | The status bar distinguishes connected, degraded, unavailable, unregistered, and unknown states. Redis unavailability takes precedence over a claim that the lead is merely unregistered. | Unit tests |
| FR-049-AC-3 | Every Redis and Herdr read is bounded by one command deadline; the query consumes no inbox, does not register a lead, and does not start an agent turn. | Unit and command tests |
| FR-049-AC-4 | A shared Herdr plugin manifest provides the same status action to Claude, Codex, OpenCode, and other coding agents hosted by Herdr. | Manifest test and demonstration |
