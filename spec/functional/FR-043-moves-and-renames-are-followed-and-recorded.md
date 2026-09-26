---
id: FR-043
title: "A moved session is followed to its new pane, and moves and renames are recorded"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-007"
    type: "implements"
---
# FR-043: A moved session is followed to its new pane, and moves and renames are recorded

## Description

While a registration names a pane on the watcher's machine that no longer hosts the
registration's session, and the session is now hosted by exactly one other pane of
that machine, `relay-watcher` shall point the registration at that pane, keeping its
bus name, and shall record one event for that relink and one for each change of a
registration's place, in a capped stream that `agent-msg doctor` shows.

herdr's ids are stable across renames, and a `pane move` within a workspace keeps the
pane id; a move to another workspace gives the pane a new id and keeps its session
([SR-005](../reviews/herdr-id-stability.md), herdr 0.9.1). The harness session id is
the key followed.

## Behavior

- Once per place-refresh period, from the one `agent list` the refresh already reads,
  the watcher shall look at each registration on its machine that has a session id and
  a pane.
- The watcher shall read a pane's session only from herdr's `agent_session` with
  `kind` `id`. A `path` (herdr's other kind), a missing kind or any other kind reads as
  no session known: never a different session, never a relink target.
- If the registered pane hosts the registered session, then the watcher shall change
  nothing.
- If the registered pane is not listed, or hosts a different session, and exactly one
  other pane hosts the registered session, then the watcher shall relink the
  registration to that pane, record its place there and stamp its `last_seen_ms`
  (herdr has just shown the lead alive). A pane whose place herdr did not name is not
  a target.
- If the registered pane shows an agent whose harness has not reported a session, if
  two or more panes host the session, if another registration of the same session
  already names the target pane, or if herdr lists nothing (a restart), then the
  watcher shall change nothing: unknown is never a reason to move a lead.
- The watcher shall relink only registrations of its own machine and never one with no
  session id.
- A lead relinked in a tick shall be left out of the rest of that tick and updated in
  the cached registry, so the tick and the cache never use the pane it left and a lead
  just found alive is never pruned.
- The relink shall be a compare-and-set against the whole registration the watcher
  read: it shall never bring back a pruned lead, overwrite a registration written
  again meanwhile, or change another machine's registration.
- The store shall make a registration's change and its event in one script, so neither
  exists without the other. The script shall check the type of every key it writes
  before its first write and, if one is wrong, refuse with a typed error having
  changed nothing.
- The store shall keep the replaced-pane record as one key per lead with an expiry, so
  the records are bounded.
- If reading or writing a replaced-pane record fails with a data error, then the
  watcher shall log one line per episode, not one per refresh.
- If the registered pane hosts a different session and the registered session is on no
  pane, then the watcher shall record that the lead's pane was replaced
  ([FR-032](./FR-032-sender-sees-undeliverable-recipients.md)) and shall not copy that
  pane's place onto the registration.
- When a registration's own workspace or tab label changes and its pane does not (a
  workspace or tab was renamed, or a pane moved to another tab of its workspace), the
  watcher shall record one rename event. It shall record none for a poll that changes
  nothing, none when only the workspace's tab count changed (the place is still
  updated), and at most one per lead per five minutes (the place is still updated).
  The watcher shall log a lead's suppressed change once.
- The event shall name the lead, its machine, its session, the pane before (relink
  only) and after, the place before and after, and the time. The stream is
  `<prefix>health:moves`, capped at about 1,000 entries, and its text is neutralised
  when printed ([FR-016](./FR-016-render-neutralises-control-characters.md)).
- If herdr cannot be read, or Redis fails in the middle of a relink, then nothing shall
  change, and the watcher shall log one line per failing episode of herdr, not one per
  refresh.
- `agent-msg doctor` shall list the moves of the last 24 hours as information: the
  check is `ok` and never fails the exit.
- `agent-msg doctor` shall read the moves by time.
- If more moves exist than `agent-msg doctor` read, then it shall say "at least".
- `agent-msg doctor` shall bound each address it prints.
- If the moves cannot be read, then `agent-msg doctor` shall report the check as
  `unknown` (`moves_unreadable`), never as a failure.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-043-AC-1 | For every set of panes herdr lists, a relink target hosts exactly the registered session, is the only other pane that does, and is not the registered pane, which is unlisted or hosts a different session. | Property |
| FR-043-AC-2 | Given a pane with no reported session or a session reported as a `path`, two panes hosting the session, a target held by another registration of the same session, a target with no place, another machine's registration with the same ids, a registration with no session id, or an empty listing, the watcher changes no registration, records no event and marks no lead's pane replaced; each lasting refusal, the missing place included, is logged once. | Test |
| FR-043-AC-3 | Given herdr's recorded output for a workspace rename, a tab rename, a move within a workspace, a move to another workspace, closed neighbours and a server restart, the watcher records one rename event per renamed lead, one relink event for the moved lead (`w1:p3` to `w2:p2`, same bus name and session, stamp moved to the relink time), none for ten unchanged polls, none for a tab-count change and none after the restart; a label that flaps is one event per five minutes; and a lead last seen a day ago that is relinked is still registered after the tick that relinked it. | Test |
| FR-043-AC-4 | For every interleaving of registrations, prunes, stale snapshots and two consumers, a relink applies exactly when the stored registration equals the snapshot, changes only its pane and place, and never changes another lead or another machine's registration; a prune or registration between the read and the script defeats the write. | Property |
| FR-043-AC-5 | The moves stream keeps about the newest 1,000 events, reads newest first, and counts an entry that does not decode. | Test |
| FR-043-AC-6 | Given a herdr that cannot answer for five refreshes, no registration changes, no event is recorded and one line is logged; given Redis cut when the script is sent, or a key of the wrong type at the moves stream or the lead's marker, the registration and the stream are unchanged and the error is typed. | Test |
| FR-043-AC-7 | Given a recorded relink and rename, `agent-msg doctor` lists both with the lead, the panes and the addresses, its exit is 0, text from herdr is neutralised and bounded, and more than 200 events in 24 hours read "at least"; given a stream it cannot read, the check is `unknown` and the exit is unchanged. | Test |

## Dependencies

- **Upstream**: [US-007](../usecase/US-007-never-reach-the-wrong-lead-after-a-move.md)
- Delivered by AGE-2033 (`relay-watcher`'s `relink` module, `Relay::relink_pane`,
  `Relay::refresh_place`). Realises part of [FR-033](./FR-033-registration-keyed-to-session-and-herdr-ids.md)
  and [FR-034](./FR-034-rename-and-move-events.md); what those still ask beyond this is
  named in their dependencies.
- Not built: recording herdr's workspace and tab ids in a registration; the machine-keyed
  session cache (see spec 15.1).
