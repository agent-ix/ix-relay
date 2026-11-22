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

When session-bound monitoring is requested, the monitor SHALL bind an explicit
Herdr address to one measured native harness session and process incarnation.
Scope, current focus, and agent assignments
SHALL be reported explicitly by the lead. Open tracker tickets alone do not
prove that an agent is working on them.

Explicit department scope and saved-report startup are governed by
[FR-059](./FR-059-department-monitor-adapter.md). An unbound department view does
not establish a session binding or authorize a work report.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-048-AC-1 | Resolve exactly one live agent by address, verify its native session and process, refuse missing/conflicting identity, and require a restart to bind a replacement. Reports are isolated by machine, native session, and process incarnation. | Unit and Redis integration tests |
| FR-048-AC-2 | Reporting atomically updates scope, focus, or one agent, preserving concurrent assignments. Multiple agents can share a ticket. At most 128 agents are retained; input and stored records are bounded and validated. Reporting does not register subagents, consume inboxes, or change Linear. | Redis integration tests |
| FR-048-AC-3 | Resolve initiative/project/ticket scope through ix-board. Progress counts non-canceled, non-duplicate leaf tasks; parent tickets do not inflate totals. Workflow stage and prerequisite state come from the same Linear snapshot. Missing focus and agent reports remain unknown; absent scope defaults to the monitored team. Ready identifiers preserve each team's ix-board queue order; teams are listed deterministically. When `agent-msg work` resolves scope or verifies tickets for a report, the read completes within 120 seconds of the credentials being ready, on a snapshot at most five minutes old; if that cannot be established it fails and writes no report. | Unit tests |
| FR-048-AC-4 | A compact scrollable view shows scope, done/total, focus, live tickets and assignments. Header labels and separators are muted while reported values remain bright. The current view is named by the View selector of [FR-065](./FR-065-dashboard-view-selector-and-help.md), with no persistent tab strip. Ready and waiting for input appear as ○ and ◇ in the identity row; status reveals their activity text. Only footer, View selector, dropdown and help hotkeys use the bright primary blue; their descriptions and separators are muted. The s key toggles source timestamps, hidden by default. The table shows the next ready ticket in tracker order. The compact scope line names the team and selected groups; Scope rows show per-project done/total counts. Details show title, project, priority, blockers and initiative progress restricted to the reported scope. Internal agents are labeled reported; pane activity is observed only for the matching native session. | Terminal rendering tests |
| FR-048-AC-5 | Sources refresh independently. Failures preserve labeled last snapshots; source ages are available through the status toggle. Errors remain visible when status is hidden. Manual Linear refresh forces a fetch rather than renewing a cache timestamp. | Unit tests |
| FR-048-AC-6 | The a key selects a scrollable Agents view that lists every reported assignment even when scope or Linear is unavailable. Rows show agent, role, ticket, tracker stage when available, reported status, and session-qualified activity. Details show the selected agent identity and evidence source. Source failure labels retained assignments as a last snapshot. View changes reset selection and scroll. | Terminal rendering tests |
| FR-048-AC-7 | The p key selects a scrollable Scope view with a blank row below Focus, an Initiative/project column heading, no redundant table title, and one terminal row per scoped initiative and project, prefixed by 🎯 and 📁 respectively, with no extra type or repeated count-label row, showing completed/total leaf tasks, remaining tasks, ready tasks, and unfinished tasks with unfinished or unknown prerequisites. Details show project tracker status and remaining workflow stages. Initiative counts are restricted to scope; overlapping groups are not summed. The t key restores Tickets. | Unit and terminal rendering tests |

| FR-048-AC-8 | If a successful Herdr response has a command payload with a missing field or invalid type, the shared decoder names that payload failure rather than discarding it behind an untagged-envelope error. Herdr reported errors and nonzero exit classifications remain distinct. | Unit tests |

| FR-048-AC-9 | Scope defaults to the bound workspace team key, with an explicit --team override. Initiative, project and ticket selectors narrow that team boundary. Foreign-team tasks never inflate counts; unprojected tasks belong to whole-team scope. Project focus does not display associated initiatives as initiative focus. Explicit --all-team clears narrower selectors without removing agent reports; the command refuses conflicting or absent selections. | Unit, Redis and terminal rendering tests |

| FR-048-AC-10 | Binding accepts omitted or null argv as specified by Herdr protocol 22 while retaining verified native session and nonzero PID requirements. Omitted foreground-process lists provide no binding evidence; malformed argument types are refused. | Unit tests |

