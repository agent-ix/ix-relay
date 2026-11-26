---
id: FR-074
title: "An operator can bind a socket-verified session to a named Herdr pane"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-033"
    type: "depends_on"
  - target: "ix://agent-ix/ix-relay/FR-047"
    type: "depends_on"
---
# FR-074: An operator can bind a socket-verified session to a named Herdr pane

## Description

An operator may repair a session placement after a harness hands a conversation to a
background job that Herdr does not associate with the pane. The operator names the exact
three-part relay address and pane. ix-relay verifies the caller from its own live session
socket record and verifies the pane exists in the local Herdr topology. This explicit claim
is an operator assertion that the two belong together; ix-relay does not infer or discover a
window-to-job attachment.

## Behavior

- `agent-msg register --claim <server:workspace:tab> --pane <pane-id>` shall be available only
  to an interactive command and shall require both options together.
- The command shall derive the caller session from its own process ancestry and validate the
  same live pid and start time against that session's Claude Code record, including its usable
  inbox socket. Inherited pane variables and job metadata shall not establish caller identity.
- The command shall read local Herdr and require the exact named pane to exist, with one
  addressable three-part location, without requiring Herdr to claim that the pane hosts the
  caller session. The operator's explicit claim supplies that relationship.
- The command shall refuse when another live registration already owns the requested address,
  when the named pane or caller session is ambiguous, or when either verification fails.
- The command shall atomically register the verified caller session at the requested address
  and the named pane's current Herdr identity, and remove the stale session's session-to-name
  cache. It shall retain the stale registration while its inbox has mail, so FR-053 can
  forward that queue to the new verified address holder; the stale registration is pruned
  under FR-053 after its inbox is empty.
- The new registration shall preserve socket-only delivery and session-bound queue identity
  from FR-033, FR-044 and FR-047. The command shall not type into the pane.
- When a session is live and verified by its socket but has no matching Herdr session
  association, `agent-msg doctor` shall identify the stale placement.
- When `agent-msg doctor` identifies a stale placement, it shall print the claim command form
  without choosing a pane or address.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-074-AC-1 | Given a live Claude background job whose own process has a live, socket-verified session record and an operator-named Herdr pane, the claim command atomically registers that session at the named address and pane, removes the stale session cache, retains the stale registration while its inbox has mail, and `whoami` resolves the new address; inherited pane variables alone, a missing or reused process, an absent or unusable socket, an unknown pane, or an ambiguous address is refused without changing the registry. | Test |
| FR-074-AC-2 | Given the claimed session and queued mail, the session receives mail over its own socket, `send` from it resolves the claimed address, FR-053 forwards the old session's queued mail to the new holder, and no command types into any pane. | Integration |
| FR-074-AC-3 | Given a live socket-verified caller with a stale launcher registration and no matching Herdr session association, `agent-msg doctor` names the recovery command form but does not guess the address or pane. | Test |

## Dependencies

- Builds on [FR-033](./FR-033-registration-keyed-to-session-and-herdr-ids.md) for immutable
  session queues, [FR-044](./FR-044-watcher-delivers-through-claude-codes-inbox-socket.md) for
  socket delivery, and [FR-047](./FR-047-herdr-placement-must-be-verified.md) for verified
  pane identity. FR-074 adds only the explicit operator assertion when Herdr does not expose
  the session-to-pane relationship.
