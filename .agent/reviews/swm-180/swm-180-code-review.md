---
id: SR-1859
title: "code-review review of SWM-180"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; spec/functional/FR-066-bounded-work-history.md; PR45 changed source"
review_set: subset
---

## Summary

FAIL: 1 finding(s) require disposition. Reviewed candidate0761f1baa421337a9419b8e9efc20988751c91dc against origin/main using the existing worktree. No applicable AssuranceProfile was found under spec/.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | Idle history reads return events older than the seven-day retention limit. | FR-066; crates/relay-core/src/store/work.rs:191-195 |

## Scope

- FR-066 (examined), spec/functional/FR-066-bounded-work-history.md: When a lead reports scope, assignment, removal or explicit pause/resume, the relay SHALL append an immutable session-qualified transition atomically with its current-state update.
- FR-066-AC-1 (examined), spec/functional/FR-066-bounded-work-history.md: Scope, short stage changes, activity corrections and removals appear in order with stable event IDs; a malformed history storage key prevents current-state mutation.
- FR-066-AC-2 (examined), spec/functional/FR-066-bounded-work-history.md: Repeated cursor reads return identical retained events without modifying current assignments or mail state; malformed cursors and oversize page requests fail.
- FR-066-AC-3 (examined), spec/functional/FR-066-bounded-work-history.md: Events preserve verified native session/process identity; focus text is absent from history and bootstrap snapshots. Replacement sessions retain their separate reporting incarnation.
- FR-066-AC-4 (examined), spec/functional/FR-066-bounded-work-history.md: More than 4,096 writes evict the oldest events; a cursor preceding retained data reports truncation and the retained lower bound.

## Evidence and verdict

The Rust lane reviewed every changed source file, Lua mutation/read boundaries, shared wire types, CLI routing, validation and real Redis tests. No production code was edited. Existing main TUI code is outside this PR review.

Age trimming occurs only in HISTORY at line39. work_events XRANGE at lines191-192 reads from the requested cursor without an age floor. The stream TTL is extended on every transition, so old entries can remain after a quiet interval even though available_from_ms advances to now minus seven days.

Focused reproduction used an isolated Redis Unix socket /tmp/swm180-review.sock. Seed a valid activity event at server-time minus eight days and another at minus one day; set the remaining stream TTL to six days. This is the state left after a write two days earlier trimmed entries at its then-current seven-day boundary. Run target/debug/agent-msg --url redis+unix:///tmp/swm180-review.sock --prefix age-review: work events --after 0-0 --limit 100. The result was observed_at_ms=1795416860184, available_from_ms=1794812060184, yet the first event had at_ms=1794725642000 and truncated=false. Reads must exclude expired rows and report consistent lower-bound/cursor metadata while preserving read-only semantics. Physical retention must also meet the specified maximum age, or the contract must explicitly describe lazy physical expiry with a strict readable horizon.

Five real Redis integration tests passed with IX_RELAY_TEST_URL set. FR066 grammar validation passed. Exact-head aggregate make ci was reported passed by the lead; this reviewer did not run the aggregate gate.

The source diff contains13 changed files. Review artifacts are reviewer custody files only; the lead must retain them under reviews/ when landing. GitHub metadata showed the reviewed head is MERGEABLE, which does not override these findings or the final gate.

## Dispositions

Round1, verified fix10999dacefe7eafe310e4d5765272a94735bf18f and final frozen candidate7316c0477386b26e22aa99a8fe6e110cd63163a2. Original findings above remain unchanged.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | work_events computes the Redis server-time cutoff before XRANGE, excludes expired rows from events and coverage, advances empty cursors and reports truncation. The idle regression passes; FR066 now explicitly distinguishes the strict readable seven-day horizon from bounded lazy physical cleanup and inactive-stream expiry. |

Latest scoped code verdict: PASS, no new findings. Seven real Redis work tests passed independently at10999da; final7316c04 changes only requirement wording and a source comment, so no repeated behavioral test was needed. The final requirement passes quire validation. Local git merge-tree --write-tree against origin/mainff618de succeeded without conflicts for7316c04, yielding tree0d5b33abfc69a8aa4a57e65ff3445ccc5466e285. No live GitHub API mergeability probe or aggregate gate was run. The lead owns the final gate and exact-head merge.

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

## Additional review (disposition pass 3)

Inspected only dea4cf..f0b4137517de4cbbb77791b50b28a5c56994fbd2: one test file, crates/agent-msg/tests/send_state.rs. No production behavior changed, no new findings, and original findings remain fixed.

The stalled-Redis fixture previously searched raw TCP chunks for LLEN, which can occur inside the enqueue Lua script. That interception could prevent enqueue rather than stall the subsequent bounded state probe. The supplied exact-main reproduction log shows exit1/error35 from that mistaken interception. The replacement uses the existing redis::Parser to read complete RESP commands and repacks binary arguments unchanged. It stalls only an EVAL with exactly two keys matching the expected session inbox and watcher heartbeat, which agrees with Relay::recipient_probe. Unrelated scripts containing LLEN are forwarded. Its classification regression checks two framed commands, positive classification and both wrong-key exclusions.

The existing degradation test still asserts status0, exact plain queued output, a lower200ms wait, upper healthy baseline plus250ms+150ms, and two queued inbox messages. No assertion, deadline or request-count oracle is relaxed. Independent isolated loopback Redis run passed all5 send_state tests and all4 round_trips tests. Local merge-tree is conflict-free (treef970beb08add204d991d21cd18ec219918edfed4). Aggregate gate remains with the lead.

Examined FR-041-AC-3, spec/functional/FR-041-send-prints-the-recipients-state.md: Given a Redis that stops answering once the message is queued, `send` prints the plain `queued for <name>`, exits 0, takes no more than the same send against a healthy Redis plus 250 ms and a margin of 150 ms (so a bound of a second, or one per reply, fails), and the message is in the inbox.

Latest scoped verdict: PASS/MERGEABLE at f0b4137517de4cbbb77791b50b28a5c56994fbd2, subject to the lead's final gate and exact-head merge.

## Additional review (disposition pass 4)

Inspected be2629b..acd36c66812aa1d05fdd6be424290a78396a8d38: exactly two additions in the cfg(test) alternate-prefix key array, receipt_stream_key and receipt_meta_key, immediately after log_key. The production builders, default table and expected key strings already include both entries in that same order. No production behavior or assertion changed.

The test still compares complete array counts before pairing every default key with its alternate-prefix key, then checks exact equality after replacing only the prefix. Adding the missing two calls restores the intended test domain; it does not reduce coverage or relax count/equality assertions. The supplied exact-main failure log reports39 versus37, matching this inherited omission; the feature branch has three additional history keys but the same two-entry omission. This is a narrowly justified test-fixture repair.

Independent focused command cargo test --locked --offline -p relay-core --lib store::keys::tests passed5/5, including prefix preservation, documented keys, builder coverage and the existing key guards. The worker-provided full affected relay-core log records415 passed,6 ignored,0 failed; that broader run was inspected as worker evidence and was not rerun by this reviewer. No aggregate gate ran here.

No new findings. All previous finding dispositions remain fixed. Local merge-tree completed conflict-free and yielded tree1b5fbc046e98fa6293ce6e0b97b99813edb679e3. Latest scoped verdict: PASS/MERGEABLE at acd36c66812aa1d05fdd6be424290a78396a8d38, subject to the lead's final gate and exact-head merge.

Examined source unit: store::keys::tests::every_key_carries_its_prefix_and_nothing_else_changes at crates/relay-core/src/store/keys.rs; count check: assert_eq!(default.len(), moved.len());
