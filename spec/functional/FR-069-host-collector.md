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
where the host profile expects it, compiler processes attributed to lanes, and the state
of each lane's pane, and SHALL publish the results as the series of
[FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md). It runs natively on every host,
including inside WSL on the Windows host (terra), with no ssh from another machine, and
replaces the shell and Python probes previously kept outside this repository. The
collector is implemented in Rust, and its host readers live in the `host-probe` crate, which
holds no Redis or OTLP code so the build scheduler can read the same probes.

## Behavior

### Lane registry

- The collector SHALL keep a lane registry for its host: every herdr pane that runs an
  agent, plus every lead registration of this host in the relay. A lane is identified by
  its address `server:workspace:tab`, which every series and event of this feature carries
  as the join key.
- Each watcher SHALL publish its host's lane list to Redis under a lease: a record valid for 90 seconds and refreshed about every 30 seconds, as the fenced herdr directory does. The fleet registry SHALL be the union of the unexpired host lists and the relay registrations. When a host's lease has expired, its lanes SHALL stay listed with `stale` set and SHALL NOT be dropped.
- Every lane in the registry SHALL appear in the lane series of FR-045 on every push,
  whether or not it has produced any hook event. A lane leaves the registry only when
  neither source lists it.

### Host measurements

- The collector SHALL compute the CPU busy ratio as the non-idle share of CPU time
  between two samples at least 1 second apart: on Linux and WSL from the delta of
  `/proc/stat` (idle plus iowait counted as idle), on macOS from the host CPU tick counters
  of the host statistics interface. A single sample SHALL NOT produce a ratio.
- The collector SHALL report memory as used and total bytes (Linux `MemAvailable` based,
  macOS from host virtual memory statistics).
- Where the host profile ([FR-070](./FR-070-collector-source-health.md)) expects a
  temperature, the collector SHALL read the highest hwmon `Tctl` or CPU package sensor in
  degrees Celsius; a host whose profile does not expect one reports the source as
  unavailable and never as zero.
- The collector SHALL read Linux pressure stall information from `/proc/pressure/cpu`, `/proc/pressure/io` and `/proc/pressure/memory` (`some` and, where the kernel gives it, `full`, each as the 10 and 60 second averages in percent) and export it as `ix.fleet.host.pressure` by `resource` and `kind`. A host without PSI (macOS, or a kernel built without it or booted with it disabled) SHALL report the `pressure` source as unavailable and never as failed; a PSI file that exists and cannot be read or parsed SHALL fail the source.
- The collector SHALL report `host_capacity_slots` from the operator-entered server
  capacity of [FR-058](./FR-058-explicit-server-capacity.md), SHALL always report
  `host_capacity_configured` as 1 or 0, and SHALL NOT estimate capacity.

### Windows and WSL

- On the Windows host, the Claude and Codex lanes, herdr and cargo run inside WSL, and the
  collector SHALL run inside the same WSL distribution, so its `/proc` view covers the
  lanes it reports. Its CPU and memory describe the WSL virtual machine.
- Native Windows processes are out of scope. The collector SHALL detect a native Windows
  agent, herdr or build process through the WSL interop process list and SHALL export the
  count as `ix.fleet.host.native_windows_agents`, so that one appearing is a defect and not
  an invisible lane.

### Compiler attribution

- The collector SHALL list processes whose executable name is in a configurable list of
  compilers and build drivers (default `rustc`, `cargo`, `cc`, `clang`, `gcc`, `ld`, `tsc`)
  with their working directory.
- The collector SHALL attribute a compiler process to a lane when its working directory is
  inside the `cwd` most recently reported for that lane by
  [FR-068](./FR-068-hook-activity-events.md), choosing the longest matching `cwd`, and SHALL
  count it as unattributed otherwise.
- For each sampling interval in which at least one compiler process is attributed to a
  lane, the collector SHALL add that interval's seconds to the lane's
  `ix.fleet.lane.compile_seconds` counter, bounded to 50 lanes as in FR-045. This is
  interim per-lane build-duty evidence until a build scheduler exports per-lane build
  seconds.
- A lane's `cwd` and `branch` mapping SHALL expire after 6 hours without an event.

### Pane-tail scan

- For each lane the collector SHALL read at most the last 80 lines (at most 16 KiB) of its
  pane and classify them into: `dialog` (a confirmation prompt awaiting input), `login` (an
  authentication prompt), `usage_limit` (a usage or rate limit notice), `queued_question`
  (a queued follow-up inputs notice), `wait_loop` (a sleep or poll wait repeated over the
  tail) or `none`.
- The collector SHALL map every class to the `ix.fleet.lane.blocked` reason of the same
  name, a `blocked` hook event with reason `permission` to `permission`, and a `Stop` with
  `last_class=question` to `question`; no class is detected and left unexported.
