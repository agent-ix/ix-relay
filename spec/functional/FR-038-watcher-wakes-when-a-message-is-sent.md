---
id: FR-038
title: "The watcher wakes when a message is sent"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-038: The watcher wakes when a message is sent

## Description

When a message is sent to the bus, `relay-watcher` shall look at the inboxes of
the leads registered on its machine without waiting for a poll interval, by
blocking on the message log that every send appends to; and while nothing is
sent, the watcher shall make no herdr call and only the Redis requests its
timers require.

## Behavior

- The watcher shall block on the message log (`XREAD BLOCK`) from the newest
  entry it has looked at, read from the log's tail *before* the first look on a
  new connection, so a message sent in between wakes the next wait.
- The watcher shall read every local lead's inbox length in one pipelined
  request; a length that cannot be read (a key of the wrong type) fails that
  lead only.
- The watcher shall keep a retry timer for each lead that had messages queued
  when it was last looked at (a lead left for its own hook or a dialog has mail
  and nothing new coming), of `interval_secs` (default 2 s), and shall look at a
  lead whose timer alone is due without looking at the others.
- When a message wakes the watcher, the watcher shall look with herdr only at a
  local lead whose inbox grew since it was last seen or that has no timer yet,
  and at most once per second per lead, and shall reuse the registry it read
  within the last 2 s (scheduling a full look for when that copy expires), so
  other machines' traffic on a busy bus costs no herdr call and at most one
  whole-registry read per 2 s.
- When a batch is requeued, the requeue shall append a wake entry (no message) to
  the log in the same transaction, so an idle lead whose batch was put back is
  looked at at once.
- The watcher shall block in waits of at most 5 s, so a tunnel that dies mid-wait
  is noticed within 6 s.
- The watcher shall sweep every local lead every `--sweep-secs` seconds (default
  15) regardless of wakes, so a
  wake lost to a dropped connection or a restarted Redis delays a message by
  seconds and loses none.
- The watcher shall build sender addresses from the registry it read for the
  tick, not from a second read per delivery.
- The watcher shall check an old registration whose session is still live for
  pruning at most once every 10 minutes.
- If the wait fails, then the watcher shall treat it as a failed poll under
  [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md): record it, back
  off with the capped delay, reconnect if the connection was lost, and sweep
  after recovery. A wait that keeps failing without losing the connection is one
  outage: the good polls between the failures do not reset the backoff, and the
  one notification fires after 60 s.
- If the message log cannot be followed (a key of the wrong type, an ACL refusing
  it), then the watcher shall still connect, say so once, and run on the retry
  timers and the sweep alone.
- The minimum gap between a poll and the poll a message woke shall be slept on
  the watcher's clock.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-038-AC-1 | Given N local leads, one request answers every inbox length in order; given one inbox key of the wrong type, that lead's entry is an error and the others are lengths. | Test |
| FR-038-AC-2 | Given the log's tail id read before a send, a wait from that id returns for the send whether it lands before or during the wait, returns nothing when no send follows, and leaves the connection usable. | Test |
| FR-038-AC-3 | Given an idle lead with the sweep 15 s away and no retry timer, a send makes the wait return and the tick it wakes types the message into the lead's pane. | Test |
| FR-038-AC-4 | Given a lead that is working with mail queued, a retry tick before its timer is due makes no herdr call for it, one after does, and a lead whose inbox has drained has no timer. | Test |
| FR-038-AC-5 | Given three local leads with empty inboxes and no OTLP endpoint, a tick makes exactly two requests (one registry read, one pipelined length check) and no herdr call; with an OTLP endpoint, the tick that pushes adds exactly the two pipelined requests [FR-045-AC-22](./FR-045-watcher-pushes-metrics-over-otlp.md) pins, and a tick inside the push interval makes two. | Test |
| FR-038-AC-6 | Given two local leads that both get a delivery in one tick, the tick reads the registry once. | Test |
| FR-038-AC-7 | Given a wait that fails with a connection error, the state file records it, the loop sleeps the next backoff delay, reconnects and polls as a sweep; given a wait that keeps failing with a data error, the backoff grows to the cap and one notification is raised; given the poll after a wait, it runs for the reason the wait gave. | Test |
| FR-038-AC-8 | Given a registration last seen over 24 hours ago whose session herdr reports live and whose inbox is empty, three consecutive ticks make one herdr call, and a tick after the recheck interval makes another, without pruning it. | Test |
| FR-038-AC-9 | Given a batch requeued after a failed delivery, a wait from the log tail read before it returns, and the history holds no entry for the requeue. | Test |
| FR-038-AC-10 | Given a message wake right after a poll, the loop sleeps the rest of the minimum gap on its clock; given a retry or sweep wake, it sleeps nothing. | Test |
| FR-038-AC-11 | Given a local lead left for its hook with mail queued and 20 sends to a lead on another machine, the watcher makes no further herdr call for the local lead and reads the registry once. | Test |
| FR-038-AC-12 | Given a message log key of the wrong type, the watcher connects, marks the log unfollowable, and a sweep still delivers a queued message. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Delivered by AGE-2028.
- Constrained by [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md) and
  related to [FR-014](./FR-014-watcher-types-only-into-the-registered-session.md)
  (the typing decision is unchanged) and
  [FR-028](./FR-028-watcher-heartbeat.md) (a tick runs at least every sweep).
