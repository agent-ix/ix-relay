---
id: FR-002
title: "Location-only routing and automatic enrollment"
type: FR
---
# FR-002: Location-only routing and automatic enrollment

Updated for 0.3.2. This replaces the earlier alias and name-precedence contract.

## Behavior

- Send shall accept a full three-part address, `:workspace:tab`, `:tab`, or explicit `all`; relative inputs require the genuine caller's verified placement.
- Send shall resolve recipients from ix-relay's fresh verified directory before any Herdr fallback. An existing fresh binding shall require no sender-side Herdr or SSH access.
- Each machine's watcher shall verify its own local Herdr topology, enroll genuine root sessions, and publish a complete leased observation through Redis, including absence and ambiguity.
- A missing or expired remote binding shall request a destination-watcher refresh through Redis, without requiring outbound SSH or saved remote Herdr profiles at the sender.
- Publication shall reject superseded and replayed observation fences. Failed/incomplete topology reads shall not renew ownership. Lease deadlines shall use Redis time.
- Enqueue shall atomically verify that the resolving snapshot is unchanged and unexpired before writing any message. A replaced or expired snapshot shall refuse enqueue; a transport timeout shall not be automatically retried.
- Local Herdr shall remain an optional cold local recovery fallback. Durable queue identity shall remain attached to the genuine session across moves, renames, closure, and pane reuse.
- Names, alias tables, raw pane IDs and four-part targets shall not participate in public resolution.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-002-AC-1 | Both shortcuts resolve in the verified caller scope. | Test |
| FR-002-AC-2 | Cold local and remote sends enroll the actual recipient automatically; remote sends work without sender-side outbound SSH or Herdr profiles. | Test |
| FR-002-AC-3 | Legacy targets do not enqueue any message. | Test |
