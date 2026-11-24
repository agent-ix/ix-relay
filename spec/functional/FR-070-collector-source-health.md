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

The watcher SHALL export, for each collector source that its host profile expects, whether
the last read succeeded, so a source that fails is visible as a failure and never as an
absent or healthy-looking reading. A source that the host cannot provide and declares so is
reported as unavailable, which is not a failure.

## Behavior

### Sources and host profile

- The sources SHALL be exactly: `cpu`, `memory`, `temperature`, `capacity`, `compilers`,
  `pane_tail`, `hook_events`, `herdr`, `redis`.
- The host profile (`[fleet] expected_sources` and `unavailable_sources` in the watcher
  configuration, per [FR-026](./FR-026-configuration-precedence.md)) SHALL list the sources
  the host is expected to provide. The default expected set SHALL be every source except
  `temperature`; a host with a readable hwmon sensor declares `temperature`. A source
  listed as unavailable SHALL be exported through `ix.fleet.probe_unavailable` and SHALL NOT
  be exported as failed.

### Health series

- The watcher SHALL export `ix_fleet_probe_ok{host,source}` with value 1 when the source's
  last read succeeded within its budget and 0 otherwise, for every expected source on every
  push, including a source that has never succeeded and one that timed out. A source
  failure SHALL NOT remove its own series nor any other source's series.
- The watcher SHALL omit a source's measured series (for example
  `ix_fleet_host_busy_ratio`) while that source reads 0, so that an old value is never
  exported as current.
- The `capacity` source SHALL read 0 while no capacity is entered for the host, and
  `ix.fleet.host.capacity_configured` SHALL be 0 then, so that unset capacity is a standing
  failure and not silence.
- The watcher SHALL count a registry lane toward `hook_events` unhealthy only when both hold: its last known state is non-idle (an open `pre_tool`, a herdr status of working, or a `prompt` with no later `stop`), and it then emits no activity event for `no_events_after` (default 30 minutes). A lane whose newest event is `stop`, `idle` or `session_end` is quiet, not unhealthy.
- The watcher SHALL export `ix.fleet.lane.no_events` for each lane counted unhealthy, and the host's `hook_events` SHALL read 0 only when at least one lane was non-idle and every such lane is counted unhealthy, or when the hook failure stream holds an `activity` failure newer than the newest event of every lane. A host whose working lanes emit nothing is therefore unhealthy, and a host of quiet lanes is not.
- The watcher SHALL log one line when a source changes between ok and failed, naming the
  source and an error class from a fixed set (`timeout`, `unreadable`, `unsupported`,
  `parse`, `unavailable`), never a path content or pane text.
- The heartbeat of [FR-028](./FR-028-watcher-heartbeat.md) SHALL list the failed sources,
  and `agent-msg doctor` SHALL report a warning `collector_source_failed` naming each.
- The collector SHALL NOT discard a source's error output, treat a timeout as success or
  treat a missing file as zero.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-070-AC-1 | A push built from healthy fixtures carries `ix_fleet_probe_ok` equal to 1 for each expected source with the host attribute, and none for a source the profile declares unavailable. | Test |
| FR-070-AC-2 | Given each expected source failing in turn (unreadable file, timeout, parse error, unsupported platform), that source's series is exported as 0 and every other source is unchanged. | Test |
| FR-070-AC-3 | Given a source that has never succeeded since start, its series is present with 0 on the first push. | Test |
| FR-070-AC-4 | Given the `cpu` source failing after a successful read, `ix_fleet_host_busy_ratio` is absent from the push, not the old value. | Test |
| FR-070-AC-5 | A transition between ok and failed logs exactly one line with the error class and no path content or pane text, and the heartbeat and doctor name the failed source. | Test |
| FR-070-AC-6 | With the watcher killed, no series is exported, so a consumer's alert on stale series fires; with the watcher running and Redis down, `redis` is 0 and the other sources still report. | Test |
| FR-070-AC-7 | Given a macOS or WSL host whose profile omits `temperature`, the push shows `probe_unavailable` for `temperature`, no `probe_ok` 0 for it and no failed-source warning; given a profile that expects it and an unreadable sensor, `probe_ok` for `temperature` is 0. | Test |
| FR-070-AC-8 | Given no entered capacity, `capacity` is 0 and `capacity_configured` is 0; given entered capacity, both are 1 and `capacity_slots` is exported. | Test |
| FR-070-AC-9 | Given lanes whose last state is an open `pre_tool`, herdr working, and a `prompt` with no later `stop`, each with no event for longer than `no_events_after`, each has `ix.fleet.lane.no_events` 1 and the host's `hook_events` is 0; given lanes whose newest event is `stop`, `idle` or `session_end`, they carry no `no_events` and `hook_events` is 1; given one lane with a recent event among silent working lanes, `hook_events` is 1 and only the silent working lanes carry `no_events`. | Test |
| FR-070-AC-10 | Given a pane class read from a `pane_tail` source that reads 0, or a class older than one scan interval, `ix.fleet.lane.blocked` carries no value from it. | Test |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-069](./FR-069-host-collector.md),
  [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md),
  [FR-028](./FR-028-watcher-heartbeat.md), [FR-029](./FR-029-doctor-reports-relay-health.md).
- Consumed by the `ProbeFailing` alert, outside this repository.
