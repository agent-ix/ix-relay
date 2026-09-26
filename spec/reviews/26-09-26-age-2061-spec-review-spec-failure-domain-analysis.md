---
id: SR-029
title: "Spec review (failure domain) of ix-relay PR #33, AGE-2061: FR-046 drift semantics on the delivery path"
type: SpecReview
analysis: failure-domain
scope: "agent-ix/ix-relay@5fe6eaa46c4b9dcd7f52f1c7506060266539c250; spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md (Drift signals, AC-3..AC-6), spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (ack-wait bullet), spec/functional/FR-040-redelivery-is-marked-bounded-and-dead-lettered.md (bound), README.md (Wire formats); checked against crates/relay-watcher/src/socket/deliverer.rs (await_outcome, settle, receive, publish), crates/relay-watcher/src/socket/transcript.rs, crates/relay-watcher/src/watch.rs (Ambiguous handling), crates/relay-core/src/store/inflight.rs (WATCHER_LEASE, MAX_REDELIVERIES), crates/relay-core/src/socket_status.rs (HeldBatch, WireDrift)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-046"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
---
# SR-029: Spec review (failure domain) of ix-relay PR #33, AGE-2061

## Summary

Ticket: AGE-2061. This looks for failure modes FR-046 does not state, in the three drift
signals it adds to the delivery path.

- **Reply-socket drift.** `Unrecognised`, `NotJson` and `Unknown` are counted, bounded
  and saturating, and the watcher otherwise goes on as it would without the line. This is
  safe. The one delivery change, a status-less receipt now going to herdr after the ack
  wait, is recorded in SR-025 FND-004.
- **Version gate.** Unchanged, and verified by a differential run in SR-025.
- **`ShapeUnrecognised`.** Walked through the state machine, it is where the unstated
  failure modes are (FND-001, FND-002).

Default-off still holds: with no `delivery` key the watcher creates and binds nothing,
measured in SR-025. So the exposure is limited to owners who opt in. The drift counters
are persisted in the existing atomic, 0600 status file. Doctor only warns and never fails.
The heartbeat is deliberately not extended, which FR-046 records, and that is acceptable
because doctor reads the status file.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | FR-046 (drift bullet 2, AC-5) and the FR-044 exception specify the handling for a `ShapeUnrecognised` transcript: leave the batch in flight, redeliver it after its lease, and never mark the lead suspect. They do not state the consequence. When the drift is real (the field was renamed and Claude Code did consume the message), the redelivery goes through the socket again. `MAX_REDELIVERIES` (FR-040) bounds this at 4 posts, so each batch reaches the lead up to 4 times. Each post raises a "may have failed" desktop notification. The batch is then dead-lettered as undeliverable, although it was delivered. This repeats for every batch until the fix. The herdr fallback this rule avoids cost about one duplicate. The rule was meant to avoid duplicates and multiplies them instead. Specify the outcome, and change it. Our fresh id appearing in the receiver's transcript is itself evidence Claude Code recorded the message, so either acknowledge it with a drift note (as `delivered_unconfirmed` does), or renew the lease like a held batch and settle it without redelivery. Add an AC and test for the post-lease cycle. | spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:67-71, spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:98, spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md:163-166, crates/relay-watcher/src/socket/deliverer.rs:1001-1014 |
| FND-002 | low | The false-positive surface of `ShapeUnrecognised` is small but not empty, and FR-046 does not name it. (a) `socket-<pid>.json` lists each held batch's `msg_id`. A lead that reads the status file (`cat`, `jq`, a doctor-like script) writes that id into its own transcript as a tool result. The next look then counts transcript drift and doctor warns `socket_transcript_shape_unrecognised`, though no format changed. The held path itself is unaffected, since its lease is renewed anyway. (b) It is unverified whether Claude Code 2.1.283 writes a queue-operation or enqueue line carrying the envelope when a busy session receives a peer message. The crate's own tests use a `queue-operation` line as a ShapeUnrecognised example. If it does, every delivery to a busy lead counts as drift and loses the silent-receiver herdr fallback. That needs one capture in a scratch session (never a real one) before the feature is turned on. A real delivery failure (the message never consumed) cannot produce ShapeUnrecognised unless Claude Code wrote the id somewhere. | crates/relay-core/src/socket_status.rs:140, crates/claude-wire/src/transcript.rs:100, crates/relay-watcher/src/socket/deliverer.rs:1001-1014 |

## Verdict

One medium finding on the delivery path. The specified reaction to transcript-shape drift
multiplies deliveries and makes false dead letters under exactly the drift it was designed
for (FND-001, consistent with SR-025 FND-001). The feature is off by default, but the
behaviour is specified and shipped, so change it before this merges or before
`delivery = "socket"` is enabled anywhere. FND-002 is low.
