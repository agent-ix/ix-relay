---
id: FR-005
title: "One rule for the shown address"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "implements"
---
# FR-005: One rule for the shown address

## Description

When ix-relay prints a lead (`who`, `find`, `whoami`, `log`, send confirmations,
the signature on a delivered message), the printer shall show a Herdr-derived
address only for a verified placement. It shall include workspace and tab, and
add the Herdr pane ID when multiple registered leads share the tab address. If
placement is unverified or the address does not identify one lead, it shall
show the bus name.

## Behavior

- The printer shall always include the tab in a shown address so the displayed
  address round-trips independently of cached tab counts.
- If the tab address is shared, the printer shall append Herdr's pane ID as a
  fourth segment, replacing the ID's colon with `-` per the address cleaning
  rule.
- The printer shall show a bus name, not an address, when the registration has
  only legacy labels or lacks an exact session-to-pane verification.
- The printer shall treat two leads as sharing an address when their full
  addresses (tab included) are equal under
  [FR-003](./FR-003-address-matching-ignores-case.md)'s comparison.
- The printer shall show the bus name for a lead with no recorded place, and for
  a lead whose address is longer than 96 bytes: a label is whatever a person typed
  into herdr, and a delivered message's size is bounded by the longest label.
- Every command that prints a lead shall apply this one rule, so the same
  registry snapshot prints the same label for a lead in every command.
- If the registry cannot be read while printing, then the printer shall show bus
  names and the command shall continue.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-005-AC-1 | Given two leads sharing `hub:EA:code` but on distinct Herdr panes, each is shown with its pane ID; a lead alone at `hub:EA:plan` is shown at that tab address. | Test |
| FR-005-AC-2 | Given a lead in a one-tab workspace `research` on `node-a`, it is printed `node-a:research:1`; the tab remains in the address regardless of cached tab count. | Test |
| FR-005-AC-3 | Every lead in any registry snapshot is printed with the same label by `who`, `find`, the send confirmation and the message signature. | Test |
| FR-005-AC-4 | For every registry snapshot, every printed address resolves back through [FR-002](./FR-002-target-resolution-priority.md) to exactly the lead it labels. | Test |
| FR-005-AC-5 | Given a registry read failure while rendering an inbox, the messages are printed signed with bus names and the command exits 0. | Test |
| FR-005-AC-6 | A sole verified registered lead at workspace `EA`, tab `code` on `hub` is printed `hub:EA:code`; tab and pane counts do not alter this address. | Test |
| FR-005-AC-7 | Given a lead whose address is 97 bytes long, it is printed by bus name; given one whose address is exactly 96 bytes, it is printed as its address. | Test |
| FR-005-AC-8 | Two verified leads at distinct tabs are printed with their tab addresses; another server's same workspace label does not change either address. | Test |
| FR-005-AC-10 | Given a registration with labels but no Herdr IDs, address labels are not advertised and the lead is printed by bus name until the watcher verifies it. | Test |
| FR-005-AC-9 | For every registry snapshot, the address book built in time linear in the number of leads prints every lead as the pairwise definition of AC-1 and AC-8 does, and building it for 10,000 leads takes under 5 s. | Test |

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md)
- Delivered by AGE-2023.
- Gap closed by AGE-2026: one rule, `Place::address` for the shown address and
  `relay_core::duplicated` for shared addresses, used by every listing.
- AGE-2028: the `AddressBook` is the one place the full-address and uniqueness
  rule lives; `who` and `whoami` take their address column from it.
  `Registration::address`, which could not know the registry, is removed.
