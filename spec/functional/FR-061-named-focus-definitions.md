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
identity, revisions, ownership, archive, bounds, retention and the
`agent-msg focuses` command family. Session activation and adoption are
[FR-062](./FR-062-session-active-focuses-and-adoption.md). Membership
calculation is [FR-063](./FR-063-shared-focus-membership-and-summaries.md).
Dashboard presentation is [FR-064](./FR-064-named-focus-dashboard-reader.md).

The existing per-session work reports, scope selectors, free-text focus and
assignments of [FR-048](./FR-048-session-work-monitor.md), and the lane focus
text of [FR-057](./FR-057-lane-focus-is-explicit-and-visible.md), are not
changed by this requirement.

## Inputs

- A definition file (JSON) naming a team, a name, an optional description, and
  inclusion and exclusion references for initiatives, projects, milestones and
  tickets.
- An expected revision or expected owner epoch for every change to an existing
  definition.
- The caller's verified session binding (explicit `--lead` and `--session`, or
  the verified calling pane), as for `agent-msg work`.
- A current ix-board workspace snapshot, used only to resolve references.

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
  keeps a display-name snapshot used only for labels.
- An empty inclusion list is a valid focus. It selects nothing; it never means
  the whole team.
- Revision numbers start at 1, increase by exactly 1 using checked arithmetic
  and are never reused. Overflow is refused.
- Every change to an existing definition carries `--expected-revision`. The
  relay compares it with the current revision and applies the change in one
  atomic server-side operation, or refuses with a typed conflict that carries the
  current revision. A repeated change with an old expectation conflicts. A change
  whose normalized content equals the current revision returns the current
  revision and writes nothing. The caller rereads before deciding to retry.
- The definition file for `update` is a complete replacement of the content, not
  a patch.

### Reference resolution

- `create` and `update` SHALL resolve each reference (stable ID, ticket
  identifier or exact name) to a canonical stable ID through ix-board and a
  current workspace snapshot. The stored revision contains only the resolved IDs
  and labels.
- A reference that matches zero or more than one object, a milestone not owned
  by its stated project, a ticket of another team, or a project whose milestone
  name is ambiguous without a project qualifier SHALL be refused, naming the
  reference.
- A snapshot that is missing, stale, partial or failed SHALL NOT validate a new
  selection. The command SHALL name the unavailable source and write nothing.
  Resolved IDs freeze which items the focus selects; the issues beneath them
  are recomputed from the current tracker snapshot at read time
  ([FR-063](./FR-063-shared-focus-membership-and-summaries.md)).
- Resolution, membership and the Linear read are the shared ix-board library's.
  The relay adds no second resolver and writes nothing to Linear.

### Ownership

- A definition has one owner. The owner record holds an owner ID, an owner
  epoch, and the last verified binding of the owner. The owner ID is the owner's
  machine and native harness session ID. Addresses, names and pane labels are
  display labels and are never identity.
- The owner epoch starts at 1 and increases by exactly 1 on every claim or
  transfer.
- Only a caller whose verified live binding has the owner's machine and native
  session ID may update or archive the definition or transfer ownership. A
  resumed process of the same native session is the same owner and refreshes the
  stored binding in the same atomic operation. A caller with a different native
  session, an unverifiable binding, or only a matching address SHALL be refused.
- The creator becomes the owner. The definition, its revisions and its owner
  record do not depend on the owner's session remaining alive. When the owner's
  session ends or is replaced, the definition remains readable and its owner
  state reads absent or replaced. Nothing grants authority to the replacement
  session.
- `claim` makes the verified caller the owner. It SHALL carry the expected
  owner epoch. It succeeds without `--force` only when the recorded owner's
  session is positively verified absent or replaced (the pane hosts a different
  session, or the owner's registration is gone or replaced in the relay
  directory). When the owner is live, or liveness cannot be established, the
  claim is refused unless `--force --reason <text>` is given. A forced claim
  records the reason (at most 256 bytes). This is the operator recovery path; it
  uses the existing trusted relay and operator model and adds no secret,
  credential or daemon.
- `transfer --to <full-address>` is made by the current owner. The target SHALL
  resolve to exactly one verified live session. Transfer carries the expected
  owner epoch and changes only the owner record; it does not activate the focus
  for the new owner.
- Each ownership change appends one event (previous owner ID, new owner ID,
  epoch, time, and the reason for a forced claim) to the definition's ownership
  history, which keeps the newest 32 events.
