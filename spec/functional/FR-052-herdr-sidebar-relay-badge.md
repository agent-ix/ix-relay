---
id: FR-052
title: "Optional Herdr sidebar relay badge"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "traces_to"
  - target: "ix://agent-ix/ix-relay/FR-049"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-051"
    type: "depends_on"
---
# FR-052: Optional Herdr sidebar relay badge

## Description

Herdr users MAY opt in to a compact ix-relay sidebar badge that combines local
relay state with a read-only pending-message count.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-052-AC-1 | When enabled, the badge shows connected/degraded/unavailable/unknown state and the pending count for the exactly verified focused lead. It is absent when the user has not enabled it. | Unit and terminal tests |
| FR-052-AC-2 | The count reads inbox length without popping, acknowledging, or moving messages in-flight. | Redis integration tests |
| FR-052-AC-3 | A missing or unverified lead, unreachable Redis, or failed read is shown as unknown/unavailable, never as zero pending. | Unit tests |
| FR-052-AC-4 | Badge work uses bounded read-only commands and the same Herdr plugin for every supported coding agent. | Unit and manifest tests |
