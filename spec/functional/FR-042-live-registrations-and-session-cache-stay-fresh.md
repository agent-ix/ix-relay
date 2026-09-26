---
id: FR-042
title: "Registrations of live leads and the session cache stay fresh"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-042: Registrations of live leads and the session cache stay fresh

## Description

While a lead's herdr pane is alive and hosts the session the lead registered,
`relay-watcher` shall keep the registration's `last_seen_ms` from ageing, and
while a session is in use `agent-msg` shall keep its remembered name and
suppression mark from expiring.

## Behavior

- When a local lead's registration is at least 15 minutes old and herdr shows an
  agent in its pane whose session id does not differ from the registered one, the
  watcher shall set the registration's `last_seen_ms` to the present time.
- If the registration has a session id and herdr shows none for the pane, then the
  watcher shall not refresh it, because a pane with no session might hold another
  agent that took the place, or the same one before its harness reported.
- If the registration has no session id, then the watcher shall treat whatever
  agent sits in the pane as the lead's, as the pruning rule already does.
- The watcher shall look at a lead's pane for this purpose at most once in ten
  minutes.
- The watcher shall change `last_seen_ms` only if the stored registration is still
  exactly the one it read: a compare-and-set, keeping every member it does not
  know. It shall never write a registration that is absent, never overwrite one
  that was registered again meanwhile, and never move `last_seen_ms` backwards.
- If herdr shows no agent in the pane, a different session, or cannot answer,
  then the watcher shall not refresh the registration, and the 24-hour pruning
  rule applies to it as before.
- While herdr's output is in a shape ix-relay cannot read
  ([FR-036](./FR-036-herdr-version-and-shape-check.md)), the watcher shall not
  refresh any registration.
- A hook or command that looks a session up shall renew the expiry of that
  session's remembered name and suppression mark to 30 days from then, in the same
  request as the lookup. Renewing shall never create either key.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-042-AC-1 | Given a lead whose registration is older than 15 minutes and whose pane herdr shows with the registered session, the watcher moves its `last_seen_ms` to the present and changes no other member of the registration. | Test |
| FR-042-AC-2 | Given a pane with another session, a pane with no agent, a lead that was registered again meanwhile, or a lead that was pruned meanwhile, the watcher changes nothing: the registration is not refreshed, not overwritten and not brought back. | Test |
| FR-042-AC-3 | Given a registration two days old, a watcher tick refreshes it when its pane is alive and prunes it when its pane is gone. | Test |
| FR-042-AC-4 | Given a session with a remembered name and a suppression mark expiring in 100 s, looking the session up leaves both expiring in about 30 days, and looking up an unknown session creates no key. | Test |
| FR-042-AC-5 | Given a registration that is pruned, or registered again, in the window between the watcher's read and its write, the write does nothing: the lead stays pruned and the newer registration stays. | Test |
| FR-042-AC-6 | Given a registration with no id, the watcher refreshes with any agent in the pane; given one with an id, it refreshes only when herdr shows the same id: no id from herdr, or another, does not. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Related: [FR-007](./FR-007-watcher-refreshes-place.md) (the same compare-and-set
  guard), [FR-038](./FR-038-watcher-wakes-when-a-message-is-sent.md) (AC-8, the
  herdr calls a stale registration cost).
- Delivered by AGE-2033. `last_seen_ms` now means the last time the lead
  registered or the watcher saw its pane alive; nothing else reads it besides
  `who`, `find` and the pruning rule.
