---
id: SR-6116
title: "Code review — ix-relay SWM-273 read-only expired-orders reader"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-core/src/store/lua/{order_expire,order_expired_list}.lua, crates/relay-core/src/store/receipts.rs, crates/relay-core/src/receipts.rs, crates/agent-msg/src/main.rs, crates/relay-core/tests/orders.rs, crates/agent-msg/tests/send_order.rs, spec/functional/FR-071-order-ack-deadline-and-timeout.md, CHANGELOG.md"
review_set: subset
---

# SR-6116: Code review — ix-relay SWM-273 read-only expired-orders reader

## Summary

Ticket: SWM-273. PR agent-ix/ix-relay#61 at `b38a341a43157823830cec763ea53ee1a68c0d1c`, reviewed as `git diff origin/main...HEAD` (8 files, +385/-2) against FR-067, FR-071 and the `Address` case rule. The rust-review lane is folded into this file.

Evidence (macOS, CARGO_BUILD_JOBS=4, CARGO_TARGET_DIR in the worktree, throwaway redis:7):

- `cargo fmt --check` clean; `cargo clippy -p relay-core -p agent-msg --all-targets --all-features -- -D warnings` clean.
- `cargo test -p relay-core --test orders` 19/19 and `cargo test -p agent-msg --test send_order` 5/5 with `IX_RELAY_TEST_URL` set.
- Reproduction of FND-001 from an out-of-tree scratch binary against the same Redis, using only public `relay-core` API: deliver (observe only), expire, escalate, Acted, then `ack` of the same batch. `expired_orders()` lists 0 after Acted and 1 after the ack (`last_state` "delivered").
- `git merge-tree --write-tree origin/main HEAD` succeeds: no conflict with main (c7b3c6a, Release 0.4.0). GitHub reports MERGEABLE.

## Verdict

**CHANGES REQUESTED.** One high finding (FND-001): the "acted after expiry" exclusion keys on the order hash's mutable `state` field, so any receipt recorded after `Acted` (acknowledged, timed_out, requeued, redelivered, dead_lettered) puts an acted order back on the list. Two medium findings: the reader has no coverage boundary, so an empty result cannot be told apart from an index that is not filled yet (pre-upgrade expiries, expiries handled by an old-version watcher during a rolling upgrade, rank trim), and CHANGELOG has no entry and no `## Unreleased` section after 0.4.0.

Checked and clean:

- **Writes nothing.** `order_expired_list.lua` issues only `TYPE` (via `receipt_ready`), `ZREVRANGE` and `HMGET`; `receipt_emit`/`order_close` are declared by the shared prelude but never called. The relay-core test compares a `DUMP` of every key under the bus prefix (`KEYS prefix*`) before and after, as a map, so a created, removed or changed key fails it. `DUMP` does not cover TTLs, but the script issues no expire command.
- **Atomic index write.** `ZADD orders-expired` runs in the same EVAL as `receipt_emit(ack_expired)`, `order_close` and `ZADD orders-escalate`; no window where AckExpired exists but the index row does not (for a watcher of this version).
- **Trim vs hash TTL.** The order hash expires 691,200 s after enqueue; the index entry is trimmed 691,200,000 ms after expiry, so an entry outlives its hash by at most the ack deadline (max 24 h). The reader skips such rows (`h[1]` nil). Both trims run inside the existing expire script: no extra round trip (FR-071-AC-9 unaffected).
- **Cap and truncation.** The scan returns exactly 500 rows and sets `truncated` only on finding a 501st qualifying row; the test pins 501 → 500 + true.
- **Deadline.** `deadline_ms` = `delivered_at` + `ack_ms`, else `enqueued_at` + `ack_ms`, matching the `orders-due` scores written at enqueue and first Delivered (FR-071 Behavior bullets 2-3).
- **Recipient.** Through the CLI the stored `to` is `registration.full_address()` (directory send) — the registry spelling at send time, server segment lowercased. See FND-005 for what that leaves open.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | Acted-after-expiry exclusion tests the mutable `state` field; a receipt after Acted (ack, lease timeout, requeue, redelivery, dead letter) overwrites it and the acted order is listed again (reproduced) | crates/relay-core/src/store/lua/order_expired_list.lua:17 |
| FND-002 | medium | No coverage boundary: an empty or short list is indistinguishable from an index not yet filled (pre-upgrade expiries, expiries scanned by an old relay-watcher on another host during a rolling upgrade, 10k rank trim while hashes live) | crates/relay-core/src/store/receipts.rs:398 |
| FND-003 | medium | CHANGELOG.md has no entry for the new command or the watcher-side index, and no `## Unreleased` section after the 0.4.0 release; the upgrade order (watcher first) is undocumented | CHANGELOG.md:3 |
| FND-004 | low | `delivered` is documented as "whether a Delivered receipt was recorded" but reads `delivered_at`, which is set only while the order is open; an order delivered after expiry reports `delivered: false` | crates/relay-core/src/receipts.rs:351 |
| FND-005 | low | `recipient` is the registry spelling at send time, not normalised and not followed across FR-043 moves; Address equality is case-insensitive, but the field doc does not tell consumers to compare it as an Address | crates/relay-core/src/receipts.rs:340 |
| FND-006 | low | Read-only is enforced only by the test: the script runs via EVALSHA/EVAL with the write-capable slim prelude, not EVALSHA_RO or a `no-writes` shebang flag | crates/relay-core/src/store/receipts.rs:400 |
| FND-007 | low | `escalation` decode maps every non-empty value other than `failed` to `Filed` (catch-all arm turns an unknown value into a valid one) | crates/relay-core/src/store/receipts.rs:589 |
| FND-008 | low | Plain-text output prints `escalation={:?}` (`Pending`) while JSON prints `pending`; the two formats disagree on the same value | crates/agent-msg/src/main.rs:543 |
| FND-009 | low | Rows whose numeric fields fail to parse are dropped by `filter_map` with no error and no `truncated`, so a corrupt row silently shortens the list | crates/relay-core/src/store/receipts.rs:410 |
| FND-010 | low | Worst-case read walks the whole 10k index with one HMGET per row inside one blocking script on the IO-starved Redis when most rows are acted or aged out; acted rows are never removed from the index | crates/relay-core/src/store/lua/order_expired_list.lua:12 |

