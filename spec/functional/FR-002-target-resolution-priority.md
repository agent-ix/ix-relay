---
id: FR-002
title: "Target resolution priority"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "implements"
---
# FR-002: Target resolution priority

## Description

When `agent-msg send` resolves the text a sender typed as the target, the
resolver shall try, in order, the broadcast word `all`, a registered bus name or
alias, a herdr pane id, and a `server:workspace[:tab]` address, and shall return
the result of the first form that matches at least one registered lead.

## Inputs

- The typed target text.
- A snapshot of the registry and the alias table.

## Outputs

- A broadcast target, a single-lead target naming a canonical bus name, or a
  resolution error ([FR-004](./FR-004-resolution-never-guesses.md)).

## Behavior

- When the typed text is exactly `all`, the resolver shall return the broadcast
  target without reading the registry.
- When the typed text is a valid bus name that is registered, or an alias whose
  canonical name is registered, the resolver shall return that canonical name.
- When the typed text is not a registered name or alias and equals the herdr
  pane id of one or more registrations on the sender's machine, the
  resolver shall resolve among those registrations only.
- The resolver shall ignore registrations on other machines when matching a
  pane id, because pane ids are unique only within one machine's herdr
  ([FR-009](./FR-009-pane-ids-scoped-by-machine.md)).
- When no earlier form matched and the typed text contains a colon, the
  resolver shall parse the typed text as an address and resolve among the
  registrations that address selects.

Address parsing is [FR-001](./FR-001-address-grammar-and-cleaning.md); address
selection is [FR-003](./FR-003-address-matching-ignores-case.md).

- If none of the above matched and the typed text contains no colon, then the
  resolver shall return an unknown-target error whose cause is "not registered".
- The resolver shall follow an alias exactly one hop.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-002-AC-1 | Given any registry, resolving `all` returns the broadcast target. | Test |
| FR-002-AC-2 | Given a registered lead `ea-plan`, resolving `ea-plan` returns `ea-plan`. | Test |
| FR-002-AC-3 | Given alias `ea` pointing at registered `ea-code`, resolving `ea` returns `ea-code`. | Test |
| FR-002-AC-4 | Given alias `ea` pointing at a name that is not registered, and no other match, resolving `ea` returns an unknown-target error and not a target. | Test |
| FR-002-AC-5 | Given one registration on the sender's machine with pane id `wC:p1`, resolving `wC:p1` returns that lead, not an address interpretation of `wC:p1`. | Test |
| FR-002-AC-6 | Given text with no `:` that is neither `all`, a registered name nor an alias, resolving it returns an unknown-target error with cause "not registered". | Test |
| FR-002-AC-7 | Resolving a name, alias, pane id or address reads no herdr state and makes no network call other than to the bus. | Test |
| FR-002-AC-8 | Given pane id `wC:p1` registered only on `node-a`, resolving `wC:p1` for a sender on `hub` falls through to address resolution and returns an unknown-target error whose cause is "unknown server". | Test |

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md)
- **Downstream**: [FR-004](./FR-004-resolution-never-guesses.md)
- Delivered by AGE-2018 (aliases), AGE-2022 (addresses) and AGE-2026 (pane ids
  scoped to the sender's machine).
