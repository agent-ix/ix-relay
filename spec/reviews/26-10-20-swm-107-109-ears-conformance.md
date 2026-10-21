---
id: SR-073
title: "ears-conformance review of SWM-107/108/109 spec PR #27"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay; PR #27; spec/functional/FR-056-repeatable-program-review-status.md, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md, spec/functional/FR-058-explicit-server-capacity.md, spec/stakeholder/StR-001-leads-exchange-messages-without-a-human-relay.md"
review_set: subset
---
## Summary

The changed FR and StR requirement statements were checked with Quire EARS grammar and semantic review; no EARS defects were found.

## Scope examined

- `FR-056` (examined, spec/functional/FR-056-repeatable-program-review-status.md): The system SHALL let a lead request and collect a fixed-form status report from selected registered lanes without treating a reply as an instruction.
- `FR-057` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): The system SHALL show a lane focus only after its lead explicitly reports it.
- `FR-058` (examined, spec/functional/FR-058-explicit-server-capacity.md): The system SHALL show server capacity only when an operator has explicitly recorded it.
- `FR-056-Behavior-1` (examined, spec/functional/FR-056-repeatable-program-review-status.md): The system SHALL assign one request identifier to each accepted status request and include that identifier in the request result and each recipient notice.
- `FR-056-Behavior-2` (examined, spec/functional/FR-056-repeatable-program-review-status.md): `agent-msg status-reply` SHALL let a selected registered lane submit the fixed status form manually. A separately tracked team-minder response path SHALL submit the same fixed form under the same request identifier.
- `FR-056-Behavior-3` (examined, spec/functional/FR-056-repeatable-program-review-status.md): The system SHALL retain only the first valid reply from each requested lane for one request identifier.
- `FR-056-Behavior-4` (examined, spec/functional/FR-056-repeatable-program-review-status.md): The system SHALL return the request's collected replies and nonresponders as structured JSON while the collection is partial or complete.
- `FR-057-Behavior-1` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): `agent-msg focus` SHALL store the supplied focus only for the caller's registered lane.
- `FR-057-Behavior-2` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): `agent-msg focus --clear` SHALL remove the caller's focus only.
- `FR-057-Behavior-3` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): `agent-msg who --focus` SHALL show every registered lane, its reported focus or an explicit unset state, and the time the focus was set.
- `FR-057-Behavior-4` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): `agent-msg who --focus --stale-after <age>` SHALL use the supplied age as its stale threshold.
- `FR-057-Behavior-5` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): The CLI SHALL mark a focus older than that threshold stale and keep the focus visible.
- `FR-057-Behavior-6` (examined, spec/functional/FR-057-lane-focus-is-explicit-and-visible.md): The system SHALL show a lane without an explicit focus as unset even when its open tickets or observed activity could suggest a focus.
- `FR-058-Behavior-1` (examined, spec/functional/FR-058-explicit-server-capacity.md): `agent-msg server-capacity set` SHALL accept RAM in GiB, store it as MiB, and display the stored amount in MiB.
- `FR-058-Behavior-2` (examined, spec/functional/FR-058-explicit-server-capacity.md): `agent-msg server-capacity clear` SHALL remove the capacity record for the named server only.
- `FR-058-Behavior-3` (examined, spec/functional/FR-058-explicit-server-capacity.md): `agent-msg who --servers` SHALL group registered lanes under their registered server and show that server's explicitly recorded capacity or an unset state.
- `FR-058-Behavior-4` (examined, spec/functional/FR-058-explicit-server-capacity.md): The system SHALL show no capacity values for a server with no operator-set capacity record.
- `StR-001-Stakeholder-Need` (context_only, spec/stakeholder/StR-001-leads-exchange-messages-without-a-human-relay.md): The message bus shall deliver each text message a team lead sends, addressed the way the operator sees the recipient on screen, to exactly the addressed lead or refuse it with a reason. Each message is therefore never delivered to a different lead, never delivered twice, and never breaks the recipient's current turn.
## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Verdict

PASS: no method-specific defect found.
