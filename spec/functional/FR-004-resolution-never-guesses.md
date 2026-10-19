---
id: FR-004
title: "Reject ambiguity without invented identities"
type: FR
---
# FR-004: Reject ambiguity without invented identities

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- More than one genuine live session in the selected tab shall reject delivery; no pane suffix, numbered name, nickname or focus guess shall select a recipient.
- An absent server profile, workspace, tab or genuine session association shall produce a diagnostic identifying the failed level.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-004-AC-1 | A duplicate tab queues nothing and reports ambiguity. | Test |
| FR-004-AC-2 | Unknown containers report live addresses; unverified sessions are not enrolled. | Test |
