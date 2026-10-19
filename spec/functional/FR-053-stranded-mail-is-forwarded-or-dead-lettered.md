---
id: FR-053
title: "Mail stranded in a dead or replaced session's inbox is forwarded or dead-lettered"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-053: Mail stranded in a dead or replaced session's inbox is forwarded or dead-lettered

## Description

When the herdr pane of a registered session no longer hosts that session, then
`relay-watcher` shall hand the registration's queued mail to the session that now
holds the same `server:workspace:tab` address, telling that session the mail was
meant for a prior session, or, when nothing holds the address, move the mail to the
dead-letter list and prune the registration, so that no message waits in an inbox
nobody will ever read.

## Background

A queue belongs to a session ([FR-033](./FR-033-registration-keyed-to-session-and-herdr-ids.md)),
and an address names a place, not a session. When a session ends or is replaced in its
pane, a sender who wrote to the address before the change has queued mail for a session
that will never pop it. The pruning rule ([FR-031](./FR-031-dead-letters-instead-of-drops.md))
never removes a registration whose inbox holds mail, so before this requirement that
mail, and the registration, stayed forever: `agent-msg who` showed the lead as
`UNDELIVERABLE: pane <id> now hosts another session` ([FR-032](./FR-032-sender-sees-undeliverable-recipients.md))
and `agent-msg doctor` showed `registration_pane_gone`, with nothing that would ever
clear either.

The owner's rule is that mail addressed to a place belongs to whoever holds that place
now, and that the new holder must be told the mail was written for someone else, because
the sender wrote it to a session whose context the new holder does not share.

## Behavior

### When the watcher acts

- The watcher shall act only on registrations of its own machine
  ([FR-009](./FR-009-pane-ids-scoped-by-machine.md)) that have a pane and a session id and
  whose inbox holds at least one message.
- The watcher shall treat a registration's session as replaced only when herdr answers
  for the registration's pane and names a session id on it that differs from the
  registered one ([FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md)'s
  evidence), and as gone only when herdr answers and lists no agent in that pane and no
  pane hosting the registered session.
- If herdr cannot be asked, answers in a shape ix-relay cannot read
  ([FR-036](./FR-036-herdr-version-and-shape-check.md)), lists nothing because it is
  restarting, names no session for the pane, or the registration has no session id or
  no pane, then the watcher shall move nothing, forward nothing and prune nothing.
- While the registered pane hosts the registered session, the watcher shall never move
  any of the registration's mail: the mail of a live session stays where it is.
- The watcher shall look at a registration for this purpose at most once per
  place-refresh period, from the `agent list` that period already reads.

### A new session holds the address

- When a session is replaced and exactly one other registration of the same machine
  has the registration's `server:workspace:tab` address, and that registration's session
  is the one herdr verifies in its pane ([FR-047](./FR-047-herdr-placement-must-be-verified.md)),
  then the watcher shall move every queued message of the replaced registration to the
  inbox of that registration, and shall leave the replaced registration in place.
- If no registration, or more than one, qualifies, then the watcher shall move nothing:
  the mail stays queued until the new session registers, and an ambiguous address is
  never resolved by guessing ([FR-004](./FR-004-resolution-never-guesses.md)).
- The move shall be one atomic bus operation per batch of messages: each message shall
  be in exactly one of the two inboxes at every instant a reader can observe, and a
  crash of the watcher or a second watcher running the same pass shall leave every
  message in exactly one inbox. The move shall follow the store's script discipline
  ([FR-020](./FR-020-hook-never-loses-popped-messages.md)): it checks every key it writes
  before its first write and writes the destination before it deletes the source, so a
  script that fails part-way leaves a message in both inboxes (a duplicate), never in
  neither. The move shall be a compare-and-set
  against the registrations the watcher read, like a relink
  ([FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md)): it shall do
  nothing if either registration changed, or was pruned, since the read.
- The moved messages shall keep their relative order and shall come before any message
  that was queued for the new session itself and has not been popped.
- Messages leased in flight to the replaced registration
  ([FR-020](./FR-020-hook-never-loses-popped-messages.md)) shall not be moved by this pass;
  when their lease expires they return to the inbox and the next pass moves them.

### The new session is told

- Each message delivered after such a move shall carry, wherever the recipient reads it
  (the hook's output, the watcher's prompt, the socket batch and the inbox listing),
  a line stating that it was sent to a prior session of this address, the time the
  sender sent it, and the sender's address as the original message gave it.
- The line shall be written by ix-relay, shall not be part of the sender's text, and
  shall be present on every rendering path of a moved message. A message moved twice
  (the new session was itself replaced) shall name its original send time and sender.
- The move shall mark each moved entry, in the same atomic operation, with a member
  recording that it was forwarded from a prior session of the address, and shall change
  no other member; a second move keeps the first mark. The mark is what every rendering
  path reads, and a moved message later dead-lettered is kept byte for byte as stored,
  mark included ([FR-031](./FR-031-dead-letters-instead-of-drops.md)).
