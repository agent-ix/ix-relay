---
id: FR-058
title: "Explicit server capacity and lane grouping"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "implements"
---
# FR-058: Explicit server capacity and lane grouping

## Description

The system SHALL show server capacity only when an operator has explicitly
recorded it.

## Inputs

- An operator-entered server capacity containing cores, RAM in GiB, an optional
  maximum agent count, and an optional list of owner rules (empty when omitted).
- A request to clear the recorded capacity for one server.

## Outputs

- Server capacity and its registered lanes, or an explicit indication that
  capacity has not been recorded.

## Behavior

- The explicit local invocation of `agent-msg server-capacity set` or
  `agent-msg server-capacity clear` is the operator action for this feature;
  the commands SHALL NOT claim or imply a separate role-based authorization
  check.
- `agent-msg server-capacity set` SHALL accept RAM in GiB, store it as MiB, and
  display the stored amount in MiB. `--owner-rule` MAY be supplied zero or more
  times; omission records an empty list.
- `agent-msg server-capacity clear` SHALL remove the capacity record for the
  named server only.
- `agent-msg who --servers` SHALL group registered lanes under their registered
  server and show that server's explicitly recorded capacity or an unset state.
- The system SHALL show no capacity values for a server with no operator-set
  capacity record.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-058-AC-1 | The operator's capacity-set invocation records the supplied cores and RAM in GiB, the optional maximum agent count, and zero or more `--owner-rule` values; omitted owner rules are stored as an empty list and displayed RAM equals the GiB value converted to MiB. | Test |
| FR-058-AC-2 | An operator's capacity clear command removes only the selected server's capacity. | Test |
| FR-058-AC-3 | `agent-msg who --servers` groups every registered lane under its registered server and displays explicitly recorded capacity fields, including RAM in MiB, for servers that have them. | Test |
| FR-058-AC-4 | A server with registered lanes but no capacity is shown with capacity unset. | Test |
| FR-058-AC-5 | A server with recorded capacity but no registered lanes remains listed. | Test |
| FR-058-AC-6 | A server without recorded capacity has no inferred capacity value in the output. | Test |

## Dependencies

- **Upstream**: [US-008](../usecase/US-008-review-program-status-from-reported-lane-facts.md)
- Server capacity is entered by the operator; the host's measured hardware is not used as a substitute.