Automatic Linear reads SHALL run on a 30-minute timer. Rate-limited reads SHALL retry on that timer, with a live countdown to the next attempt. Manual refreshes during rate limiting SHALL coalesce until that attempt; quit SHALL remain responsive. Snapshot age SHALL be labeled stale at the 30-minute polling interval or when a refresh fails. Herdr activity and Redis report reads SHALL run independently once per minute. Herdr observations SHALL remain usable between scheduled reads, with activity becoming unknown after two minutes without a successful observation. This behavior extends FR-048-AC-5.

The following rules extend FR-048-AC-5. The ix-board service reports each Linear read attempt through three typed sources. The decision state belongs to a quota decision and is local allocation held, server reserve held, server backoff, evidence unavailable, request not admissible, or available when no hold is reported. The refresh outcome is cache reused, refreshed, quota refused, source failed, refresh lock refused or unknown. The legacy errors are the rate-limited and local-budget-exhausted errors, which the service reports with an unknown refresh outcome. A decision lists blockers, each with a state, a dimension and values, and may report pending pre-fence reservations. A waiting cause is a server backoff, a local allocation hold, a server reserve hold or the legacy local-budget error when the service reports that error. A non-waiting cause is unavailable or quarantined evidence, a request that is not admissible, a refusal with no reported hold, a refused refresh lock, a source failure, or an unknown outcome that reports neither a legacy cooldown nor the legacy local-budget error.

If the shared quota protection refuses a Linear read, the monitor SHALL label the refresh by the decision state of the refusal. If the service reports a typed outcome and a legacy error for one attempt, the monitor SHALL classify the attempt by the typed outcome first. The monitor SHALL label a refresh rate-limited only for the server backoff decision state or the legacy rate-limited error. If the refresh outcome is unknown and the service reports a legacy cooldown, the monitor SHALL treat the refresh as a server backoff. If the refresh outcome is unknown and a failed refresh has no legacy cooldown, the monitor SHALL label the failure as having an unknown cause. If the refresh outcome is unknown and a stale snapshot has neither a read age nor a legacy cooldown, the monitor SHALL label the snapshot freshness unconfirmed. The monitor SHALL NOT classify a refresh from error text.

When the cause is a waiting cause, the monitor SHALL count down to its next scheduled attempt. When the cause is a waiting cause, the monitor SHALL coalesce manual refreshes until that attempt. When the cause is a non-waiting cause, the monitor SHALL NOT show a countdown. When the cause is a non-waiting cause, the monitor SHALL NOT coalesce manual refreshes. The monitor SHALL treat its own 30-minute next attempt and the earliest reevaluation time of a decision as separate times. The monitor SHALL NOT present either time as a promise that a read will be admitted.

If the service provides a quota decision, with or without a stored snapshot, the monitor SHALL report each blocker of the decision with its reported dimension and values. If the service provides a quota decision, with or without a stored snapshot, the monitor SHALL report the attempt and the earliest reevaluation time of the decision. If a decision omits a reevaluation time or a value, the monitor SHALL show it as unknown. The monitor SHALL derive each reported action from each reported blocker and pending reservation, not from the primary decision state alone. If a refresh fails and a snapshot is stored, the monitor SHALL keep the snapshot with its original source time. In the Tickets, Agents and Scope views, the monitor SHALL open a scrollable tracker inspector on the `e` key that shows every reported blocker, value, attempt, reevaluation time and action.

## Dependencies

- ix-board owns structured scope selection, leaf progress and snapshot queue computation.
- Herdr supplies live pane and native process evidence; Redis stores explicit reports.
- [FR-059](./FR-059-department-monitor-adapter.md) adds department views without
  relaxing the session and process identity requirements for bound activity or reporting.
- [FR-065](./FR-065-dashboard-view-selector-and-help.md) adds the View selector and
  F1 help. The `t`, `a` and `p` keys remain compatibility shortcuts.
- [FR-061](./FR-061-named-focus-definitions.md) through
  [FR-064](./FR-064-named-focus-dashboard-reader.md) add named, shared focuses and
  their per-session activation. They leave this requirement's per-session scope,
  free-text focus and assignments, and `work show` output, unchanged. For a session
  in named mode only, FR-064 replaces the absent-scope whole-team default of AC-3
  and AC-9 with "no active focus".
