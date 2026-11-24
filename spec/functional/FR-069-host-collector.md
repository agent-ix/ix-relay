---
id: FR-069
title: "relay-watcher measures host load and lane evidence natively"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-009"
    type: "implements"
---
# FR-069: relay-watcher measures host load and lane evidence natively

## Description

`relay-watcher` SHALL measure, on its own host, CPU busy ratio, memory, temperature
where available, compiler processes attributed to lanes, and the state of each local
lane's pane, and SHALL publish the results as the series of
[FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md). It runs natively on every host,
including inside WSL on Windows hosts, with no ssh from another machine, and replaces the
shell and Python probes previously kept outside this repository. The collector is
implemented in Rust.

## Behavior

### Host measurements

- The collector SHALL compute the CPU busy ratio as the non-idle share of CPU time
  between two samples at least 1 second apart: on Linux and WSL from the delta of
  `/proc/stat` (idle plus iowait counted as idle), on macOS from the host CPU tick counters
  of the host statistics interface. A single sample SHALL NOT produce a ratio.
- The collector SHALL report memory as used and total bytes (Linux `MemAvailable` based,
  macOS from host virtual memory statistics).
- Where a temperature sensor is readable (Linux hwmon `Tctl` or the highest CPU package
  sensor), the collector SHALL report the highest reading in degrees Celsius; where none is
  readable the source is reported as `unavailable` by
  [FR-070](./FR-070-collector-source-health.md), never as zero.
- The collector SHALL report `host_capacity_slots` from the operator-entered server
  capacity of [FR-058](./FR-058-explicit-server-capacity.md), and SHALL NOT estimate it.

### Compiler attribution

- The collector SHALL list processes whose executable name is a known compiler or build
  driver (`rustc`, `cargo`, `cc`, `clang`, `gcc`, `ld`, `tsc`, `node` under a build
  script, configurable list) with their working directory.
- The collector SHALL attribute a compiler process to a lane when its working directory is
  inside the `cwd` most recently reported for that lane by
  [FR-068](./FR-068-hook-activity-events.md), choosing the longest matching `cwd`, and SHALL
  count it as `unattributed` otherwise.
- A lane's `cwd` and `branch` mapping SHALL expire after 6 hours without an event.

### Pane-tail scan

- For each local lane the collector SHALL read at most the last 80 lines (at most 16 KiB)
  of its pane and classify them into: `dialog` (a permission or confirmation prompt
  awaiting input), `login` (an authentication prompt), `usage_limit` (a usage or rate
  limit notice), `queued_question` (a queued follow-up inputs notice), `wait_loop` (a
  sleep or poll wait repeated over the tail) or `none`.
- The patterns SHALL be a typed, tested catalogue in this repository, one entry per class
  and harness, and a classification SHALL name the class only; the matched text SHALL NOT
  be stored or exported.
- A hook event of kind `blocked` or `idle` newer than the pane tail SHALL take precedence
  over a pane classification.

### Runtime

- The collector SHALL run inside the watcher tick under the bounds of
  [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md): a slow or failing source
  SHALL NOT delay the tick beyond 500 ms per source nor end the watcher.
- The collector SHALL NOT bind a listening socket and SHALL NOT shell out through a shell
  interpreter; it reads files and runs herdr only through the bounded runner.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-069-AC-1 | Given two `/proc/stat` fixtures one second apart with known tick deltas, the CPU busy ratio equals the expected non-idle share; given one sample only, no ratio is produced. | Test |
| FR-069-AC-2 | Given macOS host CPU tick fixtures, the ratio equals the expected value, and the same code path is selected by target rather than a runtime shell call. | Test |
| FR-069-AC-3 | Given a WSL `/proc` fixture, the Linux path produces the ratio and memory without ssh and without spawning a process. | Test |
| FR-069-AC-4 | Given a host with no readable temperature sensor, no temperature value is produced and the temperature source is reported unavailable rather than zero. | Test |
| FR-069-AC-5 | Given compiler processes whose working directories are inside, nested within, outside and equal to lane `cwd`s, each is attributed to the lane with the longest matching `cwd` or counted unattributed. | Test |
| FR-069-AC-6 | Given a lane mapping older than 6 hours, its compilers are counted unattributed. | Test |
| FR-069-AC-7 | Given pane-tail fixtures for a permission dialog, a login prompt, a usage limit, "Queued follow-up inputs", a repeated sleep loop and plain output, each classifies to `dialog`, `login`, `usage_limit`, `queued_question`, `wait_loop` and `none` for both harnesses; a tail over 16 KiB is read only to the bound. | Test |
| FR-069-AC-8 | Given a pane tail classified `dialog` older than a `idle` hook event, the lane is reported with the hook event's state; no exported series or log line contains the matched pane text seeded with a secret marker. | Test |
| FR-069-AC-9 | Given a source that hangs, the tick completes within its budget plus 500 ms and the other sources still report. | Test |
| FR-069-AC-10 | The collector crate contains no Python file, no shell script and no spawn of a shell interpreter, and the retired probe scripts are not referenced by any build or service file in this repository. | Inspection |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-068](./FR-068-hook-activity-events.md),
  [FR-070](./FR-070-collector-source-health.md), [FR-058](./FR-058-explicit-server-capacity.md),
  [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md),
  [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md).
- Replaces `probe-linux.sh`, `host-probe.sh`, `util.sh`, `hosts-status.sh`,
  `lane-status.sh` and the matching `_lead-bin` scripts. Those live outside this repository
  and are retired by the program owner once this lands.
