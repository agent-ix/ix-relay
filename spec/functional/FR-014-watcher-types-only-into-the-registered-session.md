---
id: FR-014
title: "The watcher types only into the registered, idle session"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-014: The watcher types only into the registered, idle session

## Description

In herdr delivery mode, the watcher shall type queued messages into a lead's pane only if herdr reports
that the pane's live session id equals the session id the lead registered, the
lead is idle or done, and herdr's state-change sequence number is unchanged
between two reads taken immediately before the pop. Socket delivery and its
fallback follow [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md).

## Behavior

- If either the registered or the live session id is absent, or they differ,
  then the watcher shall neither pop nor type.
- If a registration is stale, then the watcher shall log it on the first tick of
  each stale episode only.
- While the lead is working, or its status is unknown, the watcher shall not pop
  or type; the lead's own hook delivers.
- While the lead is blocked on an approval or question dialog, the watcher shall
  not pop or type, and shall notify the operator once per blocked episode.
- If the pane has no agent, or herdr's status call fails, then the watcher shall
  not pop or type.
- If the state-change sequence number changed between the two reads, then the
  watcher shall not pop or type.
- The watcher shall skip leads with an empty inbox without calling herdr.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-014-AC-1 | In herdr delivery mode, for every combination of registered session id, live session id, status, both sequence numbers and herdr call outcomes, the watcher calls herdr's prompt only when both session ids are present and equal, the status is idle or done, and the two sequence numbers are equal. | Test |
| FR-014-AC-2 | Given a mismatched session id on two consecutive ticks, the stale registration is logged on the first tick only and the inbox is unchanged after both. | Test |
| FR-014-AC-3 | Given a lead blocked on a dialog for three ticks, exactly one notification is raised and the inbox is unchanged. | Test |
| FR-014-AC-4 | Given a lead with an empty inbox, a tick makes no herdr status call for it. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Delivered by AGE-2013.
- Constrained by [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md).
