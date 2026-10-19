---
type: master-requirements
name: ix-relay
org: agent-ix
component_type: rust-cli
tags:
  - redis
  - herdr
  - message-bus
implementation_language: rust
depends_on: []
relationships:
  - target: "ix://agent-ix/ix-cli-kit"
    type: "depends_on"
    cardinality: "1:1"
standards_alignment:
  - iso-iec-ieee-29148
---
# Master Requirements Specification

## ix-relay

---

## 1. Purpose

This document defines the scope and the governing requirements of ix-relay: the
lead-to-lead message bus and escalation inbox used by the team leads of the
agent-ix swarm. It is the top-level requirements artifact for this repository and
the index for the stakeholder requirements, user stories, functional and
non-functional requirements beneath it.

The requirements state the behaviour ix-relay must exhibit. Where the code on
`main` does not yet behave that way, the requirement still states the intended
behaviour and names the ticket that closes the gap (section 12). Evidence —
tests, property tests, fuzz targets, proofs — is keyed to the obligation ids
defined here.

---

## 2. Scope

Version 0.3.0 supersedes all earlier alias, slug, rename-precedence, bare-name,
raw-pane and optional-level routing clauses. FR-001..006, FR-021 and FR-022
define the current identity contract. Existing `AGENT_MSG_NAME` is preserved
as environment metadata and ignored for identity. Old namespaces are archived,
never imported as aliases into `ixr:v3:`.


### 2.1 In Scope

This specification governs:

- Addressing: `server:workspace:tab`, `:workspace:tab`, `:tab` and explicit `all`; live session verification and automatic enrollment.
- The registry: what a registration records and how it is kept current.
- Delivery: queueing, broadcast, inbox pops, requeueing, and the watcher's
  typing of messages into an idle lead's pane.
- Text safety of message and escalation text at send, at store and at render.
- The harness hook handler (`agent-msg hook claude|codex`) and its failure and
  time-budget behaviour.
- Session queue identities, canonical public locations and rejection of ambiguous tabs.
- Escalations: identifiers, status transitions and text safety.
- Configuration: precedence and malformed-file behaviour.
- Failure visibility and self-healing (AGE-2033): watcher liveness, `agent-msg
  doctor`, hook-failure spooling, dead letters, undeliverable-recipient state,
  stable-id identity across moves and renames, herdr version checks and
  escalation retention.

### 2.2 Out of Scope

This specification does not govern:

- `relay-tui`'s screen layout and key bindings (only its shared text-safety rule
  is in scope, through [FR-016](./functional/FR-016-render-neutralises-control-characters.md)).
- The opencode plugin's TypeScript implementation (it drives the same CLI and is
  bound by the CLI's requirements).
- Posting to Linear (`resolve`, `escalation post-linear`) beyond "a failed post
  never blocks resolution".
- Investigation session launch (`investigate`) mechanics in herdr and Claude Code.
- Redis deployment, the SSH tunnels between machines, and service-manager units
  under `deploy/`.
- herdr's own behaviour; ix-relay consumes herdr's CLI as an external system.
- Authentication and authorisation on the bus: every participant that can reach
  the Redis instance is trusted to be a lead (see section 3.3).
- Faster delivery paths (AGE-2028) and alternative transports (AGE-2030).
- What a recovered failure shows in an agent's context (AGE-2033 needs the
  operator's sign-off before anything changes what agents see).

---

## 3. System Overview

### 3.1 System Description

ix-relay lets the team lead agents running on several machines send each other
short messages without a human relaying them, and lets a lead escalate a decision
to the human operator. It consists of:

| Part | Job |
|---|---|
| `relay-core` | Redis-backed store; message, registration, address and escalation types; target resolution |
| `relay-ops` | shared operations (escalation flows, herdr location, derived names, `find`) |
| `agent-msg` | CLI for leads and the hook handler the harnesses run |
| `relay-watcher` | one process per machine; types queued messages into idle leads via herdr |
| `herdr-cli` | bounded-time process runner and JSON-envelope decoding for herdr calls |
| `plugins/*` | harness plugins wiring the hook handler into Claude Code, Codex and opencode |

### 3.2 Intended Users

- Team lead agents (Claude Code, Codex, opencode sessions) running inside herdr panes.
- The human operator, who receives escalations and resolves them.
- Maintainers of the swarm tooling who configure machines.

### 3.3 Trust Model

Every process that can reach the bus's Redis instance is a participant. Message
text, escalation text, stored registrations and herdr output are nonetheless
treated as **untrusted input** to rendering and to terminal typing: a participant
may be buggy, out of date, or relaying text it received from elsewhere. The
requirements on text safety and tolerant decoding follow from that stance.

---

## 4. Requirements Architecture

```
spec/
├── spec.md              # this document
├── stakeholder/         # StR-XXX
├── usecase/             # US-XXX
├── functional/          # FR-XXX
├── non-functional/      # NFR-XXX
├── tests.md             # requirements test matrix (TM-001)
├── reviews/             # SpecReview documents (base, EARS, integrity, evidence)
└── analysis/            # recorded `quoin advise` output
```

---

## 5. Requirement Classes

### 5.1 Stakeholder Requirements

`StR-XXX` in `stakeholder/`. Normative for intent; they drive the user stories.

