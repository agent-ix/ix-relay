---
id: FR-001
title: "Exact tab address grammar"
type: FR
---
# FR-001: Exact tab address grammar

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- Public addresses shall contain exactly three nonempty levels: `server:workspace:tab`.
- Labels shall be preserved, including spaces; control characters, embedded separators and addresses over 96 bytes shall be rejected without slugification.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-001-AC-1 | Malformed, two-part, bare-name and pane-qualified public inputs are rejected. | Test |
| FR-001-AC-2 | Case and whitespace do not create substitute aliases. | Test |
