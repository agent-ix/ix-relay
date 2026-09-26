---
id: FR-009
title: "Pane ids are scoped by machine"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-009: Pane ids are scoped by machine

## Description

Wherever ix-relay looks up a registration by herdr pane id — the pane the process
runs in, or a pane id a sender typed — ix-relay shall consider only
registrations whose normalised machine equals the local machine, because herdr
pane ids repeat across machines.

## Behavior

- When an interactive `agent-msg` command derives its identity from the pane it
  runs in, `agent-msg` shall match only registrations on the local machine.
- When a hook carries a place over from the pane's previous registration,
  the hook handler shall match only registrations on the local machine.
- When the watcher refreshes places, checks liveness before pruning, or types into
  a pane, the watcher shall act only on registrations on its own machine.
- When a sender types a pane id as a `send` target, the resolver shall match
  only registrations on the sender's machine; a lead elsewhere is reached by
  its address ([FR-002](./FR-002-target-resolution-priority.md)).
- If more than one registration on the local machine carries the local pane id,
  then `agent-msg` shall use the one registered most recently.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-009-AC-1 | Given lead `node-a-lead` registered on `node-a` with pane `wB:p1`, an interactive `agent-msg inbox` run on `hub` in pane `wB:p1` with no bus name set does not resolve to `node-a-lead` and leaves `node-a-lead`'s inbox unchanged. | Test |
| FR-009-AC-2 | Given the same setup, `agent-msg send` run on `hub` in pane `wB:p1` never signs a message as `node-a-lead`. | Test |
| FR-009-AC-3 | Given registrations with pane `wB:p1` on both `hub` and `node-a`, a `SessionStart` hook on `hub` carries over only the hub registration's place. | Test |
| FR-009-AC-4 | For every registry and local machine, every registration the watcher refreshes, prunes or types into has the local machine. | Test |
| FR-009-AC-5 | Given pane `wB:p1` registered only on `node-a`, `agent-msg send wB:p1 hi` run on `hub` exits non-zero and queues nothing. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- AGE-2027 phase B: the scope is a type. A pane is compared only as a
  `PaneRef` (machine and pane), which `Registration::pane_ref` builds from a
  registration's two fields, so a comparison cannot leave the machine out.
- Gap closed by AGE-2026: `Relay::registrations_for_pane(machine, pane)` and
  `resolve_target(input, sender_machine, ...)` both key on the machine.
