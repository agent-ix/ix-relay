---
id: FR-070
title: "Every collector source reports its own health"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-009"
    type: "implements"
---
# FR-070: Every collector source reports its own health

## Description

The watcher SHALL export, for each collector source on each host, whether the last read
succeeded, so a source that fails is visible as a failure and never as an absent or
healthy-looking reading.

## Behavior

- The watcher SHALL export `ix_fleet_probe_ok{host,source}` with value 1 when the source's last
  read succeeded within its budget and 0 otherwise.
- The sources SHALL be exactly: `cpu`, `memory`, `temperature`, `compilers`, `pane_tail`,
  `hook_events`, `herdr`, `redis`.
- A series SHALL be exported for every source on every push, including a source that has
  never succeeded, one the platform cannot provide (`temperature` without a sensor: 0) and
  one that timed out. A source failure SHALL NOT remove the series nor any other source's
  series.
- The watcher SHALL omit a source's measured series (for example `ix_fleet_host_busy_ratio`) when
  its source reads 0, so that an old value is never exported as current.
- The watcher SHALL log one line when a source changes between ok and failed, naming the
  source and an error class from a fixed set (`timeout`, `unreadable`, `unsupported`,
  `parse`, `unavailable`), never a path content or pane text.
- The heartbeat of [FR-028](./FR-028-watcher-heartbeat.md) SHALL list the failed sources,
  and `agent-msg doctor` SHALL report a warning `collector_source_failed` naming each.
- Redirecting a source's error output to nowhere, treating a timeout as success, and
  treating a missing file as zero SHALL NOT occur in the collector.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-070-AC-1 | A push built from healthy fixtures carries `ix_fleet_probe_ok` equal to 1 for each of the eight sources with the host attribute. | Test |
| FR-070-AC-2 | Given each source failing in turn (unreadable file, timeout, parse error, unsupported platform), that source's series is exported as 0 and every other source is unchanged. | Test |
| FR-070-AC-3 | Given a source that has never succeeded since start, its series is present with 0 on the first push. | Test |
| FR-070-AC-4 | Given the `cpu` source failing after a successful read, `ix_fleet_host_busy_ratio` is absent from the push, not the old value. | Test |
| FR-070-AC-5 | A transition between ok and failed logs exactly one line with the error class and no path content or pane text, and the heartbeat and doctor name the failed source. | Test |
| FR-070-AC-6 | With the watcher killed, no series is exported; the absence is the signal for an alert on the stale series, and with the watcher running and Redis down, `redis` is 0 and the other sources still report. | Test |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-069](./FR-069-host-collector.md),
  [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md),
  [FR-028](./FR-028-watcher-heartbeat.md), [FR-029](./FR-029-doctor-reports-relay-health.md).
- Consumed by the `ProbeFailing` alert, outside this repository.
