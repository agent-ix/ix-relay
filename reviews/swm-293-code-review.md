---
id: SR-6125
title: "Code review — ix-relay SWM-293 activity-state lock skips counted in a sidecar file"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/agent-msg/src/activity_state.rs, crates/agent-msg/src/doctor.rs, crates/agent-msg/src/doctor/{checks,facts,report}.rs, crates/relay-watcher/src/fleet/{feed,feed_tests,mod}.rs, crates/relay-watcher/src/otlp/series.rs, CHANGELOG.md"
review_set: subset
---

# SR-6125: Code review — ix-relay SWM-293 activity-state lock skips counted in a sidecar file

## Summary

Ticket: SWM-293. PR agent-ix/ix-relay#67 at `710dc2208c7beb2c408b06596b1995d92911da5b`, reviewed as `git diff origin/main...HEAD` (12 files, +197/-27). The rust-review lane is folded into this file.

Design under review: when `ActivityState::try_update` fails, `update` calls `note_lock_skip`, which appends one byte to `spool/activity.lockskips` with `O_APPEND` and no lock; the count is the file's length, capped at 1 MiB. `relay-watcher` exports it as `ix.relay.hook.activity_skipped{reason="lock"}` (an OTLP monotonic cumulative Sum) and `agent-msg doctor` warns `activity_lock_skipped` while it is non-zero.

Evidence: macOS, CARGO_BUILD_JOBS=4, target inside the worktree, native `redis-server --save '' --appendonly no` on a random port via `IX_RELAY_TEST_URL`, no Docker. `make -o test-redis ci` (fmt-check, clippy -D warnings, deny, audit-unsafe, docs, check-matrix) and `cargo test --workspace` were run back to back and exited 0, with no failing test; the five tests that touch the change (`concurrent_updates_…`, `lock_skips_from_separate_processes_…`, `lock_skips_are_read_from_the_sidecar_…`, `tc_068_11_…`, `every_code_fires_…`) pass.

## Verdict

**CONDITIONAL**: no high findings; three medium findings on the semantics of the count (silent cap, wrong reason, no way to clear).

What is correct: the append path takes no lock and cannot wait on the activity lock. A one-byte `write(2)` with `O_APPEND` on a local regular file is atomic and does not block on another writer's flock, so concurrent hooks never lose an increment and the extra cost (two `lstat`, one `open`, one `write`, one `close`) is microseconds, well inside the 300 ms hook budget and after the existing 50 ms lock bound. The OTLP kind is right: the count only grows, so a cumulative monotonic Sum (a Prometheus counter) is the correct type, matching the other three reasons; deleting the file reads as a counter reset, which Prometheus handles. The watcher reports the lock reason even when `activity.json` is missing, as FR-068 asks. The pinned doctor code name and the series description are updated.