- The notice shall be part of the message's first, signed line (for example
  `[ix-relay] from <sender> (sent <time> to a prior session of this address): `), so that
  the first line remains the only one beginning with `[ix-relay]` and the sender's text
  stays on indented continuation lines
  ([FR-016](./FR-016-render-neutralises-control-characters.md)).
- A moved entry's pop weight shall include the notice's bytes
  ([FR-012](./FR-012-inbox-pops-are-atomic-and-budgeted.md)), so that the hook-output and
  watcher-prompt bounds of [NFR-002](../non-functional/NFR-002-bounded-resources.md) hold
  for a batch of moved messages.
- The text of a moved message shall be neutralised when rendered, as for any message
  from another lead ([FR-016](./FR-016-render-neutralises-control-characters.md),
  `relay_core::text::neutralise`); moving a message shall not change what the sender
  wrote, and the line shall not give a sender's text a way to forge it.

### Nothing holds the address

- When a registration's session is gone, no registration holds its address, and the
  registration is old enough for the pruning rule (a `last_seen_ms` of 24 hours or more),
  then the watcher shall move every queued message of it to the dead-letter list with
  reason `recipient-pruned`, byte for byte ([FR-031](./FR-031-dead-letters-instead-of-drops.md)),
  and then prune the registration, as one compare-and-set against the registration it
  read.
- If the compare-and-set fails or the dead-lettering fails, then the watcher shall
  prune nothing and leave the mail where it is.
- The pruning rule's age and herdr-liveness conditions are otherwise unchanged; only
  its refusal to prune a registration with mail is replaced by dead-lettering that mail
  first.

### Reporting

- The watcher shall log one line per registration and kind of move (forwarded,
  dead-lettered, pruned), with the count of messages and the two registrations, and
  none for a pass that did nothing.
- `agent-msg who` and `agent-msg doctor` shall stop reporting a registration as
  undeliverable or `registration_pane_gone` for mail this requirement moved, because
  the registration no longer holds it.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-053-AC-1 | Given a registration with three queued messages whose pane now hosts session `s2`, and exactly one other registration of the same address whose session is `s2` in its pane, one watcher pass leaves the old inbox empty and the new one holding all three, in their original order and before the new session's own unpopped messages. | Test |
| FR-053-AC-2 | Given the move of AC-1, each message the new session reads (through the hook, the watcher prompt, the socket batch and the inbox listing) carries a line naming the prior session of this address, the original send time and the original sender, and the sender's text is unchanged. | Test |
| FR-053-AC-3 | Given a moved message whose text and sender name contain ESC, newlines and a forged closing line, the rendered output contains no control character from them and no second copy of the notice line. | Test |
| FR-053-AC-4 | Given a registration whose pane hosts its registered session, with any inbox, any age and any herdr answer other than a positive one, no message leaves its inbox and nothing is pruned. | Test |
| FR-053-AC-5 | Given herdr fails, times out, answers in an unreadable shape, lists nothing, or names no session for the pane, the watcher moves, forwards and prunes nothing; given the replaced pane's address is held by no registration, or by two, nothing is moved. | Test |
| FR-053-AC-6 | For any interleaving of a pass with sends, pops, prunes, registrations, relinks and a second watcher's pass, and with a bus failure injected at each step, every message is afterwards in the old inbox, in the new inbox, or in the dead-letter list, never in none of them, and in exactly one of them unless the injected failure stopped a script part-way, which may leave it in two. | Property |
| FR-053-AC-12 | Given a batch of moved messages, every rendering has only its first line beginning with `[ix-relay]` and carries the notice on that line, and the hook output stays within 6 KiB plus one rendered message plus the pending-count line. | Test |
| FR-053-AC-7 | Given a registration that is pruned, or registered again, between the watcher's read and the move, the move does nothing. | Test |
| FR-053-AC-8 | Given a message moved to a session that was itself replaced, it moves again and the notice still names the first send time and the original sender. | Test |
| FR-053-AC-9 | Given a registration of another machine in the same state, the watcher moves, forwards and prunes nothing for it. | Test |
| FR-053-AC-10 | Given a registration 25 hours old whose pane herdr lists no agent in, holding two queued messages and no registration holding its address, one pass leaves both in the dead-letter list byte for byte with reason `recipient-pruned` and the registration pruned; given it is 23 hours old, nothing moves; given the dead-lettering fails, the registration is not pruned and the mail stays. | Test |
| FR-053-AC-11 | Given a leased in-flight batch of a replaced registration, the pass does not move it, and after the lease expires the next pass does. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Builds on [FR-031](./FR-031-dead-letters-instead-of-drops.md) (the dead-letter path and
  the pruning rule), [FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md)
  (the evidence and the compare-and-set), [FR-047](./FR-047-herdr-placement-must-be-verified.md)
  (a verified placement) and [FR-032](./FR-032-sender-sees-undeliverable-recipients.md)
  (the undeliverable report).
- An older `agent-msg` or watcher on the receiving machine may not draw the notice line
  from a message it does not know to be moved; the machines of a fleet are upgraded
  together, and no compatibility layer is kept ([NFR-004](../non-functional/NFR-004-prerelease-no-compatibility-layers.md)).
