---
id: SR-025
title: "Code review (with Rust lane) of ix-relay PR #33, AGE-2061: claude-wire crate and drift detection"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@5fe6eaa46c4b9dcd7f52f1c7506060266539c250; crates/claude-wire/Cargo.toml, crates/claude-wire/src/lib.rs, crates/claude-wire/src/envelope.rs, crates/claude-wire/src/receipt.rs, crates/claude-wire/src/record.rs, crates/claude-wire/src/transcript.rs, crates/claude-wire/tests/contract.rs, crates/claude-wire/fixtures/claude-2.1.283/* (24 files and MANIFEST), crates/relay-watcher/Cargo.toml, crates/relay-watcher/src/socket.rs, crates/relay-watcher/src/socket/deliverer.rs, crates/relay-watcher/src/socket/session.rs, crates/relay-watcher/src/socket/transcript.rs, crates/relay-watcher/src/socket/tests.rs, crates/relay-core/src/socket_status.rs, crates/agent-msg/Cargo.toml, crates/agent-msg/src/doctor/checks.rs, crates/agent-msg/src/doctor/facts.rs, crates/agent-msg/src/doctor/report.rs, Cargo.toml, Cargo.lock, CLAUDE.md; deleted: crates/relay-watcher/src/socket/wire.rs (compared against origin/main 5e19aa4); read for context: crates/relay-watcher/src/deliver.rs, crates/relay-watcher/src/watch.rs, crates/relay-watcher/src/main.rs (delivery), crates/relay-core/src/store/inflight.rs (WATCHER_LEASE, MAX_REDELIVERIES)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-046"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
---
# SR-025: Code review (with Rust lane) of ix-relay PR #33, AGE-2061

## Summary

Ticket: AGE-2061. PR: ix-relay#33, head 5fe6eaa, base origin/main 5e19aa4. The PR moves
Claude Code's cross-session formats out of `relay-watcher::socket::wire` and `session.rs`
into a new pure crate, `claude-wire`: the envelope, `wrap`, the receipts
(`parse_control_line`), the session record with its version gate, and the transcript scan.
It adds drift detection: counters in `socket-<pid>.json`, doctor codes `socket_wire_drift`
and `socket_transcript_shape_unrecognised`, and `TranscriptMatch::ShapeUnrecognised`, which
leaves a batch ambiguous with no herdr fallback. It also adds contract fixtures under
`fixtures/claude-2.1.283`. Socket delivery stays off by default. The Rust lane
(rust-review) is folded into this file.

Method: the worktree was read only. Everything was built and run in a scratch
`git archive` copy with its own `CARGO_TARGET_DIR`. A throwaway `redis:7` was used and then
stopped. No real Claude session, socket, registry record or transcript was touched.

**Behaviour preservation, measured with a differential harness.** Main's `wire.rs`,
`has_origin` and the session gate were built next to the new crate and run on the same
generated inputs:

- **Envelope line and `wrap`: byte-identical** over 30,000 hostile inputs. The inputs
  included `<`, `>`, `&`, `"`, backslash, CR, LF, TAB, NUL, ESC, U+2028/2029, ZWJ, ZWSP,
  BOM, U+2060, U+10FFFF, emoji, and the tag in upper case, spaced, tab-spliced and
  entity-encoded. The 1 MiB boundary was also checked. The code is textually the same as
  main.
- **Receipts (40,000 lines):** the verdicts, `orig_msg_id` and `dropped_msg_ids` are
  identical wherever both sides produce a receipt. There are exactly two classes of
  difference:
  1. A `peer_message_status` naming a message but with `status` absent or null. Main gave
     `Receipt(Unknown)`: ambiguous, no suspect, a note. It is now
     `Unrecognised(StatusMissing)`: counted and otherwise ignored. After the ack wait the
     lead is marked suspect and its mail goes to herdr. This is intended (FR-046 AC-3,
     "as without it").
  2. Duplicate JSON keys. Main rejected them (serde derive). They now go through `Value`,
     where the last key wins, and may become a receipt. JavaScript's `JSON.stringify`
     never emits duplicate keys, so this does not happen in practice.
- **Session gate (25,000 records):** decode success and the
  Ok/Unsupported/NoSocket verdict are identical.
- **Transcript scan (25,000 texts):** `Acked` is identical to main's `has_origin`, except
  when a line holds two `origin` keys. Main acked it (last key wins); it is now
  `ShapeUnrecognised`. This is unreachable from `JSON.stringify`.
- **Crate purity:** `cargo tree` shows serde and serde_json only, with no relay crate.
  There is no I/O in the library sources, and the workspace denies `missing_docs`.
- **Totality:** 25,000 random-byte cases each through `parse_control_line`, `scan`,
  `SessionRecord::parse`, `parse_version` and `wrap`, with no panic. JSON nested 127, 128,
  129, 10,000 and 200,000 deep hits serde_json's recursion limit, with no stack overflow.
  A 64 KiB receipt of 6,000 ids keeps 256. A 1 MiB non-JSON line is `NotJson`. The
  reader caps a receipt at 64 KiB and a transcript window at 512 KiB, so hostile input
  cannot force an unbounded allocation.
- **Default off:** with no `delivery` key, the built watcher ran (`--once`, fake HOME,
  XDG_RUNTIME_DIR, `IX_RELAY_CONFIG`, a failing fake `herdr` on PATH) and created nothing:
  no `state/socket`, no `cc-socks`, nothing under `.claude`. The same run with
  `delivery = "socket"` created `state/socket` and tried to bind. `main.rs` `delivery()` is
  unchanged.
- **Mutations**, each on a scratch copy, each caught by a test:
  - `ShapeUnrecognised` treated as absent in the wait: caught by `tc_046_05_...never_falls_back`.
  - `settle` acking `ShapeUnrecognised`: caught by `tc_046_05_settle_never_acks...`.
  - The reply-socket counters not incremented: caught by both `tc_046_03`.
  - The shape counter not incremented: caught by both `tc_046_05`.
  - The unknown-status counter not incremented: caught by `tc_046_04`.
  - Drift not published: 5 tests fail.
  - A substring match on status, and on detail: caught by `every_verdict_is_reached...`.
  - The version gate dropped, and the protocol gate dropped: caught by
    `the_verdict_is_typed...`.
  - The `<` escape dropped: 4 tests fail.
  - `scan` reading `ShapeUnrecognised` as `Absent`: 1 test fails.
  - Deleting a fixture fails 2 contract tests, and an extra file fails the manifest test.
- **Flakiness:** the `tc_046` tests ran 20 times against the pristine binary with 0
  failures.

**The ShapeUnrecognised state machine**, walked through the code: `await_outcome` returns
`Ambiguous` with no `suspect` entry (deliverer.rs:1001-1014). `watch.rs` raises a desktop
notification ("prompt to X may have failed") and leaves the batch in flight. `held_since` is
unset, so nothing renews the lease. After `WATCHER_LEASE` (60 s) the store redelivers the
batch, marked. The lead is not suspect, so the batch goes through the socket again, with a
new msg_id. `MAX_REDELIVERIES` = 3 bounds the loop: at most 4 posts, then the batch is
dead-lettered (visible in `agent-msg inbox --dead` and the OTLP `dead_lettered` count). The
pending entry itself is never acked, because the window has moved past the line. There is
no endless loop. See FND-001 for why this bound is still a regression.

The false-positive surface for ShapeUnrecognised is small. `new_msg_id` is a fresh v4 UUID
from `/dev/urandom` for each attempt. It appears only in the posted line and in
`socket-<pid>.json` `held_batches[].msg_id` (see SR-029).

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | Under the drift this PR exists to catch, `ShapeUnrecognised` makes delivery worse than main. Take a renamed `origin` field whose line still carries our id: Claude Code consumed the message. Every such batch is ambiguous with no suspect. Its lease is not renewed. It is redelivered through the socket every 60 s until `MAX_REDELIVERIES`: 4 copies in the lead's session (3 marked `(redelivered)`), 4 desktop notifications, then a dead letter for a message that was delivered. This repeats for every batch while the drift lasts. On main the same transcript gave: no ack, then suspect, then one herdr redelivery (1 duplicate), then herdr for 10 min. Fix, needing an FR-046/FR-044 change: treat "our id is in the transcript" as evidence of receipt. Either ack with a drift note (like `delivered_unconfirmed`), or renew the lease like a held batch and settle without redelivery. At minimum, mark the lead suspect after the first shape event so later batches do not repeat the loop. | crates/relay-watcher/src/socket/deliverer.rs:1001-1014, crates/relay-watcher/src/socket/deliverer.rs:1330-1331, crates/relay-watcher/src/watch.rs:382-389 |
| FND-002 | low | Off-by-one in the new partial-line drop on a tail read. When `seen_to` is `None` or the file shrank, `at_line_start` is `start == 0`. So a window whose start falls exactly on a line start still drops its first, whole line, and an origin line there is lost. Reproduced on a scratch copy: from_tail at every offset around the origin line lost it only at offset 0 (`Absent`). Main acks that case. The window advances past the line, so it is never re-read; the adopted batch goes unacked and is redelivered. The reverse case also exists: after a line longer than a window is skipped, `seen_to` sits mid-line with `at_line_start = true`, so the remainder is scanned as a line and a false `ShapeUnrecognised` is possible if it holds the id. Fix: read one byte before `start` and treat the window as at a line start when that byte is `\n`, and track mid-line state after a skip. Only the restart and replaced-file paths are affected, and it needs byte-exact alignment. | crates/relay-watcher/src/socket/transcript.rs:112-138 |
| FND-003 | low | Three new log strings contain runs of 18 to 26 spaces. Lost `\` continuations joined the lines into the literal, e.g. "status this build                  does not know". The once-per-kind log lines print with that gap. | crates/relay-watcher/src/socket/deliverer.rs:445, crates/relay-watcher/src/socket/deliverer.rs:458, crates/relay-watcher/src/socket/deliverer.rs:1841 |
| FND-004 | low | Two behaviour changes from the move are not recorded in the PR or the spec. (a) A receipt naming a message with no or null `status` used to be `Receipt(Unknown)`: ambiguous, the lead not suspect. It is now `Unrecognised`: after the ack wait the lead becomes suspect and its mail goes to herdr. This is intended by FR-046-AC-3 but is a delivery change. (b) Duplicate keys: receipts now take the last key (main rejected the line), and transcript lines are now refused and become `ShapeUnrecognised` (main took the last key and acked). Neither is reachable from Claude Code's `JSON.stringify`. Record (a) in FR-044 or the PR; (b) needs no change. | crates/claude-wire/src/receipt.rs:181-200, crates/claude-wire/src/transcript.rs:53 |

## Rust lane (rust-review)

- **Types.** The public API is small and typed (`ControlLine`, `DriftReason`, `Ignored`,
  `RecordVerdict`, `TranscriptMatch`). `parse_control_line` checks each field's type
  before a typed decode, so `DriftReason` names the first expectation that failed.
  `RawReceipt` no longer carries `type`/`action`, which are checked on the `Value`.
- **Panics and unwrap.** There is no `unwrap` or `expect` outside `#[cfg(test)]`/`tests/`.
  `bump` uses `fetch_update` with saturation. `WireDrift` counters saturate, and
  `control_total` uses saturating adds.
- **Integer conversions.** None new at the wire boundary. The counters are `u64` in serde.
- **Resource bounds.** `MAX_RECEIPT_BYTES` (64 KiB) is still applied through `take`. It now
  uses `read_until` on bytes instead of `read_line`, so non-UTF-8 input is decoded lossily
  and counted rather than skipped. Timeout and EOF handling are unchanged (`read_until`
  errors on timeout the way `read_line` did), and the receive loop spawns nothing new, so
  no thread leaks. `MAX_DROPPED_IDS` is kept. The `Value` intermediate in
  `parse_control_line` is bounded by the 64 KiB read cap.
- **Status file.** `WireDrift` is `#[serde(default)]` on each member and on the field, so
  an older status still reads. `SocketStatus::write` is unchanged: atomic, and 0600 as
  before, since `socket_status.rs` changed only in the struct and tests.
- **Allows.** One new `#[allow(clippy::large_enum_variant, reason = ...)]` on doctor's
  `SocketState`. It is the only non-test `allow` in the workspace. The reason is accurate:
  11 construction sites, and a handful of values per run. `Box<SocketStatus>` would remove
  it at the cost of those 11 edits. It is acceptable as written, so it is not a finding.
  There are no other new allows.
- **Hygiene.** No `.github` change, and no `proptest-regressions` files. The new socket
  tests need no Redis and pass without `IX_RELAY_TEST_URL`. There is no sleep-then-assert
  in the new tests: they assert after `send` returns, which waits the full ack wait.

## Verdict

The move is behaviour-preserving for the envelope, the wrapper, the session gate and the
transcript ack. Every difference in receipts is an intended drift addition, or duplicate
keys that cannot occur. The crate is pure and total, and the mutation checks all bite.
Merge is blocked by FND-001 (medium, delivery path). Under real transcript-shape drift, the
new "no herdr fallback" rule turns each batch into 4 socket deliveries, 4 notifications and
a false dead letter, where main gave about one duplicate. FND-002 to FND-004 are low.
