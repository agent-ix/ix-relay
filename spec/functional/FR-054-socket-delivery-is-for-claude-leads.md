---
id: FR-054
title: "The socket is tried only for Claude Code leads"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-054: The socket is tried only for Claude Code leads

## Description

Where `[watcher] delivery` is `"socket"` ([FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md)),
`relay-watcher` shall try Claude Code's inbox socket only for a lead whose registration
`kind` is `claude`, and shall deliver every other lead's mail through herdr without
recording a note about the socket and without `agent-msg doctor` warning about it.

## Background

The inbox socket is a Claude Code feature, found through Claude Code's own
`~/.claude/sessions/<pid>.json` record. A Codex or opencode session has no such record
by design, so trying the socket for it can only fail, and the failure was recorded as
`no Claude Code session record names this session` and shown by `doctor` as a lead
using herdr (a warning). That is normal behaviour reported as a fault, and it hid the
warning that matters: a Claude Code lead with no record, which is a real problem.

## Behavior

- The watcher shall decide whether to try the socket from the registration's `kind`
  alone: `claude` tries it; any other value, including an unknown or empty one, does not.
- For a lead that does not try the socket, the watcher shall pass each batch straight to
  herdr exactly as in herdr mode, and shall not read Claude Code's registry for it.
- The watcher shall create no status-file lead entry or note for a lead that does not try
  the socket, and shall not count its deliveries as socket deliveries.
- If a lead whose `kind` is `claude` has a session id and no Claude Code session record
  names it, then the watcher shall go on recording the note and delivering through herdr,
  as before.
- `agent-msg doctor` shall not warn about, list as using herdr, or count a lead that does
  not try the socket, and shall keep warning about a `claude` lead using herdr with the
  reason the watcher recorded.
- A lead whose registration changes `kind` is judged by the kind of the registration the
  tick read.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-054-AC-1 | Given delivery = `"socket"` and a `codex` lead and an `opencode` lead each with a session id and no Claude Code record, each batch is typed through herdr, the status file holds no note and no lead entry for either, and Claude Code's registry is not read for them. | Test |
| FR-054-AC-2 | Given the same and a `claude` lead with a session id and no record, the batch goes through herdr and the status file holds the `no Claude Code session record names this session` note. | Test |
| FR-054-AC-3 | Given the status files of AC-1, `agent-msg doctor` has no warning, no using-herdr count and no note for the two leads; given AC-2's, it warns for the `claude` lead only. | Test |
| FR-054-AC-4 | Given a lead with kind `claude` that has a live record, delivery is as in FR-044; given an empty or unknown kind it is typed through herdr with no note. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Narrows [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md) and the
  socket check of [FR-029](./FR-029-doctor-reports-relay-health.md). The registration's
  `kind` is the harness name `agent-msg` registers (`claude`, `codex`, ...).
