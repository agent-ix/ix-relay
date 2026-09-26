---
id: StR-001
title: "Leads exchange messages without a human relay, and never to the wrong lead"
type: StR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "satisfied_by"
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "satisfied_by"
  - target: "ix://agent-ix/ix-relay/US-003"
    type: "satisfied_by"
---
# StR-001: Leads exchange messages without a human relay, and never to the wrong lead

## Stakeholder Need

The message bus shall deliver each text message a team lead sends, addressed the
way the operator sees the recipient on screen, to exactly the addressed lead or
refuse it with a reason.

Each message is therefore never delivered to a different lead, never delivered
twice, and never breaks the recipient's current turn.

## Rationale

Before ix-relay the human operator copied text between agent sessions by hand. A
bus that misdelivers is worse than no bus: an instruction meant for one lead and
acted on by another does real damage, and a lead whose turn is broken by a relay
failure loses work. The need is therefore correctness first and convenience
second.

## Validation Criteria

| ID | Criteria | Validation |
|----|----------|------------|
| StR-001-VC-1 | A lead on one machine sends to a lead on another by its on-screen `server:workspace[:tab]` address and the recipient sees the message, signed with the sender, on its next turn or when idle. | Demonstration |
| StR-001-VC-2 | A send whose target matches two leads is refused, lists both, and neither receives it. | Demonstration |
| StR-001-VC-3 | With Redis unreachable, a lead's turn proceeds with no error surfaced by the hook. | Demonstration |

## Stakeholders

Team lead agents are the senders and recipients. The human operator is accountable
for the swarm and is the party harmed by misdelivery.

## Context and Assumptions

Leads run inside herdr panes on three machines sharing one Redis instance through
SSH tunnels. Every participant that can reach Redis is trusted to be a lead, but
the text it sends is not trusted to be well-formed.

## Traceability

Satisfied by [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md),
[US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md) and
[US-003](../usecase/US-003-be-reachable-without-configuring-a-name.md).
