---
id: SR-040
title: "Code review (with Rust lane) of ix-relay PR #36, AGE-2063: idle-progress stdin deadline"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@08cdcad91dfd7e43a2970f8a2717c124f1212190; crates/agent-msg/src/stdin.rs, crates/agent-msg/src/budget.rs, crates/agent-msg/src/hook_run.rs, README.md (compared against origin/main 0fa9a36); read for context: crates/agent-msg/src/hook_run/tests.rs, crates/agent-msg/tests/hook_stdin.rs, crates/agent-msg/tests/support/mod.rs, plugins/claude/hooks/hooks.json, plugins/codex/hooks.json"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/NFR-001"
    type: "reviews"
---
# SR-040: Code review (with Rust lane) of ix-relay PR #36

## Summary

Ticket: AGE-2063. PR: ix-relay#36, head 08cdcad. The PR changes the hook's stdin read from
one 300 ms total deadline to an idle window. The window is 300 ms and restarts when bytes
arrive. A new overall cap, `STDIN_TOTAL` (900 ms), bounds the whole read. It is counted
from the same `started` reading that the event budget uses. The deadline logic moves into a
pure `collect(budget, wait)` seam, and the new tests drive that seam on a `FakeClock`.

Gates at the reviewed sha, run on a scratch copy with its own target dir: `cargo test -p
agent-msg` passes. The `hook_stdin --ignored` benchmark lane passes (4/4). `make lint`,
`make fmt-check` and `make check-matrix` all pass (345 rows).

Timing, from reading the code and running the real binary. The deadline has two parts: the
read, then the event budget. Both are counted from the one `started = clock.elapsed()` taken
in `run()`, so they cannot double-count. A mutant that restarts the event budget after the
read fails `tc_n001_m_05_the_event_budget_counts_from_before_the_stdin_read`. The longest
read is 900 ms plus one 10 ms poll. After it, `PostToolUse`/`Stop` have what is left of
900 ms, which is about nothing. `UserPromptSubmit` gets `budget/budget_exhausted` before it
connects. `SessionStart` has up to 2.4 s left, and its harness timeout is 5 s. No path gets
near the 2 s harness kill. A slow drip (1 byte per 250 ms or per 290 ms) ends at 917–919 ms
wall on every event, in a debug build, with one `decode/stdin_timeout` record.

On main, a payload that was not complete at 300 ms was a `stdin_timeout` for every event.
Now a payload that keeps arriving is read for up to 900 ms. A late `UserPromptSubmit`
therefore holds the user's prompt for up to 900 ms instead of 300 ms, measured at 817 ms and
918 ms. It pops nothing either way and it leaves exactly one record, so no mail is lost and
nothing is delivered twice (FR-020). The mail waits for the next `PostToolUse`/`Stop`. The
900 ms cap is defensible because the event is unknown until the payload is decoded. Every
event runs the same `agent-msg hook claude|codex` command, so argv does not carry the event.
The spec consequence is SR-043 FND-001.

Real-binary matrix, run against a black-hole Redis on every event: instant, 200 ms gaps,
400 ms stall, first byte at 400 ms, slow drip past 900 ms, silent, partial then close,
9 MiB instant, 9 MiB in 1 MiB/200 ms pieces, and about 8 MiB instant or in 1 MiB/150 ms
pieces. Every run exits 0 and leaves exactly one record of the expected kind:
`stdin_timeout`, `too_large`, `malformed`, `budget_exhausted` or a Redis-side timeout.
Nothing wedged.

Rust lane: the PR adds no new `unwrap`/`expect` outside tests, no `unsafe`, no new clippy
`allow` and no `.github` change. Every `Duration` sum and difference saturates. The detached
reader thread is unchanged. `const MS: fn(u64) -> Duration = Duration::from_millis;` in the
tests is an unusual idiom; the sibling `hook_run` tests use a plain `ms()` function. It is a
nit and not recorded as a finding.

Mutation results. Each mutant was run on the scratch copy against `--bin agent-msg --test
hook_stdin`, and the survivors were also run against the `--ignored` lane:

| Mutant | Default lane | Ignored lane |
|---|---|---|
| window never resets (`last_progress` not updated) | killed (3 tests) | - |
| overall cap removed in `left_within_idle` | **survives** | **survives** |
| `STDIN_TOTAL` = 1 h | **survives** | **survives** |
| `STDIN_TOTAL` = 300 ms (the pre-PR behaviour at the wiring) | **survives** | **survives** |
| `IDLE_WINDOW` doubled to 600 ms | **survives** | killed |
| idle window ignored (overall only) | **survives** | killed |
| `saturating_add` to plain `+` in the idle sum | survives (unreachable: clock values are small) | - |
| plain `-` in the idle difference | killed | - |
| plain `-` in the overall difference | killed | - |
| event budget restarted after the read | killed | - |

## Verdict

**PASS WITH FINDINGS.** The code does what the ticket asks and keeps the hard rule: it exits
0, leaves one record and ends well inside 2 s. The new overall cap is the property that
stops a slow drip from running into the harness kill with no record, and no test guards it
in either lane (FND-001). The idle window is guarded only by the wall-clock benchmark lane,
not by the fake-clock unit tests that were written for it.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | Nothing tests the 900 ms overall stdin cap, and the unit tests check only the fault kind, never when the read ended. `tc_n001_m_05_the_overall_budget_caps_input_that_keeps_arriving` passes without the cap: once its 20-step script runs out, the pipe goes quiet and the idle window returns the same `Timeout`. Three mutants survive the default lane and the `--ignored` lane: cap removed in `left_within_idle`, `STDIN_TOTAL` = 1 h (an unbounded slow drip that would reach the 2 s harness kill and leave no record), and `STDIN_TOTAL` = 300 ms (the pre-PR behaviour). Two more survive the default lane: `IDLE_WINDOW` doubled, and the idle window ignored. Fix: have `scripted` return the clock reading at the fault and assert it (exactly 900 ms for the cap test, the last arrival plus 300 ms for the stall test). Make the cap test's script never go quiet. Add a `hook_run`- or binary-level case where a payload completes after 300 ms but before 900 ms and is served, which kills `STDIN_TOTAL` = 300. | crates/agent-msg/src/stdin.rs:342-351, crates/agent-msg/src/stdin.rs:330-340, crates/agent-msg/src/hook_run.rs:116, crates/agent-msg/src/budget.rs:85-97 |
| FND-002 | low | The idle window restarts at the `now` read before the wait, not at the moment the bytes arrived, so the real window is 290–300 ms rather than 300 ms. Real binary, three pieces: gaps of 298 ms time out 4 of 4 runs, gaps of 295 ms time out 1 of 4. The fake-clock case "just under the window (299 ms) is still waiting" passes only because the scripted `wait` moves the clock 299 ms when it was asked for at most 10 ms. It asserts behaviour the binary does not have, and the correct fix (`last_progress = budget.now()` after the chunk) also passes every test. Fix: record the arrival time after the wait, and have the script never advance more than the `wait` it was given. | crates/agent-msg/src/stdin.rs:211-226, crates/agent-msg/src/stdin.rs:294-311, crates/agent-msg/src/stdin.rs:337-339 |
