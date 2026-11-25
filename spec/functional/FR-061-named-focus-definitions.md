---
id: FR-061
title: "Durable, revisioned named focus definitions with one owner"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-049"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-047"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-008"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-057"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/NFR-002"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/NFR-004"
    type: "depends_on"
  - target: "ix://agent-ix/ix-board/FR-010"
    type: "depends_on"
  - target: "ix://agent-ix/ix-board/FR-016"
    type: "depends_on"
  - target: "ix://agent-ix/ix-board/FR-004"
    type: "depends_on"
---
# FR-061: Durable, revisioned named focus definitions with one owner

## Description

The relay SHALL store a **named focus definition**: a named, shareable
selection of Linear work for one team. A definition is a library entry. It is
not a Linear object, it cannot contain another focus, and it does not say who
is working on it. Each definition has one stable focus ID, one editable name,
immutable numbered revisions, and exactly one owner. It survives the exit of
the session that created it.

This requirement owns the definition group of the named-focus contract:
identity, revisions, ownership, archive, bounds, retention, the adoption pin
primitives that retention needs, the session liveness evidence rule, and the
`agent-msg focuses` command family. It is the first delivery of the contract and
depends on none of the later requirements. Session activation and adoption
([FR-062](./FR-062-session-active-focuses-and-adoption.md)), membership
calculation ([FR-063](./FR-063-shared-focus-membership-and-summaries.md)) and
dashboard presentation ([FR-064](./FR-064-named-focus-dashboard-reader.md)) build
on it, in that order. The dependency direction is one way: those requirements
declare edges to this one, and this one declares no edge back to them, because
the reverse edges would form a cycle.

The existing per-session work reports, scope selectors, free-text focus and
assignments of [FR-048](./FR-048-session-work-monitor.md), the lane focus text of
[FR-057](./FR-057-lane-focus-is-explicit-and-visible.md) (`agent-msg focus`), and
`agent-msg work focus <summary>` are not changed by this requirement.

## Inputs

- A definition file (JSON) naming a team, a name, an optional description, and
  inclusion and exclusion references for initiatives, projects, milestones and
  tickets.
- An expected revision, or an expected owner epoch, for every change to an
  existing definition.
- The caller's verified session binding (explicit `--lead` and `--session`, or
  the verified calling pane), as for `agent-msg work`.
- A workspace snapshot from the shared ix-board read service, used only to
  resolve references.

## Outputs

- Definitions, revisions and ownership state, in text or as one compact JSON
  value per command.
- Typed refusals that name the failing rule, the field and, for conflicts, the
  current revision or owner epoch.

## Behavior

### Identity and names

- A definition has an opaque focus ID assigned by the relay at creation: the
  prefix `foc-` followed by 32 lowercase hexadecimal characters from 128 random
  bits. The ID never changes and is never reused.
- A name is 1 to 128 UTF-8 bytes (the existing display-name bound), is
  sanitized as a single line (as a work-report field) and is unique per team
  among non-archived definitions, compared case-insensitively. A name is a label,
  not an identity: renaming creates a new revision with the same focus ID.
  Archiving releases the name.
- Commands that read accept either the focus ID or the exact current name.
  Commands that change a definition resolve a name to its ID first and then
  change by ID; the same name resolving to a different ID before the change is
  refused.
- A definition names exactly one Linear team key. It SHALL NOT contain another
  focus; no field carries a focus reference.

### Revisions

- Each create or update writes one immutable revision. A revision holds the
  focus ID, revision number, team, name, description (at most 512 bytes), the
  inclusion list, the exclusion list, the owner epoch at write time and the
  write time.
- Inclusion and exclusion each hold four kinds of members using canonical
  Linear stable IDs: initiatives, projects, milestones and tickets. A milestone
  member records its owning project ID as well as its milestone ID. A ticket
  member covers its descendants, as in the existing scope semantics. Each member
  keeps a display-name label of at most 64 bytes (cut on a UTF-8 boundary), used
  only for display.
- An empty inclusion list is a valid focus. It selects nothing; it never means
  the whole team.
