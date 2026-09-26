---
id: StR-002
title: "The operator decides escalated questions once, and the lead hears the decision"
type: StR
relationships:
  - target: "ix://agent-ix/ix-relay/US-004"
    type: "satisfied_by"
  - target: "ix://agent-ix/ix-relay/US-005"
    type: "satisfied_by"
---
# StR-002: The operator decides escalated questions once, and the lead hears the decision

## Stakeholder Need

The escalation inbox shall hold each decision a lead cannot make itself as one
durable escalation, carrying the lead's context, options and recommendation, until
the operator records exactly one final decision that reaches the lead.

Separately, every ix-relay tool on a machine reads one machine-level settings file
the same way, so the operator configures the bus and the machine name once.

## Rationale

Escalations that are lost, resolved twice with conflicting decisions, or that
carry text able to act on the operator's terminal undermine the whole point of
asking. Machine configuration that each tool reads differently hides leads from
the watcher.

## Validation Criteria

| ID | Criteria | Validation |
|----|----------|------------|
| StR-002-VC-1 | A lead files an escalation; it is listed as waiting; the operator resolves it; the lead receives the decision over the bus; a second resolve is refused. | Demonstration |
| StR-002-VC-2 | A config file on a machine changes the Redis URL for both `agent-msg` and `relay-watcher`, and a flag still overrides it. | Demonstration |

## Stakeholders

The human operator (decision maker, machine owner) and the team leads that file
escalations.

## Traceability

Satisfied by [US-004](../usecase/US-004-escalate-a-decision.md) and
[US-005](../usecase/US-005-configure-a-machine-once.md).
