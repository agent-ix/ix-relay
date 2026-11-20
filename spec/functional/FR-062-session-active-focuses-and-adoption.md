---
id: FR-062
title: "Session-active named focuses with explicit per-focus adoption"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-061"
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

## Inputs

- The caller's verified session binding, as for `agent-msg work`.
- One or more references written `<focus-id>@<revision>`.
- A focus ID (or, for deactivation, an ID or the exact current name of an active
  focus).

## Outputs

- The session's named active state: for each active focus, its ID, its name at
  the adopted revision, the adopted revision, the latest revision, the pending
  state, the archived flag and the owner state.
- Typed refusals naming the focus, the expected and the current revision.

## Behavior

### Named mode and the active set

- A session is in **named mode** once it has run any `work focuses` change and
  until it runs `work focuses reset`. Before that it is in legacy mode.
- In named mode the active set may hold zero focuses. An empty active set means
  **no active focus**. It SHALL NOT mean the whole team and SHALL NOT fall back
  to the session's legacy scope.
- A session holds at most 16 active focuses (one per focus ID). Different teams
  may be active together; a focus of each team is a separate group
  ([FR-063](./FR-063-shared-focus-membership-and-summaries.md)).
- Each active entry stores the focus ID, the adopted revision number, the full
  adopted revision content (a snapshot) and the times of activation and adoption.
  The snapshot SHALL be copied into the session's own record, so it stays
  readable if the definition changes, is archived, or becomes unavailable.
- The active set is stored per session incarnation (machine, native session ID
  and process ID), under its own key and not in the work-report hash. A
  replacement process does not inherit the set, consistent with FR-048. The
  record expires 30 days after the last named-focus write by that incarnation,
  as the session's report does; a read never extends it.
- Entering named mode does not delete or change the session's legacy scope,
  free-text focus or assignments. They remain stored and are still reported by
  `agent-msg work show`, whose output does not change.

### Operations

All operations verify the caller, run as one atomic server-side operation over
the active record and every definition they read, and apply wholly or not at all.

| Command | Effect |
|---|---|
| `work focuses activate <id>@<rev>...` | add focuses to the active set at the stated revisions |
| `work focuses adopt <id>@<rev>...` | move active focuses to the stated newer revisions |
| `work focuses deactivate <focus>...` or `--all` | remove focuses from the active set |
| `work focuses set <id>@<rev>...` or `--none` | replace the whole active set |
| `work focuses reset` | leave named mode and return to legacy mode |
| `work focuses show` | read the named state |

- `activate`, `adopt` and `set` SHALL require every stated revision to equal the
  focus's current latest revision at the moment of the operation. A different
  revision is refused with `stale_revision`, carrying the latest revision.
  A stated revision is explicit; there is no "latest" shorthand.
- `activate` of a focus already active at the same revision is a no-op for that
  focus. Activation of one already active at another revision is refused
  (`already_active`); `adopt` is the operation that changes the revision.
- `adopt` of a focus that is not active is refused (`not_active`). Adopting
  replaces the stored snapshot and records the new adoption time.
- Activation and adoption of an archived or unavailable definition, an unknown
  focus ID, or one beyond the 16-focus or 32-adopter limits are refused with the
  limit or reason named. Deactivation of an archived focus is always allowed.
- `deactivate` of a focus that is not active is a no-op. `deactivate --all` and
  `set --none` leave the session in named mode with no active focus.
- `reset` removes the active record and its pins on definitions. Legacy scope
  applies again, and the dashboard labels it as the legacy fallback.
- Mutations accept focus IDs, not names, because names can be edited between
  reading and writing. `deactivate` accepts a name only as an exact match against
  the session's active entries.
- Each active adoption registers the session as a pin on the definition's revision
  ([FR-061](./FR-061-named-focus-definitions.md) retention). A pin is removed by
  `deactivate`, `set`, `reset`, or by a prune that verifies the session's active
  record is gone. A failed or ambiguous read keeps the pin.
- Operations write only relay records. They never write Linear, change the
  definition or its owner, consume an inbox or register a subagent. Dashboard
  browsing, view changes and scope selection in the dashboard never call these
  operations.

### Pending revisions and states

- For each active focus the state is one of: **current** (adopted equals
  latest), **update pending** (latest is newer; shows the adopted and latest
  numbers), **archived** (adopted revision retained; no update can follow),
  **unavailable** (the definition cannot be read now; the adopted snapshot is
  retained and labeled last adopted, with currentness unknown) or **removed**
  (a tombstone). Absence of evidence is unavailable, never current.
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
  refused with `stale_revision`. It never adopts a mixed or later revision.
- After a session's process restarts, the replacement has no active set until it
  activates focuses; the former incarnation's record is not attributed to it.
  The same native session resumed in a new process is a replacement process for
  this record.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-062-AC-1 | Given focus F at latest revision 2, `activate F@2` stores the active entry with a snapshot of revision 2; `activate F@1` is refused with `stale_revision` carrying 2 and changes nothing; a name instead of an ID is refused. | Test |
| FR-062-AC-2 | Given activation of F@2 and G@1 in one call where G's revision is stale, neither is activated. A call activating 17 focuses, an archived focus, an unavailable definition, or a focus with 32 adopters is refused with the reason and no entry is added. | Test |
| FR-062-AC-3 | Given F active at 2 and the owner publishes revision 3, the session reads update pending (adopted 2, latest 3) and its effective scope remains revision 2. `adopt F@3` then makes it current; `adopt F@2` is refused; adopt on a focus not active is refused. | Test |
| FR-062-AC-4 | Given the owner updates while an adoption of F@2 races, the result is either adoption of 2 (when 2 was still latest) with a pending 3, or `stale_revision`; the stored snapshot is never a mixture and never revision 3 by the racing call. | Test |
| FR-062-AC-5 | Given `set F@2 G@1` and then `set --none`, the active set is replaced atomically; after `--none` the session is in named mode with no active focus, its pins are released, and its legacy scope is still stored and unchanged. | Test |
| FR-062-AC-6 | Given a session with legacy scope S, free text and assignments, entering named mode leaves `agent-msg work show` bytes unchanged and the stored values intact; `reset` restores legacy mode with S applying again. | Test |
| FR-062-AC-7 | Given F adopted at revision 2 and then archived by its owner, the session still reads revision 2 labeled archived, activation of F by another session is refused, and deactivate succeeds. | Test |
| FR-062-AC-8 | Given the definition's store is unreadable, expired, or removed, the retained snapshot is shown as unavailable (last adopted, currentness unknown) or removed, never as current or empty. | Test |
| FR-062-AC-9 | Given a renamed focus, the active entry shows the adopted revision's name with the current name when different, and the effective scope does not change. | Test |
| FR-062-AC-10 | Given the active session's process restarts as a replacement, the replacement shows no active set and the old record is neither read nor deleted by it; after 30 days without a write the old record is gone and its pin is released by prune only after the absence is verified. | Test |
| FR-062-AC-11 | Given two sessions that each activate a different focus owned by one lead, each sees only its own focus; the owner's own active set, activation or exit changes neither. | Test |
| FR-062-AC-12 | Given dashboard navigation, scope selection and view changes, no relay record changes (the store is unchanged byte for byte). | Test |

## Dependencies

- [FR-061](./FR-061-named-focus-definitions.md) defines definitions, revisions
  and retention. [FR-048](./FR-048-session-work-monitor.md) defines the session
  binding and incarnation key and the legacy per-session reports.
- Out of scope: automatic adoption, following an owner's or another session's
  active set, a mandatory worker or goal budget, and monitor thresholds.
