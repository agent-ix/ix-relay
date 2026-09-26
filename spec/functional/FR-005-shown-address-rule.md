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
the signature on a delivered message), the printer shall show the lead's address
if no other registered lead has the same full address, and the lead's bus name
otherwise.

## Behavior

- The printer shall omit the tab from a shown address if and only if the lead's
  recorded place says its workspace has exactly one tab and no other registered
  lead is in the same server and workspace (compared under
  [FR-003](./FR-003-address-matching-ignores-case.md)'s comparison).
- The printer shall keep the tab in every other case: a recorded count can be
  stale, so it is never trusted alone, and when the tab count is unknown
  (recorded as 0) the tab is kept.
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
| FR-005-AC-1 | Given two leads at `hub:EA:code` and `hub:ea:Code` and a third at `hub:EA:plan`, the first two are printed by bus name and the third as `hub:EA:plan`. | Test |
| FR-005-AC-2 | Given a lead in a one-tab workspace `research` on `node-a`, it is printed `node-a:research`; given a three-tab workspace, it is printed with its tab. | Test |
| FR-005-AC-3 | Every lead in any registry snapshot is printed with the same label by `who`, `find`, the send confirmation and the message signature. | Test |
| FR-005-AC-4 | For every registry snapshot, every printed address resolves back through [FR-002](./FR-002-target-resolution-priority.md) to exactly the lead it labels. | Test |
| FR-005-AC-5 | Given a registry read failure while rendering an inbox, the messages are printed signed with bus names and the command exits 0. | Test |
| FR-005-AC-6 | Given a lead whose recorded tab count is 0 (unknown) at workspace `EA`, tab `code` on `hub`, it is printed `hub:EA:code`. | Test |
| FR-005-AC-7 | Given a lead whose address is 97 bytes long, it is printed by bus name; given one whose address is exactly 96 bytes, it is printed as its address. | Test |
| FR-005-AC-8 | Given `stale` at `hub:EA:code` recording a tab count of 1 and `fresh` at `hub:EA:plan` recording 2, they are printed `hub:EA:code` and `hub:EA:plan`; given `stale` alone, it is printed `hub:EA`; given the other lead on `node-a`, `stale` is still printed `hub:EA`. | Test |
| FR-005-AC-9 | For every registry snapshot, the address book built in time linear in the number of leads prints every lead as the pairwise definition of AC-1 and AC-8 does, and building it for 10,000 leads takes under 5 s. | Test |

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md)
- Delivered by AGE-2023.
- Gap closed by AGE-2026: one rule, `Place::address` for the shown address and
  `relay_core::duplicated` for shared addresses, used by every listing.
- AGE-2028: the tab rule needs the registry (AC-8), so the `AddressBook` is the
  one place it lives and `who` and `whoami` take their address column from it.
  `Registration::address`, which could not know the registry, is removed.
