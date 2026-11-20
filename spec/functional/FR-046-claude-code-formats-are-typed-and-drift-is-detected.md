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
`synthetic` (a drift sample of our own making). Whether a running Claude Code of another
version works is told by what the watcher observes of it (Drift signals, below), not by
comparing its version to the captured one.

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
  count it in `socket-<pid>.json` (`drift`) against the Claude Code version of the lead
  it came from, log one line per kind and version for the first of them in a run of the
  watcher, and go on exactly as it would without it. An unknown status keeps its
  ambiguous handling.
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

### Drift is attributed to a version and ages out

- The watcher shall keep, for each Claude Code version it has delivered to or read a
  line from, counts of the four drift kinds (`unrecognised_lines`, `not_json_lines`,
  `unknown_status_receipts`, `transcript_shape_unrecognised`) and a count of socket
  deliveries acknowledged by the transcript or by a `delivered` receipt, in hourly
  buckets, and shall drop a bucket once it is more than 24 hours old, so that a count
  describes the last 24 hours and not the life of the watcher. 24 hours is the window
  `agent-msg doctor` already uses for hook failures and for the age of a registration,
  long enough to span a working day of a lead that runs rarely, and short enough that a
  Claude Code upgrade that fixed the format stops showing as drift the next day.
- For each version and drift kind the watcher shall keep the last offending line's shape and
  time: for a reply-socket line, its `type` and `action` strings (absent when the line
  has none, and for a line that is not JSON, neither), each cut to 64 characters and
  neutralised; for a transcript look, the `type` of the line when it has one. It shall
  never keep a message body, a message id, a path, or any other text of the line.
  These last-line members age out with the counts.
- A lead's version shall be the version its session record reported when the watcher
  resolved it; a line from a lead with no resolved version is counted against the version
  `unknown`. A reply-socket line names no lead, so it is counted against the version of the
  lead the watcher most recently posted to: an approximation, wrong for a late line when
  two Claude Code versions run on one machine. `agent-msg doctor` shall say so in the
  detail lines of `socket_wire_drift`.
- The watcher shall keep these members for at most 16 versions
  ([NFR-002](../non-functional/NFR-002-bounded-resources.md)), each version string cut to
  64 characters and neutralised; when a further version appears it shall first drop the
  version whose newest bucket is oldest, and it shall never fail or delay a delivery to
  keep the bound.
- The counts and last-line members shall be kept apart from the resolved-lead entries
  ([FR-055](./FR-055-socket-status-keeps-only-live-leads.md)), so that a lead that left the
  file leaves its version's counts for the window.
- The status file's old cumulative `drift` object, which names no version, is replaced
  by these per-version members; a status file in the old shape reads as having none
  ([NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md)).

### What `doctor` says about each version

- For each Claude Code version in the status file, `agent-msg doctor` shall say one of
  three things, and none of them is a failure:
  - *working*, when the version has at least one acknowledged socket delivery in the window
    and no drift of any kind in the window;
  - *drift*, when the version has any drift in the window, with the counts per kind in the
    window, the last offending line's `type` and `action` (or the transcript line's `type`)
    and when it was seen. This is the warning `socket_wire_drift` for the reply-socket kinds
    and `socket_transcript_shape_unrecognised` for the transcript kind, one per version, each
    naming the version;
  - listed, with no verdict, when the version has had no acknowledged delivery and no drift in
    the window.
