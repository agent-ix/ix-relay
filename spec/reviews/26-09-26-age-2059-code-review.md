---
id: SR-018
title: "Code review (with Rust lane) of ix-relay PR #32, AGE-2059: herdr failure recorded, load-independent hook tests"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@13ed67c84a9f7e14c42cff0fe07b96849de6eada; crates/agent-msg/src/identity.rs, crates/agent-msg/src/hook_run.rs, crates/agent-msg/src/hook_run/tests.rs, crates/agent-msg/src/spool.rs, crates/agent-msg/tests/support/mod.rs, crates/agent-msg/tests/name_collision.rs, crates/agent-msg/tests/hook_failures.rs, crates/agent-msg/tests/hook_stdin.rs, crates/agent-msg/tests/round_trips.rs, crates/agent-msg/tests/pop_window.rs; read for context: crates/relay-ops/src/naming.rs (derive_from_herdr, RealLocationHerdr), crates/agent-msg/src/doctor/checks.rs (group_lines), crates/agent-msg/tests/naming.rs, crates/relay-core/src/store.rs (HEALTH_HOOKS_CAP, unresolved mark)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-030"
    type: "reviews"
---
# SR-018: Code review (with Rust lane) of ix-relay PR #32, AGE-2059

## Summary

Ticket: AGE-2059. PR: ix-relay#32, head 13ed67c. The PR fixes a real silent failure: in a
herdr pane, a hook whose herdr calls failed, timed out or were refused for want of budget,
with no other tier naming the session, returned `ChainOutcome::None` and exited as a
non-lead with no spool record. It now returns `ChainOutcome::HerdrFailed { budget_spent }`,
and the hook records one `budget`/`budget_exhausted` or `derive`/`herdr_failed` record. The
PR also makes the real-binary hook tests retry a run that ran out of time (at most 8 runs,
then fail). The Rust lane (rust-review) is folded into this file.

Method: read-only review of the worktree. Builds and mutants ran in a scratch `git archive`
copy with its own `CARGO_TARGET_DIR`. A throwaway `redis:7` ran on 127.0.0.1:16432.

Every hook path was walked. The fix is correct on each one, and no legitimate path now
records a failure:

- No `HERDR_PANE_ID`, or partial herdr env: `HerdrEnv::from_process_env` is `None`, so the
  result is `ChainOutcome::None`, a silent non-lead. Unchanged.
- herdr answers but does not know the pane: `derive_from_herdr` returns `Ok` with no name,
  and `agent_not_found` maps to `Ok(None)`. The result is still `None` (pinned by
  `a_herdr_that_answers_and_names_nothing_is_a_non_lead`).
- herdr fails, but location, herdr agent name or transcript title names the session: a
  `Winner`, and no record. The herdr error goes to stderr only, as before.
- Explicit `--as`/`AGENT_MSG_NAME`: never reaches `derive`.
- Subagent events (`agent_id`): return before derivation.
- Suppressed session: `probe.suppressed` short-circuits.
- `fork` source: suppressed first.
- `UserPromptSubmit`: makes no herdr call, so it can never be `HerdrFailed`.
- `PostToolUse`/`Stop` with a cached name: return before `resolve_chain`, so a transient
  herdr failure on a cached lead never spools.
- `unresolved` mark set: silent, as before.
- `SessionStart` and uncached `PostToolUse`/`Stop` with herdr failing: exactly one record,
  exit 0. Nothing is registered and nothing is popped, so no mail leaves the inbox.

This was confirmed against the real binary with a failing fake herdr: `SessionStart`, 3x
`PostToolUse`, `Stop` and `UserPromptSubmit` each exited 0. They left 5 `derive`/`herdr_failed`
records, about 145 bytes each, and `UserPromptSubmit` left none.

