---
id: FR-051
title: "Monitor planned focus and actual work"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
---
# FR-051: Monitor planned focus and actual work

## Description

The dashboard SHALL distinguish planned focus, adopted focus and reported activity. When planned scope is unavailable, the dashboard SHALL identify any whole-team fallback as context rather than presenting it as the lead's declared focus.

## Behavior

The existing `team-leader-dashboard --lead <full-address> [--team <key>] [--session <id>]` remains the entry point. The dashboard displays source address, source availability, latest and adopted revision, adoption state, selected project/milestone names and per-session activity. The team override changes tracker lookup only; it never rewrites the published plan's team or broadens its scope. A mismatch is a visible error. Standalone local scope/activity reports continue to render without a planner.

When viewing a planner, the dashboard lists only its explicitly registered followers, each with adoption and source state. It aggregates task progress by union of scoped leaf IDs, and assignments by follower binding plus assignment ID; a task may legitimately have multiple assignments. Replaced follower sessions are labeled unavailable/replaced, not current workers. The accepted relationship snapshot is bounded to 128 followers; registering a 129th fails atomically. Follower indexes are removed on unfollow and expire with relationship retention; stale entries are labeled, never counted as current.

Tickets, Agents and Scope views retain keyboard navigation and compact-pane layouts. A local absent scope displays `Whole-team fallback — lead focus unknown`; awaiting adoption displays unknown scope rather than team totals. A configured Layer 4 milestone displays its own counts with project and milestone labels; broader team context is explicitly separate. Activity summary is never a substitute for tracker scope.

The metric adapter consumes explicit planned/adopted revisions and versioned ix-board evidence. Adoption lag is the latest-minus-adopted revision and time since the earliest unadopted publication still available; if history is missing, time lag is unknown. Work drift evaluates events against the adoption effective at the event time, not today's scope. Historical adoption/publication events are retained for at least the longest configured metric window within the 30-day maximum. Configured windows beyond retained evidence are rejected or reported incomplete; discarded history is never interpreted as no activity.

ix-board owns out-of-scope work, stalls, ticket churn and priority-goal evaluation. A metric includes its named reporting window, thresholds, source timestamps, completeness diagnostics and evidence references. State transitions/PR/commit evidence must be completely paginated or labeled incomplete. Shared account authorship alone cannot attribute a commit/PR to a lead. A goal's transitive active prerequisites count as supporting progress and cannot also be reported as crowding-out work. Unknown assignee identity yields an unknown worker count; issue coverage is labeled a proxy, never verified headcount.

The UI SHALL distinguish reported assignment state, observed matching-session activity, tracker workflow stage and metric evidence. Partial evidence can show known counts alongside missing-source diagnostics but cannot produce a healthy verdict for an unevaluated signal. Source errors and retained snapshot labels remain visible when timestamps are hidden. Linear, reports and Herdr refresh independently on the existing intervals, with responsive quit and no tracker writes.

Repository boundaries: ix-relay owns relationships, reports and presentation; ix-board owns tracker selection and metric computation; dev-team owns publication/follow/adopt instructions. Historical evidence collection and metric wiring are later delivery tickets, not claims about the installed dashboard. Renames and source rebinds require explicit historical linkage; no retrospective reassignment of old events is inferred.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-051-AC-1 | Given standalone local project scope and activity, the monitor displays only that scope; given no scope it visibly labels whole-team fallback and lead focus unknown. | Test |
| FR-051-AC-2 | Given adopted P/Layer 4 scope, ticket and progress rows exclude Layer 5 and foreign-team tasks; project/milestone labels remain visible in compact mode. | Test |
| FR-051-AC-3 | Given followers L1 and L2 sharing a ticket, task totals count it once while both assignment identities remain visible; unrelated sessions never appear as followers. | Test |
| FR-051-AC-4 | Given 128 registered followers, the list is bounded and valid; registering follower 129 fails atomically without overwriting any relationship. | Test |
| FR-051-AC-5 | Given pending adoption, source outage and verified replacement, identity and revision states render distinctly while retained data is labeled; team override mismatch never broadens scope. | Test |
| FR-051-AC-6 | Given an event before revision adoption and one after it, deterministic replay evaluates each against its contemporaneous adopted scope; missing adoption history yields unknown attribution. | Test |
| FR-051-AC-7 | Given incomplete pagination, ambiguous shared-account attribution or absent assignee identity, affected metrics name the missing input and do not claim healthy/zero activity/verified workers. | Test |
| FR-051-AC-8 | Given a priority goal with an active transitive blocker, blocker work contributes to goal progress and is absent from crowding-out results; cyclic blocker traversal terminates. | Test |
| FR-051-AC-9 | Given narrow and wide terminal sizes, Tickets/Agents/Scope navigation, details, error labels and quit remain usable while any source is blocked. | Test |
| FR-051-AC-10 | Given latest revision 4 and adopted 2, revision lag is 2; elapsed lag is unknown if the earliest missed publication time is unavailable rather than zero. | Test |
| FR-051-AC-11 | Given report windows exceeding retained history, the metric reports incomplete history or rejects configuration instead of a complete verdict. | Test |

## Dependencies

[FR-048](./FR-048-session-work-monitor.md), [FR-049](./FR-049-revisioned-planner-focus.md) and [FR-050](./FR-050-planner-follow-and-adoption.md). Existing ix-board drift and crowding work: SWM-4 and SWM-64; blocker-aware correctness: SWM-84. Delivery: SWM-131 through SWM-135.
