---
id: FR-022
title: "A derived name that collides resolves to a free valid name"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-003"
    type: "implements"
---
# FR-022: A derived name that collides resolves to a free valid name

## Description

If a derived or renamed bus name is already registered to a different live
session, then the name resolver shall choose a valid bus name that differs from
the candidate and is not registered to any other session, and shall never register
the session under a name another live session holds.

## Behavior

- The name resolver shall treat a registration as the same caller only when both
  the registration and the caller have a session id and the two are equal.
- If the holder of the candidate name is on another machine, then the name
  resolver shall try the candidate widened with `-<machine>`.
- If the holder is on the same machine and its pane still hosts its session, then
  the name resolver shall try the candidate suffixed `-N` for N from 2 to 1000,
  in increasing order.
- The name resolver shall shorten the candidate before adding a suffix so that
  the suffixed name is at most 32 characters and still ends with the suffix.
- If shortening the candidate and adding the suffix would give the candidate
  itself, then the name resolver shall replace the last byte of the shortened
  candidate (the one next to the dash) with a different letter, keeping the
  suffix whole, so that the suffixed name differs from the candidate and keeps
  the candidate's other characters.
- The name resolver shall choose only a widened or suffixed name that it has
  checked for a collision.
- The name resolver shall not check the same name twice.
- If no free name is found within 1000 attempts, then the name resolver shall
  not register the session, and shall report the collision on stderr.
- If the check of whether the holder's pane still hosts its session fails
  (herdr answers with an error, or the hook's time is spent before it can be
  asked), then the name resolver shall treat the holder as live.
- If the name resolver treated a holder as live for that reason, then the
  hook shall record one non-fatal `derive` note that names the candidate.
- If the name resolver reports that no free name was found, then the hook shall
  register nothing, cache no name, pop nothing, print nothing, exit 0 and record
  one `derive` failure (`no_free_name`) that names the candidate. For the same
  session, later `PostToolUse` and `Stop` hooks within 5 minutes shall make no
  derivation and record nothing; `SessionStart` shall derive again.
- If a `/rename` has no free name and the session already has a name, then the
  `UserPromptSubmit` hook shall keep that name.
- The name resolver shall apply the same check at `SessionStart`, at
  `UserPromptSubmit` after a `/rename`, and on a lazy derivation at `PostToolUse`
  or `Stop`.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-022-AC-1 | Every name chosen for any registry and candidate is a valid bus name, and is either the candidate or a name not registered to another session. | Test |
| FR-022-AC-2 | Given a 32-character candidate registered on another machine, the chosen name is not the candidate. | Test |
| FR-022-AC-3 | Given a 32-character candidate held by a live session on the same machine, the chosen name is not the candidate and ends with `-2`. | Test |
| FR-022-AC-4 | Given the candidate is registered with no session id and the caller has no session id, the caller is not treated as the holder. | Test |
| FR-022-AC-5 | Given a lead `/rename`s itself to a name held by a live session on another machine, the `UserPromptSubmit` hook does not register it under that name. | Test |
| FR-022-AC-6 | Given the candidate and every variant `-2` to `-1000` are held by live sessions, the lead is not registered under the candidate and the registry entry of the candidate's holder is unchanged. | Test |
| FR-022-AC-7 | Given the candidate is held by a session with a pane on the same machine and the herdr liveness call fails, the lead is registered under the next free `-N` name, the holder's registration is unchanged, and one `derive` note names the candidate. | Test |
| FR-022-AC-8 | Given the candidate and every variant `-2` to `-1000` are held by live sessions, the hook exits 0 and prints nothing. | Test |
| FR-022-AC-9 | Given the candidate and every variant `-2` to `-1000` are held by live sessions, five `PostToolUse` hooks of that session leave exactly one `derive` record, naming the candidate. | Test |
| FR-022-AC-10 | Given a candidate of 31 or 32 characters that already ends with the suffix being added, the suffixed name is a valid bus name that differs from the candidate, ends with the whole suffix, and differs from the candidate only in the byte next to the dash. | Test |

## Dependencies

- **Upstream**: [US-003](../usecase/US-003-be-reachable-without-configuring-a-name.md),
  [FR-021](./FR-021-derived-names-are-valid.md)
- Delivered in part by AGE-2018.
- Closed by AGE-2026 for FR-022-AC-1 to FR-022-AC-5: `with_suffix` keeps the
  suffix at 32 characters, `free_variant` checks every variant and is bounded,
  `UserPromptSubmit` checks after a `/rename`, and absent session ids never match.
- FR-022-AC-10 (AGE-2056): a base already ending in the suffix was rebuilt
  unchanged by `with_suffix` at 31 or 32 characters, so `free_variant` could
  offer the candidate itself; the byte next to the dash now changes instead.
- Closed by AGE-2034 for FR-022-AC-6 to FR-022-AC-9: `resolve_collision`
  returns a typed `CollisionError` (`Relay`, `NoFreeName`) or a `Resolved` that
  carries any herdr liveness error; the hook never falls back to the candidate.
  A lead with no free name stays unregistered, and so has no inbox, until a later
  hook can resolve one; `unresolved:<session>` (`SET NX EX 300`) keeps that from
  costing a derivation and a record on every tool call.
