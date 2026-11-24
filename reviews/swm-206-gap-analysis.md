---
id: SR-1874
title: "Gap analysis — ix-relay SWM-206/207 against FR-067, FR-071, FR-045"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-067-persistent-delivery-receipts.md, spec/functional/FR-071-order-ack-deadline-and-timeout.md, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md, spec/functional/FR-038-watcher-wakes-when-a-message-is-sent.md, crates/relay-core/src/store/**, crates/relay-core/tests/orders.rs, crates/agent-msg/tests/send_order.rs, crates/relay-watcher/src/{main.rs,otlp/**}"
review_set: subset
---

# SR-1874: Gap analysis — ix-relay SWM-206/207 against FR-067, FR-071, FR-045

## Summary

Ticket: SWM-206 and SWM-207. PR agent-ix/ix-relay#56 at `c05c90067205c171914c66eba9a4d6f9b8a14721`. Planless gap audit: Plan completion was not assessed.

Matrix: `quoin matrix --json` (quoin 0.36.10) binds `#[trace(...)]` tags. FR-067-AC-9 to AC-12 and FR-071-AC-1 to AC-8 and AC-10 are tagged; FR-071-AC-9 is untagged. The watcher tests' `/// ACs:` doc comments (tc_071_03, the idle-tick test) are not read by the matrix; that is a pre-existing tooling limit.

Each tagged test was read and run against redis:7: relay-core `orders` 12/12, agent-msg `send_order` 3/3, relay-watcher 377 + 14 + 1.

Reverse gaps: none. Every new production path maps to FR-067, FR-071 or FR-045.
Stubs: none. The PR states FR-071-AC-10's `focus_info` half and FR-072 are out of scope (SWM-205 owns the hook feed).

## Verdict

**GAPS.** One medium: FR-071-AC-10 is tagged as covered but only its `expect_ticket` half is implemented. Two low: FR-071-AC-9 has no tagged test, and the cross-session negative of FR-067-AC-10 covers only another session id on the same machine.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | FR-071-AC-10 is tagged on two tests that check only the expected ticket in the receipt export; the `focus_info` half is not implemented here (deferred to SWM-205), so the matrix reports the AC covered while it is not met. | crates/relay-core/tests/orders.rs:438-458; crates/agent-msg/tests/send_order.rs:195-212 |
| FND-002 | low | FR-071-AC-9 (no key scan, at most one added pipelined request per scanning tick) has no tagged test; `an_idle_tick_is_three_requests_and_no_herdr_call` asserts it but carries no FR-071-AC-9 tag, so the matrix reports it untagged. | crates/relay-watcher/src/main.rs:4069-4113 |
| FND-003 | low | The FR-067-AC-10 "event of another session" negative uses another session id on the same machine; no test sends the same session id from another machine, the case the session-hex-only key would cross-match (SR-1873 FND-007). | crates/relay-core/tests/orders.rs:153-194 |

### Detail

**FND-001.** Remove the FR-071-AC-10 tag, or tag only the receipt-export half with a note that `focus_info` lands with SWM-205, so the AC reads as open until it does.

## Dispositions

Round 1, fix head `0e986124f43fb7125e9689b30e00100bd30dc2db`. `quoin matrix --json` re-read at this head.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | Taken the second way the finding offered: the FR-071-AC-10 tag stays on the receipt-export half, and orders.rs now states above the test that AC-10 is met only with its `focus_info` half (SWM-205). |
| FND-002 | fixed | `an_idle_tick_is_three_requests_and_no_herdr_call` carries `#[trace("FR-038-AC-5", "FR-071-AC-9")]`; the matrix reports FR-071-AC-9 tagged. |
| FND-003 | fixed | `an_event_of_the_same_session_id_on_another_machine_does_not_act` (tagged FR-067-AC-10) sends the same session id from another machine and asserts no Acted. |

## New findings (disposition pass 2)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-004 | low | `tc_045_35_a_failed_order_read_leaves_every_order_series_out_not_at_zero` is traced to FR-045-AC-11 (the catalogue equals the series table), which it does not test. No FR-045 AC states that a failed order read leaves the order series out, so the rule exists only in prose. The test is also a doc-comment tag, which the matrix does not read. | crates/relay-watcher/src/otlp/relay.rs:425-444 |
