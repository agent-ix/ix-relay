---
id: FR-021
title: "Session queue identity and mutable public location"
type: FR
---
# FR-021: Session queue identity and mutable public location

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- Lead queue identities shall be deterministic, collision-free encodings of normalized server and genuine harness session, independent of mutable labels.
- Explicit metadata, Claude rename titles, Herdr nicknames and registry hits shall never override the public three-part location.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-021-AC-1 | Location moves preserve the session queue identity. | Test |
| FR-021-AC-2 | A newer stale registry record cannot win over genuine session proof. | Test |
