---
id: NFR-003
title: "The watcher isolates failures and never gives up on Redis"
type: NFR
quality_attribute: reliability
relationships:
  - target: "ix://agent-ix/ix-relay/FR-013"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-014"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-028"
    type: "constrains"
---
# NFR-003: The watcher isolates failures and never gives up on Redis

## Statement

While `relay-watcher` runs, the watcher shall keep delivering to every other local
lead when one lead's inbox, herdr call or registration fails, and shall keep
retrying an unreachable Redis with a backoff capped at 60 seconds, exiting only
on a configuration error.

## Scope

- One `relay-watcher` process per machine. The watcher must not depend on a
  service manager restarting it to recover from a Redis or tunnel outage.

## Rationale

A watcher that stalls leaves idle leads unreachable without anyone noticing; a
watcher that dies on one lead's problem punishes every other lead on the machine;
and a watcher that exits on an outage relies on a service manager nobody has
verified is installed (AGE-2033).

## Measurement and Evaluation

| Metric | Target | Threshold | Method |
|--------|--------|-----------|--------|
| Leads delivered to in a tick where one other lead's inbox read fails | all others | all others | fault-injection |
| Watcher exits while Redis is unreachable for 10 minutes | 0 exits | 0 exits | fault-injection |
| Longest wait between reconnection attempts while Redis is unreachable | 60 s | 60 s plus one poll interval | fault-injection |
| Reconnection after Redis returns | next attempt | next attempt | fault-injection |
| Exit status and time to exit on a malformed configuration file | exit 1 at start-up | exit 1 within 1 s of start | integration-testing |
| Notifications per blocked episode | 1 | 1 | unit-testing |

## Verification

A fault-injection harness runs the watcher against a Redis instance it can pause,
drop and restore, and a scripted herdr stub that fails for one pane only, and
asserts the measurements above.

## Dependencies

- **Upstream**: [FR-013](../functional/FR-013-failed-delivery-is-requeued-in-order.md),
  [FR-014](../functional/FR-014-watcher-types-only-into-the-registered-session.md),
  [FR-028](../functional/FR-028-watcher-heartbeat.md)
- Delivered in part by AGE-2013; the first row's dependency on
  [FR-008](../functional/FR-008-tolerant-registry-decoding.md) was closed by
  AGE-2026.
- **Gap on `main`** (AGE-2033): the watcher now retries an unreachable Redis
  forever with a 1 s to 30 s backoff and exits only on a command-line or
  configuration error, so the second and third rows are met in code (the tests
  are unit tests over a fake clock, see the Test Matrix). Still open under
  AGE-2033: the heartbeat ([FR-028](../functional/FR-028-watcher-heartbeat.md))
  and `agent-msg doctor` ([FR-029](../functional/FR-029-doctor-reports-relay-health.md)).
