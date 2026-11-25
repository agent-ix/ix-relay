---
id: SR-6128
title: "Gap analysis of ix-relay PR 30 against FR-044, FR-046, FR-054, FR-014 and FR-018"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; PR #30 at f2cbcc8; spec/functional/{FR-014,FR-018,FR-044,FR-046,FR-054}*.md, spec/tests.md, crates/relay-watcher/src/**, crates/agent-msg/src/{claude_inbox,hook_run}.rs, crates/agent-msg/src/doctor/checks.rs"
review_set: subset
relationships:
  - type: references
    target: "ix://agent-ix/ix-relay/FR-044"
  - type: references
    target: "ix://agent-ix/ix-relay/FR-054"
---

# SR-6128: Gap analysis of socket-only delivery (SWM-121 to SWM-124)

## Summary

Ticket: SWM-121. PR agent-ix/ix-relay#30 at `f2cbcc8e48953c5d7d5c41b515534c1642b8773b`, changed requirements checked against code and tagged tests. Every test name listed in spec/tests.md for these requirements exists, and `check-test-matrix` passes.

## Verdict

**CONDITIONAL**: two medium findings where the code does not do what FR-044 states, two low trace findings.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | FR-044 requires the wait reason to be kept in the status file, and names unavailable socket locations as such a case. In `Sockets::Unavailable` there is no deliverer, so no status file is written; doctor says the watcher has not reported and advises starting it, while every Claude lead on the host waits. The reason is only in the watcher's start log and one line per lead. | crates/relay-watcher/src/socket.rs:141-143; crates/agent-msg/src/doctor/checks.rs:222-227 |
| FND-002 | medium | FR-044 still says doctor names the possible causes of an ambiguous batch (a transcript under another `CLAUDE_CONFIG_DIR`, authentication, a cleared or busy session, a changed wire format). The PR deleted the text that did this. In addition a `socket_pending` note is excluded from the problem count, so a transcript that can never be found reads `ok` until the batch is dead-lettered. | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md:227-229; crates/agent-msg/src/doctor/checks.rs:278-295 |
| FND-003 | low | Two tests are tagged FR-044-AC-18 and assert something else, and neither is listed in spec/tests.md: `pending_socket_consumption_is_not_reported_as_a_problem` asserts doctor classification (AC-9), and `tc_044_18_pending_capacity_defers_without_evicting_socket_ownership` asserts the 256 capacity rule, which no criterion states. | crates/agent-msg/src/doctor/checks.rs:4163-4166; crates/relay-watcher/src/socket/tests.rs:2427-2430 |
| FND-004 | low | FR-044-AC-20 says forwarded and redelivered mail stays queued; `tc_044_20` sends two ordinary messages and exercises neither forwarding nor redelivery. | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md:296; crates/relay-watcher/src/socket/tests.rs:2347-2381 |

## Coverage

`quire matrix` computes every criterion in the repository as untagged because the module set does not bind the repository's `Tracing:`/`ACs:` comment form (`trace-id-outside-marker`); this predates the PR, so a grep index was used. Every criterion of FR-044 (1 to 20), FR-046 (1 to 19) and FR-054 (1 to 4) has at least one tagged test. FR-014-AC-2 to AC-4 and FR-018-AC-1 to AC-3 have none by this index; their text did not change and they are outside this review.

Semantic review: limited to the changed tests. Plan completion: not assessed

## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | Unavailable sockets write a status file and doctor warns with the reason. |
| FND-002 | fixed | Doctor warns for old or untraceable posts and names the causes. |
| FND-003 | fixed | Tests retagged to AC-9 and AC-24 and listed in spec/tests.md. |
| FND-004 | fixed | The test now covers forwarded and redelivered mail. |
