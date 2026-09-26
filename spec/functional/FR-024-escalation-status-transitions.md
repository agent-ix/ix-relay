---
id: FR-024
title: "Escalation status only moves forward; resolved is terminal"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-004"
    type: "implements"
---
# FR-024: Escalation status only moves forward; resolved is terminal

## Description

The store shall allow exactly the escalation status transitions waiting →
investigating, investigating → investigating, waiting → resolved and
investigating → resolved, each as one atomic check-and-set, and shall refuse
every transition out of resolved with an already-resolved error. A resolve whose
decision is empty or only whitespace is refused before any of that (see the
order of checks below).

## Behavior

- When an escalation is filed, the store shall give it status `waiting` and an
  empty decision.
- If the decision of a resolve is empty or contains only whitespace (Unicode
  White_Space, as `str::trim` defines it), then the store shall refuse the
  resolve with an empty-decision error and shall change nothing. The store shall
  check the decision first, before whether the escalation exists and before its
  status, so an empty decision on an unknown or a resolved escalation is an
  empty-decision error, not a not-found or already-resolved error.
- When an escalation is resolved, the store shall record the decision in the same
  atomic step as the status change.
- If two resolves of one escalation run concurrently, then the store shall let
  exactly one succeed and return an already-resolved error to the other, and the
  recorded decision shall be the successful one's.
- If the escalation does not exist, then the store shall return a not-found error
  and create nothing.
- When an escalation is resolved, `resolve` shall send the decision to the filing
  lead over the bus; if posting to Linear fails, then the escalation shall stay
  resolved and `resolve` shall print the retry command.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-024-AC-1 | For every sequence of investigate and resolve operations on one escalation, the observed status sequence never leaves `resolved` once reached and never returns to `waiting`. | Test |
| FR-024-AC-2 | For every sequence of operations, the decision field is non-empty if and only if the status is `resolved`, and it never changes after the first resolve. | Test |
| FR-024-AC-3 | Given two concurrent resolves of one waiting escalation with different decisions, exactly one returns success, the other returns already-resolved, and the stored decision equals the successful one's. | Test |
| FR-024-AC-4 | Given an unknown escalation id (and, for resolve, a decision that is not empty or only whitespace), investigate and resolve return not-found and no key is created. | Test |
| FR-024-AC-5 | Given Linear posting fails, `resolve` leaves the escalation resolved, queues the decision for the filing lead, and prints the `escalation post-linear` command. | Test |
| FR-024-AC-6 | Given a resolve whose decision is `""` or only Unicode whitespace, on a waiting, an investigating, a resolved or an unknown escalation, the store returns an empty-decision error and no key changes. | Test |

## Dependencies

- **Upstream**: [US-004](../usecase/US-004-escalate-a-decision.md)
- Delivered by AGE-2014.