- Revision numbers start at 1, increase by exactly 1 using checked arithmetic
  and are never reused. Overflow is refused.
- Every change to an existing definition carries `--expected-revision`. The
  relay applies the change in one atomic server-side operation that re-reads the
  current revision, compares it with the expectation and either writes or
  refuses with a typed conflict that carries the current revision. A repeated
  change with an old expectation conflicts. A change whose normalized content
  equals the current revision returns the current revision and writes nothing.
  The caller rereads before deciding to retry.
- The definition file for `update` is a complete replacement of the content, not
  a patch. A definition file with a member the relay does not define (such as a
  focus reference, a second team or a misspelled key) is refused as an invalid
  argument; this strictness applies to the caller's input file, not to stored
  records.

### Reference resolution and the bounded snapshot read

- `create` and `update` SHALL resolve each reference (stable ID, ticket
  identifier or exact name) to a canonical stable ID through ix-board and a
  workspace snapshot. The stored revision contains only the resolved IDs and
  labels.
- A reference that matches zero or more than one object, a milestone not owned
  by its stated project, a ticket of another team, or a project whose milestone
  name is ambiguous without a project qualifier SHALL be refused, naming the
  reference.
- The snapshot comes only from the shared ix-board read service and its existing
  cache, whose entries live five minutes. The relay adds no Linear reader, token
  handling or cache. A snapshot fetched at most five minutes before the command
  resolves its references is accepted. An older, missing, partial or failed
  snapshot SHALL NOT validate a selection.
