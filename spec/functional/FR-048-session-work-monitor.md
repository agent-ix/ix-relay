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
| FR-048-AC-3 | Resolve initiative/project/ticket scope through ix-board. Progress counts non-canceled, non-duplicate leaf tasks; parent tickets do not inflate totals. Workflow stage and prerequisite state come from the same Linear snapshot. Missing reports remain unknown. | Unit tests |
| FR-048-AC-4 | A compact scrollable view shows scope, done/total, focus, live tickets and assignments. Details show title, project, priority and blockers. Internal agents are labeled reported; pane activity is observed only for the matching native session. | Terminal rendering tests |
| FR-048-AC-5 | Sources refresh independently. Failures preserve labeled last snapshots; source ages remain visible. Manual Linear refresh forces a fetch rather than renewing a cache timestamp. | Unit tests |

## Dependencies

- ix-board owns structured scope selection, leaf progress and snapshot queue computation.
- Herdr supplies live pane and native process evidence; Redis stores explicit reports.
