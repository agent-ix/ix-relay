---
id: FR-037
title: "Resolved escalations are retained for a bounded time"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-037: Resolved escalations are retained for a bounded time

## Description

When a resolved escalation's resolution is more than 30 days old, the store shall
remove the escalation, its index entry and its investigation-attempt counter
together.

## Behavior

- The store shall never remove a waiting or investigating escalation.
- The store shall record the resolution time when an escalation is resolved.
- Removal shall be one atomic step per escalation, so no index entry is left
  pointing at a removed escalation.
- `agent-msg escalations` shall read only the escalations it lists, never every
  escalation hash, when filtered to a status.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-037-AC-1 | Given an escalation resolved 31 days ago, after the retention pass `escalation show` for its id reports not found and `escalations` does not list it. | Test |
| FR-037-AC-2 | Given an escalation resolved 29 days ago, after the retention pass it is still listed. | Test |
| FR-037-AC-3 | Given a waiting escalation filed 400 days ago, after the retention pass it is still listed. | Test |
| FR-037-AC-4 | For every set of escalations and every retention pass, every id in the escalation index has its escalation hash, and every escalation hash has its index entry. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Related: [FR-024](./FR-024-escalation-status-transitions.md).
- **Gap on `main`** (AGE-2033): not built. FR-037-AC-1 to FR-037-AC-4 are
  expected to fail on `main`.
