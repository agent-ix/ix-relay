---
id: FR-007
title: "The watcher keeps recorded places current"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-007: The watcher keeps recorded places current

## Description

While `relay-watcher` runs, the watcher shall update the recorded place of every
lead registered on its own machine whose pane's live herdr place differs from the
recorded one, at most once per 30-second refresh period.

## Behavior

- The watcher shall read herdr's live layout at most once per refresh period of
  30 seconds.
- The watcher shall update only registrations whose normalised machine equals
  its own machine.
- The watcher shall replace a recorded place only if the stored registration is
  still byte-for-byte the one it read; if the lead re-registered in between, then
  the watcher shall leave the newer registration unchanged.
- If herdr's layout cannot be read, then the watcher shall log the failure and
  continue delivery for that tick.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-007-AC-1 | Given a local lead whose recorded tab is `2` and whose live tab is `1`, after one tick the recorded place equals the live place. | Test |
| FR-007-AC-2 | Given a lead registered on another machine with a pane id present in the local layout, a tick leaves its recorded place unchanged. | Test |
| FR-007-AC-3 | Given two ticks less than 30 seconds apart, herdr's layout is read once. | Test |
| FR-007-AC-4 | Given the lead re-registers between the watcher's read and its write, the registration after both equals the lead's own re-registration. | Test |
| FR-007-AC-5 | Given herdr's layout call fails, the same tick still attempts delivery to local leads with queued messages. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md),
  [FR-006](./FR-006-registration-records-place.md)
- Delivered by AGE-2022.