- The bounded shared read is an **explicit implementation prerequisite** of
  FR-061-AC-19 and FR-061-AC-20 and of FR-063-AC-15, carried by the single ix-board
  specification ticket SWM-178 and by
  [ix-board FR-004](ix://agent-ix/ix-board/FR-004) (cache and polling). The
  owning lane has verified that the current shared read has a fixed five-minute
  cache, a 30-second per-request timeout and no overall budget, and owns an
  additive cache-age and deadline support that keeps existing defaults and typed
  errors. The relay defines no SDK API for it, adds no raw Linear client and runs
  no detached thread it cannot cancel. Until that support exists these criteria
  are blocked, not satisfied by relay-side workarounds.
- The read has an overall deadline: 120 seconds by default, adjustable with
  `--read-timeout <seconds>` from 1 to 600. The deadline clock starts after the
  credentials are ready and before the cache lookup, the read and the selection
  validation. Obtaining credentials is outside the deadline: the normal
  operating-system keychain flow can wait for user interaction before a client
  exists, and the deadline does not bound that wait. The deadline is enforced at
  the source boundary of the shared read, which returns a typed timeout; the
  relay does not run the read on a thread it cannot cancel and starts no read that
  outlives the command. Requests already in flight stop at their own request
  timeout, so once the credentials are ready the command returns within the
  deadline plus one request timeout.
- A failed read refuses with `source_unavailable`, whose `reason` is one of
  `missing`, `stale`, `partial`, `failed` or `timeout`, naming the source, and
  exits with status 1. The relay performs no store write before resolution has
  finished and validated every reference. The one write is a single atomic
  operation, outside the deadline, so a timeout leaves no definition, index, name
  or pin changed.
- Resolved IDs freeze which items the focus selects; the issues beneath them are
  recomputed from the current snapshot at read time
  ([FR-063](./FR-063-shared-focus-membership-and-summaries.md)).
- Resolution, membership and the Linear read are the shared ix-board library's.
  The relay writes nothing to Linear.

### Ownership

- A definition has one owner. The owner record has these fields: `owner_id` (the
  owner's machine and native harness session ID, written `<machine>/<session>`),
  `epoch`, `binding` (`machine`, `native_session_id`, `process_id`, `address` and
  `verified_at`) and `change` (the last change only: `kind` of `create`, `claim`,
  `forced_claim` or `transfer`, `previous_owner_id`, `reason` of at most 256
  bytes, and `at`). Addresses, names and pane labels are display labels and never
  identity. Only `owner_id` identifies the owner; `process_id`, `address` and
  `verified_at` are refreshed without changing the owner.
- The owner epoch starts at 1 and increases by exactly 1 on every claim or
  transfer.
- Only a caller whose verified live binding has the owner's machine and native
  session ID may update or archive the definition or transfer ownership. A
  resumed process of the same native session is the same owner, not a
  replacement, and refreshes the stored binding in the same atomic operation. A
  caller with a different native session, an unverifiable binding, or only a
  matching address SHALL be refused.
- Every restricted operation SHALL be one atomic server-side operation that
  re-reads the stored owner record and compares before any write:
  - `update`, `archive` and `transfer` SHALL carry the caller's `owner_id` and the
    owner epoch the caller verified, and the operation SHALL compare both with
    the stored `owner_id` and epoch. A mismatch of `owner_id` refuses with
    `not_owner`; a mismatch of epoch refuses with `owner_epoch_conflict`; nothing
    is written. A claim or transfer that lands between the caller's check and the
    write therefore makes the displaced owner's write fail.
  - `claim` is made by a caller who is not the stored owner, so it SHALL NOT
    compare the caller's `owner_id` with the stored one. The CLI SHALL read the
    definition, validate the evidence below, and carry the **observed stored
    `owner_id`** from that read together with `--expected-owner-epoch`. The
    operation SHALL compare both with the stored owner record and, only if both
    match, assign the caller as owner with the epoch increased by 1. A mismatch
    of the observed `owner_id` or of the epoch refuses with `owner_epoch_conflict`
    carrying the current owner and epoch, and nothing is written. The observed
    `owner_id` comes from the validated read, so the CLI needs no new flag.
- The creator becomes the owner. The definition, its revisions and its owner
  record do not depend on the owner's session remaining alive. When the owner's
  session ends or is replaced, the definition remains readable and its owner
  state reads absent or replaced. Nothing grants authority to the replacement
  session.
- `claim` makes the verified caller the owner. It SHALL carry the expected
  owner epoch and the observed stored owner ID as above. Without `--force` it succeeds only when the owner's session is
  **absent** or **replaced** by the evidence rule below. When the owner is live
  or unknown, the claim is refused unless `--force --reason <text>` is given. A
  forced claim records its reason in the owner `change`. This is the operator
  recovery path for an owner whose liveness cannot be established. It uses the
  existing trusted relay and operator model and adds no secret, credential or
  daemon.
- `transfer --to <full-address>` is made by the current owner. The target SHALL
  resolve to exactly one verified live session. Transfer carries the expected
  owner epoch and changes only the owner record; it does not activate the focus
  for the new owner.

### Session liveness evidence

One rule decides the state of a session (machine M, native session ID S). It is
used for owner state and claim here, and for pin and record reclamation in this
requirement and in FR-062. It uses only existing evidence: the relay directory
(the registry read with its skipped-record report) and the FR-047 placement state
that the machine's watcher stored for each registration. It applies equally to a
session on another machine, because both are shared relay records.

- **live**: exactly one registration with machine M and session S exists and its
  FR-047 placement is `Verified`.
- **absent**: a successful registry read with no skipped record shows no
  registration with machine M and session S.
- **replaced**: absent as above, and the owner's recorded address belongs to a
  registration with a different native session ID. A different native session ID
  is the only meaning of replaced; the same native session ID resumed in a new
  process is live, not replaced.
- **unknown**: every other case, including a failed or partial read, any skipped
  record, and a registration for M and S whose placement is `Claimed`,
  `Unverifiable` or `Unplaced`. Unknown is never shown as live and never counts
  as absent.

An unknown owner requires `--force --reason` to claim. An unknown pin or record
is kept.

### Archive

- `archive` carries the expected revision, the owner `owner_id` and the owner
  epoch. It marks the definition archived, releases its name and writes no new
  revision.
- An archived definition refuses `update`, new activation, new adoption and
  transfer. It remains readable by ID, and sessions that already adopted a
  revision keep it visible, labeled archived, from their own snapshot.
- Archive is final in this contract; no command restores an archived definition.

### Adoption pins

A **pin** records that one session incarnation has adopted a definition. The pin
primitives belong to this requirement so that definition retention is complete
without FR-062; FR-062 calls them.

- A pin is one field `pin:<incarnation>` on the definition's record, where
  `<incarnation>` is `<machine>/<native session ID>/<process ID>`. Its value has
  `incarnation`, `adopted_revision` and `pinned_at`, at most 512 bytes. A
  definition holds at most one pin per incarnation.
- `pin put` is the only writer. It is atomic with the caller's other writes: it
  refuses an archived definition (`archived`), refuses a new incarnation when 32
  pins exist (`limit`, naming the adopters limit), and otherwise creates or
  replaces the incarnation's pin.
- `pin remove` is the only releaser. It removes a pin only if the stored value
  still equals the value the caller read (compare-and-delete), so a concurrent
  re-pin is never lost. It is called by FR-062 on deactivation, replacement of the
  active set, reset and supersession, and by reclamation below.
- **Reclamation** is a bounded internal step. It examines at most the 32 pins of
  one definition, takes the session of each pin from the liveness rule, and calls
  `pin remove` for each whose session is **absent** or **replaced**. Pins whose
  session is live or unknown are kept. It runs (a) on a `pin put` that would be
  refused for the adopters limit, after which the `pin put` is retried once; (b)
  at each `update`, before older revisions are trimmed; and (c) on each
  definition examined for removal below. It applies to every definition, archived
  or not. It adds no command, daemon or timer, and its evidence read happens
  outside the atomic operation.

### Bounds and retention

The relay SHALL keep every definition store bounded. These choices reuse the
existing bounded-protocol limits where one applies:

| Limit | Value | Reason |
|---|---|---|
| Selectors per revision (inclusions plus exclusions) | 128 | the existing agent-assignment cap |
| Bytes per stable ID or selector | 128 | the existing selector bound |
| Member label | 64 bytes | display only, so cut rather than refused |
| Name | 128 bytes | the existing display-name bound |
| Description | 512 bytes | the existing focus-summary bound |
| Encoded revision | 56 KiB | 128 selectors of at most 384 bytes plus name, description and header fit below the 64 KiB per-field bound |
| Non-archived definitions on one relay | 128 | one operator and a few machines |
| Definitions including archived | 256 | bounds growth |
| Revisions kept per definition | 8 | recent history |
| Pins (adopting incarnations) per definition | 32 | bounds the reclamation step |

- **Layout.** One record per definition holds the fields `meta` (focus ID, team,
  name, state, latest revision, archive time), `owner`, one field `rev:<n>` per
  kept revision and one field `pin:<incarnation>` per pin. Every field is at
  most 64 KiB, and a revision is at most 56 KiB, so no field reaches the existing
  per-field bound. Two small indexes (definitions by ID and names by team and
  folded name) are updated in the same atomic operation as the record they
  describe. A read of one definition reads at most 8 revision fields, 32 pin
  fields and two small fields.
- Beyond the 8 newest revisions, a revision is deleted in the same atomic
  operation that writes a newer one. The revision a session adopted stays
  readable from that session's own snapshot (FR-062), not from the definition.
- Definition, revision, pin and owner records have no time-to-live. Nothing
  expires by age. The 30-day session report time-to-live of FR-048 does not apply
  to definitions.
- Removal of an archived definition is the only deletion of a definition. It is
  attempted only by `create`, only when the 256-definition limit would otherwise
  refuse the new definition, for at most 8 archived definitions, oldest archived
  first. A definition is removed only if it has been archived for at least 90
  days and, after reclamation, holds no pin. A definition with a pin whose
  session is live or unknown is kept. Nothing removes a non-archived definition.
- A write that would exceed a limit is refused with a typed `limit` error that
  names the limit. It never evicts a live definition.
- A definition ID that is not stored reads as `not_found`. A session that holds
  an adopted snapshot of it reports it as removed (FR-062), because its own record
  proves it existed.

### Stored-record tolerance

- A stored definition record is read tolerantly, consistent with
  [FR-008](./FR-008-tolerant-registry-decoding.md) and
  [NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md): an
  unknown member of a record or of a JSON field is ignored on read and is kept
  unchanged when the relay rewrites that field. The relay adds no stored version
  field, versioned reader or migration of its own, so mixed relay versions on
  different machines neither refuse nor drop a newer machine's record.
- A stored record that fails to decode is reported as a typed unavailable
  definition naming the focus ID. It is never treated as empty or healthy and is
  never overwritten by a read.
- One exception, with precedence over NFR-004-AC-1 for this field only: a
  revision embeds the shared ix-board selection, which carries ix-board's own
  explicit `schema_version`. A selection whose `schema_version` the relay's
  pinned ix-board does not support is reported as an unavailable definition with
  the reason `unsupported_schema`, is never overwritten, and is never treated as
  empty. This is the only stored version field and the only version branch; it
  belongs to the shared model and is not a relay format version.

### Command contract

`agent-msg focuses` is a new command family, spelled in the plural so that
`agent-msg focus [text] [--clear]` (FR-057) and `agent-msg work focus <summary>`
keep their meanings and no subcommand is placed under either existing name.

| Command | Effect |
|---|---|
| `focuses create --file <json> [--read-timeout <s>]` | resolve and store revision 1; caller becomes owner |
| `focuses list [--team <key>] [--archived]` | read definitions, current revision, owner state |
| `focuses show <focus> [--revision <n>]` | read one kept revision (the current one by default) |
| `focuses update <focus> --file <json> --expected-revision <n> [--read-timeout <s>]` | write the next revision (rename included) |
| `focuses archive <focus> --expected-revision <n>` | archive |
| `focuses claim <focus> --expected-owner-epoch <n> [--force --reason <text>]` | take ownership |
| `focuses transfer <focus> --to <full-address> --expected-owner-epoch <n>` | hand ownership to a verified session |

- Every command accepts `--json`. With it, standard output is one compact JSON
  value followed by exactly one newline, encoded through the shared CLI JSON
  encoder of [FR-049](./FR-049-shared-cli-json-and-version.md). The
  `show --json` value of a revision uses the same field names as the definition
  file, so its content can be saved and used as an `update` file after the
  expected revision is supplied separately. [FR-063](./FR-063-shared-focus-membership-and-summaries.md)
  adds the `--members` option to `show`.
- A failure exits with status 1 (usage errors keep the status the CLI already
  gives) and, with `--json`, prints one compact JSON error object (`code`,
  `message`, and the conflicting current value or the `reason` when relevant) on
  standard error. The relay adds no other exit status. The stable error codes of
  the whole named-focus contract, including those FR-062 returns, are:

| Code | Meaning |
|---|---|
| `revision_conflict` | the expected or stated revision is not the latest; carries the latest |
| `owner_epoch_conflict` | the expected owner epoch is not current; carries the current epoch |
| `not_owner` | the verified caller is not the stored owner |
| `name_taken` | a non-archived definition of the team has the name |
| `archived` | the definition is archived |
| `limit` | a bound is reached; names the bound |
| `unresolved_reference` | a reference matched no or several objects, or the wrong team |
| `source_unavailable` | the snapshot is unusable; `reason` is `missing`, `stale`, `partial`, `failed` or `timeout` |
| `unavailable_definition` | a stored definition cannot be used; `reason` is `undecodable` or `unsupported_schema` |
| `not_found` | no such definition |
| `already_active` | the focus is already active at another revision (FR-062) |
| `not_active` | the focus is not in the session's active set (FR-062) |
| `invalid_argument` | a malformed input, a name where an ID is required, or a definition file member the relay does not define |

- `--lead`, `--session` and the verified-pane default select and verify the caller
  as for `agent-msg work`. Reads need no binding.
- The commands are normal agent commands: no interactive prompt, editor or
  terminal UI is required. They write only relay records: no inbox is consumed,
  no registration is changed and no Linear field is written.
- `focuses` commands do not change the output of `agent-msg work show`.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-061-AC-1 | `focuses create` with a definition file whose initiative, project, milestone and ticket inclusions and exclusions resolve in a snapshot stores revision 1 with canonical IDs only, a `foc-` ID, and the caller as owner at epoch 1; a reference by exact name resolves to the same stored ID; `show --json` reproduces the file's field names. | Test |
| FR-061-AC-2 | An ambiguous name, a zero-match reference, a milestone not owned by its stated project and a ticket of another team are each refused with `unresolved_reference` naming the reference, and no record changes. | Test |
| FR-061-AC-3 | Of two updates both expecting revision 1, exactly one writes revision 2 and the other receives `revision_conflict` carrying revision 2; repeated use of the old expectation conflicts; content identical to the current revision returns it and writes nothing; revision overflow is refused. | Test |
| FR-061-AC-4 | A rename by update keeps the focus ID and frees the old name; a duplicate name in the same team is refused with `name_taken`; the same name in another team is accepted; a name that resolves to a different ID between resolution and change is refused. | Test |
| FR-061-AC-5 | An empty inclusion list creates successfully, `show` reports an empty selection, and no output describes it as whole-team; a definition file with a focus reference field, two teams or an unknown member is refused with `invalid_argument`. | Test |
| FR-061-AC-6 | After the owner's session exits, the definition, revisions and owner record stay readable with owner state absent; a different session's update, archive and transfer are refused with `not_owner`; a caller with the same address and a different native session is refused; a resumed process of the owner's native session updates successfully and refreshes `process_id`, `address` and `verified_at` without changing `owner_id` or the epoch. | Test |
| FR-061-AC-7 | A claim with the expected epoch succeeds without `--force` and makes epoch 2 when the owner session is absent (no registration, clean registry read) or replaced (the owner's address belongs to a registration with a different native session ID); the same native session ID resumed in a new process is live and the claim is refused; a live owner and each unknown case (failed read, skipped record, placement not `Verified`, a registration on another machine with unverified placement) are refused without `--force`; with `--force --reason` the claim succeeds and the reason is stored in `change`. | Test |
| FR-061-AC-8 | Of two simultaneous claims by different sessions, both carrying the observed owner ID and epoch 1, one wins at epoch 2 and the other receives `owner_epoch_conflict` carrying the new owner and epoch; a claim whose observed owner ID differs from the stored owner ID is refused with `owner_epoch_conflict` even when the epoch matches; the claimer's own ID is never compared with the stored owner ID; and the fences of AC-7 (absent, replaced, live, unknown, same-session resume) still apply. | Test |
| FR-061-AC-9 | An `update`, `archive` or `transfer` by the former owner that reaches the store after a claim or transfer has made another session the owner is refused inside the atomic operation, with `not_owner` for the changed owner ID or `owner_epoch_conflict` for a changed epoch, and the definition, revisions and owner record are unchanged; a `transfer` racing a `claim` of the same epoch yields exactly one new owner. | Test |
| FR-061-AC-10 | After a transfer to a verified live session the epoch increments, the new owner can update and the old owner cannot; a target that resolves to zero or several sessions, or a stale epoch, changes nothing; the target's active focuses are unchanged. | Test |
| FR-061-AC-11 | For an archived definition, `update`, transfer and new pin are refused with `archived`; `show` and `list --archived` still read it; its name is reusable. | Test |
| FR-061-AC-12 | Boundary-sized valid input (128 selectors of 128-byte IDs with milestone project IDs, a 128-byte name, a 512-byte description, 64-byte labels, an encoded revision of exactly 56 KiB) is accepted; one extra selector, one extra byte of any bounded field, or one extra encoded byte is refused with `limit` naming the bound and no change; no stored field exceeds 64 KiB. | Test |
| FR-061-AC-13 | The 129th non-archived definition and the 257th definition including archived are refused with `limit` naming the definition limit, and no definition is evicted. | Test |
| FR-061-AC-14 | After 20 updates only the 8 newest revisions are kept, no record has a time-to-live, and after an idle clock of 31 days the definition, its revisions and its owner remain readable. | Test |
| FR-061-AC-15 | `pin put` creates or replaces one pin per incarnation, is refused with `archived` for an archived definition, is refused with `limit` for a 33rd incarnation, and `pin remove` removes a pin only if its stored value is unchanged, so a pin replaced between read and remove survives. | Test |
| FR-061-AC-16 | Given 32 pins whose sessions are absent or replaced, a `pin put` of a 33rd incarnation on a non-archived definition succeeds after reclamation and the 32 dead pins are gone; given 32 pins whose sessions are live, unknown (failed registry read, skipped record, placement not `Verified`) or on another machine with unverified placement, the 33rd is refused with `limit` and every pin is kept. | Test |
| FR-061-AC-17 | `create` at the 256-definition limit removes at most 8 archived definitions, oldest first, only those archived at least 90 days with no pin after reclamation; an archived definition younger than 90 days, one with a live or unknown pin, and every non-archived definition is kept; a removed ID reads as `not_found`; with no removable definition the create is refused with `limit`. | Test |
| FR-061-AC-18 | A stored record with an unknown member is read with its known members intact and keeps the unknown member after an update of that record; an undecodable record and a selection with an unsupported embedded `schema_version` are each listed as `unavailable_definition` by ID with their reason, other definitions are still listed, and a write to them is refused without overwriting. | Test |
| FR-061-AC-19 | With a snapshot fetched four minutes earlier, `create` and `update` resolve without a refresh; a missing, older than five minutes, partial or failed snapshot is refused with `source_unavailable` and the matching `reason`, exit status 1, and no store change. | Test |
| FR-061-AC-20 | Given a source that never completes, `create` exits with status 1 and `source_unavailable` with reason `timeout` within the deadline plus one request timeout, measured from the point the credentials are ready (the default and a `--read-timeout` of 1 and 600 both honored), starts no background read, and leaves the store byte-identical. This criterion and AC-19 are blocked until the bounded shared read of SWM-178 exists. | Test |
| FR-061-AC-21 | `focuses` commands in `--json` mode print exactly one compact JSON value and one newline on success and a typed error object with a code from the stable list on standard error on failure, with exit status 1; no `focuses` command consumes an inbox, changes a registration or writes to Linear. | Test |
| FR-061-AC-22 | `agent-msg focus`, `agent-msg work focus` and `agent-msg work show` keep their previous behavior and bytes, and `agent-msg focuses` is a separate command family. | Test |

## Dependencies

- [FR-048](./FR-048-session-work-monitor.md) defines session binding and the
  per-session reports this requirement leaves unchanged;
  [FR-047](./FR-047-herdr-placement-must-be-verified.md) defines the verified
  placement used by the liveness rule;
  [FR-057](./FR-057-lane-focus-is-explicit-and-visible.md) keeps `agent-msg focus`.
- [FR-049](./FR-049-shared-cli-json-and-version.md) defines the shared compact
  JSON encoding. [FR-008](./FR-008-tolerant-registry-decoding.md),
  [NFR-002](../non-functional/NFR-002-bounded-resources.md) and
  [NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md) are
  reconciled above.
- The shared ix-board library owns reference resolution, the wire type of a
  selection and the workspace read. Its existing contracts are
  [ix-board FR-010](ix://agent-ix/ix-board/FR-010) (membership, team, leaf and
  qualified selector rules) and [ix-board FR-016](ix://agent-ix/ix-board/FR-016)
  (the versioned focus record and its schema). Stored qualification, exclusions,
  an explicit empty named selection and the batch result are **future ix-board
  extensions**, not available today; they are tracked by the single ix-board
  specification ticket SWM-178, which is an implementation prerequisite of the
  relay work. The bounded read (cache age and an operation-wide deadline with a
  typed timeout, [ix-board FR-004](ix://agent-ix/ix-board/FR-004)) is an explicit
  SWM-178 prerequisite of FR-061-AC-19 and FR-061-AC-20, as stated under
  "Reference resolution and the bounded snapshot read"; this requirement defines
  no SDK API for it. See [FR-063](./FR-063-shared-focus-membership-and-summaries.md) for
  what the relay needs. No intermediate dependency pin is specified here.
- Out of scope: following another session's active set, planner publication,
  worker budgets and goal thresholds (an optional monitor policy, never required
  to create a definition), nested focuses, unarchive, history or drift metrics, a
  separate prune command, and Linear writes.
