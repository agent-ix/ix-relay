---
id: FR-019
title: "Subagents and investigation forks never use the bus"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-019: Subagents and investigation forks never use the bus

## Description

If the hook input carries an `agent_id`, then the hook handler shall make no
Redis call, register nothing, pop nothing and print nothing.

## Behavior

- If the hook input carries an `agent_id` (a subagent of a lead, in Claude Code or
  Codex), then the hook handler shall exit 0 before connecting to Redis.
- If `SessionStart` reports `source` `fork`, or the pane's tab label or herdr agent
  name starts with `esc-`, then the hook handler shall record the session as
  suppressed and shall neither register it nor pop any inbox for it.
- While a session is recorded as suppressed, every later hook call for that
  session shall neither register nor pop.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-019-AC-1 | Given hook input with `agent_id` set and a lead's inbox holding messages, the hook exits 0, prints nothing, and the inbox and registry are unchanged. | Test |
| FR-019-AC-2 | Given hook input with `agent_id` set and an unreachable Redis URL, the hook exits 0 without attempting a connection to that URL. | Test |
| FR-019-AC-3 | Given a `SessionStart` with `source` `fork`, the session registers nothing, and a later `Stop` for the same session id pops nothing. | Test |
| FR-019-AC-4 | Given a pane in a tab labelled `esc-e7-1`, no hook event registers the session or pops an inbox. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Delivered by AGE-2010, AGE-2012 and AGE-2018.
