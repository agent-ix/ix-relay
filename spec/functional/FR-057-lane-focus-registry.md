---
id: FR-057
title: "Record and display each lead's declared lane focus"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "implements"
---
# FR-057: Record and display each lead's declared lane focus

## Description

A lead shall set or clear its declared project, layer, or ticket focus. `agent-msg who --focus` shall display registered lanes with their focus and set time, marking old focus as stale while keeping it visible.

## Behavior

- `agent-msg focus <text>` sets the current lead's focus; `agent-msg focus --clear` clears it.
- `agent-msg who --focus` prints each registered lane's focus and when it was set, or says that focus is unset.
- Focus older than `--stale-after` is marked `STALE`; the default age is seven days. A stale value remains visible.

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md)

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-057-AC-1 | Given a registered lead, `focus <text>` stores that text and `focus --clear` removes it. | Test |
| FR-057-AC-2 | Given registered lanes with set and unset focus, `who --focus` displays each lane, its focus or unset state, and the set time. | Test |
| FR-057-AC-3 | Given a focus older than the configured stale age, `who --focus --stale-after` marks it `STALE` and still prints the focus. | Test |
