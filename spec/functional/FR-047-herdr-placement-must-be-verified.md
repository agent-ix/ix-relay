---
id: FR-047
title: "Herdr placement is verified against the harness session"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-003"
    type: "constrains"
---
# FR-047: Herdr placement is verified against the harness session

## Description

ix-relay SHALL derive an addressable lead's location from Herdr's workspace,
tab, and pane identities and verify it against the harness session ID recorded
by ix-relay. A process environment pane value is a claim, not proof. The watcher
is the component that establishes an addressable location.

## Behavior

- ix-relay SHALL classify a registration with no pane claim and no cached
  location as `Unplaced`.
- ix-relay SHALL classify a registration with a pane claim but no verified
  Herdr location as `Claimed`.
- ix-relay SHALL classify a registration whose cached location conflicts with
  its current pane or session claim as `Unverifiable`; this state SHALL never
  be treated as `Verified` for routing or displayed-address selection.
- ix-relay SHALL classify a registration as `Verified` only when it has a pane claim, a harness session
  ID, and a complete Herdr workspace/tab identity obtained from the current
  Herdr layout for the pane reporting that same session ID. The cached Herdr
  evidence SHALL bind those stable IDs to that exact session ID.
- ix-relay SHALL leave a registration without prior verified evidence
  non-routable by address when its session ID is missing, Herdr session
  association is missing or mismatched, pane is absent, or stable IDs are
  incomplete.
- A failed Herdr refresh SHALL NOT create or renew placement evidence; the last
  successfully verified cached placement remains available until a readable
  snapshot confirms or contradicts it.
- The watcher SHALL reject `HERDR_PANE_ID` alone as placement verification.
- The Codex plugin SHALL pass the `SessionStart` session ID into registration;
  ix-relay SHALL use that ID for the exact Herdr join.
- ix-relay SHALL decode legacy label-only registrations and records without
  complete stable identities into an unverified state.
- ix-relay SHALL continue to resolve a bus name or explicit relay alias when
  placement is unverified.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-047-AC-1 | A pane claim plus session ID is not addressable until the current Herdr layout associates that exact session with that exact pane and provides stable workspace/tab IDs. | Unit and integration test |
| FR-047-AC-2 | Missing, mismatched, or sessionless Herdr evidence is represented as `Claimed` or `Unverifiable`; address lookup does not select it, while its bus name remains usable. | Property and unit test |
| FR-047-AC-3 | A Codex registration with a session ID differing from the session at an inherited pane is never assigned that pane's location. | Watcher test |
| FR-047-AC-4 | Legacy records without a session-bound Herdr proof decode with their human-readable labels but have no addressable verified location. | Wire compatibility test |
| FR-047-AC-5 | A refresh transitions a registration to `Verified` only after the exact-session Herdr join succeeds; failed refreshes do not create or renew proof, and a readable conflicting snapshot makes the location non-routable. | State transition test |

## Dependencies

- **Upstream**: Herdr's authoritative session, pane, tab, and workspace report;
  harness lifecycle hooks that provide stable session IDs.
- **Related**: [FR-003](./FR-003-address-matching-ignores-case.md),
  [FR-005](./FR-005-shown-address-rule.md).
