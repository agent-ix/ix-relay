---
id: FR-050
title: "Searchable online lead directory"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "traces_to"
  - target: "ix://agent-ix/ix-relay/FR-047"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-028"
    type: "depends_on"
---
# FR-050: Searchable online lead directory

## Description

The Herdr plugin SHALL provide a searchable, read-only directory of live panes
and their verified relay registrations. It SHALL use the same Herdr and relay
data for every supported coding agent.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-050-AC-1 | Each live Herdr pane is joined to a relay lead only when the live Herdr session id exactly matches one registration on the same machine and pane. Unregistered panes remain visible without a sendable address. | Unit tests |
| FR-050-AC-2 | The directory reports Herdr reachability, watcher freshness/health, and last-seen time. An unreachable machine or unreadable heartbeat is marked unknown/unavailable; it is never presented as offline merely because a remote read failed. | Unit tests |
| FR-050-AC-3 | A stable machine-readable snapshot contains rows, per-machine health, completeness, and truncation. The snapshot is bounded to 500 rows and a 2 MiB reader limit, and changes no inbox or registration. | Unit and command tests |
| FR-050-AC-4 | A Herdr popup supports local search by canonical name, alias, machine, workspace, tab, pane, session, or address; refresh updates the snapshot without blocking input. | Unit and terminal tests |
| FR-050-AC-5 | The popup shows live activity and verified relay state, and copies an address only for an exactly verified lead. Labels and remote error details are neutralized before terminal rendering. | Unit and terminal tests |
| FR-050-AC-6 | The plugin action and popup work through Herdr for Claude, Codex, OpenCode, and other coding agents without starting an agent turn. | Manifest test and demonstration |

## Dependencies

- FR-047 supplies the session and Herdr-placement verification rule.
- FR-028 supplies watcher heartbeat semantics.
