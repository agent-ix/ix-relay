---
id: FR-008
title: "One bad registration never hides the registry"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-008: One bad registration never hides the registry

## Description

When the store reads the registry or the alias table, the store shall ignore fields of a stored
registration or place that it does not recognise, and shall skip and report any
record that still fails to decode, returning every other registration.

## Behavior

- The store shall decode a registration carrying fields it does not know, keeping
  the fields it does know.
- If a registry record fails to decode, then the store shall return every other
  registration and shall report the failing record's key and the reason.
- When the store reads one registration (a single-record read), and that record
  is undecodable, then the store shall return a decode error naming the record's
  key, not an absent registration and not a storage error.
- When the store reads the alias table, the store shall skip a pair that is not
  two valid bus names, and shall return the others.
- The watcher shall continue its tick when the registry contains an undecodable
  record.
- If a stored record's session id, herdr pane id or machine name breaks the rule
  of its type (`SessionId`, `PaneId`, `MachineName`: not empty, at most 256 bytes,
  no control, bidirectional or invisible character), then every reader shall skip
  and report the record under the rules above.
- When the record of a name a message is addressed to is undecodable for that
  reason, `agent-msg send` shall queue the message to that name.
- When the name a hook derived is held by an undecodable record, the hook shall
  hold the name and shall not register over it.
- If a stale registration's record is undecodable, then the watcher shall prune it
  by the same rule as any other stale registration.
- When a stored machine name differs from its normal form only by case or a
  domain suffix, the store shall decode it as its normal form.
- The serialised form of every stored or emitted type that carries a session id,
  herdr pane id or machine name shall be identical to its form when those were
  strings.
- Stored messages keep a strict shape: a queued message with an unknown field is
  reported as a rejected entry of the pop that took it, never delivered.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-008-AC-1 | Given a stored registration JSON with an extra top-level field and an extra field inside `place`, reading the registry returns that registration with every known field intact. | Test |
| FR-008-AC-2 | Given a registry holding three valid registrations and one record that is not JSON, reading the registry returns the three valid registrations and one report naming the bad record. | Test |
| FR-008-AC-3 | Given a registry holding an undecodable record, a watcher tick delivers to a local idle lead with a queued message and the watcher does not exit. | Test |
| FR-008-AC-4 | Over any byte strings stored as registry records (including bytes that are not UTF-8), every list read (the registry, the alias table, the broadcast recipients) returns without panicking and without failing as a whole, skipping undecodable records, and every single-record read of an undecodable record returns a decode error naming its key. | Test |
| FR-008-AC-5 | Given a queued message with an unknown field, the pop that takes it reports it as rejected and returns the other messages of the batch. | Test |
| FR-008-AC-6 | Given registrations, a heartbeat, a health-stream entry, a spool record and a hook payload whose machine, session or pane breaks its type's rule (empty, over 256 bytes, a control character, a machine of `.local`), the registry read returns every other registration and reports each bad record by key, a heartbeat reads as undecodable, a stream entry is left out, a spool record does not decode and a hook payload reads as having no session; and a registration stored with machine `Node-B.local` decodes with machine `node-b`; `send` to a name whose record is unreadable queues as it did on `main`, a hook whose derived name matches one never registers over it, a stale one is pruned by the watcher, and the entries or fields the health and marker reads leave out are counted. | Test |
| FR-008-AC-7 | Given a `Registration` (with and without place, session, pane), a `Message`, a `WatcherHeartbeat`, a `WatcherFile`, a spool record, a `find --json` row and the raw bytes Redis holds for a registration, a heartbeat, a health-stream entry and the session cache, each serialises to exactly the bytes captured from `main` at 8578b01 for the same input and decodes them back to the same value. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Constrained by [NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md).
- AGE-2027 phase B added `SessionId`, `PaneId`, `TabId`, `WorkspaceId`,
  `MachineName` and `PaneRef` (AC-6, AC-7). Tolerance per type is in
  [spec.md section 15.2](../spec.md#152-typed-ids-and-the-tolerance-rule-per-type).
- Gap closed by AGE-2026: `Registration` and `Place` accept unknown fields, and
  `Relay::agents` returns `AgentList { registrations, skipped }`; the watcher
  reports each skipped record once and carries on.
