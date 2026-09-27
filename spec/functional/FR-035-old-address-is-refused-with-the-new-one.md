---
id: FR-035
title: "An old address is refused, naming the new one"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-007"
    type: "implements"
---
# FR-035: An old address is refused, naming the new one

## Description

If a sender resolves an address that a lead left by a rename or move recorded
in the event log, and no registered lead now matches that address, then the
resolver shall return an unknown-target error that names the lead's new address
from the most recent such event, and `agent-msg send` shall queue nothing.

## Behavior

- The resolver shall not resolve an old address to the lead that left it,
  whatever the time since the move (no grace window; the operator's decision of
  2026-09-25).
- If another lead now matches the old address, then the resolver shall resolve
  to that lead ([FR-003](./FR-003-address-matching-ignores-case.md)); an address
  names a place, not a lead.
- This requirement governs `server:workspace[:tab[:pane]]` addresses. Bus-name aliases
  kept after a `/rename` or a herdr agent rename
  ([FR-002](./FR-002-target-resolution-priority.md)) are a separate mechanism
  and are not changed by it.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-035-AC-1 | Given a lead moved from `hub:EA:code` to `hub:EA:plan` and no lead now at `hub:EA:code`, `agent-msg send hub:EA:code hi` exits non-zero, its error contains `hub:EA:plan`, and no inbox changes. | Test |
| FR-035-AC-2 | Given the same move one second ago, a send to `hub:EA:code` still does not reach the moved lead. | Test |
| FR-035-AC-3 | Given another lead has since registered at `hub:EA:code`, a send to `hub:EA:code` reaches that lead. | Test |
| FR-035-AC-4 | Given a lead moved twice, from `A` to `B` to `C`, resolving `A` names `C`. | Test |

## Dependencies

- **Upstream**: [US-007](../usecase/US-007-never-reach-the-wrong-lead-after-a-move.md)
- **Gap on `main`** (AGE-2033): not built. FR-035-AC-1 to FR-035-AC-4 are
  expected to fail on `main`.
