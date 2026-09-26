---
id: FR-023
title: "Escalation, ticket and session identifiers are validated"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-004"
    type: "implements"
---
# FR-023: Escalation, ticket and session identifiers are validated

## Description

The store shall issue escalation ids of the form `e<N>`, with N drawn from a
strictly increasing counter starting at 1, and shall refuse any escalation id that
does not match `^e[0-9]{1,20}$` before building a Redis key from it.

## Behavior

- The store shall never issue the same escalation id twice on one bus.
- If an escalation command (`investigate`, `resolve`, `escalation show`,
  `escalation post-linear`) is given an id that does not match the id grammar,
  then the command shall exit non-zero with an invalid-id error before making any
  Redis call for it.
- If the escalation index holds an entry that does not match the id grammar,
  then `escalations` shall report it as rejected without looking it up.
- If a ticket id does not match an uppercase ASCII prefix, `-`, and ASCII digits,
  then `escalate` shall refuse it before writing anything.
- If the lead's session id is empty, longer than 64 characters, or contains
  anything other than ASCII letters, digits and `-`, then `escalate` shall refuse
  it before writing anything.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-023-AC-1 | Every escalation id issued on one bus is unique, and N escalations filed on a fresh bus receive exactly `e1` to `eN`. | Test |
| FR-023-AC-2 | Every id not matching `^e[0-9]{1,20}$` (including `E7`, `e`, `seq`, `index`, `e7:attempts`, `e7 `, `*`, the empty string and `e` followed by 21 digits) makes `escalation show` exit non-zero without reading any key. | Test |
| FR-023-AC-3 | Every ticket id matching `^[A-Z]+-[0-9]+$` is accepted, and every ticket id containing a character outside `[A-Z0-9-]` is refused. | Test |
| FR-023-AC-4 | Given session ids `sid;rm -rf /`, `$(whoami)`, `../../etc/passwd` and 65 `a` characters, `escalate` is refused and no escalation key is written. | Test |

## Dependencies

- **Upstream**: [US-004](../usecase/US-004-escalate-a-decision.md)
- Delivered in part by AGE-2014.
- Gap closed by AGE-2026: the `EscalationId` newtype is parsed at the CLI and
  is the only type the store builds an escalation key from.
