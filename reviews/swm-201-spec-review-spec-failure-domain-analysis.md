---
id: SR-1866
title: "Failure-domain review of SWM-201 W1 lane ground truth spec PR #52"
type: SpecReview
analysis: failure-domain
scope: "agent-ix/ix-relay; PR #52; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md, spec/functional/FR-067-persistent-delivery-receipts.md, spec/functional/FR-068-hook-activity-events.md, spec/functional/FR-069-host-collector.md, spec/functional/FR-070-collector-source-health.md, spec/functional/FR-071-order-ack-deadline-and-timeout.md, spec/non-functional/NFR-001-hook-time-budget.md"
review_set: subset
---
# SR-1866: Failure-domain review of SWM-201 W1 lane ground truth spec PR #52

## Summary

Ticket: SWM-201. Reviewed against the W1 intent: deterministic ground truth of lane activity in which missing data is never silence or unknown, so idle or blocked lanes cannot hide (overnight 2026-11-23/24: terra probe failed silently 12h50m, UNKNOWN treated as working, queued relay treated as delivered). Checked every path where a missing input becomes absence, unknown or a healthy reading; the strength of Acted as evidence; hook latency vs never blocking a turn; Windows/WSL; clock and identity confusion.

Done well: FR-070 makes every source a 1/0 series present on every push, failed sources drop their measured series, error output to nowhere and timeout-as-success are forbidden; dropped hook events are recorded as failures and never replayed as fresh; pane text and message text are never stored; TimedOut is idempotent across watcher restarts; dead-pane orders time out from enqueue.

## Scope examined

- `FR-045` new catalogue rows ix.fleet.lane.state, lane.last_activity, lane.blocked, host.busy_ratio, host.capacity_slots (examined).
- `FR-067` Acted statements and `FR-067-AC-8` (new), `FR-067-AC-9` (examined).
- `FR-068` Failure behavior, `FR-068-AC-8` (examined).
- `FR-069` Host measurements, Runtime (examined).
- `FR-070` Behavior, `FR-070-AC-6` (examined).
- `FR-071` Behavior (examined).
- `NFR-001` new paragraph and benchmark row (examined).

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | Missing data becomes unknown or absence: a lane with no event exports lane.state=unknown and no last_activity series, and the lane set (one series per local lane) is never defined. If the set is derived from events, a lane whose hooks never fire (plugin not wired, hook failures, Codex build) is absent entirely. Intent is the opposite: every registered lane (registrations plus herdr agent panes) appears, and no evidence within N minutes is idle plus an explicit defect series, never unknown. | FR-045 lane.state and lane.last_activity rows, FR-045-AC-30 |
| FND-002 | high | False idle during long tool calls: working requires a pre_tool/post_tool/prompt within 5 minutes, so a lane running one 30-minute cargo build or test emits pre_tool and then nothing, and reads idle after 5 minutes. The newest event being pre_tool with no matching post_tool (a tool in flight) must count as working, bounded by a maximum, and/or be fused with FR-069 compiler attribution. | FR-045 lane.state row, FR-068 |
| FND-003 | medium | Acted is weak evidence. Mail is delivered by PostToolUse/Stop hooks inside an active turn, so the very next tool call in the same session records Acted within seconds whether or not the lane takes up the order; Acted only separates dead or idle sessions from live ones. FR-067 states this honestly, but the plan uses Acted to confirm fleet assign. Either add an order-scoped condition (e.g. optional expected ticket matched against the lane focus from hook branch/cwd, or the first prompt of a new turn after delivery) or name the state for what it proves and require the consumer to pair it with focus. | FR-067 Acted statements, FR-071 |
| FND-004 | medium | No fail-fast on Redis outage: PreToolUse and PostToolUse each may spend up to 900 ms when Redis is blackholed, so every tool call of every lane on the host is delayed about 1.8 s for the whole outage. NFR-001 bounds one call only. PreToolUse should have the 300 ms class budget and the handler should skip event writes for a backoff window after a recorded failure (still counting drops). | NFR-001 new paragraph and row, FR-068 Failure behavior |
| FND-005 | medium | Windows/WSL assumption unstated: the collector runs inside WSL, so it sees only WSL2 VM processes and VM CPU (/proc/stat), and temperature is never readable. If any terra lane, harness or cargo runs as a native Windows process, its compilers and load are invisible and agent-msg hook needs a Windows build. The spec must state that terra lanes and herdr run inside WSL (and test that), or add a Windows source reported as unsupported. | FR-069 Description, FR-069-AC-3 |
| FND-006 | medium | capacity_slots is absent when no capacity is entered and no probe_ok source covers capacity, so the unset capacity found on all four hosts is silence; HostUnderutilized cannot fire. Add a capacity source (0 when unset) or an explicit capacity_set series. | FR-045 capacity_slots row, FR-070 source list |
| FND-007 | medium | The hook_events source is listed but what a successful read means is undefined. If it means Redis readable, a host where every lane's hooks are silently broken (the terra class of failure) still reports hook_events=1. Define it so that live agent panes with no hook event for N minutes, or a non-empty hook-failure spool, set it to 0. | FR-070 source list |
| FND-008 | low | Acted ordering compares event ts (writing host clock) with the Delivered time; skew between hosts or a fast clock can record a false Acted (silence) or miss one (false TimedOut). Order by Redis TIME or stream ids inside the atomic write. | FR-067 Acted statements, FR-068 ts |
| FND-009 | low | Lanes past the 50-lead bound fold into other, so an idle lane can lose its identity in the series; acceptable at today's lane counts but should be a counted drop with a doctor warning rather than silent folding. | FR-045-AC-30 |

