---
id: FR-046
title: "Claude Code's cross-session formats are typed in one crate and drift is detected"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-046: Claude Code's cross-session formats are typed in one crate and drift is detected

## Description

The relay's rule is that every external interface it touches has a typed wire format, in
its own crate or module, with a drift signal, so that a format change on the other side is
*detected* and not silently absorbed. That rule is delivered for Claude Code's
cross-session formats by this requirement; spec section 15.3 lists where every other
interface stands, including those with no drift signal yet. Claude Code's cross-session
formats are undocumented and can change in any release, so they live in one crate,
`claude-wire`, with no I/O.

`claude-wire` owns: the line posted to a session's inbox socket (`Envelope`) and the
`<cross-session-message>` wrapper (`wrap`) with its escaping rules; the control lines
Claude Code writes back to the watcher's reply socket (`ControlLine`, `Receipt`,
`Verdict`, `classify`); the `~/.claude/sessions/<pid>.json` record (`SessionRecord`,
`RecordVerdict`) and the version gate (`KNOWN_PEER_PROTOCOL`, `MIN_VERSION`,
`parse_version`); and the transcript's `origin.msg_id` (`scan`, `TranscriptMatch`).
`relay-watcher` keeps only the reading of files and sockets, the state machine and the
delivery logic ([FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md)).

**Provenance.** The formats were captured on Claude Code `claude_wire::CAPTURED_ON`
(2.1.283). Only some shapes were seen live: the envelope and wrapper, the `held` and
`refused` receipts, the blank liveness connection, the session record's field names and the
transcript `origin.msg_id`. The `delivered`, `denied`, `dropped`, `hold_timeout` and
duplicate receipts were read from the installed program's text and are covered by fixtures
only. `crates/claude-wire/fixtures/claude-2.1.283/MANIFEST` marks every fixture with one
label, pinned per file by a test: `captured` (unedited live bytes: only the blank liveness
connection), `captured-shape` (a shape seen live, or our own emitted bytes as a golden, with
reconstructed or placeholder values), `derived-from-program-text` (never seen live) or
`synthetic` (a drift sample of our own making). A running Claude Code of another version is
*unverified*, not necessarily broken.

**Unverified, to capture once before delivery = "socket" is enabled anywhere.** Whether
Claude Code 2.1.283 writes a queue-operation (or similar) transcript line that carries the
envelope's `msg_id` when a *busy* session receives a peer message has not been checked. If
it does, that line is a transcript line naming our id without a structured `origin.msg_id`,
and it is handled as received but unconfirmed (below), which is the right outcome but is
counted as drift for every busy lead. It must be captured once in a scratch session (never a
real one) and recorded here as a fixture. Likewise `socket-<pid>.json` lists the `msg_id` of
each held batch: a lead that reads that file writes the id into its own transcript, which is
counted as transcript drift (a false positive) and, for a held batch, acknowledges it: the
message is already in Claude Code's hands, which delivers it on approval.

## Behavior

### The crate boundary

- `claude-wire` shall depend on `serde` and `serde_json` only, do no file, socket, process,
  environment or thread I/O, and not depend on `relay-core`, `relay-ops` or `relay-watcher`.
- `relay-watcher` shall read Claude Code's formats only through `claude-wire`'s typed
  results, not through `serde_json::Value` pointers of its own.

### Total parsing and drift types

- `parse_control_line` shall map every input to exactly one `ControlLine`: `Receipt`,
  `KnownIgnored` (a blank line or connection, a status naming no message), `Unrecognised`
  (valid JSON that is not a known shape: not an object, `type` or `action` missing or
  another value, `status` missing, a field of the wrong type), or `NotJson`. It shall
  never panic.
- `scan` shall map any transcript text and message id to `Acked` (a line whose
  `origin.msg_id` is the id), `ShapeUnrecognised` (a line containing the id's text with no
  structured `origin.msg_id`, or an `origin` of another shape) or `Absent`, `Acked` taking
  precedence, and shall never panic.
- `SessionRecord::verdict` shall return `Supported`, `UnsupportedProtocol`,
  `UnsupportedVersion` or `Malformed`, protocol first, then version, then the fields needed
  to deliver.

### Drift signals

- When the watcher reads a reply-socket line (decoded lossily as UTF-8, then classified)
  that is `Unrecognised` or `NotJson`, or a receipt whose verdict is `Unknown`, it shall
  count it in `socket-<pid>.json` (`drift`), log one line per kind for the first of them
  in a run of the watcher, and go on exactly as it would without it. An unknown status
  keeps its ambiguous handling.
- When a transcript look finds `ShapeUnrecognised`, the watcher shall acknowledge the batch
  as received but unconfirmed, like `delivered_unconfirmed`, with a drift note, count it,
  and log one line for the first: our fresh `msg_id` in the receiver's transcript is
  evidence Claude Code received the message.
- If a transcript look finds `ShapeUnrecognised`, then the watcher shall not mark the lead
  suspect, so there is no herdr fallback while the shape is unrecognised, and shall not
  redeliver the batch after its lease or dead-letter it.
- A transcript with no line naming the id is `Absent` (not yet consumed) and shall not be
  counted.
- Counters shall saturate and shall never fail a delivery.
- `agent-msg doctor` shall warn `socket_wire_drift`, with the counts, when the reply-socket
  counters are not zero, and `socket_transcript_shape_unrecognised`, with the count, when
  that counter is not zero; neither is ever a failure.
- `agent-msg doctor` shall show the version the wire format was captured on and, when a
  resolved Claude Code reports another version, say `wire format captured on 2.1.283;
  running X: unverified`, without failing. The version gate (`MIN_VERSION`,
  `peerProtocol`) is unchanged.

