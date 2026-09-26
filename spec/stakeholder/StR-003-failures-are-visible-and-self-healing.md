---
id: StR-003
title: "Relay failures are visible, and the relay recovers without a restart"
type: StR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "satisfied_by"
  - target: "ix://agent-ix/ix-relay/US-007"
    type: "satisfied_by"
---
# StR-003: Relay failures are visible, and the relay recovers without a restart

## Stakeholder Need

The relay shall make every failure that stops a message reaching its recipient
visible to the operator or to the sender, and shall recover from a transient
outage without anyone restarting a process.

## Rationale

A relay that drops or stalls mail without anyone knowing is worse than one that
errors (the operator, 2026-09-25, AGE-2033). Hook failures are deliberately
silent to protect the agent's turn, the watcher may not be installed or may have
exited, and a lead that moves or is renamed can leave an address pointing at the
wrong place. Each of these is invisible today.

## Validation Criteria

| ID | Criteria | Validation |
|----|----------|------------|
| StR-003-VC-1 | With the watcher on one machine stopped, `agent-msg doctor` on any machine names that machine as having no live watcher, and a send to a lead there says it is waiting on a watcher. | Demonstration |
| StR-003-VC-2 | With the SSH tunnel down for a minute, the hook failures of that minute are reported by `agent-msg doctor` after the tunnel returns, and the watcher resumes delivery without being restarted. | Demonstration |
| StR-003-VC-3 | After a lead's tab is renamed, a send to its old address is refused and the error names its new address. | Demonstration |

## Stakeholders

The human operator, who owns the machines and currently learns of relay failures
only by their effects, and the team leads, who send messages without knowing
whether they arrive.

## Traceability

Satisfied by [US-006](../usecase/US-006-see-when-the-relay-is-failing.md) and
[US-007](../usecase/US-007-never-reach-the-wrong-lead-after-a-move.md). Tracked
as AGE-2033.