### 5.2 User Stories

`US-XXX` in `usecase/`. Informative; they drive the functional requirements.

### 5.3 Functional Requirements

`FR-XXX` in `functional/`. Normative and binding. Each defines observable
behaviour, is independently verifiable, and traces to a user story.

### 5.4 Non-Functional Requirements

`NFR-XXX` in `non-functional/`. Normative and binding quality constraints, stated
only where the code carries a real obligation (a budget, a bound, a policy).

### 5.5 Acceptance Criteria

`{FR-XXX}-AC-N` / `{NFR-XXX}-AC-N`, inside each requirement. Each acceptance
criterion is an obligation: it names an observable outcome that a wrong
implementation would fail.

---

## 6. Requirement Identification

| Artifact | Format | Example |
|---|---|---|
| Stakeholder Requirement | `StR-XXX` | `StR-001` |
| User Story | `US-XXX` | `US-002` |
| Functional Requirement | `FR-XXX` | `FR-014` |
| Non-Functional Requirement | `NFR-XXX` | `NFR-003` |
| Acceptance Criterion | `{FR}-AC-N` | `FR-014-AC-1` |

Identifiers are immutable once assigned. Tickets are Linear keys (`AGE-NNNN`).

---

## 7. Requirement Quality Policy

Functional requirements SHALL define observable behaviour, be atomic, avoid
implementation detail unless the detail is itself the observable contract (a
wire format, a byte limit), and carry acceptance criteria that can fail.

---

## 8. State and Execution Model

### 8.1 State Semantics

Durable state lives in Redis under one key prefix: per-lead inbox lists, the
registry hash, the alias hash, a capped history stream, per-session name and
suppression caches, and per-escalation hashes with an index. The escalation
lifecycle (`waiting` → `investigating` → `resolved`) is the only state machine;
see [FR-024](./functional/FR-024-escalation-status-transitions.md).

### 8.2 Transition Semantics

A status transition is a single atomic check-and-set. `resolved` is terminal.

### 8.3 Determinism Guarantees

Target resolution is a pure function of the registry snapshot, the alias table
and the typed input. Inbox pops are single Redis commands (or single scripts), so
concurrent consumers are serialised by Redis.

---

## 9. Events and Signals

### 9.1 Event Model

The hook handler acts on four harness events: `SessionStart`,
`UserPromptSubmit`, `PostToolUse` and `Stop`. Every other event is ignored.

### 9.2 Event Guarantees

- Ordering: messages in one inbox are delivered oldest first; a requeue restores
  the original order.
- Delivery: at most once per message per recipient (see
  [FR-010](./functional/FR-010-send-queues-one-copy-to-the-addressed-lead.md)).
- Failure: a delivery that definitely did not reach the pane is requeued; one
  whose outcome is ambiguous is not retried, stays in the history log, and the
  human is notified.

---

## 10. Error and Failure Model

### 10.1 Error Classification

- Configuration errors (malformed file, empty required value, unparsable env).
- Resolution errors (invalid address, ambiguous target, unknown target).
- Validation errors (invalid name, oversize message, unsafe text, invalid ticket,
  session or escalation id).
- Store errors (Redis unreachable, timeout, undecodable record).
- herdr errors (spawn failure, timeout, reported error, unexpected output).

### 10.2 Failure Handling Guarantees

- A hook never fails the agent's turn ([FR-017](./functional/FR-017-hook-never-fails-the-turn.md)).
- A rejected send queues nothing.
- One undecodable stored record never hides the rest
  ([FR-008](./functional/FR-008-tolerant-registry-decoding.md)).
- An uncertain herdr answer never deletes a registration and never types into a pane.

---

## 11. Traceability and Verification

Traceability runs StR → US → FR/NFR → acceptance criterion → test, and is
recorded in frontmatter relationships and in `tests.md`. The verification method
for each acceptance criterion is its `Verification` cell; the advisor output in
`analysis/` and the evidence review in `reviews/` record which method the catalog recommends for each.

---

## 12. Known Gaps Against Current Code

