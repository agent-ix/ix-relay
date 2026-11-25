---
id: SR-6121
title: "Code review — ix-relay SWM-276 test flakes under host load"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/agent-msg/src/activity_state.rs, crates/agent-msg/src/doctor/facts.rs, crates/agent-msg/tests/{doctor,hook,hook_stdin,location_enrollment,pane_machine,program_review,stranded_notice}.rs, crates/agent-msg/tests/support/{mod,location}.rs"
review_set: subset
---

# SR-6121: Code review — ix-relay SWM-276 test flakes under host load

## Summary

Ticket: SWM-276. PR agent-ix/ix-relay#64 at `a83dc87cf548c7da39828c5695b77f7a4403eb75`, reviewed as `git diff origin/main...HEAD` (11 files, +211/-107, two commits). The rust-review lane is folded into this file.

The coder named five root causes: doctor herdr probe deadlines (fixed with an `IX_RELAY_DOCTOR_PROBE_SCALE` multiplier in production code), an over-strict `activity_state` exactness assertion, a same-millisecond `--stale-after 0s` in tc_057, Python stub interpreter start-up eating the hook budget (stubs now shell over prepared JSON, plus `until_in_time` re-runs), and `hook_stdin` racing the stdin deadline.

Evidence: macOS, CARGO_BUILD_JOBS=4, CARGO_TARGET_DIR inside the worktree, `make ci` (fmt-check, clippy, test-redis on throwaway redis:7, deny, audit-unsafe, docs, check-matrix); result in `## Verdict`. Shell-stub behaviour reproduced in the scratchpad with `/bin/sh` (bash 3.2 POSIX mode) and `/bin/dash`.

## Verdict

Gate: `make ci` exit 0 once at load 7–15 (1,603 tests passed, 0 failed; no `until_in_time` re-run printed). A green run does not clear the findings below, because they are about what a green run can hide.

**CHANGES REQUESTED.** Two high findings. FND-001: the doctor gains an unspecified, undocumented environment knob in the shipped binary that multiplies every child-process deadline (herdr, systemctl, launchctl). Any value above 2 lets `doctor` exceed FR-029's 5 s bound. FR-029-AC-3 only asks that the herdr check be *reported*, so the test can accept `ok` or `unknown` without any production hook. FND-002: the new `until_in_time` callers in hook.rs and stranded_notice.rs re-run on *any* missing registration or empty stdout, not on a spool-proven `budget_exhausted`/`timeout` record. That breaks the helper's own documented contract, and it would hide intermittent real failures, including the "redis: unexpected end of file" drops the ticket records for tc_053_02/tc_053_12. One medium finding (FND-003): four more default-lane `hook_stdin` tests write after spawn and are exposed to the same stdin-deadline race that the PR fixed for `a_stdin_closed_empty_leaves_one_malformed_record`.

Checked and clean:

