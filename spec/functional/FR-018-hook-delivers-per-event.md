---
id: FR-018
title: "The hook delivers in the shape each event needs"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-018: The hook delivers in the shape each event needs

## Description

When a hook event for a lead pops one or more messages, the hook handler shall
print them as added context for `SessionStart`, `UserPromptSubmit` and
`PostToolUse`, and as a blocking decision whose reason is the messages for
`Stop`, followed by a count line when more messages remain queued.

## Behavior

- The hook handler shall pop with the budgeted pop of
  [FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md) with a count limit of
  50 and a byte budget of 6 KiB.
- When messages remain queued after the pop, the hook handler shall end the text
  with the line `(N more ix-relay messages pending)`, where N is the remaining count.
- The hook handler shall connect without sending any command (no client-info
  handshake), so connecting costs no round trip of its own.
- The hook handler shall read everything it needs to know about a session
  before deciding who it is (suppressed mark, cached name, unresolved mark) in
  one request, and shall read the registry for a lead's place by a point read of
  that lead's own registration; only when that registration has no place does it
  read the whole registry, for the place the pane registered under a previous
  name.
- The hook handler shall render each message as in
  [FR-016](./FR-016-render-neutralises-control-characters.md), one message per
  line group, oldest first.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-018-AC-1 | Given two queued messages and a `PostToolUse` event, stdout is `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":...}}` with both rendered messages in queue order. | Test |
| FR-018-AC-2 | Given two queued messages and a `Stop` event, stdout is `{"decision":"block","reason":...}` with both rendered messages. | Test |
| FR-018-AC-3 | Given more queued messages than fit the 6 KiB budget, the output ends with a pending-count line equal to the number still queued, and the next hook call delivers the next oldest. | Test |
| FR-018-AC-4 | Every hook output, for any inbox contents, has text of at most 6 KiB plus the rendered length of one message plus the pending-count line. | Test |
| FR-018-AC-5 | Given a fresh connection, the hook handler has sent no command until its first read. | Test |
| FR-018-AC-6 | Given a `UserPromptSubmit` for a lead in a herdr pane whose registration has a place, the requests before the pop are at most 3 for a derived name (one pipelined session read, one registration read, one commit) and at most 2 for an explicit name (registration read, commit), and none reads the whole registry. | Test |
| FR-018-AC-7 | Given a `PostToolUse` or `Stop` for a session whose name is cached, the requests before the pop are one pipelined session read; given an explicit name, none. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Delivered by AGE-2010 and AGE-2012. AGE-2025 tracks how the `Stop` output is
  surfaced by Claude Code.
- Constrained by [NFR-001](../non-functional/NFR-001-hook-time-budget.md) and
  [NFR-002](../non-functional/NFR-002-bounded-resources.md).
- Closed by AGE-2031: the budgeted pop counts each message by the bytes it
  prints as, not by its stored JSON (a line feed is 2 stored bytes and 5 printed,
  and the sender label adds bytes). The Lua pop cannot call Rust, so it adds a
  fixed allowance and the text's expansion to the stored length, an upper bound of
  `Message::rendered_len_max`; an address longer than 96 bytes is printed as the
  bus name (FR-005) so the label has a maximum. No field is
  added to the stored message.