Measured against `main` at de799f9, after AGE-2026 (#13). The requirements below
state behaviour that code does not yet exhibit; their acceptance criteria are
expected to fail when evidence is written against them, until the named ticket
lands.

| Requirement | Gap on `main` | Ticket |
|---|---|---|
| [NFR-003](./non-functional/NFR-003-watcher-keeps-running.md) (rows 2, 3) | the watcher exits after 60 s without a successful poll and relies on a service manager | AGE-2033 |
| [FR-028](./functional/FR-028-watcher-heartbeat.md) to [FR-037](./functional/FR-037-escalation-retention.md) | built by AGE-2033: watcher heartbeat (FR-028), `agent-msg doctor` (FR-029), herdr version and shape check (FR-036). Built by AGE-2033 as [FR-043](./functional/FR-043-moves-and-renames-are-followed-and-recorded.md): following a session to its new pane, rename and relink events, and the `who` line for a pane that hosts another session (FR-032-AC-4). Built by AGE-2033 as [FR-041](./functional/FR-041-send-prints-the-recipients-state.md): the `send` state line (FR-032-AC-1 and AC-3). Not built: dead letters (FR-031), the workspace and tab ids of FR-033/FR-034 and their 7-day, 10,000-event log, old-address refusal (FR-035), escalation retention (FR-037) | AGE-2033 |

Closed by AGE-2032 and removed from this table: FR-020 (a popped batch stays in
flight until acknowledged, and returns when its lease expires) and FR-040
(redeliveries are marked, bounded and dead-lettered).

Closed by AGE-2026 and removed from this table: FR-005 (one address rule),
FR-008 (tolerant registry decoding), FR-009 (pane ids scoped by machine,
including `send`), FR-013 (atomic budgeted watcher pop), FR-015 and FR-016 (one
unsafe-character rule, refused at send and neutralised at render and in `log`),
FR-022-AC-1..5 (collision suffixes), FR-023 (`EscalationId`), FR-025 (C1 controls
and the decision text), the NFR-001 transcript row (8 MiB scan window) and the
NFR-002 herdr row (output drained on threads).

---

## 13. Change Management

Requirements change by pull request against this directory. A change to an
obligation's statement changes its identity for evidence purposes; evidence
recorded against the old statement is stale until re-run.

---

## 14. References

- ISO/IEC/IEEE 29148 — Requirements Engineering
- AGE-2007 — the ix-relay epic; AGE-2010, AGE-2013, AGE-2014, AGE-2015, AGE-2016,
  AGE-2017, AGE-2018, AGE-2022, AGE-2023, AGE-2024 — the delivered features;
  AGE-2026 — hardening; AGE-2029 — this specification; AGE-2031, AGE-2032 and
  AGE-2033 — open gaps.

---

## 15. Wire-Format Inventory

Informative (AGE-2027). Every format ix-relay reads or writes, the type that owns
it, whether unknown fields are refused or tolerated and why, and whether a
requirement and a test cover it. The Claude Code inbox-socket formats are typed in the
`claude-wire` crate ([FR-046](./functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md),
AGE-2061) and 15.3 lists every external format with its drift signal. "Pinned" means a test
asserts the exact field list.

| Format | Owning type | Unknown fields | Requirement | Test |
|---|---|---|---|---|
| Redis key layout `<prefix>{inbox:<name>, agents, aliases, log, health:hooks, health:watcher:<machine>, health:watcher-seen, health:moves, health:replaced:<lead>, esc:<id>, esc:<id>:attempts, esc:seq, esc:index, session:<sid>, suppressed:<sid>, unresolved:<sid>, inflight:<name>, deadletter:<name>, inflight-seq}` | `relay_core::store::keys::Keys`, one builder per key taking typed ids only (`AgentName`, `EscalationId`, `SessionId`, `MachineName`) | n/a (key names) | [NFR-004](./non-functional/NFR-004-prerelease-no-compatibility-layers.md) (no old key names) | every key pinned exactly (`the_keys_are_the_documented_ones`); a builder missing from that table fails (`every_builder_is_in_the_table`); every builder is checked under a second prefix (`every_key_carries_its_prefix_and_nothing_else_changes`); a source guard (`keys_are_built_only_here`, a grep) refuses the prefix, `DEFAULT_PREFIX`, a `"ixr:` literal, a `format!` beside a builder call and a method chained on a builder's result. The guard is a tripwire, not the barrier: the barrier is that `Keys` is private to `store` and a builder takes typed ids |
| Lua script args (registration commit, demote/remove, pop, requeue, escalation transition) | positional `KEYS`/`ARGV` in `store.rs`, `store/commit.rs` | n/a (positional) | FR-010, FR-012, FR-013, FR-024, FR-033 | behaviour tested through `Relay`; arg order is not a type |
| `Message` (inbox list entry, log `msg` field) | `relay_core::Message` | refused (`deny_unknown_fields`): a message is written by one binary version and drained within minutes, and an unknown field is a rejected pop entry, never delivered (FR-008) | FR-008-AC-5, FR-015, FR-016 | store tests; field list pinned (`stored_registration_and_message_pin_their_field_lists`); byte-exact against `main` (`a_message_is_byte_identical_to_main`) |
| `Registration` and `Place` (hash `agents`) | `relay_core::Registration`, `Place`; `machine`, `session_id` and `herdr_pane` are `MachineName`, `SessionId`, `PaneId` | tolerated: read by every machine, a newer binary may add a field, and refusing would hide a live lead (FR-008) | FR-006, FR-008, FR-033 | FR-008-AC-1..4 tests; field list pinned; byte-exact against `main`, and malformed ids skipped and reported (FR-008-AC-6, AC-7: `wire_golden`) |
| alias table (hash `aliases`, name to canonical) | string pair, no type | pair skipped unless both are valid `AgentName`s | FR-008 (alias table clause) | `registry_decoding` tests |
| session cache (`session:<sid>` name, `suppressed:<sid>` and `unresolved:<sid>` markers) | key: `SessionId`; value: a bare name or `1`, no type | n/a (opaque values, TTL) | FR-033 | store tests; the raw bytes are pinned (`what_redis_holds_is_byte_identical_to_main`) |
| escalation record (hash `esc:<id>`: `meta` JSON, `status`, `decision`) | `Escalation` (in memory only, not serialised) = id + `EscalationFiling` + status + decision; `EscalationMeta` (id + filing, flattened) is the `meta` blob | tolerated: `meta` has no `deny_unknown_fields` (serde `flatten` and `deny_unknown_fields` do not combine), so unknown members are ignored; a `meta` that does not decode is `Error::Decode`, listed as rejected. `CorruptEscalation` is a missing `meta` or `status`, or an empty hash | FR-023, FR-024, FR-025 | `escalation_model`, `store` tests (Redis); `meta` pinned byte for byte (`the_stored_meta_blob_is_byte_exact_in_both_directions`) |
| log stream entry (`msg`, `requeued` fields) | literal field names in `store.rs` | n/a | FR-013 | store tests; names are literals at write and read |
| `health:hooks` stream entry (`machine`, `lead`, `rec`) | `relay_core::HookHealthEntry` (record is one spool line) | n/a (fixed fields written by one binary) | FR-030 | `hook_failures` tests |
| `health:moves` stream entry (`machine`, `lead`, `rec`) | `relay_core::moves::MoveEvent` (`rec`, JSON) and `MoveKind` | tolerated: a newer watcher may add a member or a kind; an unknown kind reads as `unknown` | FR-043 | field list pinned (`the_event_has_exactly_the_documented_fields`), a newer event reads; written by the same script that changes the registration |
| `health:replaced:<lead>` string (JSON, expiring) | `relay_core::moves::PaneReplaced` | tolerated on read; a field that does not decode is counted, not dropped silently | FR-032 | field list pinned (`the_replaced_marker_has_exactly_the_documented_fields`) |
| hook spool record (`hook.jsonl`) | `agent-msg` `SpoolRecord` | tolerated: shipped to the stream byte for byte, and an older binary must not drop a newer record; `machine` is a `MachineName`, `session_id` stays text (a bounded excerpt of whatever the harness sent) | FR-030 | `spool` tests incl. a future-record test; field list pinned; byte-exact against `main` (`a_spool_record_is_byte_identical_to_main`) |
| `watcher.json` | `relay_core::health::WatcherFile` and `LastError` (not `relay_core::config::WatcherFile`, which is the `[watcher]` table of `config.toml`) | tolerated: written by the watcher and read back by `agent-msg doctor`, which may be older, so a member it does not know is ignored | FR-029 | every key pinned, nested `last_error` included (`the_file_has_exactly_the_documented_fields`) |
| `health:watcher:<machine>` heartbeat (string with expiry) | `relay_core::health::WatcherHeartbeat`, `HeartbeatError` and `OtlpStatus` and `OtlpCounters` (`disabled`; `ignoring` with a note naming an ignored inherited setting; `sending` with a `host:port` target, a `dropped` count when not zero and the exporter's counters when not all zero; `failing` with a target, a reason code and the counters; an absent member reads as none and prints `unknown`, an unknown or undecodable state as `unknown` without failing the record; FR-045) | tolerated: a newer watcher may add a member and an older `agent-msg` must still see it is alive; an unknown error kind reads as `unknown` | FR-028, FR-029, FR-045 | field list pinned (`the_heartbeat_has_exactly_the_documented_fields`), a newer record reads; byte-exact (`wire_golden`) |
| OTLP `ExportMetricsServiceRequest` (`POST <endpoint>/v1/metrics`, OTLP/HTTP JSON encoding, `Content-Type: application/json`; the relay **writes** it, over plain `http://` only) | `relay_watcher::otlp::encode` over the typed registry (`otlp::registry`) and the instrument catalogue (`otlp::series`): `herdr.pane.info`, `herdr.pane.status`, `ix.relay.messages`, `ix.relay.inbox.depth`, `ix.relay.inbox.oldest_age`, `ix.relay.inflight.entries`, `ix.relay.deadletter.entries`, `ix.relay.hook.failures.recent`, `ix.relay.watcher.tick.duration`, `ix.relay.registrations`; attribute values are the typed `PaneId`, `TabId`, `WorkspaceId`, `SessionId`, `MachineName`, `AgentName` or a bounded, neutralised string | n/a (emitted; the response is read tolerantly: a 2xx body is used only for `partialSuccess.rejectedDataPoints`, a string or a number, and any other body is ignored); label values sanitised and cut to 128 characters, JSON-escaped by `serde_json`; a request is at most 4 MiB | FR-045 | the whole request byte for byte (`tc_045_03_the_request_for_a_fake_registry_is_byte_exact`), the exported series pinned against the catalogue and the spec table (`tc_045_11_...`), hostile labels, the HTTP client against a real listener |
| `agent-msg doctor --json` | `doctor::Report`, `Check` and the code enums | n/a (emitted) | FR-029 | field list and every code pinned; severity per code in the fires-with-its-severity test |
| `config.toml` | `relay_core::config::FileConfig` and its `[watcher]` table `relay_core::config::WatcherFile` (`interval_secs`, `delivery`, `otlp_endpoint`, `otlp_headers`, `otlp_interval_secs`; the three OTLP keys and `delivery` are raw values here and validated by `relay-watcher` only, so a typo never stops `agent-msg`) | refused (`deny_unknown_fields`): a typo must not silently fall through to a default, with line and column reported | FR-026, FR-027, FR-045 | `config` tests |
| Claude Code inbox-socket line (written to a session's `messagingSocketPath`) | `claude_wire::Envelope` and `claude_wire::wrap` (typed, emitted) | n/a (emitted); **not documented by Claude Code**, measured on 2.1.283, so used only for a known-good `peerProtocol` and version | FR-044, FR-046 | golden bytes (`the_envelope_line_is_byte_exact`, `the_wrapper_is_byte_exact`, `tc_044_03_the_line_posted_is_exactly_the_measured_one`) and the `envelope-line.json` and `wrapper.txt` fixtures |
| Claude Code `peer_message_status` receipt (read from the watcher's reply socket) | `claude_wire::ControlLine`, `Receipt` and `Verdict` (`parse_control_line` is total) | tolerated: Claude Code owns it and adds fields; a shape it does not know is `Unrecognised` or `NotJson` and a status it does not know is `Unknown` (ambiguous, never delivered); all three are counted as drift | FR-044, FR-046 | the `receipt-*` and `drift-*` fixtures parse to their exact typed result (`every_fixture_parses_to_its_exact_typed_result`), `tc_044_03_receipts_are_read_as_measured_and_junk_is_no_receipt` |
| Claude Code session registry (`~/.claude/sessions/<pid>.json`), read only | `claude_wire::SessionRecord` and `RecordVerdict` (`Supported`, `UnsupportedProtocol`, `UnsupportedVersion`, `Malformed`); the file and process reading stays in `relay_watcher::socket::session` | tolerated: Claude Code owns it; only the fields the lookup needs are read | FR-044, FR-046 | `tc_044_02_*`, `the_verdict_is_typed_for_each_way_to_be_unsupported`, the `session-record.json` and `drift-session-record-*` fixtures |
| `socket-<pid>.json` (one per watcher) | `relay_core::socket_status::SocketStatus` and `LeadSocket` | tolerated: written by the watcher and read back by `agent-msg doctor` | FR-044-AC-9 | round trip (`a_status_round_trips_through_its_file`) |
| hook stdin | `agent_msg::hook::HookInput` | tolerated: the payload belongs to the harness, which adds fields between releases | FR-017, FR-018 | `hook` tests |
| hook stdout | `agent_msg::hook::HookOutput` (`Context` / `Block`) | n/a (emitted) | FR-018 | `hook` tests assert the JSON values |
| `agent-msg inbox --json` | `InboxReport` | n/a (emitted) | none | value pinned in `inbox_report` tests and `inbox_json` |
| `agent-msg whoami --json` | `identity::WhoamiJson` | n/a (emitted) | none | both shapes pinned (`whoami_json_pins_both_shapes`) |
| `agent-msg find --json` | `find::FoundJson` | n/a (emitted) | FR-005 (address shown) | field list pinned (`found_json_pins_the_field_list`); byte-exact against `main` (`found_json_is_byte_identical_to_main`) |
| rendered line `[ix-relay] from <lead>[ (to all)]: <text>` (hook context, watcher prompt, `inbox` text) | `relay_core::Message::render` | n/a (emitted; text neutralised per FR-016) | FR-016 | `message` tests (`render_signs_with_sender_and_marks_broadcast`); agents and the ix-relay skill parse it, nothing cross-checks the skill's wording |
| `plugins/opencode/ix-relay.ts` `WhoamiReport`, `InboxMessage`, `InboxReport` | a hand-written TypeScript schema for `inbox --json` and `whoami --json`, separate from the Rust types | tolerated by construction (TypeScript ignores extra members: it does not read `address` or `location_alias`) | none | none: nothing cross-checks it against `InboxReport`/`WhoamiJson` |
| Claude Code session transcript, `origin.msg_id` (the strong acknowledgement, read at the end of the file) | `claude_wire::scan` and `TranscriptMatch` (`Acked`, `Absent`, `ShapeUnrecognised`); the windowed file read stays in `relay_watcher::socket::transcript` | tolerated: Claude Code owns it; only `origin.msg_id` is read, and a line that names our id without it is `ShapeUnrecognised`, counted, and the batch is acknowledged as received but unconfirmed (no herdr fallback, no redelivery) | FR-044, FR-046 | `tc_044_04_*`, `tc_046_05_*`, `tc_046_06_*`, the `transcript-*` and `drift-transcript-*` fixtures |
| Claude Code session transcript (JSONL), read at `identity.rs` `parse_custom_title_bytes` | none: an untyped `serde_json::Value`, only `type == "custom-title"` and `customTitle` are read | tolerated: the harness owns it | FR-021 (derived names) | `identity` tests |
| `agent-msg send` confirmation line `queued for <lead> (<state>)` | plain text built in `send_state::describe` | n/a (emitted; machine names sanitised) | FR-041 | `send_state` unit tests and `tests/send_state.rs` assert each state's text |
| `who`, `log` | plain text only; no `--json` exists | n/a | none | none |
| herdr CLI JSON (read) | `herdr_cli` envelope plus per-call structs in `relay-ops`, `relay-watcher`; pane, tab and workspace ids in them are `PaneId`, `TabId`, `WorkspaceId` (`herdr_cli` takes plain argument strings and does not know them) | tolerated: herdr owns the format; the version and shape check (`relay_ops::herdr_check`) lists the members ix-relay requires | FR-036 | fake-runner tests |
| `linear` CLI (invoked) | text through `RealLinear` | n/a | FR-024-AC-5 | fake-runner tests |

### 15.1 Gaps found

Fixed in AGE-2027 phase B: the session id, herdr pane id and machine name were
bare `String`s compared by hand at about twenty sites (`normalize_machine_name(&r.machine)
== machine`, `r.herdr_pane.as_deref() == Some(pane)`), and the store's key builders
took `&str`. They are now `SessionId`, `PaneId`, `MachineName` (plus `TabId` and
`WorkspaceId` for herdr's own ids), a pane is compared only as a `PaneRef` (machine
and pane together), and every Redis key is built in one file from typed ids.
Nothing else changed on the wire (15.2). The alias demotion compares the machine too, which closes the first step of a session resumed on two machines in the same pane only: the session cache (`session:<sid>`) knows no machine, so the second machine's next `UserPromptSubmit` can still read the first machine's derived name and register it on itself (pre-existing, rare). AGE-2033's relink (FR-043) does not touch it: the relink follows a *registration* to the pane that hosts its session on the same machine, and the cache is read before any registration exists. Keying the cache by machine is a key change that needs an owner decision (NFR-004), and the measurements of SR-005 say nothing that makes it cheaper, so it is left. The demotion: the Lua `claim_alias` compares the *normalised* stored machine (lowercase, cut at the first `.`, the rule of `MachineName::normalise`), so a record stored as `Hub` is still claimed by `hub` and by nothing else; a shared vector list is run against both (`the_lua_and_rust_machine_rules_agree_on_every_vector`), and the two-machine case is a regression test (`a_demotion_from_another_machine_never_claims_a_lead_with_the_same_session_and_pane`).

Fixed in AGE-2027 phase A: `find --json` was a `serde_json::Value` literal (now
`FoundJson`); `find --json`, `whoami --json`, the stored `Registration`/`Message`
and the spool record had no field-list pin (now pinned); the escalation's filed
fields were declared three times (now one `EscalationFiling`); the escalation `meta` blob and every `watcher.json` key (nested too) are pinned exactly; `watcher.json` lost its `version` field, which nothing read.

Not fixed here (phase A and B):

- `Relay::set_place` splices the new place into the stored registration as a
  `serde_json::Value` on purpose, so members a newer binary added survive the
  rewrite. It is an untyped emit; changing it drops that guarantee (owner
  decision, see the PR).
- The log and health stream field names (`msg`, `requeued`, `machine`, `lead`,
  `rec`) are string literals at the writer and the reader; they belong in
  constants or a typed entry. Left because `store.rs` pop/requeue is in flight
  in AGE-2032.
- Lua `KEYS`/`ARGV` layouts are positional with no type or test that pins them.
- The alias table and session cache values are untyped strings.
- `--json` outputs and `watcher.json` have no requirement id of their own.
- The opencode plugin's TypeScript schema is not checked against the Rust output types.
- Two Claude Code formats are read outside `claude-wire`, with no drift signal or fixture
  yet: the transcript `customTitle` for the derived name (`identity.rs`
  `parse_custom_title_bytes`, an untyped `Value`) and the hook stdin payload
  (`agent_msg::hook::HookInput`, typed and tolerant).
- Two more reads decode an untyped `serde_json::Value` and have no drift signal: the OTLP
  response's `partialSuccess.rejectedDataPoints` (`relay_watcher::otlp::export`
  `rejected_points`: a renamed field silently reads as 0 rejected points) and the hook-health
  record the OTLP relay reads (`relay_watcher::otlp::relay` `hook_failure`: `at_ms`, `stage`,
  `error_kind`, written by `agent-msg`).
- `LeadBacklog.lead`, the lead in a health-stream entry and the alias table stay
  text: a key suffix or a spooled name need not be a bus name. `doctor` reads
  leases only for backlog leads that parse as an `AgentName`.
- `herdr_cli` (no dependency on `relay-core`) and the herdr `--machine` profile
  label take plain strings; `find` turns a saved machine's label into a
  `MachineName` to join it to the registry, and a label that is not one is skipped
  and reported.
- The `SessionId` shape (256 bytes, no control character) is wider than what
  `escalate` accepts for a session it will resume (1 to 64 ASCII letters, digits
  and `-`, FR-023-AC-4); the strict rule still applies at `escalate`.

### 15.2 Typed ids and the tolerance rule per type

Informative (AGE-2027 phase B). Each id is a `String` newtype with
`#[serde(try_from = "String", into = "String")]`: on the wire and in Redis it is
the bare string it always was ([FR-008-AC-7](./functional/FR-008-tolerant-registry-decoding.md)).
The rule below is applied on construction from user input (`FromStr`) and on
decode; there is no separate lenient constructor. A stored value that breaks it
does not decode, and every reader then does what FR-008 already requires of an
undecodable record: skip and report, never fail the whole read.

The shape rule shared by `SessionId`, `PaneId`, `TabId` and `WorkspaceId`: not
empty, at most 256 bytes, and no control, bidirectional or invisible character
(`relay_core::text::is_unsafe_char`, plus line feed and tab).

This is **stricter than what `main`'s writers stored**. `main`'s hook stored the
harness `session_id` unchecked, so an empty, over-long (over 256 bytes) or
control-, tab- or zero-width-bearing session id could be in the registry, and a
pane id with such a character likewise (`main` neither bounded nor refused
either). This binary refuses those values on decode. Values a binary of this
repository writes when the harness and herdr behave (UUIDs, `w1:p1`) pass. A
record that breaks the rule is *registered but unreadable*: its name is taken (a
hook never registers over it, `resolve_collision` numbers past it), `send` queues
to it as `main` did, the registry read skips and reports it, and the watcher's
prune rule still applies to it (read tolerantly from the raw JSON). Only a hand
`agent-msg register --as` or an older binary repairs one.

| Type | Rule | Where it is read | A stored or received value that breaks the rule |
|---|---|---|---|
| `SessionId` | shape rule | `Registration.session_id`, the escalation `meta` blob, the keys `session:`, `suppressed:`, `unresolved:`, hook stdin `session_id`, herdr `agent get` | registration: skipped and reported by the registry read (FR-008), and *registered but unreadable* for a single-record read: `send` and the hook's collision check treat the name as held (see above), the watcher prunes it when stale; `meta`: `Error::Decode`, listed as rejected; hook stdin: read as no session, the hook never fails (FR-017); herdr `agent get`: treated as a session that is not the registered one |
| `PaneId` | shape rule | `Registration.herdr_pane`, `HERDR_PANE_ID`, herdr JSON | registration: as for `SessionId`; `HERDR_PANE_ID`: not running in a pane; herdr list JSON, `find` and the watcher's place refresh (`agent list`, `workspace list`, `tab list`): that entry is dropped and counted (`MachineView::dropped`; the watcher reports it once per episode, `doctor` reports its panes as unchecked), because a dropped agent is a missing row and a dropped tab or workspace a missing place update, never a wrong one; a list in which *every* entry fails is a decode failure as before (FR-036 check, `doctor` unchecked); the lists name derivation reads (`workspace list`, `tab list`, `pane list` in `relay_ops::naming`) stay strict, because derivation needs every sibling (which tab is first, how many tabs), so one undecodable entry fails the call and nothing is derived; herdr single-object JSON (`agent get`, `tab create`): that call fails its shape check (FR-036) |
| `TabId`, `WorkspaceId` | shape rule | herdr JSON only | as for `PaneId` (strict in name derivation, dropped and counted in `find` and the place refresh) |
| `MachineName` | lowercase and cut at the first `.`, then not empty, at most 256 bytes, no unsafe character | `Registration.machine`, the escalation `meta` blob, `WatcherHeartbeat.machine`, spool records, health-stream entries, the `health:watcher:<machine>` key suffix and the `health:watcher-seen` fields, the configured machine, the labels of saved herdr machines | decodes normalised when it only differs in case or suffix; otherwise registration: skipped and reported; heartbeat: `Undecodable`; stream entry: left out and counted (`HookHealth::unreadable`, shown by `doctor`); spool record: does not decode, and the flush that removes such a line records the count as `dropped` in a record of its own; key suffix or marker field: left out and counted (`Heartbeats::unreadable`, shown by `doctor`); marker fields that normalise to one machine (`Hub`, `hub`) are one machine and the newest tick wins; configured machine: refused (FR-026); saved machine label: skipped and reported by `find` |
| `PaneRef` (machine and pane) | none of its own | never stored or emitted | n/a: built from a registration's two fields, or from the local machine and `HERDR_PANE_ID` |

Normalising a stored machine name is not a compatibility layer
([NFR-004](./non-functional/NFR-004-prerelease-no-compatibility-layers.md)): the
rule is the one comparison the code already made at about twenty sites, now made once, in
`MachineName::normalise`, and a record is not rewritten unless its owner rewrites it.

A pane id means something only on the machine whose herdr issued it
([FR-009](./functional/FR-009-pane-ids-scoped-by-machine.md)); the type system now
says so: the store, the resolver, `find`, the watcher and `agent-msg` compare
panes only as a `PaneRef`, so `machine == local && pane == p` cannot be written
with one half missing.

### 15.3 Every external format: owner, decode, drift signal

Informative (AGE-2061). The rule: every external interface the relay touches has a typed
wire format in its own crate or module, and a drift signal, so that a change on the other
side is detected and not silently absorbed. "Strict" refuses what it does not know;
"tolerant" reads what it needs and ignores the rest; "emitted" is written only. A
signal that reads "none" is a gap, stated as one.

| External format | Owner (type, crate) | Decode | Drift signal | Fixtures and tests |
|---|---|---|---|---|
| Redis keys | `relay_core::store::keys::Keys` (`relay-core`), typed ids only | n/a (names the relay owns on both ends) | none needed: one writer and one reader; a renamed key would be a code change | every key pinned; a grep guard refuses a key built elsewhere |
| Redis Lua scripts (`KEYS`/`ARGV`) | positional, `store.rs` and `store/commit.rs` | n/a (positional) | none: a layout change is a code change, and a gap (15.1) | behaviour tests through `Relay` |
| Stored records: `Message`, `Registration` and `Place`, escalation `meta`, health entries, `watcher.json`, the heartbeat, spool records, `socket-<pid>.json` | `relay_core` types (`relay-core`), `SocketStatus` in `relay_core::socket_status` | `Message`: strict; the others tolerant (a newer binary may add a member) | a record that does not decode is skipped, counted and shown (`registration_undecodable`, `Heartbeats::unreadable`, `HookHealth::unreadable`, rejected escalations) | field lists pinned; byte-exact against `main` (`wire_golden`) |
| Hook stdin | `agent_msg::hook::HookInput` (`agent-msg`) | tolerant: the harness owns it | none: a payload that does not decode reads as no session and the hook never fails (FR-017); a gap | `hook` tests |
| Hook stdout | `agent_msg::hook::HookOutput` (`agent-msg`) | emitted | none: emitted bytes are asserted, not observed on the other side | `hook` tests assert the JSON values |
| herdr CLI JSON | `herdr_cli` envelope and per-call structs (`herdr-cli`, `relay-ops`, `relay-watcher`) | tolerant, with a version and shape check | `agent-msg doctor` `herdr_unsupported_version`, `herdr_shape_mismatch`, `herdr_unchecked`; undecodable entries dropped and counted (`MachineView::dropped`) | fake-runner tests |
| OTLP `ExportMetricsServiceRequest` | `relay_watcher::otlp::encode` (`relay-watcher`) | emitted; the response is read tolerantly | a rejected point or failing push is counted and shown (`otlp` heartbeat state, `watcher_otlp_failing`) | the whole request byte for byte (`tc_045_03_*`) |
| Claude Code session record `~/.claude/sessions/<pid>.json` | `claude_wire::SessionRecord`, `RecordVerdict` (`claude-wire`) | tolerant | `RecordVerdict` other than `Supported`: the lead is not sent to the socket, and `doctor` names it (`socket_fallback`); `doctor` shows the captured version and says `unverified` for another | `session-record.json`, `drift-session-record-*` fixtures, `the_verdict_is_typed_for_each_way_to_be_unsupported` |
| Claude Code inbox-socket line | `claude_wire::Envelope`, `wrap` (`claude-wire`) | emitted | the version and `peerProtocol` gate; `doctor` `unverified` line | `envelope-line.json`, `wrapper.txt`, golden bytes, escaping property tests |
| Claude Code receipts (`peer_message_status`) | `claude_wire::ControlLine`, `Receipt`, `Verdict` (`claude-wire`) | tolerant, total | `Unrecognised`, `NotJson` and `Unknown` counted in the status file; `doctor` `socket_wire_drift`; one log line per kind | `receipt-*` and `drift-receipt-*`, `drift-control-*`, `drift-not-json.txt` fixtures; `tc_046_03`, `tc_046_04` |
| Claude Code transcript `origin.msg_id` | `claude_wire::scan`, `TranscriptMatch` (`claude-wire`) | tolerant, total | `ShapeUnrecognised` counted; the batch is acknowledged as received but unconfirmed, with no herdr fallback and no redelivery; `doctor` `socket_transcript_shape_unrecognised` | `transcript-*` and `drift-transcript-*` fixtures; `tc_046_05`, `tc_046_06` |
| Claude Code transcript `customTitle` (derived names) | untyped `Value`, `agent-msg` `identity.rs` | tolerant | none: a gap (15.1) | `identity` tests |
| `config.toml` | `relay_core::config::FileConfig` and `WatcherFile` (`relay-core`) | strict (`deny_unknown_fields`) | a typo is refused with line and column (FR-027); `doctor` `config_invalid`, `watcher_otlp_config_invalid` | `config` tests |
| `/proc/<pid>/stat` start time and `/proc/self/ns/pid`, `/etc/machine-id` (Linux kernel formats, the pid identity of a Claude Code process) | `relay_core::socket_status::start_time_from_stat` and `relay_watcher::socket::session::ProcFs` | tolerant: the field is counted from the last `)` | none: an unreadable value makes the record stale and the lead uses herdr (`Unresolved::Stale`); a gap | `tc_044_02_the_start_time_is_read_past_a_command_name_with_parentheses` |
| `agent-msg inbox --json`, `whoami --json`, `find --json` as read by the opencode plugin | Rust `InboxReport`, `WhoamiJson`, `FoundJson`; the plugin's TypeScript schema is separate | emitted here; the plugin ignores extra members | none: nothing cross-checks the plugin's schema against the Rust types; a gap (15.1) | field lists pinned on the Rust side |
| Rendered line `[ix-relay] from <lead>: <text>` read by agents and the ix-relay skill | `relay_core::Message::render` | emitted | none: agents parse it by convention; a gap | `message` tests |
| `linear` CLI | text through `RealLinear` (`relay-ops`) | n/a | none | fake-runner tests |

This table covers the interfaces named above; the kernel, plugin and rendered-line rows are
listed so their lack of a drift signal is stated, not because they are typed in a crate of
their own.

The Claude Code rows share one version stamp: `claude_wire::CAPTURED_ON` (2.1.283). Each
fixture is marked `captured` (unedited live bytes), `captured-shape` (a live shape or our own
golden, with reconstructed values), `derived-from-program-text` or `synthetic` in
`crates/claude-wire/fixtures/claude-2.1.283/MANIFEST`. When Claude Code changes a format,
the fix is a new fixture directory for the new version, the changed type, and a new
`CAPTURED_ON`.
