---
id: SR-1860
title: "gap-analysis review of SWM-180"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-066-bounded-work-history.md; PR45 changed source"
review_set: subset
---

## Summary

FAIL: 3 finding(s) require disposition. Reviewed candidate0761f1baa421337a9419b8e9efc20988751c91dc against origin/main using the existing worktree. No applicable AssuranceProfile was found under spec/.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | No test exercises seven-day expiry during an idle interval; the count-retention test cannot detect age-retention violations. | FR-066; crates/relay-core/tests/work.rs:331-363 |
| FND-002 | medium | History tests never publish a replacement session or process and replay both incarnations, leaving the replacement-incarnation obligation unverified. | FR-066-AC-3; crates/relay-core/tests/work.rs:246-329 |
| FND-003 | medium | The installed computed matrix rejects both new Trace doc comments as unmarked-trace-id, so none of the four FR-066 criteria has a bound source test. | FR-066-AC-1; crates/relay-core/tests/work.rs:243-244,331-332 |

## Scope

- FR-066 (examined), spec/functional/FR-066-bounded-work-history.md: When a lead reports scope, assignment, removal or explicit pause/resume, the relay SHALL append an immutable session-qualified transition atomically with its current-state update.
- FR-066-AC-1 (examined), spec/functional/FR-066-bounded-work-history.md: Scope, short stage changes, activity corrections and removals appear in order with stable event IDs; a malformed history storage key prevents current-state mutation.
- FR-066-AC-2 (examined), spec/functional/FR-066-bounded-work-history.md: Repeated cursor reads return identical retained events without modifying current assignments or mail state; malformed cursors and oversize page requests fail.
- FR-066-AC-3 (examined), spec/functional/FR-066-bounded-work-history.md: Events preserve verified native session/process identity; focus text is absent from history and bootstrap snapshots. Replacement sessions retain their separate reporting incarnation.
- FR-066-AC-4 (examined), spec/functional/FR-066-bounded-work-history.md: More than 4,096 writes evict the oldest events; a cursor preceding retained data reports truncation and the retained lower bound.

## Evidence and verdict

Plan completion: not assessed

The whole-repository computed matrix was run with quoin matrix --repo . --json. It enumerated 68 requirements. The PR introduces FR066; inherited criteria and inherited Python gate scripts are outside this bounded change review and are not certified clean. All four FR066 criteria report static_status=untagged, evidence_status=no run evidence, with unbacked_claims reason=unmarked-trace-id for tests/work.rs lines243 and331. quire coverage reports active Rust form rust-trace-attribute, 1437 candidates, 915 tagged and222 bound; the current active module does not bind these doc-comment claims. The house Rust style permits Trace comments, so this is an integration mismatch requiring reconciliation, not a claim that the behavioral tests never run. Do not rewrite inherited whole-repo tags as part of this PR.

Manual semantic inspection confirms the first test asserts stages/removal, repeated cursor reads, owner binding, focus exclusion and basic cursor refusal; the second asserts malformed storage atomicity and exact4096 count. Neither tests time passage, nor replay of writes from replacement sessions/PIDs. Existing FR048 tests only inspect absent current state for other incarnations.

Reverse ownership mapping: new shared wire, conversion module, CLI events/snapshot/pause/resume, Redis event keys and storage are owned by FR066; original current reports remain FR048. No hollow stub or duplicated wire owner found in changed code. Intentional wire reexports are compatibility, not stubs. Optional automated semantic judgment was not invoked. The five focused Redis tests passed with configured Redis, so no early-return skip was counted as evidence.

The source diff contains13 changed files. Review artifacts are reviewer custody files only; the lead must retain them under reviews/ when landing. GitHub metadata showed the reviewed head is MERGEABLE, which does not override these findings or the final gate.

## Dispositions

Round1, inspected frozen fix10999dacefe7eafe310e4d5765272a94735bf18f. Seven focused real Redis work tests passed independently with IX_RELAY_TEST_URL set to an isolated Unix socket. The coder's red log shows the idle regression failed against the original implementation before passing with the fix.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | idle_history_excludes_expired_events_and_advances_empty_cursors_without_writes seeds server-relative expired and recent entries, asserts age exclusion, pagination, truncation, unchanged storage and empty-cursor progress. |
| FND-002 | fixed | replacement_sessions_and_processes_keep_separate_history_and_current_state writes three incarnations, replays all bindings and checks independent current state. |
| FND-003 | fixed | All four new FR066 criteria now bind through recognized #[trace] attributes; the computed matrix reports each tagged. Inherited untagged criteria remain outside this PR scope. |

Latest scoped gap verdict: PASS. Plan completion: not assessed. This verdict does not certify inherited repository-wide coverage or run evidence.
