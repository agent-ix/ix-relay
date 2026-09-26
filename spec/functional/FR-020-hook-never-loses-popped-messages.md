---
id: FR-020
title: "A popped message stays in flight until its delivery is acknowledged"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-020: A popped message stays in flight until its delivery is acknowledged

## Description

When a consumer (the hook or the watcher) pops a lead's inbox, the store shall
move the popped messages in the same atomic operation into an in-flight record for
that lead, and shall remove that record only when the consumer acknowledges the
delivery, so that no failure between the pop and the delivery loses a message. A
record that is not acknowledged before its lease expires returns to the front of
the inbox.

Scope: the hook and the watcher. The opencode plugin delivers through
`agent-msg inbox --json`, which pops outright and stays at-most-once until it gets a
claim and an acknowledgement (AGE-2051), and the redelivery marker is not shown
on that path (also AGE-2051).

## Behavior

- The in-flight record shall hold, per popped entry, the batch id, the popper
  (`hook` or `watcher`), a lease deadline measured on the Redis server's clock,
  and the entry byte for byte.
- The hook shall acknowledge a batch after its output was written to stdout and
  flushed, and the watcher when the delivery transport reports success. "Delivered"
  means written to the pipe or accepted by the transport: what the harness does
  with the bytes afterwards is beyond the hook, and a batch acknowledged and then
  never read is lost. A failed or ambiguous delivery shall not be acknowledged.
- An acknowledgement shall delete only the rows of the batch whose id it names;
  an acknowledgement of a batch that was already returned to the inbox, redelivered
  under a new id, or acknowledged shall delete nothing.
- If an acknowledgement finds its batch no longer in flight, then the hook shall
  record that the messages are redelivered.
- If a consumer knows a delivery did not happen (the hook's stdout write failed,
  or herdr documents that no input was sent), then it shall return the batch to
  the front of the inbox in original order in one atomic operation, without
  counting a redelivery.
- The hook shall return every due batch of its own lead to the front of the inbox,
  in the same request as its pop. The watcher shall do the same for every
  registered lead at start-up and on every sweep, in the same request as the inbox
  lengths.
- The store shall return each due batch on its own lease, in the order the entries
  were first popped when several are due. Messages keep their order within a batch
  and from one pop to the next; a redelivered batch is marked and may follow newer
  mail.
- The hook's lease shall be 20 s and the watcher's 60 s: each is several times
  the longest time its popper legitimately holds a batch (a hook's whole budget is
  at most 3.3 s; a herdr prompt times out after 10 s), so a slow but live delivery
  is not redelivered, while a killed one returns within the lease.
- The pop shall take its lease as a parameter and the clock the lease is measured
  on as a replaceable parameter, so a test expires a lease without waiting, and the
  store shall clamp a lease to one day.
- The store shall keep at most 16 in-flight batches and 1 MiB of in-flight rows per
  lead, not counting rows that could not be dead-lettered: a pop that would exceed
  either while something is in flight, counting each entry it would take as its
  length plus 64 bytes, shall take nothing, leave the inbox unchanged and report
  that the cap was reached. A pop with nothing waiting reports nothing, and a pop
  with nothing in flight takes its first entry however large it is.
- The store shall hold every batch, redelivered or not, to the pop's byte budget
  ([FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md)).
- If a key a script writes is of the wrong type, then the store shall refuse with
  an error naming the lead, having changed nothing: the store checks every such key
  before its first write.
- The store shall write the destination of each move before it deletes the source,
  so that a script that fails part-way leaves a message in both places rather than
  in none.
- If a lead's dead-letter key holds the wrong type, then the store shall leave the
  rows that would be dead-lettered in flight, report how many, exclude them from
  the in-flight caps, and keep popping.
- If the batch counter is not an integer, then the store shall refuse the pop as an
  unreadable lead before any write.
- The hook shall send the acknowledgement as the next request after the write and
  flush, needing nothing but the batch id, and shall read the sender addresses only
  when the time left covers that read and the acknowledgement.
- If a hook's pop takes nothing because of the in-flight cap, then the hook shall
  record it, except at `UserPromptSubmit`, whose deferral is expected.
- If the in-flight cap stops a watcher pop, then the watcher shall log the cap
  once per episode.
