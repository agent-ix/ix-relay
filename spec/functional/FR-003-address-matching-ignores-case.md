---
id: FR-003
title: "Exact case-insensitive location matching"
type: FR
---
# FR-003: Exact case-insensitive location matching

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- Full addresses shall match server, workspace and tab case-insensitively, without omitting any level.
- Queues shall be scoped by machine and genuine session, even when pane and session strings repeat on other machines.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-003-AC-1 | Uppercase input resolves the canonical Herdr location. | Test |
| FR-003-AC-2 | Repeated pane and session identifiers on two servers cannot cross inboxes. | Test |
