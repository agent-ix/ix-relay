---
id: SR-1873
title: "Code review — ix-relay SWM-206/207 Acted receipts and order ack deadlines"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-core/src/store/lua/{activity,common,receipts,receipt_send,order_expire,order_escalate}.lua, crates/relay-core/src/store/{receipts,activity,directory}.rs, crates/relay-core/src/{receipts,config,store}.rs, crates/agent-msg/src/main.rs, crates/relay-watcher/src/main.rs, crates/relay-watcher/src/otlp/{mod,registry,relay,series}.rs, crates/relay-core/tests/orders.rs, crates/agent-msg/tests/send_order.rs"
review_set: subset
---

# SR-1873: Code review — ix-relay SWM-206/207 Acted receipts and order ack deadlines

## Summary

Ticket: SWM-206 (Acted receipts) and SWM-207 (order ack deadlines, AckExpired, escalation). PR agent-ix/ix-relay#56 at `c05c90067205c171914c66eba9a4d6f9b8a14721`, reviewed as `git diff origin/main...HEAD` against FR-067, FR-071 and FR-045 on main. The rust-review lane is folded into this file.

Evidence (macOS, CARGO_BUILD_JOBS=4, throwaway redis:7):

- `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --all --check` clean.
- `cargo test -p relay-core --test orders` 12/12, `cargo test -p agent-msg --test send_order` 3/3, `cargo test -p relay-watcher` 377 + 14 + 1, all with `IX_RELAY_TEST_URL` set.
- `cargo test --workspace` against Redis: see the report (all green).
- Script sizes measured with the same comment/blank stripping as `inflight::script`: the activity write was 572 bytes and is now 13,163 bytes; the expire scan is 13,526 bytes; the escalate script 13,012 bytes.

## Verdict

**CHANGES REQUESTED.** No high findings. Five medium findings: the order escalation's fake `ORDER-0` ticket (lead decision), Acted being written by the very hook call that delivered the message, fleet-global order series pushed by every host, a poison escalation that blocks the pending queue and logs every tick on every host, and a 23x larger activity-write script on every hook event against the starved titan Redis.

Checked and clean:

- **Acted matching.** `receipt_observe` adds an `unacted:<session hex>` row only for a `session/<m>/<s>` recipient at Delivered; `receipt_acted` reads at most `limit` (50) rows, removes each before emitting, and runs inside the activity EVAL, so an event before Delivered cannot see the row (relay execution order, not host clocks; FR-067-AC-12 test). Only prompt/pre_tool/post_tool kinds act.
- **Never Acted twice.** The event id is `<original>/acted/acted` (attempt-independent) and `receipt_emit` refuses an existing `event:` field; `order_close` only closes an `open` order, so the latency is counted once. Residual: the `event:` field is deleted when its event is evicted from the 10,000-event stream, so a redelivery after eviction could emit a second Acted. Bounded by retention; not filed.
- **Deadline clock.** Enqueue sets `due = enqueue + ack`; the first Delivered (`HSETNX delivered_at`) resets it to `delivered + ack` while the order is open; redelivery and lease timeouts do not move it (AC-6 test). Never delivered: expires from enqueue (AC-4 test).
- **Exactly one AckExpired and escalation across restarts and across hosts.** `expire_orders` is one EVAL: it reads due ids, checks `status == open`, emits `<original>/order/ack_expired` (event-id dedup), closes the order and adds it to `orders-escalate`, all atomically, so two watchers cannot both expire it. `escalate_expired_order` is one EVAL that checks `HEXISTS escalation`, files, sets `escalation` and removes the pending row atomically, so two watchers handed the same pending order file one escalation; the second gets 0. Redis scripts are serialised, so this holds across hosts.
- **Acted after AckExpired** stays recorded and the escalation stays (AC-5 test).
- Lead decisions 2 and 3: refusing an order to `all`, and using the receipt outcome name as the last state, are accepted and not filed.
- **Per-tick request count 2 to 3.** The extra request is one EVAL every 2 s (default interval) per host; with nothing due it is two O(log N) sorted-set reads. The request count does not matter on its own; the 13.5 KB script body sent with it does, and is part of FND-005.
- Not a finding: `tc_045_11` (`otlp/mod.rs`) and `otlp/series.rs` will conflict textually with ix-relay#55 (SWM-205), which edits the same NOT_YET_EXPORTED list and CATALOGUE.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | Order escalations use a fake ticket placeholder `ORDER-0` when `--expect-ticket` is absent or not a valid key, and name the session queue as `lead`. Lead decision: give order escalations their own shape (order id, recipient lane, expect_ticket when present, last receipt state), or make the escalation ticket optional for kind=order. | crates/relay-core/src/store/receipts.rs:393-472 |
| FND-002 | medium | Acted is recorded by the same hook call that delivered the message: `run` calls `deliver_mail` (pop, ack, Delivered) and then `emit_activity` (prompt/post_tool), so every hook-delivered order is Acted about 0 ms after delivery, by an event that describes a tool use from before the agent saw it. Acted only discriminates for socket delivery, and the act-latency histogram is dominated by these. | crates/agent-msg/src/hook_run.rs:842-859; crates/relay-core/src/store/lua/activity.lua:19-22 |
| FND-003 | medium | `ix.relay.orders.unacted` and `ix.relay.order.act_latency` are fleet-global store values, but every host's watcher reads and pushes them; the series differ only by `host.name`, so a sum across hosts counts each order N times. | crates/relay-watcher/src/otlp/relay.rs:207-210,286-323 |
| FND-004 | medium | An escalation that fails with a non-connection error stays in `orders-escalate` forever: every watcher retries it every tick and logs it every time (no episode dedup), and the pending read is `ZRANGE 0 19`, so 20 such orders block every later escalation. | crates/relay-watcher/src/main.rs:1141-1172; crates/relay-core/src/store/lua/order_expire.lua:25-28 |
| FND-005 | medium | The hook activity write now prepends common.lua and receipts.lua: the EVAL body goes from 572 to 13,163 bytes, sent and SHA-hashed on every prompt/pre_tool/post_tool of every lane, plus a 13.5 KB scan body every 2 s per watcher host, all against the starved titan Redis. | crates/relay-core/src/store/activity.rs:15-20; crates/relay-core/src/store/receipts.rs:337-349 |
| FND-006 | low | `order_stats` is its own pipelined round trip; it could share the pipeline of the hook-health read, keeping PUSH_REQUESTS at 2 and FR-045's request budget unchanged. | crates/relay-core/src/store/receipts.rs:447-467; crates/relay-watcher/src/otlp/mod.rs:394 |
| FND-007 | low | The Acted match keys only on the session hex (`unacted:<session hex>`), dropping the machine hex the queue name carries; the same session id on two machines would cross-match. | crates/relay-core/src/store/lua/receipts.lua:244-246; crates/relay-core/src/store/activity.rs:96-101 |
| FND-008 | low | `escalate_expired_order` re-implements `Relay::escalate`'s writes without its `validate_session_id` / `validate_safe_text` checks, and derives the key stem by stripping `"e1"` from a probe key. | crates/relay-core/src/store/receipts.rs:421-425; crates/relay-core/src/store.rs:884-912 |
| FND-009 | low | The `with_orders` doc line was inserted inside `for_test`'s doc comment, so `for_test` lost its doc; `OrderLatency` says "cumulative bucket counts" but the buckets are per bucket; `fresh` is an unused local in order_expire.lua. | crates/relay-watcher/src/otlp/relay.rs:337-348; crates/relay-core/src/receipts.rs:316-318; crates/relay-core/src/store/lua/order_expire.lua:14 |
| FND-010 | low | A plain at-most-once inbox pop (`agent-msg inbox`) of an order records `removed`, never Delivered, so the order can never be Acted and AckExpires and escalates although the recipient took it. FR-071 does not address this path. | crates/relay-core/src/store/lua/receipt_plain_pop.lua:7-15 |

### Detail

**FND-001.** Lead decision, recorded as a finding for the coder: an escalation must not carry a fake ticket. Either a dedicated order-escalation shape (order id, recipient lane address, `expect_ticket` when present, last receipt state) or an optional ticket for `kind=order`. The `lead` field should name the recipient lane, not the hex `session/...` queue.

**FND-002.** FR-067 decides "after Delivered" by relay write order, and this ordering satisfies that wording, but FR-067 says Acted means "that the recipient session acted after delivery". For hook delivery the event written after Delivered is the hook's own event (`post_tool` is the tool that just finished). Options: skip `receipt_acted` for the activity write of a call that delivered mail (and let the next event act), or require the acting event to be a later hook invocation. If the team accepts the current semantics, FR-067 should say so explicitly.