- If the watcher's reap of a lead fails, then the watcher shall log the failure
  once per episode.
- `agent-msg inbox --in-flight` shall list, for every lead found by scanning at
  most 1,000 keys of each kind, its in-flight and dead-letter entry counts,
  registered or not, show a lead whose key holds the wrong type as unreadable, and
  say when the scan stopped at its bound.
- Delivery is at-least-once: if a batch was delivered but is not acknowledged before
  its lease expires, then the store shall deliver it again.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-020-AC-1 | Given two queued messages and a stdout that is closed before the hook writes, after the hook exits the inbox holds the same two messages in the same order. | Test |
| FR-020-AC-2 | Given two queued messages and a registry read failure after the pop, the hook output carries both messages signed with bus names. | Test |
| FR-020-AC-3 | For every point at which the hook or the watcher can fail or be killed between the pop and the acknowledgement, every popped message is, until acknowledged, in the inbox or in flight, and is delivered again once its lease has expired. | Test |
| FR-020-AC-4 | Given a hook that pops two messages and is killed before it writes, after the lease expires the next pop by the same lead returns the same two messages in the same order. | Test |
| FR-020-AC-5 | Given a batch acknowledged by a stale id (after its lease expired and it was popped again), the newer batch is still in flight and is delivered again if not acknowledged. | Test |
| FR-020-AC-6 | Given a watcher prompt that fails ambiguously, the batch is not acknowledged and is returned to the inbox by the reap after its lease expires. | Test |
| FR-020-AC-7 | For any interleaving of pops, acknowledgements, requeues and lease expiries by two consumers over one inbox, no message is lost, none is returned twice unless its lease expired or it was requeued, and the messages of one batch keep their order. | Test |
| FR-020-AC-8 | Given in-flight rows that leave one byte less room than the batch a pop would take, or 16 unacknowledged batches, the pop takes nothing, leaves the inbox unchanged and reports the cap; given exactly enough room it takes the batch; given nothing in flight it takes a single entry larger than the cap; given an empty inbox it reports no cap. | Test |
| FR-020-AC-9 | Given a redelivered batch of two whose marked renders joined by a newline total one byte more than the hook's 6 KiB budget, the pop takes only the first. | Test |
| FR-020-AC-10 | For each script and each key it may write, given that key holding the wrong type, the call refuses naming the lead and every message stays where it was; given a wrong-type dead-letter key, pops still work, including with more than 16 stuck batches, and leave the spent rows in flight; given a batch counter that is not an integer, the pop refuses before writing; given a script that fails after its first write, no message is left in neither the inbox nor in flight. | Test |
| FR-020-AC-11 | Given a watcher batch popped before a hook batch, each returns when its own lease expires, the hook's first; when both are due they return in the order they were popped. | Test |
| FR-020-AC-12 | Given a lease of any length, the row's deadline is at most one day after the pop, and the row is redelivered, not dead-lettered, when it expires. | Test |
| FR-020-AC-13 | Given in-flight rows and dead letters under a name that is not registered, `agent-msg inbox --in-flight` lists that name with its counts; given more than 1,000 keys it says it stopped. | Test |
| FR-020-AC-14 | Given a lead at its in-flight cap, a `Stop` hook records the cap, a `UserPromptSubmit` hook does not, and the watcher logs the cap once over three ticks and not as an emptied inbox; given a lead whose reap keeps failing the watcher logs one line over three sweeps. | Test |
| FR-020-AC-15 | Given a 60 ms round trip, a `UserPromptSubmit` with an explicit name prints and acknowledges with no failure record, and with a derived name it defers without a record; given 50 ms the derived name prints and acknowledges; and the acknowledgement is the next request after the write. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md),
  [FR-017](./FR-017-hook-never-fails-the-turn.md),
  [FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md)
- Delivered by AGE-2032. Redelivery marking, its bound and the dead-letter
  list are [FR-040](./FR-040-redelivery-is-marked-bounded-and-dead-lettered.md); the
  general dead-letter list of [FR-031](./FR-031-dead-letters-instead-of-drops.md)
  (AGE-2033) is separate and not built here.
- `agent-msg inbox` (a human's read, and the opencode plugin's `--json` delivery)
  keeps the plain pop and is at-most-once; the opencode delivery needs a claim and
  an acknowledgement (AGE-2051).
