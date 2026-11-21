---
id: FR-062
title: "Session-active named focuses with explicit per-focus adoption"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-061"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-049"
    type: "depends_on"
---
# FR-062: Session-active named focuses with explicit per-focus adoption

## Description

A verified agent session SHALL record the particular focuses it is
working on, each pinned to the exact revision it chose. The set is the session's
**active set**. It is separate from the definitions of
[FR-061](./FR-061-named-focus-definitions.md), from the owner of a definition,
and from the session's legacy scope, free-text focus and assignments.

Nothing is adopted automatically. A newer revision of a focus the session uses
is shown as pending until the session adopts it. A session does not follow its
owner's active set or any other session's.

The session commands live in the `agent-msg work named` group. The group name is
not a prefix of, and not one letter from, the existing `agent-msg work focus
<summary>` (free text), so a mistyped verb cannot store a focus ID as free text.

## Inputs

- The caller's verified session binding, as for `agent-msg work`.
- One or more references written `<focus-id>@<revision>`.
- A focus ID (or, for deactivation, an ID or the exact adopted name of an active
  focus).

## Outputs

- The session's named active state: for each active focus, its ID, its name at
  the adopted revision, the adopted revision, the latest revision, the pending
  state, the archived flag and the owner state.
- Typed refusals, with the stable codes of FR-061, naming the focus, the stated
  and the current revision.

## Behavior

### Named mode and the active record

- The relay SHALL treat a session as in **named mode** exactly while that
  session's active record has the mode `named`, and as in legacy mode while the
  record has the mode `legacy` or does not exist.
- The first `work named` change of an incarnation SHALL create its active record
  with the mode `named`, and `work named reset` SHALL set the mode to `legacy`.
- In named mode the relay SHALL allow the active set to hold zero focuses. An
  empty active set SHALL mean **no active focus**; it SHALL NOT mean the whole
  team and SHALL NOT fall back to the session's legacy scope.
- The relay SHALL hold at most 16 active focuses per session, one per focus ID,
  and SHALL allow focuses of different teams to be active together, each team's
  focuses forming a separate group
  ([FR-063](./FR-063-shared-focus-membership-and-summaries.md)).
- The relay SHALL store the active record as one record per session incarnation,
  keyed `<machine>/<native session ID>/<process ID>` (the FR-048 key) and separate
  from the work-report hash. The record SHALL hold the field `mode`, the field
  `seq` (incremented by every write of the record) and one field
  `focus:<focus-id>` per active focus. Each focus field SHALL hold the focus ID,
  the adopted revision number, the full adopted revision content as a snapshot,
  and the times of activation and adoption.
- The relay SHALL keep every focus field at most 58 KiB (a revision of at most 56
  KiB under FR-061 plus its entry fields), so that no field reaches the 64 KiB
  per-field bound. A read of the record SHALL read at most 18 fields, about 1 MiB
  in the worst case, once per refresh.
- The relay SHALL copy the adopted snapshot into the session's own record, so
  that it stays readable if the definition changes, is archived, is removed or
  becomes unavailable.
- The relay SHALL NOT give the active record a time-to-live and SHALL NOT delete
  it because of age, so a live session never loses named mode to the passing of
  time. The relay SHALL remove a record only by `reset` (which sets the mode and
  clears entries, keeping the record), by supersession, or by reclamation below;
  a missing record therefore means only that the session was never in named
  mode, or that its record was superseded or reclaimed (see FR-064).
- The relay SHALL bound growth by at most 64 active records per relay, and SHALL
  restore the bound by the supersession and reclamation below, never by expiry.
- Entering named mode SHALL NOT delete or change the session's legacy scope,
  free-text focus or assignments. They SHALL remain stored and SHALL still be
  reported by `agent-msg work show`, whose output SHALL NOT change.

### Operations

All operations verify the caller and run as one atomic server-side operation over
the active record and every definition they read or pin. They re-check, inside the
operation, each stated revision, the archived flag and every limit, and apply
wholly or not at all.

| Command | Effect |
|---|---|
| `work named activate <id>@<rev>...` | add focuses to the active set at the stated revisions |
| `work named adopt <id>@<rev>...` | move active focuses to the stated newer revisions |
| `work named deactivate <focus>...` or `--all` | remove focuses from the active set |
| `work named set <id>@<rev>...` or `--none` | replace the whole active set |
| `work named reset` | leave named mode and return to legacy mode |
| `work named show` | read the named state |

Every command accepts `--json`, encoded as in FR-061.

