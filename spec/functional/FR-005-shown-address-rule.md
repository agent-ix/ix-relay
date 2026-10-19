---
id: FR-005
title: "Canonical public identity on every surface"
type: FR
---
# FR-005: Canonical public identity on every surface

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- A verified lead's public identity shall always be `server:workspace:tab`, including a workspace with one tab.
- Unverified or ambiguous registry entries shall be marked unverified instead of displaying a made-up alias or opaque queue ID as a public name.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-005-AC-1 | whoami, who, find, send, log and delivered signatures use canonical locations. | Test |
| FR-005-AC-2 | Ambiguous and unplaced records cannot provide a signed public identity. | Test |
