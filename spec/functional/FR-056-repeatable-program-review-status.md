---
id: FR-056
title: "Repeatable program review status requests and replies"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "implements"
---
# FR-056: Repeatable program review status requests and replies

## Description

The system SHALL let a lead request and collect a fixed-form status report from
selected registered lanes without treating a reply as an instruction.

## Inputs

- A status request addressed to `all` or to one or more registered lane addresses.
- A reply containing the responding lane's tickets, pull requests, blocker ticket
  IDs, lane state (`active`, `idle`, or `blocked`), spare coder slots, and owner
  decisions.
- A status request identifier used to associate replies with one request.

## Outputs

- The status request identifier and the lanes selected for that request.
- A structured JSON snapshot of collected replies and lanes that have not replied.

## Behavior

- The system SHALL assign one request identifier to each accepted status request
  and include that identifier in the request result and each recipient notice.
- `agent-msg status-reply` SHALL let a selected registered lane submit the fixed
  status form manually. A separately tracked team-minder response path SHALL
  submit the same fixed form under the same request identifier.
- The system SHALL retain only the first valid reply from each requested lane for
  one request identifier.
- The system SHALL return the request's collected replies and nonresponders as
  structured JSON while the collection is partial or complete.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-056-AC-1 | Given a status request for `all` or selected registered lanes, `agent-msg status-request` returns one request identifier and names the selected responders; `all` excludes the requesting lane. | Test |
| FR-056-AC-2 | Every queued notice for one request names the same request identifier and gives the recipient the command needed to submit the fixed form. | Test |
| FR-056-AC-3 | A requested lead can submit one reply containing tickets, pull requests, blocker ticket IDs, lane state, spare coder slots, and owner decisions, associated with the request identifier. | Test |
| FR-056-AC-4 | Repeating a lane's reply for the same request identifier leaves the first stored reply unchanged and reports that it was already recorded. | Test |
| FR-056-AC-5 | `agent-msg status-replies <request-id>` returns valid JSON containing the request identifier, requested lanes, received replies, and every nonresponding lane when collection is partial; the nonresponder list is empty when all requested lanes replied. | Test |
| FR-056-AC-6 | A team-minder response to a selected lane's request uses the request identifier and the same fixed reply fields as a manual `status-reply`. | Test |

## Dependencies

- **Upstream**: [US-008](../usecase/US-008-review-program-status-from-reported-lane-facts.md)
- The team-minder response path is tracked separately; the CLI reply path is
  independently usable. The team-minder integration is not claimed as
  implemented by this requirement's current test evidence.