- The patterns SHALL be a typed, tested catalogue in this repository, one entry per class
  and harness. A classification SHALL name the class only; the matched text SHALL NOT be
  stored or exported.
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
| FR-069-AC-2 | Given macOS host CPU tick fixtures, the ratio equals the expected value, and the platform path is selected by build target rather than a runtime shell call. | Test |
| FR-069-AC-3 | Given a WSL `/proc` fixture, the Linux path produces the ratio and memory without ssh and without spawning a process. | Test |
| FR-069-AC-4 | Given a host profile that does not expect a temperature, no temperature value is produced and the source is reported unavailable, not zero and not failed; given a profile that expects it and an unreadable sensor, the source is failed. | Test |
| FR-069-AC-5 | Given compiler processes whose working directories are inside, nested within, outside and equal to lane `cwd`s, each is attributed to the lane with the longest matching `cwd` or counted unattributed. | Test |
| FR-069-AC-6 | Given a lane mapping older than 6 hours, its compilers are counted unattributed. | Test |
| FR-069-AC-7 | Given pane-tail fixtures for a confirmation dialog, a login prompt, a usage limit, "Queued follow-up inputs", a repeated sleep loop and plain output, each classifies to `dialog`, `login`, `usage_limit`, `queued_question`, `wait_loop` and `none` for both harnesses; a tail over 16 KiB is read only to the bound. | Test |
| FR-069-AC-8 | Given a pane tail classified `dialog` older than an `idle` hook event, the lane is reported with the hook event's state; no exported series or log line contains the matched pane text seeded with a secret marker. | Test |
| FR-069-AC-9 | Given a source that hangs, the tick completes within its budget plus 500 ms and the other sources still report. | Test |
| FR-069-AC-10 | The collector crate contains no Python file, no shell script and no spawn of a shell interpreter, and the retired probe scripts are not referenced by any build or service file in this repository. | Inspection |
| FR-069-AC-11 | Given herdr agent panes and relay registrations that overlap and differ, the registry is their union by lane address, and every registry lane has state and last-activity series on every push, including a lane with no event. | Test |
| FR-069-AC-16 | Given three hosts publishing lane lists, the fleet registry is the union of their unexpired lists and the relay registrations; given one host whose lease (90 seconds, refreshed about every 30) has expired, its lanes stay listed as `stale` and are not removed; a refresh restores them. | Test |
| FR-069-AC-12 | Every pane class and hook cause of the behavior list produces an `ix.fleet.lane.blocked` series of its mapped reason, including `wait_loop`, `queued_question`, `permission` and `question`; a test enumerates the classes and fails when one has no reason. | Test |
| FR-069-AC-13 | Given a compiler process attributed to a lane for 90 seconds of ticks, that lane's `compile_seconds` rises by 90 seconds, and a lane beyond 50 is not exported. | Test |
| FR-069-AC-14 | Given a WSL interop process list holding a native Windows agent process, `native_windows_agents` is nonzero; given none, it is 0. | Test |
| FR-069-AC-17 | Given `/proc/pressure` fixtures for cpu, io and memory, the reading holds `some` and `full` avg10 and avg60 per resource, and a cpu file with no `full` line gives no `full` reading. | Test |
| FR-069-AC-18 | Given a `/proc/pressure` directory with a file absent the read is `unsupported`; given malformed text the read is a parse failure. | Test |
| FR-069-AC-19 | Given a pressure reading, `ix.fleet.host.pressure` carries one point per resource and kind, and no `full_*` point for a resource without a `full` reading. | Test |
| FR-069-AC-20 | Given an unsupported pressure read, `ix_fleet_probe_unavailable` is exported for `pressure`, no `probe_ok` 0 and no failed source; given an unreadable one, `probe_ok` is 0 and no value is exported. | Test |
| FR-069-AC-21 | Given a pressure read refused as unsupported (`EOPNOTSUPP`, PSI disabled at boot) the read is `unsupported`; any other read failure is `unreadable`. | Test |
| FR-069-AC-22 | Given a `pressure` source that failed and then reads `unsupported`, the failed state is cleared: no failed source is listed and `probe_unavailable` is exported. | Test |
| FR-069-AC-15 | On each of the hosts draco, titan, luna and terra (terra inside WSL), `relay-watcher` run on that host alone, without an ssh session from another machine, exports the host series, per-lane series for the host's lanes and `ix_fleet_probe_ok`, and they are queried in Prometheus by their `ix_fleet_*` names with that host's label. | Demonstration |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-068](./FR-068-hook-activity-events.md),
  [FR-070](./FR-070-collector-source-health.md), [FR-058](./FR-058-explicit-server-capacity.md),
  [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md),
  [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md).
- Replaces `probe-linux.sh`, `host-probe.sh`, `util.sh`, `hosts-status.sh`,
  `lane-status.sh` and the matching `_lead-bin` scripts. Those live outside this repository
  and are retired by the program owner once this lands.