**FND-003.** Other relay series are per local lead or carry `machine`. Fleet-wide values belong to one emitter, as the `[fleet]` collector profile does for the fleet series. Push the order series only from the collector host, or document that consumers must take max/any across hosts.

**FND-004.** `escalate_expired_order` can fail before its EVAL on `queue.parse`, `machine.parse`, `session.parse` (for example a non-session queue that does not parse as a machine name) or on serialisation. Fix: on a non-connection failure, mark the order (for example `escalation = failed`) and remove it from the pending set in the store, or log once per order per episode, and read past failed rows.

**FND-005.** `record_activity` deliberately sends the script body (FR-068 allows the hook one request), so its size is paid on every tool call. Only `receipt_ready`, `receipt_emit`, `order_close`, `receipt_acted` and `order_key` are needed: a slim script with just those keeps the hook write near its old size. The same applies to `order_expire`. The extra watcher request itself (2 to 3 per tick) is cheap; the body is the cost.

**FND-010.** Either treat a plain pop of an order as Delivered-equivalent for the deadline and the Acted match, or refuse/flag orders for the plain-pop path, or state in FR-071 that a plain pop expires the order. Low confidence: a spec decision.

## Dispositions

Round 1, fix head `0e986124f43fb7125e9689b30e00100bd30dc2db` (`c05c900..0e98612`: `210f6de` fix round, `0e98612` escalation_failed series). At this head on macOS against redis:7: `cargo clippy --workspace --all-targets -D warnings` clean, `cargo fmt --check` clean, `cargo test --workspace` green (see the report). Activity-write script body now 4.9 KB, sent by SHA.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `ORDER_ESCALATION_TICKET` is gone; `EscalationFiling.ticket` is `Option<TicketId>`, `None` only for `kind = "order"` (enforced by `EscalationFiling::validate`); `lead` is the recipient lane address (queue name only as fallback); the expected ticket is named in the context. Linear posting refuses a ticketless escalation with `LinearError::NoTicket`. Compatibility note filed as FND-014. |
| FND-002 | fixed | `run` sets `event.delivered_mail` from `deliver_mail`'s result and `record_activity` passes `ARGV[10] = 0`, so `receipt_acted` is skipped for that call; tested in hook_run (`the_event_of_a_call_that_delivered_mail_is_marked_...`) and orders.rs. FR-067 now states the rule. |
| FND-003 | fixed | `hook_health_and_order_stats(max, machine)` returns only unacted rows whose lane server is this machine, the latency of `order-latency:<machine hex>` and this machine's failed count; FR-045's series table now says a sum over hosts counts each order once. Edge filed as FND-013. |
| FND-004 | fixed | A non-connection escalation error calls `order_escalation_failed`; after 3 attempts the order gets `escalation = failed`, leaves `orders-escalate` and joins a per-host failed set (7 days, 1,000 cap), exported as `ix.relay.orders.escalation_failed` (FR-045-AC-34). The log still prints once per attempt, at most 3 times per order. |
| FND-005 | fixed | Hot scripts use `slim_script` (slim_prelude + receipt_core): activity write 13,163 to about 4.9 KB, sent by `EVALSHA` with `EVAL` on `NOSCRIPT`; `the_hot_scripts_carry_only_the_receipt_core` pins the size. FR-068 round-trip note filed as FND-012. |
| FND-006 | fixed | The order reads ride the hook-health pipeline (`XREVRANGE, HGETALL, HGETALL, ZCARD`); `PUSH_REQUESTS` is back to 2 and FR-038/FR-045 say two. |
| FND-007 | fixed | The unacted key is `unacted:<machine hex>:<session hex>`, with the machine taken from the lane address in `record_activity`; `an_event_of_the_same_session_id_on_another_machine_does_not_act` tests it. |
| FND-008 | fixed | `escalate_expired_order` calls `filing.validate()` (shared with `Relay::escalate`) and uses the new `Keys::esc_stem`. |
| FND-009 | fixed | The `with_orders` doc no longer splits `for_test`'s doc; `OrderLatency` says per-bucket; the unused `fresh` local is gone. |
| FND-010 | fixed | `receipt_plain_pop.lua` calls `receipt_observe(..., 'inbox_read')` before `removed`, so the order's deadline restarts and the next activity event can act; new `DeliveryEvidence::InboxRead`; FR-067 states it. |
| FND-011 | fixed | Round 2 (`3a93eeb`, rebased on main `0ef4efc`): `Health.orders` is `Option<OrderStats>`, set only by a successful `hook_health_and_order_stats`; `fill_orders` returns before emitting anything when it is `None`. The failure stays visible as `health_snapshot.read_failures{snapshot=hook_health_newest}` and the once-per-episode log (lead decision: the shared label is acceptable because the order read rides that request); FR-045 says so. `tc_045_35_a_failed_order_read_leaves_every_order_series_out_not_at_zero` tests it (its trace tag is filed as SR-1874 FND-004). |
| FND-012 | fixed | Round 2: FR-068 now states that the write is sent by SHA and that the first write after a Redis restart or script flush takes two round trips; the CHANGELOG repeats it. |
| FND-013 | fixed | Round 2: `Relay::send_as` refuses an order whose stored `to` does not parse as an `Address`, before anything is written, and `agent-msg send --order` refuses a recipient without a directory proof; `an_order_to_a_recipient_without_a_lane_address_is_refused_and_nothing_is_queued` (relay-core) and `an_order_to_a_recipient_that_is_not_a_lane_address_is_refused_and_nothing_is_queued` (agent-msg) test both. |
| FND-014 | fixed | Round 2: the CHANGELOG entry gives the upgrade order: agent-msg, relay-tui and ix-board on every machine before any relay-watcher runs this version. |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-011 | high | A failed hook-health/order read leaves `health.orders` at its default, and `fill_orders` still emits `ix.relay.orders.escalation_failed = 0` and an all-zero `ix.relay.order.act_latency`: a failed source reads as a healthy 0, against FR-045 ("a read that fails leaves its series out") and the FR-070 principle. Order series must be omitted when the read fails and the failure must be visible. | crates/relay-watcher/src/otlp/relay.rs:207-238,284-327 |
| FND-012 | low | `CachedScript::run` makes the hook activity write two round trips (`EVALSHA`, `NOSCRIPT`, `EVAL`) on the first write after a Redis restart or script flush; FR-068 says an event write adds at most one round trip. | crates/relay-core/src/store/inflight.rs:159-176; crates/relay-core/src/store/activity.rs:46-61 |
| FND-013 | low | Orders whose stored `to` is not a `server:workspace:tab` address (a session queue without a verified place, or a plain lead name) are counted in `orders-unacted` but exported by no host, and a given-up escalation of such an order is counted under `orders-escalate-failed:` with an empty host, which no watcher reads. | crates/relay-core/src/store/receipts.rs:510-518; crates/relay-core/src/store/lua/order_escalate_failed.lua:12-14 |
| FND-014 | low | The stored `EscalationFiling.ticket` became optional: an agent-msg, relay-tui or ix-board build from before this change cannot decode a ticketless order escalation (it lands in `rejected`), so readers must be upgraded before watchers file order escalations; this is not documented. | crates/relay-core/src/escalation.rs:213-216 |