- **tc_057 (FR-057).** FR-057 says stale means age strictly greater than the window. A zero window at the same millisecond is fresh by spec, so the old test was racing the clock. A 1.2 s sleep and then `--stale-after 1s` gives an age of at least 1,200 ms, which is greater than 1,000 ms. That is deterministic and only gets more stale under load. Root cause confirmed.
- **activity_state (FR-068-AC-11 tag).** The `LOCK_WAIT` of 50 ms skips an update by design (module doc), so `counts == 40` was not a guaranteed property. `counted + skipped == 40` together with `file == counted` still detects a lost update, which is the real invariant. The held-lock half still pins skip and recovery. Root cause confirmed (see FND-007 for a weak spot).
- **hook_stdin `spawn_closed`.** `Stdio::null()` gives EOF on the first read with no test-side timing. That is a correct, deterministic fix for the closed-empty case.
- **Shell stubs vs Python stubs (question d).** I traced production's ssh command (`relay-ops/src/find.rs:476-497`: `HERDR_ENV=1 PATH="$HOME/...:$PATH" herdr --session '<s>' '<arg>'...`). `eval "set -- $last"` yields `--session default agent list`, the same as `shlex.split`. Expansion touches only the discarded `PATH=` word, and `quote()` single-quotes every argument, so `eval` cannot run anything new. The flag stripping (`--session`/`--machine` and their values), the `machine` → profiles answer, the `{"result":{<field>:...}}` envelopes and `TEST_LOCAL_MACHINE=terra` on the ssh hop all match. `write_state` covers every machine the tests use (draco, terra), and all three direct `state.json` writers were moved onto it. No remaining test writes `state.json` behind the stubs' back. Divergence exists only on malformed argument lists (FND-004, FND-005).
- **No Python added.** The PR removes four inline Python stubs. No `.py`, `python3` or logic-bearing script was added outside test-fixture `sh` stubs embedded in Rust.
- **Clippy/fmt.** Covered by `make ci` (see gate result).

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | `IX_RELAY_DOCTOR_PROBE_SCALE` is a test-only env knob in the shipped `doctor`, with no spec line or doc. It scales every herdr/systemctl/launchctl deadline, so any value above 2 breaks FR-029's 5 s bound. `base_cmd` does not clear it, so a developer's exported value leaks into the hang-deadline tests. FR-029-AC-3 needs only the herdr check *reported* | crates/agent-msg/src/doctor/facts.rs:67-86 |
| FND-002 | high | New `until_in_time` callers re-run on any missing registration / empty stdout, not on a spool record proving `budget_exhausted`/`timeout` (the helper's documented contract, support/mod.rs:390-396; `run()` deletes the spool dir). Up to 8 silent re-runs hide intermittent real failures, such as a Redis EOF or a dropped registration | crates/agent-msg/tests/stranded_notice.rs:41-56 |
| FND-003 | medium | Four default-lane `hook_stdin` tests still write the payload after spawn and race the hook's stdin deadline exactly as the closed-empty case did: `a_whole_payload_is_served_without_the_pipe_being_closed`, `a_payload_written_in_two_pieces_is_served`, `a_complete_non_hook_object_on_an_open_pipe_is_malformed_not_a_timeout`, `bytes_after_the_first_object_are_ignored` | crates/agent-msg/tests/hook_stdin.rs:236-334 |
| FND-004 | low | The shell `ssh` stub never terminates under macOS `/bin/sh` when the remote command has no `herdr` token (`while [ "$1" != herdr ]; do shift; done` with `$#`=0, reproduced). The `herdr` stubs also loop on a trailing `--session`/`--machine` with no value (`shift 2` fails without shifting). The Python stubs exited non-zero | crates/agent-msg/tests/support/mod.rs:502-508 |
| FND-005 | low | The transport `herdr` stub exits 0 with empty stdout on an unknown subcommand, where the Python stub failed (KeyError, exit 1). A wrong call now shows up as a JSON decode error instead of a failed process | crates/agent-msg/tests/support/mod.rs:478-484 |
| FND-006 | low | Comment says the hook has "a 300 ms budget a test cannot widen", but this is a `SessionStart` hook, whose budget is 3300 ms (NFR-001; hook_run.rs:183) | crates/agent-msg/tests/stranded_notice.rs:38-40 |
| FND-007 | low | `concurrent_updates_are_exact_...` keeps "exact" in its name after the exactness assertion was dropped. Nothing asserts `counted > 0`, so an implementation that always skips passes the concurrent half | crates/agent-msg/src/activity_state.rs:235-255 |

### Detail

- **FND-001.** Suggested fix: delete `PROBE_SCALE_ENV`/`probe_scale` and restore the constants. In tc_029_03, assert the herdr check is present and not `fail`, so `ok` or `unknown`/`herdr_unchecked` are both accepted. That is what FR-029-AC-3 states ("still reports the herdr check"). The deterministic `ok` path is already reachable through the injectable `Probes`/`ProcessRunner` seams (`herdr_check::check_within(runner, timeout, budget)`) in a unit test. If a production knob is really wanted, it needs an FR-029 line, a config field (not a hidden env var), a cap that keeps the 5 s bound, and an entry in `base_cmd`'s cleared list.
- **FND-002.** Suggested fix: give these calls an explicit `IX_RELAY_CONFIG` (as `hook_failures.rs`'s `Env` does), read `spool_path(&config)` after each run, and return `false` only when `support::ran_out_of_time(&records)`. Otherwise assert the outcome directly. The ticket's own comment attributes the tc_053_02/tc_053_12 failures to `redis: unexpected end of file`. A hook that hits that exits 0, spools a non-timeout record and does not register, and the current loop re-runs it silently. The same applies to `hook.rs:219` (`session_start_hook_records_the_herdr_pane_from_env`) and `stranded_notice.rs:107` (`session_start_output`).
- **FND-003.** All four call `Hook::spawn` (piped) and then `hook.write(...)`. If the test thread is descheduled past `STDIN_TOTAL`/the idle window after the child starts reading, the hook records `decode/stdin_timeout` with 0 connections, and `assert_connections(1)` / `assert_ne!(stage, "decode")` / `rec("decode","malformed")` fail. Fix: build the stdin from `std::io::pipe()` (stable on the pinned 1.98.1), write the payload into the pipe *before* `spawn`, and hold the writer for the "open pipe" cases (drop it for the two-pieces case).

## Dispositions

Round 1, fix head `56266b17b52d5ee4c0a6bcdf8a7455b859ad156b` (cff98b1, 56266b1 after a83dc87). Each fix was checked against the code on the branch. Reviewer gate on this head: `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -D warnings`, and `cargo test --workspace --no-fail-fast` against native redis-server 8.10.2 on a random port, at load about 16: 1,604 passed, 0 failed, no out-of-time re-run printed. `make ci` with redis:7 ran green at a83dc87 in the review pass; the round-1 delta is tests, test support and one spec bullet.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `PROBE_SCALE_ENV`/`probe_scale` are deleted and the constants restored. tc_029_03 accepts `ok`, or `unknown` with code `herdr_unchecked`. `relay_ops::herdr_check` adds a stub-runner unit test that pins `ok` under doctor's 1 s/2 s deadlines. |
| FND-002 | fixed | `support::run_hook_recording` gives each run its own config dir and returns the spool records. All three callers return `false` only on `ran_out_of_time(&records)` and otherwise assert that `records` is empty and that the hook registered or printed. |
| FND-003 | fixed | `Hook::spawn_with_input` pre-fills a `std::io::pipe()` before spawn (and holds the writer for the open-pipe cases). All four named tests use it. |
| FND-004 | fixed | The ssh stubs loop only while `$# -gt 0` and then `exit 1` when there is no `herdr` token. Both herdr stubs `exit 1` on a `--session`/`--machine` with no value. |
| FND-005 | fixed | The transport stub has a `*)` arm that prints to stderr and exits 1. |
| FND-006 | fixed | The comment now says 3300 ms for SessionStart. |
| FND-007 | fixed | The test is renamed `concurrent_updates_are_each_applied_or_returned_skipped_and_a_held_lock_never_blocks` and asserts `counted > 0`. |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-008 | low | `canonical_dump` replaces `DUMP` for every type, but only hash and set iteration order can move under incremental rehash. For streams it drops what XRANGE + XINFO GROUPS do not show (last-generated-id, entries-added, max-deleted-id, per-consumer PEL ownership), so an XADD+XDEL or XCLAIM by a "read" would pass. An unknown type reduces to its type name. Keep `DUMP` for string, list, zset and stream, and canonicalise only hash and set | crates/relay-test-support/src/lib.rs:160-216 |
