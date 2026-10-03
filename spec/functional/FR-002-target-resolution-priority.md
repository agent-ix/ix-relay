---
id: FR-002
title: "Location-only routing and automatic enrollment"
type: FR
---
# FR-002: Location-only routing and automatic enrollment

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- Send shall accept a full three-part address, `:workspace:tab`, `:tab`, or explicit `all`; relative inputs require the genuine caller's verified placement.
- Send shall refresh the recipient's live Herdr topology and enroll its genuine session before queueing, even if it has never registered.
- Names, alias tables, raw pane IDs and four-part targets shall not participate in public resolution.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-002-AC-1 | Both shortcuts resolve in the verified caller scope. | Test |
| FR-002-AC-2 | Cold local and remote sends enroll the actual recipient automatically. | Test |
| FR-002-AC-3 | Legacy targets do not enqueue any message. | Test |