### Detail (disposition pass 1)

**FND-011.** Coordinator ruling, recorded as a finding. Make `Health.orders` an `Option<OrderStats>` set only on a successful read, emit no order series when it is `None`, and keep the failure visible (the existing `health_snapshot_read_failures` counter with its own snapshot label, plus the once-per-episode log). Add a test that a failing read produces no `orders.*` or `order.act_latency` point.

**Rebase note (not a finding).** ix-relay#55 (SWM-205) merges first; #56 then needs a rebase that touches the `NOT_YET_EXPORTED` list in `tc_045_11` (`otlp/mod.rs`) and the CATALOGUE in `otlp/series.rs`.

**Round 2 rebase check.** `3a93eeb` is one squashed commit on main `0ef4efc` (#55 merged). A comparison of this PR's own patch before (`a5333bf..0e98612`) and after (`0ef4efc..3a93eeb`) shows only the round 2 fixes above and the rebase resolutions. `NOT_YET_EXPORTED` in `tc_045_11` is now empty. The CATALOGUE holds #55's series (`ix.fleet.lane.no_events`, `compile_seconds`, `focus_info`, `ix.fleet.config`, `ix.relay.hook.activity_skipped`) and this PR's three order series. FR-045's table lists all of them. #55's `activity_last_many` sits beside this PR's slim `record_activity` in `store/activity.rs` without conflict, and `relay.rs` keeps #55's read failures counting with the order stats on the hook-health request. `make ci` result is in the report.
