---
id: SR-019
title: "Gap analysis of ix-relay PR #32 (AGE-2059) against FR-030 and the hook requirements it touches"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@13ed67c84a9f7e14c42cff0fe07b96849de6eada; spec/functional/FR-030-hook-failures-are-spooled-and-reported.md (Behavior, AC-8), spec/functional/FR-017-hook-never-fails-the-turn.md, spec/functional/FR-022-name-collisions-resolve-to-a-free-name.md (AC-6..AC-8), spec/tests.md (TC-030-08 row), crates/agent-msg/src/identity.rs, crates/agent-msg/src/hook_run.rs, crates/agent-msg/src/hook_run/tests.rs, crates/agent-msg/src/spool.rs, crates/agent-msg/tests/{name_collision,hook_failures,hook_stdin,round_trips,pop_window,naming}.rs, crates/agent-msg/tests/support/mod.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-030"
    type: "reviews"
---
# SR-019: Gap analysis of ix-relay PR #32 (AGE-2059)

## Summary

Ticket: AGE-2059. PR: ix-relay#32, head 13ed67c. This is a planless gap analysis. It follows
each requirement to the Test Matrix, then to the tagged tests, then to the code, for the
FR-030 amendment and the hook ACs whose tests the PR changes. Plan completion: not assessed.

The new FR-030 clause, carried by AC-8, is implemented in `identity::chain_from` and in two
match arms each in `identity.rs` (`derive_at_session_start`, `derive_from_cache_or_lazily`)
and `hook_run::try_run`. The TC-030-08 row names the two new deterministic tests. Both are
real (not stubs) and each fails under the mutant that reverts its own seam. `make
check-matrix` passes (290 rows).

## Verdict

**PASS WITH FINDINGS.** AC-8's new clause is covered at the chain seam and at the hook seam,
but not across the join between them. A mutant that reinstates the exact production bug at
that join survives the whole suite (FND-001). That gap is cheap to close and should be
closed in this PR.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | The wiring from `ChainOutcome::HerdrFailed` to `HookIdentity::HerdrFailed` is unpinned. Mutant: both arms in `derive_at_session_start` and `derive_from_cache_or_lazily` changed to `Ok(HookIdentity::NotALead)`, which is exactly the pre-fix silent non-lead. It passes all of `cargo test -p agent-msg` against Redis (every unit and integration binary green). The unit test drives `chain_from` directly, and the `hook_run` test injects `HookIdentity::HerdrFailed` through the fake bus, so nothing crosses the join. The real-binary scenario already exists: `tests/naming.rs::herdr_failure_at_session_start_stays_silent_and_exits_zero` runs `SessionStart` against a fake herdr that fails every call. It only asserts exit 0, empty stdout and nothing registered. Add an assertion there that the spool holds exactly one `derive`/`herdr_failed` record, plus an uncached `PostToolUse` case, and the mutant dies. | crates/agent-msg/src/identity.rs:848-852, crates/agent-msg/src/identity.rs:955-960, crates/agent-msg/tests/naming.rs:163 |
| FND-002 | low | TC-030-08's matrix row cites `identity::tests::a_failing_herdr_that_names_nothing_is_a_failure_not_a_non_lead`, but that test carries no `Tracing: TC-030-08` / `ACs: FR-030-AC-8` tag. Its sibling in `hook_run/tests.rs` does, as the other 27 tagged tests there do. A tag-driven trace recount would not find it. Add the tag. | crates/agent-msg/src/identity.rs:1095-1099, spec/tests.md:224 |

## AC-by-AC

- FR-030-AC-8, existing clauses (closed stderr, panic): unchanged code. The real-binary
  `tc_030_08_a_closed_stderr_...` oversize case now accepts `too_large` or `stdin_timeout`,
  as one `decode` record in either case. Nothing is lost: the `too_large` decision is pinned
  deterministically by `stdin::tests` (`InputFault::TooLarge`) and by
  `hook_run::tests::an_oversized_input_is_one_too_large_decode_record_and_one_flush`.
- FR-030-AC-8, new clause (herdr failure in a pane): covered at both seams, not end to end
  (FND-001).
- FR-022-AC-6/7/8 (name_collision.rs): every run still asserts the holder untouched, the
  session unregistered and exactly one record. That record is either `no_free_name` with
  candidate `qsl`, or an explicit out-of-time pair (`budget`/`budget_exhausted`,
  `derive`/`herdr_failed`, `derive`/`timeout`). After 8 out-of-time runs the test panics; it
  never skips. Forcing `is_out_of_time` true makes tc_022_06 and tc_022_08 fail, so neither
  can pass without reaching `no_free_name`.
- FR-020-AC-1/2/4, FR-030-AC-2 (hook_failures.rs): these retry via `support::until_in_time`
  and fail after 8 attempts. Forcing `ran_out_of_time` true fails tc_020_01, tc_020_02 and
  tc_020_04. The skip paths assert less than the passing path (SR-018 FND-003).
- FR-030-AC-7 slow link (pop_window.rs): every attempt asserts no pop and no delivery.
  The deferral without a record is proven on the first run without a `budget` record. A
  regression that records on deferral fails after 8 attempts, and the deterministic
  `tc_030_07_a_user_prompt_submit_with_too_little_budget...` pins it too.
- NFR-001/FR-018 round trips (round_trips.rs): request counts are asserted only on a run
  that finished (SR-018 FND-003 on the retry predicate).

## Code with no owning requirement

None added. The interactive `identity::derive` (`whoami`, `send`) still collapses
`ChainOutcome::HerdrFailed` to "not a lead" when a herdr pane has no registration. That is
outside FR-030, which covers hooks only, and it predates this PR. The herdr error still
reaches stderr. Noted for completeness, not as a finding.

## Stub and inflation check

No stubs. Both new tests assert concrete stage and kind pairs per outcome and per event,
with no `matches!(_, any)`. The TC-030-08 row adds two test names and no new TC ids, so the
counts are unchanged and `make check-matrix` agrees.
