---
id: FR-028
title: "The watcher publishes a heartbeat"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-028: The watcher publishes a heartbeat

## Description

While `relay-watcher` runs, the watcher shall refresh a heartbeat record for its
machine on the bus after every successful sweep tick, and after a message-wake or retry
tick that is at least one second after the last write, with an expiry of three sweep
intervals and at least 30 seconds, so that a missing heartbeat means no watcher
has ticked on that machine recently. A tick runs when a message is sent
([FR-038](./FR-038-watcher-wakes-when-a-message-is-sent.md)) and at least every
sweep interval (`--sweep-secs`, default 15 s), so the heartbeat is never older
than a sweep interval plus the tick's own duration; AC-4's 20 s is set on the
command line.

## Behavior

- The heartbeat record, a JSON string under `<prefix>health:watcher:<machine>`, shall
  carry the machine name, the watcher's process id and version, its start
  time, the time of its last successful tick, the time of its last backup sweep,
  its sweep interval, whether herdr passes its version and shape check
  ([FR-036](./FR-036-herdr-version-and-shape-check.md)), what failed and whether
  the watcher types into no pane because of it (`herdr_blocks_delivery`; a check
  that could not run, such as a timeout, is neither), whether a
  Redis connection error was ridden out since the previous heartbeat (`redis_ok` is
  `false` for that one heartbeat: a flap the expiry would hide), how many
  leads the tick looked at, and the kind and time of its last failed tick (a
  stable code, never message text). The source pins the field list
  (`the_heartbeat_has_exactly_the_documented_fields`).
- The watcher shall write, in the same request as the heartbeat, its tick time to
  a non-expiring marker (`<prefix>health:watcher-seen`, one field per machine), so
  that `doctor` can tell a watcher that died an hour ago from one that never ran.
- A reader of the record shall tolerate members it does not know, so that a newer
  watcher's heartbeat still shows an older `agent-msg` that the watcher is alive.
- When the heartbeat write fails with a connection error, the watcher shall fail
  the tick and redial, like any other lost connection.
- If the heartbeat write fails for another reason (a key of the wrong type), then
  the watcher shall log it once and keep delivering.
- A watcher started with `--once` shall publish no heartbeat.
- If a tick fails, then the watcher shall leave the heartbeat unrefreshed, so the
  record expires if failures persist.
- The watcher shall exit on a configuration error only at start-up, before its
  first tick, so it has no heartbeat record to delete then.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-028-AC-1 | Given a watcher on `hub`, the heartbeat record for `hub` exists and its last-tick time is at most 20 s old at any sampled moment over one minute, whether or not any message is sent. | Test |
| FR-028-AC-2 | Given the watcher on `hub` is stopped with SIGKILL, the heartbeat record for `hub` no longer exists 31 s later. | Test |
| FR-028-AC-3 | After a 60 s Redis outage seen from `hub`, the heartbeat record for `hub` exists again within one sweep interval of Redis returning, and the watcher process is the same one that was running before the outage. | Test |
| FR-028-AC-4 | Given a sweep interval of 20 s, the heartbeat record's expiry is at least 60 s. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Constrained by [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md).
- Read by [FR-029](./FR-029-doctor-reports-relay-health.md), which shows every
  member: the process's uptime, its last sweep, a ridden-out flap and the last
  failed tick.
- Delivered by AGE-2033. Mixed versions: no earlier binary reads the record, so it
  is additive; an old watcher publishes none, and `doctor` reports that machine's
  watcher as having no heartbeat (worded "watcher stopped, or older than the
  heartbeat"). `herdr_blocks_delivery` is absent from an older watcher's record and
  reads as `false`.
