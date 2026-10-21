---
id: FR-058
title: "Record explicit server capacity and group its registered lanes"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-005"
    type: "implements"
---
# FR-058: Record explicit server capacity and group its registered lanes

## Description

The operator shall record or clear explicit build-server capacity. `agent-msg who --servers` shall group registered lanes by server and display recorded capacity, or identify capacity as unset.

## Behavior

- `server-capacity set` records cores, RAM, optional maximum agents, and optional owner rules for a server; capacity is operator-entered and is not inferred.
- `server-capacity clear` removes the server's recorded capacity.
- `who --servers` groups registered lane names by server and shows capacity values or `capacity unset`.

## Dependencies

- **Upstream**: [US-005](../usecase/US-005-configure-a-machine-once.md)

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-058-AC-1 | Given valid capacity values, `server-capacity set` records them and reports the server; `clear` removes the record. | Test |
| FR-058-AC-2 | Given registered lanes on multiple servers, `who --servers` groups their names by server and displays the explicit capacity or unset state. | Test |
