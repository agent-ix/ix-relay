---
id: SR-071
title: "code-review review of SWM-107/108/109 spec PR #27"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; PR #27; spec/functional/FR-056-repeatable-program-review-status.md, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md, spec/functional/FR-058-explicit-server-capacity.md, spec/stakeholder/StR-001-leads-exchange-messages-without-a-human-relay.md, spec/tests.md, spec/usecase/US-008-review-program-status-from-reported-lane-facts.md"
review_set: subset
---
## Summary

The change is Markdown-only. I checked duplication, source/spec claims, and matrix-to-test alignment; there is no implementation code in scope.

## Scope examined

- `FR-056` (examined, spec/functional/FR-056-repeatable-program-review-status.md): The system SHALL let a lead request and collect a fixed-form status report from selected registered lanes without treating a reply as an instruction.
- `FR-057` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): The system SHALL show a lane focus only after its lead explicitly reports it.
- `FR-058` (examined, spec/functional/FR-058-explicit-server-capacity.md): The system SHALL show server capacity only when an operator has explicitly recorded it.
- `FR-056-AC-1` (examined, spec/functional/FR-056-repeatable-program-review-status.md): Given a status request for `all` or selected registered lanes, `agent-msg status-request` returns one request identifier and names the selected responders; `all` excludes the requesting lane.
- `FR-056-AC-2` (examined, spec/functional/FR-056-repeatable-program-review-status.md): Every queued notice for one request names the same request identifier and gives the recipient the command needed to submit the fixed form.
- `FR-056-AC-3` (examined, spec/functional/FR-056-repeatable-program-review-status.md): A requested lead can submit one reply containing tickets, pull requests, blocker ticket IDs, lane state, spare coder slots, and owner decisions, associated with the request identifier.
- `FR-056-AC-4` (examined, spec/functional/FR-056-repeatable-program-review-status.md): Repeating a lane's reply for the same request identifier leaves the first stored reply unchanged and reports that it was already recorded.
- `FR-056-AC-5` (examined, spec/functional/FR-056-repeatable-program-review-status.md): `agent-msg status-replies <request-id>` returns valid JSON containing the request identifier, requested lanes, received replies, and every nonresponding lane when collection is partial; the nonresponder list is empty when all requested lanes replied.
- `FR-056-AC-6` (examined, spec/functional/FR-056-repeatable-program-review-status.md): A team-minder response to a selected lane's request uses the request identifier and the same fixed reply fields as a manual `status-reply`.
- `FR-057-AC-1` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): A lead's set command stores the supplied focus for its registered lane without changing another lane's value.
- `FR-057-AC-2` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): A lead's clear command removes its registered lane's focus without changing another lane's value.
- `FR-057-AC-3` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): `agent-msg who --focus` lists each registered lane with its explicit focus and set time, or says the focus is unset.
- `FR-057-AC-4` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): With a caller-supplied stale-age threshold, a focus older than that threshold is marked stale and remains in the listing.
- `FR-057-AC-5` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): A focus newer than the caller-supplied stale-age threshold is not marked stale.
- `FR-057-AC-6` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): A registered lane with no explicit focus remains unset even when its open tickets or observed activity could suggest a focus.
- `FR-058-AC-1` (examined, spec/functional/FR-058-explicit-server-capacity.md): An operator's capacity set command records the supplied core count, RAM amount in GiB, optional maximum agent count, and owner rules; the displayed RAM amount is the equivalent MiB value.
- `FR-058-AC-2` (examined, spec/functional/FR-058-explicit-server-capacity.md): An operator's capacity clear command removes only the selected server's capacity.
- `FR-058-AC-3` (examined, spec/functional/FR-058-explicit-server-capacity.md): `agent-msg who --servers` groups every registered lane under its registered server and displays the explicitly recorded capacity fields for servers that have them.
- `FR-058-AC-4` (examined, spec/functional/FR-058-explicit-server-capacity.md): A server with registered lanes but no capacity is shown with capacity unset.
- `FR-058-AC-5` (examined, spec/functional/FR-058-explicit-server-capacity.md): A server with recorded capacity but no registered lanes remains listed.
- `FR-058-AC-6` (examined, spec/functional/FR-058-explicit-server-capacity.md): A server without recorded capacity has no inferred capacity value in the output.
- `TC-056-01` (examined, spec/tests.md): Given `all` or an explicit set of registered lane addresses, a status request returns one request id and the selected responders; `all` excludes the requester. | Integration | FR-056-AC-1 | 🚧 not written
- `TC-056-02` (examined, spec/tests.md): Given a status request, each queued recipient notice carries the same request id and the manual reply command. | Integration | FR-056-AC-2 | 🚧 not written
- `TC-056-03` (examined, spec/tests.md): Given a requested lane and a request id, the CLI accepts the fixed reply form and associates all submitted fields with that lane and request. | Integration | FR-056-AC-3 | 🚧 not written
- `TC-056-04` (examined, spec/tests.md): Given a repeated reply delivery for one lane and request id, the first valid reply remains stored and the second submission reports the existing reply. | Integration | FR-056-AC-4 | 🚧 not written
- `TC-056-05` (examined, spec/tests.md): Given zero, some, or all requested lanes have replied, the JSON result lists received replies and exactly the outstanding nonresponders. | Integration | FR-056-AC-5 | 🚧 not written
- `TC-056-06` (examined, spec/tests.md): Given a team-minder response for a selected lane, the response is stored with the request id and the same fixed fields as a manual reply. | Integration | FR-056-AC-6 | 🚧 not written
- `TC-057-01` (examined, spec/tests.md): Given two registered lanes, setting one lane's focus changes only that lane's reported focus. | Integration | FR-057-AC-1 | 🚧 not written
- `TC-057-02` (examined, spec/tests.md): Given two registered lanes with focus values, clearing one lane's focus leaves the other value intact. | Integration | FR-057-AC-2 | 🚧 not written
- `TC-057-03` (examined, spec/tests.md): Given registered lanes with and without focus, `who --focus` shows each address, explicit value or unset state, and the set time for reported values. | Integration | FR-057-AC-3 | 🚧 not written
- `TC-057-04` (examined, spec/tests.md): Given a focus older than a supplied threshold, `who --focus` retains the value and marks it stale. | Integration | FR-057-AC-4 | 🚧 not written
- `TC-057-05` (examined, spec/tests.md): Given a focus newer than a supplied threshold, `who --focus` does not mark it stale. | Integration | FR-057-AC-5 | 🚧 not written
- `TC-057-06` (examined, spec/tests.md): Given a registered lane with open tickets but no focus record, `who --focus` reports the lane as unset. | Integration | FR-057-AC-6 | 🚧 not written
- `TC-058-01` (examined, spec/tests.md): Given an operator-set capacity, the stored cores, RAM in MiB, optional agent limit, and owner rules match the supplied values; the RAM value equals the input GiB converted to MiB. | Integration | FR-058-AC-1 | 🚧 not written
- `TC-058-02` (examined, spec/tests.md): Given capacities for two servers, clearing one leaves the other server's capacity unchanged. | Integration | FR-058-AC-2 | 🚧 not written
- `TC-058-03` (examined, spec/tests.md): Given registered lanes on multiple servers, `who --servers` groups each lane under its registered server and displays configured capacity. | Integration | FR-058-AC-3 | 🚧 not written
- `TC-058-04` (examined, spec/tests.md): Given a registered server with no capacity record, `who --servers` displays capacity as unset. | Integration | FR-058-AC-4 | 🚧 not written
- `TC-058-05` (examined, spec/tests.md): Given a server with recorded capacity and no registered lanes, `who --servers` still lists that server and its capacity. | Integration | FR-058-AC-5 | 🚧 not written
- `TC-058-06` (examined, spec/tests.md): Given a registered server with no operator-set capacity, output contains no derived cores, RAM, or agent limit. | Integration | FR-058-AC-6 | 🚧 not written
- `US-008-Story` (examined, spec/usecase/US-008-review-program-status-from-reported-lane-facts.md): **As an** operator coordinating work across team-lead lanes **I want** to collect each lane's stated work and blocker status, see its declared focus, and inspect explicitly recorded server capacity **So that** I can review progress and allocation using attributable reports and operator-entered facts.
- `StR-001-Stakeholder-Need` (context_only, spec/stakeholder/StR-001-leads-exchange-messages-without-a-human-relay.md): The message bus shall deliver each text message a team lead sends, addressed the way the operator sees the recipient on screen, to exactly the addressed lead or refuse it with a reason. Each message is therefore never delivered to a different lead, never delivered twice, and never breaks the recipient's current turn.
## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Verdict

PASS: no method-specific defect found.
