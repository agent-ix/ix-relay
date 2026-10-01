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
| FR-048-AC-3 | Resolve initiative/project/ticket scope through ix-board. Progress counts non-canceled, non-duplicate leaf tasks; parent tickets do not inflate totals. Workflow stage and prerequisite state come from the same Linear snapshot. Missing focus and agent reports remain unknown; absent scope defaults to the monitored team. Ready identifiers preserve each team's ix-board queue order; teams are listed deterministically. | Unit tests |
| FR-048-AC-4 | A compact scrollable view shows scope, done/total, focus, live tickets and assignments. Only footer hotkeys use the bright primary blue; their descriptions and separators are muted. The s key toggles source timestamps, hidden by default. The table shows the next ready ticket in tracker order. The compact scope line names the team and selected groups; Scope rows show per-project done/total counts. Details show title, project, priority, blockers and initiative progress restricted to the reported scope. Internal agents are labeled reported; pane activity is observed only for the matching native session. | Terminal rendering tests |
| FR-048-AC-5 | Sources refresh independently. Failures preserve labeled last snapshots; source ages are available through the status toggle. Errors remain visible when status is hidden. Manual Linear refresh forces a fetch rather than renewing a cache timestamp. | Unit tests |
| FR-048-AC-6 | The a key selects a scrollable Agents view that lists every reported assignment even when scope or Linear is unavailable. Rows show agent, role, ticket, tracker stage when available, reported status, and session-qualified activity. Details show the selected agent identity and evidence source. Source failure labels retained assignments as a last snapshot. View changes reset selection and scroll. | Terminal rendering tests |
| FR-048-AC-7 | The p key selects a scrollable Scope view with a row per scoped initiative and project, showing completed/total leaf tasks, remaining tasks, ready tasks, and unfinished tasks with unfinished or unknown prerequisites. Details show project tracker status and remaining workflow stages. Initiative counts are restricted to scope; overlapping groups are not summed. The t key restores Tickets. | Unit and terminal rendering tests |

| FR-048-AC-8 | If a successful Herdr response has a command payload with a missing field or invalid type, the shared decoder names that payload failure rather than discarding it behind an untagged-envelope error. Herdr reported errors and nonzero exit classifications remain distinct. | Unit tests |

| FR-048-AC-9 | Scope defaults to the bound workspace team key, with an explicit --team override. Initiative, project and ticket selectors narrow that team boundary. Foreign-team tasks never inflate counts; unprojected tasks belong to whole-team scope. Project focus does not display associated initiatives as initiative focus. Explicit --all-team clears narrower selectors without removing agent reports; the command refuses conflicting or absent selections. | Unit, Redis and terminal rendering tests |

| FR-048-AC-10 | Binding accepts omitted or null argv as specified by Herdr protocol 22 while retaining verified native session and nonzero PID requirements. Omitted foreground-process lists provide no binding evidence; malformed argument types are refused. | Unit tests |

Automatic Linear reads SHALL run on a 30-minute timer. Rate-limited reads SHALL retry on that timer, with a live countdown to the next attempt. Manual refreshes during rate limiting SHALL coalesce until that attempt; quit SHALL remain responsive. Snapshot age SHALL be labeled stale at the 30-minute polling interval or when a refresh fails. Herdr activity and Redis report reads SHALL run independently once per minute. Herdr observations SHALL remain usable between scheduled reads, with activity becoming unknown after two minutes without a successful observation. This behavior extends FR-048-AC-5.

## Dependencies

- ix-board owns structured scope selection, leaf progress and snapshot queue computation.
- Herdr supplies live pane and native process evidence; Redis stores explicit reports.
