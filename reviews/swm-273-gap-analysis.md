---
id: SR-6117
title: "Gap analysis — ix-relay SWM-273 against FR-071 (expired-orders reader)"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-071-order-ack-deadline-and-timeout.md, spec/functional/FR-067-persistent-delivery-receipts.md, crates/relay-core/src/store/lua/{order_expire,order_expired_list}.lua, crates/relay-core/src/store/receipts.rs, crates/relay-core/src/receipts.rs, crates/agent-msg/src/main.rs, crates/relay-core/tests/orders.rs, crates/agent-msg/tests/send_order.rs"
review_set: subset
---

# SR-6117: Gap analysis — ix-relay SWM-273 against FR-071 (expired-orders reader)

## Summary

Ticket: SWM-273. PR agent-ix/ix-relay#61 at `b38a341a43157823830cec763ea53ee1a68c0d1c`, scoped to `git diff origin/main...HEAD`. Matrix computed with `quire matrix --scope . --format json` (quoin 0.36.10). The diff adds one section and one criterion to FR-071 (FR-071-AC-11).

## Verdict

**CONDITIONAL.** FR-071-AC-11 is tagged by three tests (`orders_expired_json_lists_the_expired_orders_and_changes_nothing` in agent-msg/tests/send_order.rs:250, and `expired_orders_lists_every_unacted_expired_order_and_changes_nothing` and `expired_orders_returns_at_most_the_cap_and_says_when_more_exist` in relay-core/tests/orders.rs); all pass against redis:7. No stubs. Gaps: the AC-11 test exercises "acted after expiry" only with Acted as the last receipt, which hides the defect in SR-6116 FND-001; the index bounds (8 days after expiry, 10,000 entries) are new behaviour the spec states only as "the receipt bounds of FR-067" (7 days, 10,000 events), which they are not.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | FR-071-AC-11's "acted one excluded" is tested only with Acted as the final receipt; no test records a later receipt (ack, timeout, redelivery), which is the case that fails | crates/relay-core/tests/orders.rs:685 |
| FND-002 | low | The `orders-expired` index bounds (8 days after expiry, 10,000 entries) and the 8-day order-hash TTL from enqueue are unspecified; FR-071 says "aged out under the receipt bounds of FR-067", which are 7 days and 10,000 events | spec/functional/FR-071-order-ack-deadline-and-timeout.md:63 |

## Coverage

- FR-071-AC-11: tagged (3 binders), method Test, passing. Examined.
- FR-071-AC-5 (Acted after AckExpired leaves the escalation): context only, unchanged.
- Reverse gap: `expired_at_ms` is returned but not named in FR-071's item list (harmless extra field, not filed); index trim and coverage boundary are unspecified (FND-002, and SR-6116 FND-002).
- Stubs / coverage inflation: none found.
- Semantic review: skipped (not opted in); intent checks done manually for AC-11 only.
- Plan completion: not assessed

## Dispositions

Round 1, fix head `f984fb7031cdb3c153d0ff8aa5d27a3f52fc68ee`. `quire matrix` shows FR-071-AC-11 bound by five tests, all passing against redis:7.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `an_acted_order_stays_out_of_the_list_whatever_receipt_follows_its_acted` (FR-071-AC-11) records ack, requeue plus redelivery, dead letter and lease timeout after the post-expiry Acted and asserts the list stays empty; AC-11 text names those receipts. |
| FND-002 | fixed | FR-071 now states the index's own bounds (8 days, 10,000 entries, oldest first) and separates them from FR-067's stream bounds; the trim and scan cap are tested in `the_list_reports_its_coverage_unreadable_rows_and_the_spelling_of_the_lane_lists`. |