## Verdict

CHANGES REQUESTED. FND-001 and FND-002 contradict the purpose of W1 (missing data must never read as unknown, and a working lane must not read idle). FND-003..007 are real gaps against the overnight failure classes and should be closed in this PR; FND-008/009 can be stated as known limits.

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-010 | medium | `no_events` and host `hook_events` flag a legitimately idle lane or host: any lane whose newest event is older than `no_events_after` is a defect, and a host whose lanes all stopped overnight reads `hook_events`=0, which turns ProbeFailing into noise and invites silencing. Lead decision: a lane counts as unhealthy only when its last known state is not idle (an open `pre_tool`, herdr reporting working, or a `prompt` with no later `stop`) and it then emits no event for `no_events_after` (default 30 minutes). A lane whose newest event is `stop`, `idle` or `session_end` is quiet, not unhealthy. FR-070-AC-9 currently asserts the opposite and must change. | FR-070 Health series, FR-070-AC-9, FR-045 lane.no_events row |
| FND-011 | low | FR-072 accepts writes only from a caller with the orchestrator role in the relay configuration, but the relay does not authenticate callers (every participant that can reach Redis is trusted, SR-003 FND-006). State that the role check guards against accidental writes by lanes and is not authentication. | FR-072 Behavior, FR-072-AC-3 |

## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | The lane registry is the union of herdr agent panes and relay registrations (FR-069, AC-11); `unknown` is removed; a lane with no events is `idle` with `no_events`; every registry lane has state and last-activity series. |
| FND-002 | fixed | A `pre_tool` with no later `post_tool` within `max_tool_runtime`, or an attributed compiler, counts as `working`; past that limit it is `stuck`; FR-045-AC-30 covers both. |
| FND-003 | fixed | FR-067 says `Acted` is not acceptance; FR-071 adds `--expect-ticket`, exposes focus and gives acceptance to the consumer, which accepts only on a focus match. |
| FND-004 | fixed | `PreToolUse` budget 300 ms; per-host exponential backoff from 5 s to 5 min; skips counted in `ix.relay.hook.activity_skipped`; FR-068-AC-11 and NFR-001 rows. |
| FND-005 | fixed | FR-069 states terra lanes, herdr and cargo run inside WSL; `native_windows_agents` makes a native process a defect (FR-069-AC-14). |
| FND-006 | fixed | `capacity` source (0 when unset) plus the always-exported `capacity_configured` (FR-070-AC-8). |
| FND-007 | fixed | `hook_events` is now defined (per-lane age against `no_events_after`, plus activity failures), so a host whose hooks are broken reads 0. Its handling of idle lanes is the subject of new finding FND-010. |
| FND-008 | fixed | Acted ordering and deadlines use relay server (Redis) time; FR-067-AC-12 covers clocks 10 minutes behind and ahead. |
| FND-009 | fixed | No folding: `ix.fleet.lane.dropped` plus the doctor warning `fleet_lanes_dropped` (FR-045-AC-31). |
