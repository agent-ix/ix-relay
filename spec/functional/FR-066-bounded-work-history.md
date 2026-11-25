---
id: FR-066
title: Retain bounded session-qualified work transitions
type: FR
relationships:
  - target: ix://agent-ix/ix-relay/FR-048
    type: depends_on
---
# FR-066: Retain bounded session-qualified work transitions

## Description

When a lead reports scope, assignment, removal or explicit pause/resume, the
relay SHALL append an immutable session-qualified transition atomically with
its current-state update. The shared `relay-work-wire` crate owns the version-one
wire contract consumed by metrics readers. Free-text focus and message bodies
remain outside this event stream.

## Behavior

Redis server time and native stream IDs establish event observation and order.
The stream retains at most 4,096 entries, each bounded to 8,192 producer JSON
bytes. History reads enforce a strict seven-day readable horizon using Redis
server time, including during idle intervals. Read-only pages expose retained
coverage, source observation time, truncation and an exclusive next cursor.
History reads require no caller pane, never consume inboxes, and reject malformed
cursors or page sizes outside 1..1000.

Writes prune entries older than seven days and renew the stream's seven-day
inactivity TTL. Physical cleanup is lazy: an entry retained by the last write
can remain stored until that TTL expires, approaching fourteen days after its
observation. Expired entries are excluded from readable coverage; reads do not
trim storage or renew the TTL.

A current snapshot exposes verified identity and current reports, omitting
private focus text. Snapshot observation time does not establish historical
assignment starts. Explicit pause/resume corrects assigned-scope intent and
never completes a tracker ticket or proves execution.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-066-AC-1 | Scope, short stage changes, activity corrections and removals appear in order with stable event IDs; a malformed history storage key prevents current-state mutation. | Test |
| FR-066-AC-2 | Repeated cursor reads return identical retained events without modifying current assignments or mail state; malformed cursors and oversize page requests fail. | Test |
| FR-066-AC-3 | Events preserve verified native session/process identity; focus text is absent from history and bootstrap snapshots. Replacement sessions retain their separate reporting incarnation. | Test |
| FR-066-AC-4 | More than 4,096 writes evict the oldest events; a cursor preceding retained data reports truncation and the retained lower bound. | Test |

## Dependencies

- [FR-048](./FR-048-session-work-monitor.md) owns verified current reporting identity.