### Behaviour changes from moving the code

Recorded because they are delivery-visible, though neither is reachable from Claude Code's
`JSON.stringify`:

- A `peer_message_status` naming a message with `status` absent or null was `Receipt(Unknown)`
  (ambiguous, the lead not suspect). It is now `Unrecognised(StatusMissing)`: counted and
  otherwise ignored, so after the ack wait the lead is marked suspect and its mail goes
  through herdr.
- Duplicate JSON keys: a receipt line is now read with the last key winning (it used to be
  refused), and a transcript line with two `origin` keys is `ShapeUnrecognised` (it used
  to be acknowledged, last key winning).

### Fixtures and outgoing bytes

- Every fixture under `crates/claude-wire/fixtures/claude-2.1.283/` shall be parsed through
  the crate and the exact typed result asserted; the manifest, the directory and the
  expectations shall list the same files.
- Every outgoing byte (the envelope line, the wrapper text, for hostile names and bodies)
  shall be asserted against golden bytes, and no `</cross-session-message>` in any case or
  spacing, and no `<`, shall survive in a wrapped body.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-046-AC-1 | Given the crate's manifest, its dependencies are exactly `serde` and `serde_json`, and no relay crate is a dependency. | Test |
| FR-046-AC-2 | Given arbitrary strings, arbitrary bytes read lossily, and JSON-shaped input with random fields, `parse_control_line`, `scan`, `SessionRecord::parse` and `parse_version` never panic; `classify` maps every triple to a verdict and only the six known status words to anything but `Unknown`; a well-formed receipt line parses to a `Receipt` whose verdict is `classify` of its fields. | Property |
| FR-046-AC-3 | Given a reply-socket line that is JSON in a renamed shape, one that is not JSON, and one whose bytes are not UTF-8 (decoded lossily, then classified), the status file counts each in `drift` (`unrecognised_lines`, `not_json_lines`) and delivery is the same as without it; a liveness connection and a receipt for another message are not counted. | Test |
| FR-046-AC-4 | Given a receipt with an unknown status, the delivery is ambiguous as before and `drift.unknown_status_receipts` is 1; a known status is not counted. | Test |
| FR-046-AC-5 | Given a transcript line that names the message id under a renamed field, during the ack wait, the delivery is acknowledged, `drift.transcript_shape_unrecognised` is 1, and the next batch for the lead is still posted to the socket (not sent to herdr), whereas a receiver that is only silent makes the next batch go to herdr. | Test |
| FR-046-AC-6 | Given a transcript with no line naming the id (another message's structured `origin.msg_id` included), `scan` is `Absent`, the delivery is ambiguous as before and nothing is counted as drift; a line naming the id with no structured origin (renamed field, an `origin` string, non-JSON, another field) is `ShapeUnrecognised`, and an acknowledgement anywhere in the text wins. | Test |
| FR-046-AC-7 | Given session records with an unknown `peerProtocol`, an absent, unparseable or older version, an absent `messagingSocketPath` and an absent `sessionId`, the verdicts are `UnsupportedProtocol`, `UnsupportedVersion`, `Malformed` and no record, each with its fields, and `2.1.224`, `2.2.0` and `3.0.0-beta.1` are `Supported`. | Test |
| FR-046-AC-8 | Given every fixture of the captured version, each parses to its exact typed result, and the manifest lists exactly the files on disk, each as `captured`, `derived-from-program-text` or `synthetic`, with every `drift-` fixture synthetic. | Test |
| FR-046-AC-9 | Given the envelope line and the wrapper for plain and hostile names and bodies, the bytes are the golden ones; for any text, no `<` survives in the wrapped body and the wrapper's own two tags are the only ones. | Property |
| FR-046-AC-10 | Given a status whose resolved Claude Code is 2.1.283, `agent-msg doctor` shows the captured version and no `unverified` line; given another version, it says `wire format captured on 2.1.283; running X: unverified`, and the check is not a failure. | Test |
| FR-046-AC-11 | Given a status with drift counts, `agent-msg doctor` warns `socket_wire_drift` with the counts and `socket_transcript_shape_unrecognised` with the count, never a failure; given none, neither is present. | Test |
| FR-046-AC-12 | Given a transcript line naming the id under a renamed field that arrives after the ack wait, a `settle` acknowledges the batch once, with a `transcript_shape_unrecognised` reason and note, and counts it once. | Test |
| FR-046-AC-13 | Given a batch acknowledged as received but unconfirmed for an unrecognised shape, when its lease expires and the watcher ticks over many rounds, nothing is posted again, nothing is typed through herdr, nothing is redelivered and nothing is dead-lettered. | Test |
| FR-046-AC-14 | Given the crate's library sources, no identifier of a file, socket, process, environment or thread API appears in the code in any import form (a grouped `std::{..}` import included). | Test |
| FR-046-AC-15 | Given a tail read whose window starts at every byte offset around the origin line, a window that starts on the line or before it acknowledges it, one that starts inside it drops the partial line and reports nothing, and none reports a shape change; given a line longer than a window, the rest of it is not scanned as a line and the next whole line still acknowledges. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Related: [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md) (the
  transport these formats serve), [FR-029](./FR-029-doctor-reports-relay-health.md) (the
  report), [FR-008](./FR-008-tolerant-registry-decoding.md) (the tolerance rule for
  another program's records), [NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md)
  (the old locations were deleted, not aliased).
- Delivered by AGE-2061 (with the review fixes SR-025..SR-029). The heartbeat (FR-028) is not extended: the counts are in the
  watcher's socket status file, which `doctor` already reads.
