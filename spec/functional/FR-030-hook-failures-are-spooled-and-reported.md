---
id: FR-030
title: "Hook failures are spooled locally and reported by the next healthy hook"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-030: Hook failures are spooled locally and reported by the next healthy hook

## Description

If a hook call fails, then the hook handler shall append a failure record to a
bounded local spool on that machine, and when a later hook call on that machine
reaches Redis, the hook handler shall move the spooled records to the bus's
hook-failure stream.

## Behavior

- A failure record shall carry the time, the machine (normalised), the lead's bus
  name when known, the hook event when known, the session id cut to 64
  characters, the stage at which the hook failed (`args`, `config`, `decode`,
  `connect`, `derive`, `register`, `pop`, `output`, `budget`, `requeue_failed` or
  `panic`) and an `error_kind` code. A `decode` record's code is `malformed` (not
  the hook JSON), `io` (unreadable), `too_large` (more than 8 MiB on stdin) or
  `stdin_timeout` (no complete first JSON object on stdin: 300 ms without new
  bytes, or 900 ms from the start of the call, NFR-001-M-5). Only the first JSON object of stdin is read
  and bytes after it are ignored; invalid UTF-8 in a string and an object that is
  not a hook payload are `malformed`. It shall carry no failure message text.
  A `requeue_failed` record shall also carry the popped messages the failed
  requeue left in no other place ([FR-020](./FR-020-hook-never-loses-popped-messages.md)).
- If the hook runs in a herdr pane, and herdr fails or times out, or a herdr call
  is refused because the time budget is spent, and nothing else names the
  session, then the handler shall register and pop nothing and record one
  failure: stage `budget` (`budget_exhausted`) when any herdr call was refused
  because the time budget was spent, otherwise stage `derive` (`herdr_failed`).
  It shall not treat the pane as a session that is not a lead.
- If the hook's payload completes after its event's time budget is already spent
  (a `UserPromptSubmit` payload later than 300 ms from the start of the call,
  NFR-001), then the handler shall register and pop nothing, make no connection
  and record one failure: stage `budget` (`budget_exhausted`).
- While that failure persists, the handler shall record it once per session in
  each 300-second window, and shall still attempt the derivation on every call,
  so that the lead registers on the first call after herdr answers. The handler
  shall keep that window apart from the `no_free_name` one, and when it cannot
  set the window it shall record the failure.
- The handler shall also record an inbox entry it popped and could not decode
  (`decode`) and a demotion the bus refused (`register`), without failing the hook.
- The spool shall be the file `spool/hook.jsonl` beside the configuration file,
  mode 0600 in a directory of mode 0700. The handler shall not write it under a
  relative configuration path, and shall not follow a symlink.
- The spool shall hold at most 256 KiB. If an append reaches that bound, then the
  handler shall keep the newest records that fit in half the bound and count the
  discarded records in the `dropped` field of the first record kept.
- The handler shall write the record to the spool before it writes to stderr, a
  failure to write stderr shall not change the exit status
  ([FR-017](./FR-017-hook-never-fails-the-turn.md)), and a panic shall be caught,
  recorded with stage `panic` and exit 0.
- The flush shall ship every spooled record, in chunks each within the remaining
  time budget, to the one stream `<prefix>health:hooks` (capped at about 1000
  entries), each entry carrying the record, its machine and its lead, and shall
  remove from the spool exactly the records shipped. A flush that fails leaves the
  records it did not ship in the spool and is not itself recorded. A crash after
  shipping and before the removal ships those records again: delivery is at least
  once.
- If a hook fails before it can connect for a reason that repeats on every call
  (an invalid `--as` name, unreadable hook input), then the handler shall connect
  once and flush. A configuration or command-line failure has no Redis address
  and waits for another hook on the machine.
- When the handler is about to pop, it shall require a remaining budget of at
  least the pop window, the greater of 30 ms and the time the connection took
  plus 20 ms (the pop is one request and one reply; the connection setup is two
  round trips). If less remains, then the handler shall not pop.
- The handler shall record a skipped `PostToolUse`, `Stop` or `SessionStart` pop
  with stage `budget`. A skipped `UserPromptSubmit` pop is an expected deferral to
  the next `PostToolUse` or `Stop`, and is not recorded.
- Anything that shows a record's `messages` to a person or an agent shall
  neutralise it as in [FR-016](./FR-016-render-neutralises-control-characters.md).
- Whether a recovered failure is also shown in the agent's context is not
  specified here: anything that changes what agents see needs the operator's
  sign-off (AGE-2033).

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-030-AC-1 | Given Redis is unreachable, three `PostToolUse` hook calls each exit 0 with empty stdout and leave three failure records in the local spool. | Test |
| FR-030-AC-2 | Given three spooled records and Redis reachable again, the next hook call leaves the spool empty and the hook-failure stream holds those three records, each with its machine and lead. | Test |
| FR-030-AC-3 | Given more failures than fit the bound while Redis is unreachable, the spool never exceeds 256 KiB plus one record, and the records kept plus their `dropped` counts equal the failures recorded. | Test |
| FR-030-AC-4 | After a flush that ships some of the spooled records and then fails, the unshipped records stay in the spool, and after the next successful flush every record has been shipped at least once and the spool is empty. | Test |
| FR-030-AC-5 | Every spooled record's fields are timestamps, identifiers and stable codes: for any failure it contains no text taken from the failure's message. | Test |
| FR-030-AC-6 | Given a `PostToolUse` hook with less than the pop window remaining, the hook does not pop, every message stays queued, and one `budget` record is spooled. | Test |
| FR-030-AC-7 | Given a `UserPromptSubmit` hook with less than the pop window remaining, the hook does not pop, every message stays queued, and no record is spooled. | Test |
| FR-030-AC-8 | Given a closed stderr, a hook failing at each stage it can reach exits 0 and leaves exactly one record; given a panic at any stage, the hook exits 0 and leaves one `panic` record. | Test |
| FR-030-AC-9 | Given a relative `IX_RELAY_CONFIG`, a failing hook exits 0 and creates no spool in its working directory. | Test |
| FR-030-AC-10 | Given a hook in a herdr pane whose herdr fails or times out and no other name for the session, the hook registers and pops nothing and leaves one `derive` record with code `herdr_failed`; given that a herdr call was refused because the time budget was spent, it leaves one `budget` record with code `budget_exhausted` instead. | Test |
| FR-030-AC-11 | Given herdr failing for five uncached `PostToolUse` calls of one session, the spool holds one record and every call attempted the derivation; given herdr answering on the sixth call, the lead is registered by it and no further record is left. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Related: [FR-017](./FR-017-hook-never-fails-the-turn.md), [FR-020](./FR-020-hook-never-loses-popped-messages.md),
  [NFR-001](../non-functional/NFR-001-hook-time-budget.md).
- Delivered by AGE-2033. The pop window is an interim guard until AGE-2032's
  in-flight list, which removes the window.
