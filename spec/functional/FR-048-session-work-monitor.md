---
id: FR-048
title: "Session-specific team-leader work monitor"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-047"
    type: "constrains"
---
# FR-048: Session-specific team-leader work monitor

## Description

The monitor SHALL bind an explicit Herdr address to one measured native harness
session and process incarnation. Scope, current focus, and agent assignments
SHALL be reported explicitly by the lead. Open tracker tickets alone do not
prove that an agent is working on them.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-048-AC-1 | Resolve exactly one live agent by address, verify its native session and process, refuse missing/conflicting identity, and require a restart to bind a replacement. Reports are isolated by machine, native session, and process incarnation. | Unit and Redis integration tests |
| FR-048-AC-2 | Reporting atomically updates scope, focus, or one agent, preserving concurrent assignments. Multiple agents can share a ticket. At most 128 agents are retained; input and stored records are bounded and validated. Reporting does not register subagents, consume inboxes, or change Linear. | Redis integration tests |
| FR-048-AC-3 | Resolve initiative/project/ticket scope through ix-board. Progress counts non-canceled, non-duplicate leaf tasks; parent tickets do not inflate totals. Workflow stage and prerequisite state come from the same Linear snapshot. Missing reports remain unknown. Ready identifiers preserve each team's ix-board queue order; teams are listed deterministically. | Unit tests |
| FR-048-AC-4 | A compact scrollable view shows scope, done/total, focus, live tickets and assignments. Only footer hotkeys use the bright primary blue; their descriptions and separators are muted. The s key toggles source timestamps, hidden by default. The table shows the next ready ticket in tracker order. The scope line shows project done/total counts. Details show title, project, priority, blockers and initiative progress restricted to the reported scope. Internal agents are labeled reported; pane activity is observed only for the matching native session. | Terminal rendering tests |
| FR-048-AC-5 | Sources refresh independently. Failures preserve labeled last snapshots; source ages are available through the status toggle. Errors remain visible when status is hidden. Manual Linear refresh forces a fetch rather than renewing a cache timestamp. | Unit tests |

Automatic Linear reads SHALL run on a 30-minute timer. Rate-limited reads SHALL retry on that timer, with a live countdown to the next attempt. Manual refreshes during rate limiting SHALL coalesce until that attempt; quit SHALL remain responsive. Snapshot age SHALL be labeled stale at the 30-minute polling interval or when a refresh fails. Herdr activity and Redis report reads SHALL run independently once per minute. Herdr observations SHALL remain usable between scheduled reads, with activity becoming unknown after two minutes without a successful observation. This behavior extends FR-048-AC-5.

## Dependencies

- ix-board owns structured scope selection, leaf progress and snapshot queue computation.
- Herdr supplies live pane and native process evidence; Redis stores explicit reports.
