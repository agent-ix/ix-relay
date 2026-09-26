---
id: FR-003
title: "Address matching ignores case; the tab is optional"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "implements"
---
# FR-003: Address matching ignores case; the tab is optional

## Description

When an address is used as a query, the resolver shall select every registered
lead whose full address (server, workspace and tab, with the tab always present)
has the same server, the same workspace ignoring case, and — only when the query
names a tab — the same tab ignoring case.

## Behavior

- The resolver shall compare the server segment after machine-name normalisation.
- The resolver shall compare workspace and tab segments without regard to letter
  case.
- When the query names no tab, the resolver shall select every tab of the
  matching workspace on the matching server.
- The resolver shall never select a lead on a different server from the query's,
  whatever its workspace label.
- The resolver shall never select a lead whose registration has no recorded place.

A query without a tab therefore delivers only when exactly one lead is in that
workspace; otherwise [FR-004](./FR-004-resolution-never-guesses.md) applies.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-003-AC-1 | Given a lead at `hub:EA:code`, each of `hub:ea:code`, `HUB:EA:CODE` and `Hub.local:Ea:Code` resolves to it. | Test |
| FR-003-AC-2 | Given exactly one lead in workspace `research` on `node-a`, both `node-a:research` and `node-a:research:1` resolve to it. | Test |
| FR-003-AC-3 | Given leads in tabs `code` and `plan` of `hub:EA`, resolving `hub:EA` is refused as ambiguous and names both leads. | Test |
| FR-003-AC-4 | Given workspace `EA` on both `hub` and `node-a` with one lead each, `node-a:EA` resolves to the node-a lead and never the hub lead. | Test |
| FR-003-AC-5 | For every registry and every query address, every lead the query selects has the query's server. | Test |
| FR-003-AC-6 | Given a registration with no recorded place, no address query selects it. | Test |

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md),
  [FR-001](./FR-001-address-grammar-and-cleaning.md)
- Delivered by AGE-2022.
