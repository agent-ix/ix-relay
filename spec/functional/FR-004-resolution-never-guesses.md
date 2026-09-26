---
id: FR-004
title: "Resolution never guesses"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "implements"
---
# FR-004: Resolution never guesses

## Description

When the resolver resolves a pane id or an address, the resolver shall return a
single-lead target only if exactly one registered lead matches.

If more than one lead matches, then the resolver shall return an ambiguous-target
error listing every matching lead.

If no lead matches, then the resolver shall return an unknown-target error naming
the most specific likely cause and the closest known names and addresses.

## Outputs

- Ambiguous-target error: the typed input and every matching lead, each with its
  bus name and full address when it has one, sorted by bus name.
- Unknown-target error: the typed input, one cause, and at most five suggestions,
  closest first.

## Behavior

- If more than one registration matches, then the resolver shall list all of
  them and shall not choose one.
- If an address matches no registration and no registration is on the query's
  server, then the resolver shall report "unknown server" with the servers that
  have leads.
- If an address matches no registration, some registration is on the server and
  none is in the workspace, then the resolver shall report "unknown workspace"
  with the workspaces on that server.
- If an address matches no registration and some registration is in the
  workspace, then the resolver shall report "unknown tab" with the tabs of that
  workspace that have a lead.
- The resolver shall draw suggestions only from registered bus names, aliases and
  full addresses that contain the input, are contained in it, or are within
  `max(2, len/4)` single-character edits of it, all compared ignoring case.
- If resolution returns any error, then `agent-msg send` shall exit non-zero
  without adding anything to any inbox or to the history log.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-004-AC-1 | Every single-lead target returned by resolution, for any registry and typed input, is the only registration matching the first form that matched. | Test |
| FR-004-AC-2 | Given two registrations on the sender's machine `hub` with pane id `wB:p1`, resolving `wB:p1` returns an ambiguous-target error with exactly those two leads, and a third registration with that pane id on `node-a` is not among the candidates. | Test |
| FR-004-AC-3 | Given two leads registered at the same full address, resolving that address returns an ambiguous-target error whose candidates are both leads sorted by name, each with its full address. | Test |
| FR-004-AC-4 | Given an ambiguous or unknown target, `agent-msg send` exits non-zero, every inbox length is unchanged, and the history log gains no entry. | Test |
| FR-004-AC-5 | Given leads only on `hub` and `node-a`, resolving `node-b:EA:code` reports "unknown server" listing exactly `hub` and `node-a`. | Test |
| FR-004-AC-6 | Given only workspace `EA` on `hub`, resolving `hub:EAA:code` reports "unknown workspace" listing `EA` and suggests `hub:EA:code`. | Test |
| FR-004-AC-7 | Given tabs `code` and `plan` in `hub:EA`, resolving `hub:EA:nope` reports "unknown tab" listing exactly `code` and `plan`. | Test |
| FR-004-AC-8 | Given registered `ea-code`, resolving `ea-cod` reports "not registered" with `ea-code` as the first suggestion, and resolving `EA-CODE` also suggests `ea-code`. | Test |
| FR-004-AC-9 | For every registry and typed input, an unknown-target error carries at most five suggestions and every suggestion is a registered name, alias or full address. | Test |

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md),
  [FR-002](./FR-002-target-resolution-priority.md)
- Delivered by AGE-2024.
