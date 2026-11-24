---
id: SR-6108
title: "Code review — ix-relay SWM-238 host-probe crate and PSI pressure probes"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/host-probe/**, crates/relay-watcher/src/fleet/{mod,health,procs,feed,feed_tests}.rs, crates/relay-watcher/src/{main,liveness}.rs, crates/relay-watcher/src/otlp/series.rs, Cargo.toml, docs/configuration.md, CHANGELOG.md"
review_set: subset
---

# SR-6108: Code review — ix-relay SWM-238 host-probe crate and PSI pressure probes

## Summary

Ticket: SWM-238. PR agent-ix/ix-relay#58 at `a4caeb0ff9bd213c8025134c375872459760c88b`, reviewed as `git diff origin/main...HEAD` (2 commits, 24 files). The rust-review lane is folded into this file. The second consumer's contract was read from dev-team `origin/main:spec/functional/FR-009..FR-016`.

Gate evidence (macOS, CARGO_BUILD_JOBS=4, load average 39-50 from other lanes):

- `make ci` at a4caeb0: exit 2. fmt-check and lint passed. test-redis failed on agent-msg `activity_state::tests::concurrent_updates_are_exact_and_a_held_lock_never_blocks` (39 against 40, a lock wait bounded by `LOCK_WAIT` that skips the update under load), and stopped at that binary.
- That test alone: 3/3 pass at head, and it passes on origin/main `c7b3c6a`. `tc_029_03_with_redis_down_doctor_fails_fast_and_still_reports_herdr` alone with redis:7: 3/3 pass at head, and it passes on origin/main.
- `scripts/test-with-redis.sh --no-fail-fast` at head: 1574 passed, 0 failed. `make deny audit-unsafe docs check-matrix`: exit 0.
- Both are pre-existing load flakes independent of this PR. agent-msg is untouched and depends on neither host-probe nor relay-watcher.

## Verdict

**APPROVE WITH FOLLOW-UPS (CONDITIONAL).** No high findings. One medium correctness finding: a kernel that has PSI built in but disabled at boot reports `pressure` as a standing failure. One medium gap: the crate does not yet carry the probes and reading model that locked-build (dev-team FR-009) must take from it, and no ticket tracks adding them. Two low findings.

Checked and clean:

- **Behaviour-preserving extraction.** `bound.rs`, `host.rs` and `mac.rs` are renames at 98-99% similarity. The only changes are import paths and the source thread name (`fleet-source` to `host-probe-source`, which is not exported anywhere). `ErrorClass` and the compiler reader move unchanged. The `native_windows` reader stays in relay-watcher because it runs through herdr-cli. Every existing series and `probe_ok` row is unchanged. The diff only adds the `pressure` source: on Linux it appears as `probe_ok{source="pressure"}` plus `ix.fleet.host.pressure`, and off Linux as `probe_unavailable{source="pressure"}`. The edits to existing tests (`tc_070_01` and the `expected()` helper) only make room for the new source and leave temperature's semantics as they were.
- **PSI parsing.** A missing `full` line gives `full: None`, which is tested. A missing `some` line, a missing avg10 or avg60, a non-numeric figure, or a non-finite figure (`nan`, `inf`) gives `Parse`. Truncated text (`some avg10=1.0 avg6`) fails on the missing avg60. Lines with an unknown first word, and unknown keys such as `avg300` and `total`, are skipped. A garbled `full` line fails the whole resource, which is the conservative choice and matches FR-069.
- **macOS.** `read_pressure()` returns `Unsupported` off Linux, and this is asserted under `cfg(not(target_os = "linux"))`. The default profile does not expect `pressure` off Linux, so it is exported as `probe_unavailable`. A profile that expects it explicitly also gets `probe_unavailable`: the read is `Unsupported`, the outcome is not recorded, and `fill` takes the unavailable branch (tc_069_20). `failed_sources()` does not list it.
- **Explicit `expected_sources` without `pressure`.** This should not produce a warning. FR-070 defines a source that is not listed as unavailable, and FR-070 says unavailable is not a failure. Temperature already follows the same opt-in rule. The omission is visible: `ix_fleet_probe_unavailable{source="pressure"}` is exported. A warning would fire on every tick, or at least on every start, on hosts that leave the source out on purpose, such as macOS hosts with an explicit list. The CHANGELOG already tells operators to add `pressure` to an explicit list. No finding.
- Budgets: the pressure read runs under its own `Slot` with `SOURCE_BUDGET`. `Slot::run` takes the budget as a parameter, so a consumer can apply its own `probe_timeout`.
- Series catalogue: `FLEET_PRESSURE` has labels `host, resource, kind` with bounded values (3 resources x 4 kinds). This matches the FR-045 row added by the diff.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | A kernel with PSI compiled in but disabled at boot (`psi=0`, or `CONFIG_PSI_DEFAULT_DISABLED=y` without `psi=1`, as in RHEL/CentOS 8 kernels) can keep `/proc/pressure/*` while reads fail with `EOPNOTSUPP`. `read_pressure_at` maps every error other than `NotFound` to `Unreadable`, so such a host reports `pressure` as a standing failure (probe_ok 0, doctor `collector_source_failed`) when it has no PSI. Map `raw_os_error() == EOPNOTSUPP` to `Unsupported`, and widen the FR-069 rule "a kernel built without it" to also cover PSI disabled at boot. | crates/host-probe/src/pressure.rs:133-142 |
| FND-002 | medium | host-probe has no probes or reading model for the second consumer. dev-team FR-009 requires locked-build to take every probe from this crate and forbids a second implementation (FR-009-AC-6). The probes it needs are package power, macOS compressor and swap, WSL available memory and memory limit, one-minute load, free pool disk and per-job peak memory (cgroup `memory.peak` or `proc_pid_rusage`), and each reading needs a value, unit, observation time and unavailable reason, addressed by `[[probe]] name/kind`. The crate today has cpu, memory, temperature, pressure and compilers. No Linear ticket tracks the rest, so FR-009 will either stall or grow a second probe implementation. | crates/host-probe/src/lib.rs:1-17 |
| FND-003 | low | The fixture entry points (`parse_pressure`, `read_pressure_at`, `compilers_in_proc` and the host.rs text parsers) are `#[cfg(any(target_os = "linux", test))]`. `cfg(test)` applies only to host-probe's own tests, so a downstream crate (locked-build) cannot call them from its tests on macOS. The lib.rs promise "each parser takes text or a root directory so tests drive it from fixtures" holds only inside this crate. | crates/host-probe/src/pressure.rs:81-88; crates/host-probe/src/lib.rs:7 |
| FND-004 | low | When the read is `Unsupported`, the pressure outcome is not recorded at all, so a previously recorded `Failed` (for example a timeout on an earlier tick) stays in `Health`. `failed_sources()`, and with it the heartbeat and doctor, then lists `pressure` as failed while `fill` exports it as `probe_unavailable`. This contradicts FR-069 ("never as failed"). Clear the source's outcome when the read is `Unsupported`. | crates/relay-watcher/src/fleet/mod.rs:561-573 |

## Dispositions

Round 1, fix head `0fb73d636d2f908dfb1efece9063a092309a321a`, reviewed as `git diff a4caeb0 0fb73d6`. Linux evidence from terra (WSL2, kernel 6.18.54.1-microsoft-standard-WSL2, PSI present): a fresh clone at 0fb73d6 with `nice -n 10` and `-j 4`. `cargo test -p host-probe -p relay-watcher` passed (13 + 392 + 14 + 1). `cargo clippy -p host-probe -p relay-watcher --all-targets -D warnings` passed, and so did `clippy -p host-probe --features fixtures`. `std::io::Error::from_raw_os_error(95).kind()` is `Unsupported` on that Linux toolchain. The clone was deleted afterwards. On macOS, the fixtures-feature clippy run and the host-probe and relay-watcher tests also passed.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `read_failure` maps `ErrorKind::NotFound | ErrorKind::Unsupported` to `Unsupported`. On Linux std decodes EOPNOTSUPP (95) to `ErrorKind::Unsupported`, which was checked on terra. FR-069 now says "booted with it disabled", and AC-21 is tested by tc_069_21. |
| FND-002 | deferred | Filed as SWM-275 (host-probe: probes and reading model for locked-build, project Fleet W6). The PR body says SWM-238 stays open. |
| FND-003 | fixed | A `fixtures` feature exposes the text and directory parsers on every platform. Clippy passes with `--features fixtures` on macOS and on Linux. |
| FND-004 | fixed | When the read is `Unsupported`, `Health::clear(Source::Pressure)` runs and the status is published. AC-22 is tested by tc_069_22. |