- Ownership state in output is one of: live (verified now), absent (verified not
  running), replaced (the owner's pane hosts a different session) or unknown
  (evidence unavailable). Unknown is never shown as live.

### Archive

- `archive` carries the expected revision and the owner check above. It marks the
  definition archived, releases its name and writes no new revision.
- An archived definition refuses `update`, new activation, new adoption and
  transfer. It remains readable by ID, and sessions that already adopted a
  revision keep it visible, labeled archived.
- Archive is final in this contract; no command restores an archived definition.

### Bounds and retention

The relay SHALL keep every definition store bounded. These choices reuse the
existing bounded-protocol limits where one applies:

| Limit | Value | Reason |
|---|---|---|
| Selectors per revision (inclusions plus exclusions) | 128 | the existing agent-assignment cap |
| Bytes per stable ID or selector | 128 | the existing selector bound |
| Name | 128 bytes | the existing display-name bound |
| Description | 512 bytes | the existing focus-summary bound |
| Encoded revision | 32 KiB | half the existing 64 KiB per-field read bound |
| Non-archived definitions on one relay | 128 | one operator and a few machines |
| Definitions including archived | 256 | bounds growth |
| Newest revisions kept per definition | 8 | recent history |
| Adopting sessions per definition | 32 | bounds pinned revisions |
| Ownership events kept | 32 | bounded audit trail |

- Beyond the 8 newest revisions, a revision is deleted in the same atomic
  operation that writes a newer one, unless a live adoption pins it. A pinned
  revision, and the definition record itself, are never deleted while any
  adoption pins them.
- Definition, revision and ownership records have no time-to-live. Nothing
  expires by age. The 30-day session report time-to-live of FR-048 does not apply
  to definitions.
- `agent-msg focuses prune` (and the same bounded step run at most 8 removals
  per `create` when a limit is reached) deletes only an archived definition that
  has been archived for at least 90 days and has no live adoption. An adoption
  is released only by its session's deactivation or when the session's active
  record is verified absent. A failed or ambiguous read keeps the adoption. Each
  removal leaves a tombstone (focus ID, last name, removal time) kept for 365
  days, at most 256 tombstones, so a reader reports "removed" rather than
  unknown. `prune --dry-run` lists what would go.
- A write that would exceed a limit is refused with a typed limit error
  that names the limit. It never evicts a live definition.
- A stored record that fails to decode, carries an unknown field, or embeds a
  shared-model selection with an unsupported ix-board schema version SHALL be
  reported as a typed unavailable definition naming the focus ID. It is never
  treated as empty or healthy and is never overwritten by a read. The relay adds
  no version field, reader or migration of its own, consistent with
  [NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md);
  the embedded selection carries the shared model's own `schema_version`.

### Command contract

`agent-msg focuses` is a new command family. The existing `agent-msg focus
[text] [--clear]` (FR-057) and `agent-msg work focus <summary>` keep their meanings,
so no new subcommand is placed under either name.

| Command | Effect |
|---|---|
| `focuses create --file <json>` | resolve and store revision 1; caller becomes owner |
| `focuses list [--team <key>] [--archived]` | read definitions, current revision, owner state |
| `focuses show <focus> [--revision <n>] [--members]` | read one revision; `--members` adds the current membership from FR-063 |
| `focuses update <focus> --file <json> --expected-revision <n>` | write the next revision (rename included) |
| `focuses archive <focus> --expected-revision <n>` | archive |
| `focuses claim <focus> --expected-owner-epoch <n> [--force --reason <text>]` | take ownership |
| `focuses transfer <focus> --to <full-address> --expected-owner-epoch <n>` | hand ownership to a verified session |
| `focuses prune [--dry-run]` | remove eligible archived definitions |

- Every command accepts `--json`. With it, standard output is one compact JSON
  value followed by exactly one newline, encoded through the shared CLI JSON
  encoder of [FR-049](./FR-049-shared-cli-json-and-version.md). The
  `show --json` value of a revision uses the same field names as the definition
  file, so its content can be saved and used as an `update` file after the
  expected revision is supplied separately. A failure exits nonzero and, with
  `--json`, prints one compact JSON error object (`code`, `message`, and the
  conflicting current value when relevant) on standard error. Error codes are
  stable: `revision_conflict`, `owner_epoch_conflict`, `not_owner`,
  `name_taken`, `archived`, `limit`, `unresolved_reference`,
  `source_unavailable`, `unavailable_definition`, `not_found`.
- `--lead`, `--session` and the verified-pane default select and verify the caller
  as for `agent-msg work`. Reads need no binding.
- The commands are normal agent commands: no interactive prompt, editor or
  terminal UI is required. They write only relay records: no inbox is consumed,
  no registration is changed and no Linear field is written.
- `focuses create`, `update` and `show --json` do not change the output of
  `agent-msg work show`.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-061-AC-1 | Given a definition file with initiative, project, milestone and ticket inclusions and exclusions that resolve in a snapshot, `focuses create` stores revision 1 with canonical IDs only, a `foc-` ID, and the caller as owner at epoch 1. A reference by exact name resolves to the same stored ID. `show --json` reproduces the file's field names. | Test |
| FR-061-AC-2 | Given an ambiguous name, a zero-match reference, a milestone not owned by its stated project, a ticket of another team, and a missing, stale or failed snapshot, each `create` or `update` is refused with its typed code and the failing reference or source named, and no record changes. | Test |
| FR-061-AC-3 | Given two updates both expecting revision 1, exactly one writes revision 2 and the other receives `revision_conflict` carrying revision 2; repeated use of the old expectation conflicts; content identical to the current revision returns it and writes nothing; revision overflow is refused. | Test |
| FR-061-AC-4 | Given a rename by update, the focus ID is unchanged, the old name is free, a duplicate name in the same team is refused with `name_taken`, the same name in another team is accepted, and a name that resolves to a different ID between resolution and change is refused. | Test |
| FR-061-AC-5 | Given an empty inclusion list, create succeeds, `show` reports an empty selection, and no output describes it as whole-team. A definition file containing a focus reference field, two teams, or an unknown field is refused. | Test |
| FR-061-AC-6 | Given the owner's session exits, the definition, revisions and owner record remain readable with owner state absent. A different session's update, archive and transfer are refused with `not_owner`; a caller with the same address but a different native session is refused. A resumed process of the owner's native session updates successfully and refreshes the stored binding. | Test |
| FR-061-AC-7 | Given an owner whose pane now hosts a different session, a claim with the expected epoch succeeds without `--force` and makes epoch 2. Given a live owner, or evidence that cannot establish liveness, the claim is refused unless `--force --reason` is given and the reason is recorded in the history. Two simultaneous claims with epoch 1 produce one winner and one `owner_epoch_conflict`. | Test |
| FR-061-AC-8 | Given the owner transfers to a verified live session, the epoch increments, the new owner can update and the old owner cannot; a target that resolves to zero or several sessions, or a stale epoch, changes nothing; the target's active focuses are unchanged. | Test |
| FR-061-AC-9 | Given an archived definition, `update`, transfer and new activation are refused with `archived`, `show` and `list --archived` still read it, its name is reusable, and a session that adopted revision 2 before archive still reads that revision with an archived label. | Test |
| FR-061-AC-10 | Given boundary-sized valid input (128 selectors, 128-byte IDs and name, 512-byte description, 32 KiB revision) it is accepted; one extra selector or byte, the 129th live definition, the 257th definition, and the 33rd adopting session each fail with `limit` and no change. | Test |
| FR-061-AC-11 | Given 20 updates with one revision pinned by a live adoption, the pinned revision and the 8 newest remain and no other revision does. No record has a time-to-live. After 31 days of an idle clock the definition, its revisions and its owner remain readable. | Test |
| FR-061-AC-12 | Given archived definitions, `prune --dry-run` lists only those archived for at least 90 days with no live adoption; `prune` removes only those, writes tombstones, and keeps a definition whose adopter record cannot be read. A removed ID reads as removed, not unknown. | Test |
| FR-061-AC-13 | Given a stored record that is undecodable, has an unknown field, or embeds an unsupported ix-board schema version, `show` and `list` report it as an unavailable definition by ID, other definitions are still listed, and a write to it is refused without overwriting it. | Test |
| FR-061-AC-14 | `focuses` commands in `--json` mode print exactly one compact JSON value and one newline on success and a typed error object on standard error on failure. `agent-msg focus`, `work focus` and `work show` retain their previous behavior and bytes. No `focuses` command consumes an inbox, changes a registration or writes to Linear. | Test |

## Dependencies

- [FR-048](./FR-048-session-work-monitor.md) defines session binding and the
  per-session reports this requirement leaves unchanged;
  [FR-047](./FR-047-herdr-placement-must-be-verified.md) defines verified
  placement used for owner liveness.
- [FR-049](./FR-049-shared-cli-json-and-version.md) defines the shared compact
  JSON encoding. [NFR-002](../non-functional/NFR-002-bounded-resources.md) and
  [NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md)
  are reconciled above.
- ix-board owns reference resolution and the wire type of a selection; it must
  carry exclusions and milestone qualification as listed in
  [FR-063](./FR-063-shared-focus-membership-and-summaries.md). Relay work
  starts after that contract is accepted; no intermediate dependency pin is
  specified here.
- Out of scope: following another session's active set, planner publication,
  worker budgets and goal thresholds (an optional monitor policy, never required
  to create a definition), nested focuses, unarchive, and Linear writes.