### Detail

- **FND-001.** `receipt_emit` sets `HSET okey state <outcome>` for every receipt of an original whose order hash exists (receipt_core.lua:33). `receipt_acted` sets it to `acted`, but `order_close(…, 'acted')` is a no-op on an expired order, so nothing else records Acted on the hash. The next receipt for that original replaces `acted`. Reproduced with deliver(observe only) → expire → escalate → Acted → `ack`: listed count goes 0 → 1. A redelivery after Acted also shows the order again (the second Acted is deduped by event id, so `state` stays `delivered`). Fix: record Acted durably (e.g. `HSET okey acted_at now` in `receipt_acted` when `fresh and m.ord`, independent of `order_close`), filter on it, and `ZREM orders-expired` there too (also fixes FND-010). Add a test with a receipt after the post-expiry Acted.
- **FND-002.** `relay-watcher` runs `scan_orders` on every host each tick (relay-watcher/src/main.rs:1164), so during a rolling upgrade an old watcher can record AckExpired without indexing it, permanently. FR-067's export reports instrumentation start and the retention boundary for exactly this reason. Suggest `HSETNX meta orders_expired_since_ms now` in order_expire.lua and return `indexed_since_ms` (and the rank-trim boundary when hit) in `ExpiredOrders`, so ix-board can show "index since …" rather than "no expired orders".
- **FND-003.** Every user-facing change since 0.3.4 added a bullet under `## Unreleased` (e.g. #56); release #60 renamed it to `## 0.4.0`. This PR adds neither the section nor a bullet. The bullet should say: upgrade `relay-watcher` on every host first; orders that expired before then are not listed.

## Dispositions

Round 1, fix head `f984fb7031cdb3c153d0ff8aa5d27a3f52fc68ee` (one commit after b38a341). Re-checked against the code: `cargo fmt --check` and clippy (`-p relay-core -p agent-msg --all-targets --all-features -D warnings`) clean; `orders` 21/21, `send_order` 5/5, relay-core lib 187/187, `activity` 6/6 against redis:7.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `receipt_acted` sets `acted_at` on the order hash and ZREMs `orders-expired` when a fresh Acted lands on an order; the reader filters on `acted_at`. New test covers ack, requeue plus redelivery, dead letter and lease timeout after Acted. |
| FND-002 | fixed | `orders_expired_since_ms` (HSETNX each expiry scan) is returned as `indexed_since_ms`; a rank trim stamps `orders_expired_trim_ms` and sets `truncated` for 8 days; field docs and CHANGELOG name the rolling-upgrade gap. |
| FND-003 | fixed | `## Unreleased` added with an entry naming the upgrade order (relay-watcher first). The section now conflicts with main; see FND-013. |
| FND-004 | fixed | Renamed `delivered_before_expiry`, with doc and spec text saying a delivery after expiry does not set it. |
| FND-005 | fixed | Recipient is respelled from the host lane lists by `Address` equality, else the stored spelling; doc says compare as an `Address`. |
| FND-006 | fixed | Script body is prefixed `#!lua flags=no-writes`; Redis 7 and 8 (the supported servers) refuse any write. |
| FND-007 | fixed | Only `e<digits>` decodes as `Filed`; anything else is `Unknown`. |
| FND-008 | fixed | Text output uses `OrderEscalationState::as_str`, the same names as JSON. |
| FND-009 | fixed | Undecodable rows (and a partial trailing chunk) are counted in `skipped`, printed in text output and tested. |
| FND-010 | fixed | Acted rows leave the index (FND-001) and one call examines at most 1,000 index rows (`EXPIRED_ORDERS_SCAN_CAP`), setting `truncated` past it. |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-011 | low | "In one request" is no longer true: when any order is listed the reader adds `lane_lists()` (HGETALL of the fleet lanes hash plus a TIME), three round trips, and a lane-list read error fails the whole call; method doc, FR-071 heading and AC-11 still say one request | crates/relay-core/src/store/receipts.rs:394 |
| FND-012 | low | The host lane lists are cited as FR-072; they are FR-069's leased per-host lane lists (FR-072 is the orchestrator assignment list) | crates/relay-core/src/receipts.rs:357 |
| FND-013 | medium | Branch conflicts with main in CHANGELOG.md: #58 (e9e256c) added its own `## Unreleased` section; GitHub reports CONFLICTING, and #59 will add more entries under the same heading | CHANGELOG.md:3 |