- `activate`, `adopt` and `set` SHALL require every stated revision to equal the
  focus's latest revision at the moment of the operation. A different revision is
  refused with `revision_conflict`, carrying the latest revision. A stated
  revision is explicit; there is no "latest" shorthand, and a name where an ID is
  required is refused with `invalid_argument`.
- `activate` of a focus already active at the same revision is a no-op for that
  focus. Activation of one already active at another revision is refused with
  `already_active`; `adopt` is the operation that changes the revision.
- `adopt` of a focus that is not active is refused with `not_active`. Adopting
  replaces the stored snapshot and records the new adoption time.
- Activation and adoption of an archived definition (`archived`), an unavailable
  definition (`unavailable_definition`), an unknown focus ID (`not_found`), or
  beyond the 16-focus, 32-pin or 64-record limits (`limit`, naming the bound) are
  refused. Deactivation of an archived focus is always allowed.
- `deactivate` of a focus that is not active is a no-op. `deactivate` by name
  matches exactly one active entry's adopted name; a name that matches no entry
  is a no-op and one that matches several is refused with `invalid_argument`
  listing the focus IDs. `deactivate --all` and `set --none` leave the session in
  named mode with no active focus.
- `reset` removes all active entries and their pins and sets the mode to
  `legacy`. Legacy scope applies again, and the dashboard labels it as the legacy
  fallback.
- Mutations accept focus IDs, not names, because names can be edited between
  reading and writing.
- Each activation or adoption writes the pin of FR-061 for this incarnation and
  definition in the same atomic operation (`pin put`). Deactivation, `set`,
  `reset` and adoption of another revision release or replace it in the same
  operation (`pin remove`). No pin is written or released outside an operation of
  this table, apart from the reclamation of FR-061.
- Operations write only relay records. They never write Linear, change the
  definition or its owner, consume an inbox or register a subagent. Dashboard
  browsing, view changes and scope selection in the dashboard never call these
  operations.

### Supersession and reclamation

- A verified caller is the one live incarnation of its native session (FR-048).
  The first mutating operation of an incarnation SHALL, in the same atomic
  operation, remove the active records and their pins of every other incarnation
  of the same machine and native session ID. The same native session resumed in a
  new process therefore releases the pins of its earlier process, and 33
  restarts of one session leave exactly one record and its pins. Removal of a
  record is by compare-and-delete on its `seq`, so a record written in between is
  not lost.
- When an operation would create the 65th active record on the relay, reclamation
  examines the other records, takes the session of each from the liveness rule of
  FR-061, and removes (compare-and-delete on `seq`, with its pins) each whose
  session is **absent** or **replaced**, then retries once. Records of a session
  that is live or unknown are kept, and the operation is refused with `limit` if
  none can be removed. Reclamation adds no command, daemon or timer, and runs only
  inside an operation of the table above.
- An incarnation whose record or pins were removed by supersession or
  reclamation reads, on its own `work named show`, as having no named record. It
  is the caller's explicit decision to activate again; nothing is re-created
  automatically.

### Pending revisions and states

- For each active focus the state is one of: **current** (adopted equals
  latest), **update pending** (latest is newer; shows the adopted and latest
  numbers), **archived** (adopted revision retained; no update can follow),
  **unavailable** (the definition cannot be read now; the adopted snapshot is
  retained and labeled last adopted, with currentness unknown) or **removed** (the
  definition is no longer stored although the entry proves it existed). Absence of
  evidence is unavailable, never current.
- Effective scope for a focus is always its adopted snapshot. A pending revision,
  a rename or an owner change does not alter the adopted snapshot or the
  effective scope.
- An owner's edit while another session has the focus active creates a revision,
  and that session sees update pending. An owner's change of the active set, or
  the owner leaving, changes no other session.

### Concurrent and restart behavior

- Two sessions, or two commands of one session, racing on the same focus or
  active record produce one consistent result: each succeeds as if serialized or
  receives a typed conflict.
- An adoption that races with an update to the definition either adopts the
  revision it named, if that was still latest when the operation ran, or is
  refused with `revision_conflict`. It never adopts a mixed or later revision.
- A replacement process of a session has no active set until its first
  `work named` change; the former incarnation's record is not attributed to it
  and is not read by it. That first change supersedes the former record as above.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-062-AC-1 | With focus F at latest revision 2, `activate F@2` stores the active entry with a snapshot of revision 2 and pins F for this incarnation; `activate F@1` is refused with `revision_conflict` carrying 2 and changes nothing; a name instead of an ID is refused with `invalid_argument`; `latest` is refused. | Test |
