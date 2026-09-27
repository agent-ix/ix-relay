---
id: FR-006
title: "A registration records where the lead sits"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-006: A registration records where the lead sits

## Description

When a lead registers, the store shall record under its bus name the lead's
normalised machine name, harness kind, harness session id and working directory
when known, and its Herdr pane claim when known. The store shall record a place
only when Herdr's workspace, tab, and pane identities bind to the exact harness
session; labels and environment values alone are claims, not a verified place.

## Behavior

- When the hook handler runs at `SessionStart` or `UserPromptSubmit` for a lead,
  the hook handler shall register the lead.
- When the hook handler registers a lead without a freshly read place, the hook
  handler shall record the place already registered under that name, else the
  place last registered by the same pane on the same machine under any name.
- When a name registers, the store shall remove that name from the alias table
  in the same atomic step, so a name is never both registered and an alias.
- When a name is made an alias, the store shall remove that name's registration
  in the same atomic step.
- The registration record for a lead with no known pane or place shall still be
  stored, and that lead shall remain reachable by bus name and alias.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-006-AC-1 | A stored verified place includes Herdr's workspace, tab, and pane IDs and the same session ID as the registration. | Test |
| FR-006-AC-2 | Given a lead registered with a place, a `UserPromptSubmit` re-registration (which reads no herdr state) leaves the recorded place unchanged. | Test |
| FR-006-AC-3 | Given a lead renamed so its old name becomes an alias, the new registration carries the place the pane had under the old name. | Test |
| FR-006-AC-4 | For every sequence of register and alias operations, no name is both a key of the registry and a key of the alias table afterwards. | Test |
| FR-006-AC-5 | Given a registration with no pane and no place, `send` to its bus name queues the message for it. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- **Downstream**: [FR-007](./FR-007-watcher-refreshes-place.md)
- Delivered by AGE-2010, AGE-2013, AGE-2018 and AGE-2022.