The new kind is typed and wire-safe. `ErrorKind::HerdrFailed` serialises as `herdr_failed`.
`main` already decodes an unknown kind as `ErrorKind::Unknown` (`#[serde(other)]`) and ships
it as written (spool test "shipped exactly as written"). An older binary flushing the shared
spool therefore loses nothing. `doctor` groups by wire name, so it shows `derive /
herdr_failed` with no change.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | With herdr failing, every uncached `PostToolUse`/`Stop` of a lead in a herdr pane now spools one `derive`/`herdr_failed` record. The PR body says so, and a real-binary run confirms it: 5 hooks gave 5 records. Nothing rate-limits it the way the `unresolved:<sid>` mark (300 s, `SET NX EX`) limits `no_free_name`. Volume: about 145 B a record. The 256 KiB spool compacts to its newest ~900 records, and the shared `health:hooks` stream keeps about 1,000 entries for the whole bus. A few leads making a tool call every 5 to 10 s through a herdr outage of tens of minutes turn over the whole stream. That evicts every other machine's failure history, including rare `requeue_failed` records that carry message text, from both the spool and the stream. Before this PR, the same case was silent. Recommendation: record once per session per TTL with a separate NX mark, and keep retrying the lazy derivation on every call. Reusing `unresolved:` would also stop the retry, leaving the lead unregistered for up to 300 s after herdr recovers. If left as is, the effect matches the existing per-call `connect` records during a Redis outage: a noisy stream and lost history, but no lost mail from this path. That makes it acceptable as a follow-up ticket if the owner decides. | crates/agent-msg/src/identity.rs:955-960, crates/agent-msg/src/hook_run.rs:798-807 |
| FND-002 | low | The existing real-binary test `herdr_failure_at_session_start_stays_silent_and_exits_zero` exercises exactly the scenario this PR changes: a fake herdr that fails every call, at `SessionStart`. Its name now contradicts FR-030 as amended, since the hook is no longer silent: it records `derive`/`herdr_failed`. It asserts no spool content. Rename it and assert the one record there (see SR-019 FND-001 for the coverage consequence). | crates/agent-msg/tests/naming.rs:163 |
| FND-003 | low | Several out-of-time skip paths assert less than the PR body claims ("the out-of-time outcomes ... are asserted explicitly on the others"). In `hook_failures.rs` a skipped run needs only one record of kind `budget_exhausted`/`timeout`. There is no exact-one-record check, and in tc_020_01 no check that the popped mail is still in the inbox or in flight (FR-020). `an_invalid_name_ships_...` skips on `shipped.is_empty() && records.len() == 1` without checking the record is `invalid_name`. `round_trips::measure` retries on any non-empty stderr, not only an out-of-time one. A persistent regression still fails after 8 runs, but an intermittent one (fails 1 run in 2) passes. Tighten each skip to assert the exact out-of-time record, and in tc_020_01 that inbox plus in-flight still hold both messages. | crates/agent-msg/tests/hook_failures.rs:342, crates/agent-msg/tests/hook_failures.rs:393, crates/agent-msg/tests/hook_failures.rs:461, crates/agent-msg/tests/hook_failures.rs:606, crates/agent-msg/tests/round_trips.rs:257 |
| FND-004 | low | `assert_at_most_connections` sleeps 50 ms and then asserts an upper bound. This sleep-then-assert can only miss a late stray connection (a false pass), never flake. It also weakens the `too_large` branch from exactly 1 connection on `main` to at most 1. The weakening is justified (the flush attempt is budget-bound), and the decision is pinned deterministically by `hook_run::tests::an_oversized_input_is_one_too_large_decode_record_and_one_flush`. Note it, or drop the sleep and assert `<= 1` after the child is reaped. | crates/agent-msg/tests/hook_stdin.rs:81-84 |

## Rust lane (rust-review)

- Typed errors with stable codes. `HookError::HerdrFailed` is added to every exhaustive match
  (`stage`, `kind`, the persistent-fault and flush lists). `ErrorKind::HerdrFailed` is
  `snake_case` on the wire. No message text reaches the record.
- There is no `unwrap`/`expect` on a request path. The new ones are all in `#[cfg(test)]` or
  `tests/`.
- `chain_from` is extracted as a generic seam over `BoundedHerdr<F>`, which makes the
  outcome testable without a process. That is the right boundary.
- `pane_id: env.pane_id.clone()` is needed now that `env` is borrowed. It is a cheap newtype
  clone.
- `#[allow(dead_code, reason = ...)]` on the three new support items follows the existing
  support-module pattern, and the reason is given.
- `budget_spent` is true when any herdr call was refused for want of budget, including after
  an earlier call failed. The doc comment says "(or while it was)". See SR-020 FND-002 for
  the spec wording.
- No new `unsafe`, no `.github` change, no new proptest-regressions file (the existing
  `stdin.txt` predates the PR), and no load generator left behind.
- The hook exits 0 on the new path, never panics, and registers and pops nothing.
- Clippy `-D warnings` (agent-msg, all targets) is clean, and `cargo fmt --check` is clean.

## Verification performed

- `cargo test -p agent-msg` with `IX_RELAY_TEST_URL`: all pass.
- Affected binaries (name_collision, hook_failures, hook_stdin, pop_window, round_trips):
  10 runs x 2 parallel copies at normal load (load average about 2). All 760 test
  executions passed, and zero retries were taken (no "trying again" lines), so at normal
  load the retry never hides a result.
- Without `IX_RELAY_TEST_URL` the same binaries pass cleanly, and the Redis tests return
  early.
- Mutants in the scratch copy:
  - Reverting `chain_from` to `ChainOutcome::None`: `identity::tests::a_failing_herdr...`
    fails.
  - Mapping `HookIdentity::HerdrFailed` to `Ok(())` in `try_run`: `tc_030_08_a_herdr_failure...`
    fails.
  - `is_out_of_time` forced true: tc_022_06 and tc_022_08 fail after 8 attempts.
  - `ran_out_of_time` forced true: tc_020_01, tc_020_02 and tc_020_04 fail after 8 attempts.

  So the retry helpers never let a test pass without the run it exists to prove. One mutant
  survived; see SR-019 FND-001.
- Wall-clock thresholds stay in the `#[ignore]` lane. The new deterministic tests use the
  injected clock and the fake bus.
- The PR body states the idle-progress stdin deadline (NFR-001-M-5) as an owner decision
  and does not change it.

## Verdict

**PASS WITH FINDINGS.** The production fix is correct and fail-safe on every path examined.
It adds no false-positive record on a legitimate path, and the new kind is wire-safe across
mixed versions. FND-001 (record flood during a herdr outage) is an owner decision. It is
acceptable as a follow-up ticket and does not block. FND-002 to FND-004 are test-hygiene
lows.