| FR-062-AC-2 | A call activating F@2 and G@1 where G's revision is stale activates neither and pins neither. | Test |
| FR-062-AC-3 | A call activating 17 focuses, an archived focus, an unavailable definition or an unknown ID is refused with its stable code naming the reason, and no entry or pin is added. | Test |
| FR-062-AC-4 | With F active at 2 and the owner publishing revision 3, the session reads update pending (adopted 2, latest 3) and its effective scope remains revision 2; `adopt F@3` makes it current and moves its pin to revision 3; `adopt F@2` is refused with `revision_conflict`; `adopt` of an inactive focus is refused with `not_active`. | Test |
| FR-062-AC-5 | An adoption of F@2 racing an owner update stores either revision 2 (if 2 was still latest) with pending 3, or is refused with `revision_conflict`; the stored snapshot is never a mixture and never revision 3 from the racing call. | Test |
| FR-062-AC-6 | `activate F@2` for a focus already active at revision 2 changes nothing and succeeds; `activate F@3` for a focus active at 2 is refused with `already_active`; `deactivate` of an inactive focus succeeds and changes nothing. | Test |
| FR-062-AC-7 | `deactivate` by the exact adopted name removes that entry and its pin; a name matching two entries of different teams is refused with `invalid_argument` listing both IDs and changes nothing. | Test |
| FR-062-AC-8 | After `set F@2 G@1` and `set --none`, the active set is replaced atomically, the session is in named mode with no active focus, every pin of the incarnation is released, and the legacy scope is stored unchanged. | Test |
| FR-062-AC-9 | For a session with legacy scope S, free text and assignments, entering named mode leaves `agent-msg work show` bytes and stored values unchanged; `reset` sets the mode to `legacy`, releases every pin and S applies again. | Test |
| FR-062-AC-10 | With F adopted at revision 2 and archived by its owner, the session still reads revision 2 labeled archived, activation of F by another session is refused with `archived`, and `deactivate` succeeds. | Test |
| FR-062-AC-11 | With the definition's store unreadable or the definition removed, the retained snapshot is shown as unavailable (last adopted, currentness unknown) or removed, never as current or empty. | Test |
| FR-062-AC-12 | For a renamed focus the active entry shows the adopted revision's name and the current name when different, and the effective scope does not change. | Test |
| FR-062-AC-13 | A record under a read-only fixture clock moved 31 days forward is still present and still in named mode, and a dashboard-style read of it never extends or changes it. | Test |
| FR-062-AC-14 | After a process restart, the replacement shows no active set before its first change; its first `work named` change removes the earlier incarnation's record and pins in the same atomic operation, and a record rewritten between the read and the removal is kept. | Test |
| FR-062-AC-15 | Thirty-three consecutive incarnations of one native session each activate F; afterward one record and one pin of F remain, and the 33rd activation is not refused with `limit`. | Test |
| FR-062-AC-16 | Thirty-three incarnations of different, now absent sessions each hold a pin on non-archived F; a 34th session's `activate F@n` succeeds after reclamation removes the dead pins and their records, while pins and records of live or unknown sessions are kept and, with 32 of them, the activation is refused with `limit`. | Test |
| FR-062-AC-17 | With 64 active records on the relay, a 65th incarnation's first change succeeds only after records of absent or replaced sessions are removed, and is refused with `limit` naming the record limit if every other record belongs to a live or unknown session. | Test |
| FR-062-AC-18 | Two sessions that each activate a different focus owned by one lead each see only their own focus; the owner's own active set, activation or exit changes neither. | Test |
| FR-062-AC-19 | Dashboard navigation, scope selection and view changes leave every relay record byte-identical. | Test |
| FR-062-AC-20 | Every refusal of a `work named` command carries a code from the stable list of FR-061, and `agent-msg work focus "activate foc-…@2"` still stores that text as free text, exactly as before, while the named verbs are accepted only under `work named`. | Test |

## Dependencies

- [FR-061](./FR-061-named-focus-definitions.md) defines definitions, revisions,
  the pin primitives, the liveness evidence rule, reclamation and the stable
  error codes this requirement calls and returns.
  [FR-048](./FR-048-session-work-monitor.md) defines the session binding and
  incarnation key and the legacy per-session reports.
- [FR-049](./FR-049-shared-cli-json-and-version.md) defines the shared compact
  JSON encoding for `--json`.
- Out of scope: automatic adoption, following an owner's or another session's
  active set, a mandatory worker or goal budget, and monitor thresholds.