- `agent-msg doctor` shall not say that a version is *unverified* because it differs from
  `claude_wire::CAPTURED_ON`. `CAPTURED_ON` remains the version the fixtures and the
  provenance above describe, and `doctor` may print it as such. The version gate
  (`MIN_VERSION`, `peerProtocol`) is unchanged.

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
| FR-046-AC-3 | Given a reply-socket line that is JSON in a renamed shape, one that is not JSON, and one whose bytes are not UTF-8 (decoded lossily, then classified), the status file counts each against the sending lead's version (`unrecognised_lines`, `not_json_lines`) and delivery is the same as without it; a liveness connection and a receipt for another message are not counted. | Test |
| FR-046-AC-4 | Given a receipt with an unknown status, the delivery is ambiguous as before and that version's `unknown_status_receipts` is 1; a known status is not counted. | Test |
| FR-046-AC-5 | Given a transcript line that names the message id under a renamed field, during the ack wait, the delivery is acknowledged, that version's `transcript_shape_unrecognised` is 1, and the next batch for the lead is still posted to the socket (not sent to herdr), whereas a receiver that is only silent makes the next batch go to herdr. | Test |
| FR-046-AC-6 | Given a transcript with no line naming the id (another message's structured `origin.msg_id` included), `scan` is `Absent`, the delivery is ambiguous as before and nothing is counted as drift; a line naming the id with no structured origin (renamed field, an `origin` string, non-JSON, another field) is `ShapeUnrecognised`, and an acknowledgement anywhere in the text wins. | Test |
| FR-046-AC-7 | Given session records with an unknown `peerProtocol`, an absent, unparseable or older version, an absent `messagingSocketPath` and an absent `sessionId`, the verdicts are `UnsupportedProtocol`, `UnsupportedVersion`, `Malformed` and no record, each with its fields, and `2.1.224`, `2.2.0` and `3.0.0-beta.1` are `Supported`. | Test |
| FR-046-AC-8 | Given every fixture of the captured version, each parses to its exact typed result, and the manifest lists exactly the files on disk, each as `captured`, `derived-from-program-text` or `synthetic`, with every `drift-` fixture synthetic. | Test |
| FR-046-AC-9 | Given the envelope line and the wrapper for plain and hostile names and bodies, the bytes are the golden ones; for any text, no `<` survives in the wrapped body and the wrapper's own two tags are the only ones. | Property |
| FR-046-AC-10 | Given versions 2.1.302 with acknowledged deliveries and no drift, 2.1.303 with acknowledged deliveries and one `not_json` line, and 2.1.290 with neither, `agent-msg doctor` says 2.1.302 is working, warns for 2.1.303 with its counts, the last line's `type` and `action` and when, and lists 2.1.290 with no verdict; no output contains `unverified`, and none is a failure, whatever the captured version. | Test |
| FR-046-AC-11 | Given per-version drift counts, `agent-msg doctor` warns `socket_wire_drift` with the version, the counts and the last offending line's `type` and `action` and time, and `socket_transcript_shape_unrecognised` likewise, never a failure, with a detail line saying that reply-socket lines are attributed to the version of the lead most recently posted to; given none, neither is present. | Test |
| FR-046-AC-12 | Given a transcript line naming the id under a renamed field that arrives after the ack wait, a `settle` acknowledges the batch once, with a `transcript_shape_unrecognised` reason and note, and counts it once. | Test |
| FR-046-AC-13 | Given a batch acknowledged as received but unconfirmed for an unrecognised shape, when its lease expires and the watcher ticks over many rounds, nothing is posted again, nothing is typed through herdr, nothing is redelivered and nothing is dead-lettered. | Test |
| FR-046-AC-14 | Given the crate's library sources, no identifier of a file, socket, process, environment or thread API appears in the code in any import form (a grouped `std::{..}` import included). | Test |
| FR-046-AC-15 | Given a tail read whose window starts at every byte offset around the origin line, a window that starts on the line or before it acknowledges it, one that starts inside it drops the partial line and reports nothing, and none reports a shape change; given a line longer than a window, the rest of it is not scanned as a line and the next whole line still acknowledges. | Test |
| FR-046-AC-16 | Given drift recorded 25 hours ago and none since, the counts, the last-line members and the warning are gone and a version with acknowledged deliveries reads as working; given drift 23 hours ago, they are still there. | Test |
| FR-046-AC-17 | Given an offending reply-socket line whose `type`, `action` and body hold control characters, a message body, a path and 10,000 characters, the status file and `doctor` hold no body or path, at most 64 characters each of `type` and `action`, and no control character. | Test |
| FR-046-AC-18 | Given a lead that leaves the status file's resolved leads (FR-055), its version's counts remain for the rest of the window. | Test |
| FR-046-AC-19 | Given drift from 20 distinct versions, one of them with a 1,000-character version string, the status file holds 16 versions, the ones with the newest buckets, no version string over 64 characters and no control character, and every delivery proceeds as without drift. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Related: [FR-055](./FR-055-socket-status-keeps-only-live-leads.md) (the resolved-lead entries the counts are kept apart from), [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md) (the
  transport these formats serve), [FR-029](./FR-029-doctor-reports-relay-health.md) (the
  report), [FR-008](./FR-008-tolerant-registry-decoding.md) (the tolerance rule for
  another program's records), [NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md)
  (the old locations were deleted, not aliased).
- Delivered by AGE-2061 (with the review fixes SR-025..SR-029). The heartbeat (FR-028) is not extended: the counts are in the
  watcher's socket status file, which `doctor` already reads.
