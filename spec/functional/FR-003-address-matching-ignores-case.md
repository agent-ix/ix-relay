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

When an address is used as a query, the resolver shall match only relay-registered
leads whose Herdr placement is verified for that harness session. Herdr's
workspace, tab, and pane hierarchy defines the address. Empty panes and Herdr
agents without a relay registration are not recipients and do not make an
otherwise unique query ambiguous.

## Behavior

- The resolver shall compare the server segment after machine-name normalisation.
- The resolver shall compare workspace and tab segments without regard to letter
  case.
- The resolver shall compare an optional pane segment against Herdr's pane ID,
  after applying the address segment cleaning rule.
- A `server:workspace` query shall resolve to its sole verified registered
  recipient across the workspace's tabs; when more than one such recipient
  matches, it shall report ambiguity and list their Herdr addresses.
- A `server:workspace:tab` query shall resolve to its sole verified registered
  recipient in that tab; when more than one matches, it shall report ambiguity
  and list pane-qualified Herdr addresses.
- A `server:workspace:tab:pane` query shall match the Herdr pane ID exactly.
- Tab and pane segments are optional specificity. The resolver shall not reject
  a query merely because Herdr reports other tabs or panes that have no
  registered recipient.
- The resolver shall never select a lead on a different server from the query's,
  whatever its workspace label.
- The resolver shall never select a lead whose registration has no recorded place.

When Herdr topology cannot be read, ix-relay SHALL NOT create or refresh
placement evidence from that failed read; the last successfully verified
placement remains cached until a readable Herdr snapshot confirms or contradicts
it. A missing or conflicting session-to-pane join makes the affected
registration non-routable by address. Name and explicit alias delivery remain
available. See [FR-047](./FR-047-herdr-placement-must-be-verified.md).

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-003-AC-1 | Given a lead at `hub:EA:code`, each of `hub:ea:code`, `HUB:EA:CODE` and `Hub.local:Ea:Code` resolves to it. | Test |
| FR-003-AC-2 | Given exactly one lead in workspace `research` on `node-a`, both `node-a:research` and `node-a:research:1` resolve to it when its Herdr IDs are cached. | Test |
| FR-003-AC-3 | Given registered leads in tabs `code` and `plan` of `hub:EA`, resolving `hub:EA` is refused as ambiguous and names both verified candidates. | Test |
| FR-003-AC-4 | Given workspace `EA` on both `hub` and `node-a` with one lead each, `node-a:EA` resolves to the node-a lead and never the hub lead. | Test |
| FR-003-AC-5 | For every registry and every query address, every lead the query selects has the query's server. | Test |
| FR-003-AC-6 | Given an empty Herdr pane or a Herdr agent without a relay registration, a sole registered recipient in the requested workspace/tab still resolves. | Test |
| FR-003-AC-7 | Given two registered workspaces on one server with the same workspace and tab labels but distinct Herdr IDs, either matching address is refused as ambiguous and names both candidates with their IDs. | Test |
| FR-003-AC-8 | Given several verified registered leads across tabs and panes, broad queries are valid and report only matching registered candidates; a pane-qualified address resolves exactly its matching lead. | Test |

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md),
  [FR-001](./FR-001-address-grammar-and-cleaning.md)
- Delivered by AGE-2022.
