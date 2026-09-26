---
id: FR-039
title: "Read-only commands retry a timed-out read"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-039: Read-only commands retry a timed-out read

## Description

If a Redis read made by `find`, `who`, `whoami`, `log`, or the address lookup
behind `inbox`'s display times out (including `EAGAIN` from a socket timeout),
then the command shall retry the read on a fresh connection, at most twice in all
and inside one 5 s deadline for the whole command, before failing.

## Behavior

- The command shall retry on a new connection, never the one that timed out,
  because that connection may still hold the late reply.
- The command shall retry only a timeout; a refused or reset connection, and a
  server-reported error, are returned at once.
- The command shall not retry `send`, the inbox pop, or the hook's pop, none of
  which is idempotent: a timed-out `send` may have queued its message and a
  timed-out pop may have removed it.
- The command shall share its retries (100 ms and 250 ms backoff) and its 5 s
  deadline across every read it makes, and shall arm no attempt with more than
  what is left of the deadline or 3 s, so a Redis that accepts and never answers fails
  `who`, `whoami`, `find` (after its herdr gathering) and `log` in about 5 s: the
  measured times are 5.0 s for `who` and `whoami`, 5.2 s for `log` (its second
  read gets the 0.2 s minimum attempt) and 3.0 s for `inbox`, whose pop is not
  retried.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-039-AC-1 | A read that times out once or twice and then succeeds returns its result after 100 ms and 250 ms backoffs, each on a fresh connection; three timeouts return the third; any other error is returned at once. | Test |
| FR-039-AC-2 | Given a Redis whose first connection never answers, `who` and `log` print their answer after dialling exactly two connections and exit 0. | Test |
| FR-039-AC-3 | Given a Redis that answers the send transaction after the read timeout, `send` reports the timeout after dialling exactly one connection and the message is queued exactly once. | Test |
| FR-039-AC-4 | Given a Redis that accepts connections and never answers, `who`, `whoami` and `log` fail or finish within 6 s and `inbox --json` within 4 s, however many reads the command makes. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Delivered by AGE-2028.
- Related: [FR-017](./FR-017-hook-never-fails-the-turn.md) (the hook's pop is never
  retried).