## Findings

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-001 | medium | The 1 MiB cap makes further skips silent. `note_lock_skip` returns without writing once the file holds `MAX_LOCK_SKIPS` bytes, and nothing records that the cap was hit. The doctor detail then reads "1048576 hook activity update(s) were dropped", an exact number that is wrong, and the exported counter flatlines, so `rate()` reads zero while updates are still being dropped. FR-068 says a dropped update is "a visible defect, never silence" and states no cap. A cap must be reported: for example say "at least N" and name the cap in the doctor detail when the length reaches it, or export a separate saturated signal. | crates/agent-msg/src/activity_state.rs:28-29, 95-111; crates/agent-msg/src/doctor/checks.rs:1252-1269 |
| FND-002 | medium | Every `try_update` error is counted as `reason="lock"`, not only a lock timeout. The symlink refusal of `activity.json`, a failed `DirBuilder::create`, an unopenable lock file, `TryLockError::Error`, a failed temp-file open or write, and a failed rename all land in the lock count. The doctor cause then says "concurrent hooks held spool/activity.json's lock past the 50 ms bound" and the fix says to look for a stuck hook process, which is the wrong lead for a full disk, a read-only or wrong-owner spool, or a symlinked state file. FR-068 says the count is of updates "dropped for want of the lock", and the field doc says "could not take the lock or write the file", so code, field doc and spec disagree. Either count only `ErrorKind::WouldBlock` from `lock()` as `lock` and the rest under a distinct reason (e.g. `state_io`), or rename the reason and the doctor text to cover both. Note also that when the spool directory itself is unwritable, the sidecar append fails as well and the drop is silent. | crates/agent-msg/src/activity_state.rs:49-52, 159-200; crates/agent-msg/src/doctor/checks.rs:1263-1267; spec/functional/FR-068-hook-activity-events.md:106-111 |
| FND-003 | medium | The count never resets and nothing says how to clear it, so one lock skip makes `activity_lock_skipped` warn forever and `agent-msg doctor --strict` exit non-zero for good. The neighbouring hook checks are time-bounded (`hook_failures_recorded` looks at 24 h, `hook_spool_pending` drops to warn after 15 min). The metric is fine as a monotonic counter; the problem is only the doctor check, which reads the cumulative total as a current fault. Either warn only on growth (compare the file's mtime with a window, as the spool check does with `modified_ms`) or document a reset (delete `spool/activity.lockskips`) in the fix text and in FR-068/FR-029. | crates/agent-msg/src/doctor/checks.rs:1252-1269; crates/agent-msg/src/doctor/facts.rs:399-404 |
| FND-004 | low | Three copies of "the sidecar's length" disagree. `activity_state::lock_skips_at` and the doctor fact reader use `symlink_metadata` and count only a regular file; the watcher's `read_hook_counts` uses `fs::metadata`, which follows a symlink and counts a directory's size, and turns any non-NotFound error on the sidecar into `ErrorClass::Unreadable`, which also drops the backoff, failed and unregistered series. Doctor and metric can therefore disagree about the same file. The doctor reader could call `activity_state::lock_skips_at` instead of repeating it, and the watcher should use the same rule. | crates/agent-msg/src/activity_state.rs:218-229; crates/agent-msg/src/doctor/facts.rs:399-404; crates/relay-watcher/src/fleet/feed.rs:363-367 |

## New findings (disposition pass 1)

Head `7a3f2260ebc6f5e6cf5b7ae7e39b1ec05d4a3a7c`, fix commit reviewed as `git diff 710dc22..7a3f226`.

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-005 | medium | `activity_spool_unwritable` decides from permission bits: `fs::metadata(dir).permissions().readonly()` is true only when no write bit is set for anyone (mode & 0o222 == 0). It misses the likely real cases: a spool created by another user (root-owned 0755, so the hook user cannot write but the owner bit is set), a read-only mount (bits still say writable), and a macOS ACL deny. It raises a false warning when doctor runs as root on a 0555 dir. In those missed cases neither sidecar can be written either, so the drop is silent again, which is the case this check was added for. Ask the kernel instead: `rustix::fs::access(dir, Access::WRITE_OK)` (rustix with `fs` is already a workspace dependency) or `nix::unistd::access` (already in relay-core), which accounts for owner, group, root, ACLs and `EROFS` and creates nothing, so FR-029-AC-5 still holds. | crates/agent-msg/src/doctor/facts.rs:651-656 |
| FND-006 | medium | At the cap, doctor drops to information after 24 hours while drops may still be happening. `hook_skips::record` returns without writing once a sidecar holds `MAX_SKIPS` bytes, so its mtime stops advancing. `activity_skips` then reads the frozen mtime as "none in the last 24 hours" and reports `ok` with "at least 1048576 … none in the last 24 hours", which is a claim doctor cannot know at the cap. The watcher's saturated gauge stays 1, so the metric side is visible; doctor is the gap. Keep a saturated count a warning regardless of age (its recency is unknown), or keep touching the mtime at the cap (for example `File::set_modified` without appending). | crates/relay-core/src/hook_skips.rs:90-92; crates/agent-msg/src/doctor/checks.rs:1305-1331 |

## Dispositions

| FND | outcome | reason |
| --- | ------- | ------ |
| FND-001 | fixed | A count at `MAX_SKIPS` now reads "at least N (cap reached)" in doctor, and the watcher exports `ix.relay.hook.activity_skip_saturated{reason}` as 1, so the flat counter is explained. The doctor going to `ok` after 24 h at the cap is new finding FND-006. |
| FND-002 | fixed | `update` maps only `ErrorKind::WouldBlock` to `Reason::Lock` and every other error to `Reason::WriteFailed`, each in its own sidecar with its own doctor code, cause and fix. Verified by the spawned-process test (held lock counts only `lockskips`; a symlinked `activity.json` counts only `writefails`). The unwritable-spool case now has a direct check, whose implementation is new finding FND-005. |
| FND-003 | fixed | Doctor warns only when the sidecar's mtime is within 24 h (`DAY_MS`), otherwise reports `ok` with the cumulative count, and every message says that deleting the sidecar resets it. Verified in `tc_068_15_doctor_reads_the_skip_sidecars_and_warns_only_on_recent_growth` with real files and `set_modified`. |
| FND-004 | fixed | One reader, `relay_core::hook_skips::count`, used by the hook, doctor and the watcher. It uses `symlink_metadata`, rejects a non-regular file, and in the watcher an unreadable sidecar leaves out only its own series (`an_unreadable_sidecar_leaves_out_only_its_own_count`). |
