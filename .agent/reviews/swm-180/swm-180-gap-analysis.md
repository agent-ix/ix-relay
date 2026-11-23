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

## Additional review (disposition pass 2)

Inspected added changes from3f31fa07 through frozenb0ee841ee1fe9130e699d23e40ab6bda5c1540cd. All original history findings retain their prior fixed dispositions; no new findings were identified.

The combined ack_delivered uses one EVAL after the hook's successful stdout write and flush. Matching batch/pop ownership is checked before observation; stale or mismatched batches cause no writes. receipt_ready preflights all receipt/inflight key types. Delivery is recorded before acknowledged, pending uncertainty is cleared only for an observed entry, and inflight removal follows successful receipt writes. Plain ack retains its prior uncertainty semantics. Deduplication handles prior observation and a lost committed reply without another original, attempt or receipt. Receipt helpers share one owner rather than duplicate the observation logic.

The test-oracle migrations are justified by the accepted stored envelope. Canonical InboxEntry projection retains every public Message field without sender/recipient/timestamp normalization. The deadletter test still asserts dead.messages == [message(0)] through full Message equality, and additionally compares exact raw final-attempt envelope bytes while checking no redelivery spaces and stable original/attempt metadata. Thus it preserves public content equality and storage byte preservation. Byte-cap fixtures use actual queued envelope lengths plus the unchanged row-overhead bound and keep the one-byte-too-large/exact-fit assertions. Receipt retention fixtures anchor their query interval before writes, so slower execution cannot exclude the events under test. Existing CLI round-trip counts are unchanged.

Independent focused checks at this exact source head:27 inflight model tests,16 receipt tests,7 work tests,2 receipt CLI tests and4 round-trip tests passed. The first round-trip invocation used the Unix Redis URL and all four tests refused their documented TCP-only fixture precondition; rerunning against the same isolated Redis over loopback TCP passed4/4. This was a fixture mismatch, not a source failure. No gate assertions were relaxed.

The computed matrix reports all4 FR066 criteria and all8 FR067 criteria tagged. Whole-repository inherited coverage remains outside this scoped assurance claim; Plan completion: not assessed. The aggregate gate was not run. Local git merge-tree --write-tree against origin/main completed conflict-free, yielding treed1a7cc775c8a9030c5bc2de659e1d78bb718ef3d. Latest scoped verdict: PASS/MERGEABLE, subject to the lead's final gate and exact-head merge.

### Additional examined scope

- FR-067-AC-1: In an isolated Redis fixture, enqueue, deliver and acknowledge one message, then export its receipts. Queued, delivered and acknowledged events have distinct stable event identities in recorded order, one original identity, the applicable attempt identity, source-native sender/target, event/observation times and delivery evidence kind. A queued-only message has no delivered or acknowledged event.
- FR-067-AC-2: Enqueue two byte-identical messages, expire the first attempt of one, restart the watcher or consumer, redeliver and acknowledge it. The two originals remain distinct; the recovered message retains its original identity with distinct attempt identities and timeout/redelivery outcomes. Repeating the same export or settlement does not create or count another event.
- FR-067-AC-3: With equal or backwards source times, export order remains stable. An unavailable source event time is explicitly unknown. A held or ambiguous transport observation produces no fabricated delivery; socket delivery acknowledged as unconfirmed retains that evidence kind and never claims recipient reading or action.
- FR-067-AC-4: Drop the connection around enqueue and acknowledgement transactions. After recovery, each committed enqueue has its queued receipt, and each committed valid acknowledgement has its acknowledged receipt; an uncommitted or stale acknowledgement does not manufacture a success. Inject a delivery-observation recording failure and verify that the failure and incomplete interval remain visible after recovery while existing recovery retains the original message.
- FR-067-AC-5: Advance the fixture clock beyond seven days and separately append 10,001 events. Retained history obeys both bounds; evicted history and an evicted continuation cursor report incomplete coverage with a reason. A message still waiting across receipt expiry retains its original identity on later delivery.
- FR-067-AC-6: Request an interval before instrumentation, an interval with a known recording gap, a source that cannot be read, and a covered interval with no events. Only the last is complete empty history. A request with more than 1,000 matching events returns bounded pages and continuation metadata without silently dropping the remainder.
- FR-067-AC-7: Snapshot inbox, in-flight and receipt state, export all pages twice and compare state byte for byte. Reads leave all three unchanged, including events past their retention age that have not yet been trimmed by a writer. Exported data contains none of a seeded secret message body, transcript or credential value and no prose-derived semantic classification.
- FR-067-AC-8: Exercise successful delivery, definite failure, ambiguous failure, lease recovery, exhausted redelivery, stale acknowledgement and a plain-inbox pop with receipt collection enabled. Existing ordering, markings and recovery outcomes remain unchanged; ordinary message-history JSON still contains each original send once and no requeue entry.
