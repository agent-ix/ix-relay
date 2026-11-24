---
id: SR-6118
title: "Code review — ix-relay SWM-277 host_registry_stale probe_ok export"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-watcher/src/fleet/mod.rs, crates/relay-watcher/src/main.rs, spec/functional/FR-072-ordered-lane-assignment-list.md, CHANGELOG.md"
review_set: subset
---

# SR-6118: Code review — ix-relay SWM-277 host_registry_stale probe_ok export

## Summary

Ticket: SWM-277. PR agent-ix/ix-relay#63 at `2f01a976793638a58321bd2f848ed76bfa3d1231`, reviewed as `git diff origin/main...HEAD` (4 files, +120/-2) against FR-072 (AC-9), FR-070 and FR-069. The rust-review lane is folded into this file.

Evidence (macOS, CARGO_BUILD_JOBS=4, CARGO_TARGET_DIR in the worktree, throwaway redis:7 via `scripts/test-with-redis.sh`):

- `cargo fmt --all -- --check` clean; `cargo clippy -p relay-watcher --all-targets -- -D warnings` clean.
- `scripts/test-with-redis.sh -p relay-watcher`: every relay-watcher test binary passes, 0 failed (8 ignored, pre-existing); `tc_072_09` run on its own passes with `IX_RELAY_TEST_URL` set.
- `git merge-tree --write-tree origin/main HEAD` succeeds against d3027cf (Release 0.4.1); GitHub reports MERGEABLE. The diff does not touch `scripts/test-with-redis.sh` or any test file outside `relay-watcher/src/main.rs`, so it does not overlap the SWM-276 change.

## Verdict

**CHANGES REQUESTED.** One high finding (FND-001): FR-070 says the `ix_fleet_probe_ok` sources SHALL be exactly eleven named values, and FR-072 now requires the watcher to export a twelfth (`host_registry_stale`). The code is right to keep it out of `Source::ALL` (that enum is the host profile: one series per expected source of the reporting host, heartbeat and `collector_source_failed`), so FR-070 must change, not the enum. Two medium findings and two low.

Checked and clean:

- Reads the stored defects through `Relay::assignment_probe_defects`; no second store, no new key.
- Clears on republish: the defect hash field is removed when the host publishes its lane list (relay-core), and `set_stale_hosts` flips the host's series to 1 on the next refresh; the test plants an expired lease, gets the refusal, sees 0, republishes, sees 1.
- The series uses the existing `FLEET_PROBE_OK` instrument; no new metric name.
- CHANGELOG has an `## Unreleased` entry.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | FR-070 says the probe sources SHALL be exactly eleven values; FR-072 now requires `ix_fleet_probe_ok{source="host_registry_stale"}`, which FR-070 neither lists nor exempts from the host profile, heartbeat and `collector_source_failed` rules | spec/functional/FR-070-collector-source-health.md:22 |
| FND-002 | medium | A failed `assignment_probe_defects` read is dropped silently: the comment says the `redis` source reports it, but the result is not passed to `set_refresh_redis`, so the stale export can stay at an old value with every source reading 1 | crates/relay-watcher/src/main.rs:433 |
| FND-003 | medium | Every watcher exports the series for every stale host, so a fleet of N watchers emits N copies of `probe_ok{host=X,source=host_registry_stale}` that differ only in resource `host.name`; the `host` attribute here means the stale host, not the reporting host as in FR-070 | crates/relay-watcher/src/fleet/mod.rs:733 |
| FND-004 | low | `stale_hosts` only grows: a recovered host keeps a `probe_ok` 1 series for the life of the process and the map is never pruned, while a watcher restart drops it, so whether the 1 is exported depends on process age | crates/relay-watcher/src/fleet/mod.rs:498 |
| FND-005 | low | The test covers 0 then 1 but not the documented "failed read keeps the last known defects" path, so FND-002's behavior is unpinned | crates/relay-watcher/src/main.rs:2243 |

### Detail

- **FND-001.** Do not add the source to `Source::ALL`: the enum drives `expected_sources`, `probe_unavailable`, the heartbeat failed-source list and `collector_source_failed`, all of which describe the reporting host's own readers. `host_registry_stale` is a defect about another host, exported only while the relay holds (or recently held) a defect. Fix in the spec: add a bullet to FR-070 "Sources and host profile" saying the `ix_fleet_probe_ok` `source` attribute also takes `host_registry_stale` (FR-072), exported per defective host rather than per profile, not listed in `expected_sources`/`unavailable_sources`, not in the heartbeat failed list, surfaced in doctor as `assignment_host_registry_stale`; and reword "The sources SHALL be exactly" to "The collector sources SHALL be exactly". Consumers of the `ProbeFailing` alert (outside the repo) should be told the new value exists.
- **FND-002.** Either include the read in the `redis` health (`collector.set_refresh_redis(published.is_ok() && read && activity_ok && defects_ok, log)`) or fix the comment. Keeping the last known state is fine; hiding the failure is not (FR-070: a timeout is never success).
- **FND-003.** Pick one: export only from the watcher whose host is the stale host's peer of record (not obvious), or accept duplicates and say so in FR-072 so the alert aggregates with `max by (host, source)`. At minimum the FR-072 sentence should state that every watcher exports it.

## Coverage

Reviewed every hunk of the 4 files. Code paths: `refresh_fleet` defect read, `Collector::set_stale_hosts`, `Collector::fill` stale loop, `tc_072_09`. Spec: FR-072 Behavior bullet and AC-9 edit, FR-070 Sources/Health series, FR-069 lane lease. Out of scope: pre-existing tree state.

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-006 | low | `STALE_RECOVERED_EXPORT` and `StaleHost` were inserted between `PANE_SCAN_BUDGET`'s doc comment and its `const`, so the pane-scan doc now prefixes the new constant's doc and `PANE_SCAN_BUDGET` has none | crates/relay-watcher/src/fleet/mod.rs:52 |

## Dispositions

Round 1, fix head `ac0ea3e1b2cd24e21cfa06c0b254267b8e0c2a09` (one commit after 2f01a97). Re-checked against the code: `cargo fmt --check`, `cargo clippy -p relay-watcher --all-targets -D warnings` and `cargo doc -p relay-watcher` clean; `cargo test -p relay-watcher` with redis:7 397/14/1 passed, 0 failed, including `tc_072_09`, `tc_072_10` and `tc_072_11`. A workspace run hit `agent-msg` `activity_state::tests::concurrent_updates_are_exact_and_a_held_lock_never_blocks` once (agent-msg is untouched by this PR); it passed 3/3 alone. `git merge-tree` against origin/main d3027cf is clean.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | FR-070 now says "The collector sources SHALL be exactly" and adds a bullet exporting `host_registry_stale` per defective host outside the profile, heartbeat and `collector_source_failed`; `Source::ALL` unchanged |
| FND-002 | fixed | `apply_stale_defects` returns false on a failed read, which is ANDed into `set_refresh_redis`; last-known state kept; pinned by `tc_072_10` |
| FND-003 | fixed | FR-070, FR-072 and CHANGELOG state every watcher exports it, `host` names the stale host, consumers aggregate with `min by (host, source)` |
| FND-004 | fixed | `StaleHost.recovered` timestamps recovery; entries are retained for `STALE_RECOVERED_EXPORT` (1 h) and then dropped; pinned by `tc_072_11` |
| FND-005 | fixed | FR-072 behavior bullet and AC-9 now own the keep-last-known/redis-0 behavior; `tc_072_10` tests it |
