---
id: SR-078
title: "failure-domain review of SWM-107/108/109 spec PR #27"
type: SpecReview
analysis: failure-domain
scope: "agent-ix/ix-relay; PR #27; spec/functional/FR-056-repeatable-program-review-status.md, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md, spec/functional/FR-058-explicit-server-capacity.md, spec/stakeholder/StR-001-leads-exchange-messages-without-a-human-relay.md"
review_set: subset
---
## Summary

The changed behavior was checked for trust boundaries, identity keys, and failure cases. No separate failure-domain finding was added beyond the reply-schema and untestable-guarantee findings in the base and integrity reviews.

## Scope examined

- `FR-056` (examined, spec/functional/FR-056-repeatable-program-review-status.md): The system SHALL let a lead request and collect a fixed-form status report from selected registered lanes without treating a reply as an instruction.
- `FR-056-AC-3` (examined, spec/functional/FR-056-repeatable-program-review-status.md): A requested lead can submit one reply containing tickets, pull requests, blocker ticket IDs, lane state, spare coder slots, and owner decisions, associated with the request identifier.
- `FR-056-AC-4` (examined, spec/functional/FR-056-repeatable-program-review-status.md): Repeating a lane's reply for the same request identifier leaves the first stored reply unchanged and reports that it was already recorded.
- `FR-056-AC-6` (examined, spec/functional/FR-056-repeatable-program-review-status.md): A team-minder response to a selected lane's request uses the request identifier and the same fixed reply fields as a manual `status-reply`.
- `FR-057` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): The system SHALL show a lane focus only after its lead explicitly reports it.
- `FR-057-AC-4` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): With a caller-supplied stale-age threshold, a focus older than that threshold is marked stale and remains in the listing.
- `FR-057-AC-5` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): A focus newer than the caller-supplied stale-age threshold is not marked stale.
- `FR-058` (examined, spec/functional/FR-058-explicit-server-capacity.md): The system SHALL show server capacity only when an operator has explicitly recorded it.
- `FR-058-AC-1` (examined, spec/functional/FR-058-explicit-server-capacity.md): An operator's capacity set command records the supplied core count, RAM amount in GiB, optional maximum agent count, and owner rules; the displayed RAM amount is the equivalent MiB value.
- `FR-058-AC-6` (examined, spec/functional/FR-058-explicit-server-capacity.md): A server without recorded capacity has no inferred capacity value in the output.
- `StR-001-Stakeholder-Need` (context_only, spec/stakeholder/StR-001-leads-exchange-messages-without-a-human-relay.md): The message bus shall deliver each text message a team lead sends, addressed the way the operator sees the recipient on screen, to exactly the addressed lead or refuse it with a reason. Each message is therefore never delivered to a different lead, never delivered twice, and never breaks the recipient's current turn.
## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Verdict

PASS: no method-specific defect found.
