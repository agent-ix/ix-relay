---
id: SR-6129
title: "Integrity review of the FR-044 and FR-054 changes in ix-relay PR 30"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay; PR #30 at f2cbcc8; spec/functional/{FR-014,FR-018,FR-044,FR-046,FR-054}*.md, spec/spec.md, spec/usecase/US-002-receive-messages-without-losing-the-turn.md"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-054"
    type: "reviews"
---

# SR-6129: Integrity review of the socket-only requirement changes

## Summary

Ticket: SWM-121. PR agent-ix/ix-relay#30 at `f2cbcc8e48953c5d7d5c41b515534c1642b8773b`. The changed statements were checked for consistency with each other.

## Verdict

**CONDITIONAL**: two low findings.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | A retained FR-044 bullet says the lead's own hook delivers a redelivered batch while the socket copy is held, after a lost status file. The new rule says a hook of a session with a socket pops nothing, so the stated known limit cannot happen as written; the real limit is that the watcher posts it again. | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md:174 |
| FND-002 | low | FR-054's Background still presents `using herdr` as what doctor shows for a Claude lead with no record; under this change such a lead waits and nothing goes through Herdr. | spec/functional/FR-054-socket-delivery-is-for-claude-leads.md:19-25 |
