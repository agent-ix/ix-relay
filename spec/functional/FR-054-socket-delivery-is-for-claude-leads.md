---
id: FR-054
title: "A session's inbox socket is used whatever kind its lead registered"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-054: A session's inbox socket is used whatever kind its lead registered

## Description

Where `[watcher] delivery` is `"socket"`, the default ([FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md)),
`relay-watcher` shall use the inbox socket of every session that Claude Code's registry
shows to have one, whatever the lead's registration `kind` says, and shall deliver the
mail of a lead of another harness whose session no record names through herdr, without a
note about the socket and without `agent-msg doctor` warning about it.

## Background

The inbox socket belongs to the session and is found through Claude Code's own
`~/.claude/sessions/<pid>.json` record. The registration `kind` is a label: herdr reports
it from its `agent` member and it reads `unknown` when herdr omits it, so it cannot prove
that a session has no socket. A Codex or opencode session has no such record by design,
which is normal and is not reported. A `claude` lead with no record is a real problem:
it cannot be shown to have no socket, so its mail waits.

## Behavior

- For every lead with a session id the watcher shall look the session up in Claude Code's
  registry before it decides the route.
- If a live record shows a socket that can take mail, then the watcher shall post to it,
  for any `kind`.
- If a record shows a socket that cannot take mail now (parked, several processes, an
  unsupported version, an unusable path, a process that cannot be verified, an unreadable
  registry or an unbound reply socket), then the watcher shall make the lead wait, for any
  `kind`.
- The watcher shall type into a lead whose `kind` is not `claude` only when no record
  names its session, when its own live record shows it has no socket
  ([FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md)), or when
  every record naming it is stale and none names a socket.
- The watcher shall create no status-file lead entry or note for a lead it types into.
- If a lead whose `kind` is `claude` has a session id and no Claude Code session record
  names it, then the watcher shall record the note and leave its mail queued: it cannot
  tell that the session has no socket, so it types nothing.
- A lead with no session id, or one that is not a plain id, shall wait when its `kind` is
  `claude` and be typed into otherwise.
- If sockets cannot be used on the host, then the watcher shall make every `claude` lead
  wait, and every lead whose session a record names with a socket.
- `agent-msg doctor` shall not warn about, list as waiting, or count a lead that is typed
  into, and shall keep warning about a waiting lead with the reason the watcher recorded.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-054-AC-1 | Given delivery = `"socket"` and a `codex` lead and an `opencode` lead each with a session id and no Claude Code record, each batch is typed through herdr and the status file holds no note and no lead entry for either; a lead that then has no record loses the entry it had. | Test |
| FR-054-AC-2 | Given the same and a `claude` lead with a session id and no record, nothing is typed or posted, its mail stays queued and the status file holds the `no Claude Code session record names this session` note. | Test |
| FR-054-AC-3 | Given the status files of AC-1, `agent-msg doctor` has no warning, no problem count and no note for the two leads; given AC-2's, it warns for the `claude` lead only. | Test |
| FR-054-AC-4 | Given a session with a live record and a socket, a lead of kind `claude`, `codex`, unknown or empty is posted to and never typed into; given a record that shows a socket which cannot take mail (parked, or a process that cannot be verified), a lead of another kind waits and nothing is typed. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Narrows the socket check of [FR-029](./FR-029-doctor-reports-relay-health.md); applies
  [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md). The
  registration's `kind` is the harness name `agent-msg` registers (`claude`, `codex`, ...).
