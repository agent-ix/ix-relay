---
id: FR-051
title: "Formatted message history and verified reply"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-009"
    type: "traces_to"
  - target: "ix://agent-ix/ix-relay/FR-047"
    type: "depends_on"
---
# FR-051: Formatted message history and verified reply

## Description

The Herdr plugin SHALL make recent ix-relay messages readable and support
non-destructive pending-message preview and replies from the verified focused
lead. Message content is untrusted data.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-051-AC-1 | A bounded stable read-only history response presents sender, sent time, target/broadcast, body, and redelivery state as separate fields. It does not change the existing agent-facing `[ix-relay] from ...` line. | Unit and integration tests |
| FR-051-AC-2 | `agent-msg inbox --peek` reads at most the requested bounded number of pending messages without removing or acknowledging them; opening and refreshing the popup leaves inbox length and in-flight state unchanged. | Redis integration tests |
| FR-051-AC-3 | Direct, broadcast, redelivered, multiline, long, and control-character bodies render as data with sender and metadata kept distinct. Bodies are neutralized and bounded before display. | Unit and terminal tests |
| FR-051-AC-4 | Reply targets the canonical sender address and signs as the verified focused Herdr session. If that exact identity cannot be proven, the UI disables reply and explains why. | Unit and integration tests |
| FR-051-AC-5 | The same popup and message data are available to Claude, Codex, OpenCode, and other coding agents in Herdr, without starting an agent turn. | Manifest test and demonstration |

## Dependencies

- FR-047 supplies exact focused-session verification.
- AGE-2021 supplies the non-destructive `inbox --peek` read.
