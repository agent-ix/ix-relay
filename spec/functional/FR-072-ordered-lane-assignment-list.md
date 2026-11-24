---
id: FR-072
title: "Each lane has an ordered assignment list written by the orchestrator"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-009"
    type: "implements"
  - target: "ix://agent-ix/ix-relay/FR-066"
    type: "depends_on"
---
# FR-072: Each lane has an ordered assignment list written by the orchestrator

## Description

The relay SHALL hold, per lane, an ordered list of assigned tickets that an authorized
orchestrator command writes and a consumer such as ix-board reads, with a bounded
transition history. This differs from the current work state of
[FR-066](./FR-066-bounded-work-history.md), which holds one ticket per agent that the agent
reports itself; the assignment list is the plan, written by the orchestrator and not by the
lane.

## Inputs

- An orchestrator command (`fleet assign`, through `relay-core`) naming a lane address and
  an ordered list of tickets, each with a text reference.
- A read request for one lane or for all lanes.

## Outputs

- The lane's current ordered list, with a version and the time of the last write.
- A bounded page of assignment transitions.

## Behavior

- The relay SHALL key an assignment list by the lane address `server:workspace:tab`.
- A list SHALL hold at most 50 tickets in order. Each entry SHALL hold a ticket id (at most
  64 bytes) and a text reference (at most 512 bytes: a link or an identifier that locates
  the ticket text; the relay SHALL NOT fetch it).
- The role check is a guard against accidental writes by lanes and is not authentication; every participant that can reach the bus is trusted, as in the trust model of the master specification.
- The relay SHALL accept a write only from an authorized orchestrator command: a caller
  holding the orchestrator role in the relay configuration. The relay SHALL refuse a write
  from a lane or an unauthorized caller, naming the refusal, and SHALL leave the list
  unchanged.
- A write SHALL be `set` (replace the list), `append`, `remove` of one ticket, or
  `reorder`, and SHALL apply atomically with its transition record. A write naming a lane
  address that is not in the fleet registry SHALL be refused by the relay.
- The relay SHALL refuse with the error `host_registry_stale` a write naming a lane of a host whose registry lease has expired (a `stale` lane, [FR-069](./FR-069-host-collector.md)).
- When the relay refuses a write as `host_registry_stale`, it SHALL raise the collector probe defect of [FR-070](./FR-070-collector-source-health.md). Until the watcher exports it as `ix_fleet_probe_ok` (SWM-277, which blocks this criterion from counting as done), the relay stores the defect and `agent-msg doctor` warns `assignment_host_registry_stale`; that warning is the interim surfacing.
- The relay SHALL NOT accept a write to a stale lane silently.
- A write MAY carry an `order_id` (at most 64 bytes, printable text): the identity of the order
  of [FR-071](./FR-071-order-ack-deadline-and-timeout.md) that carried the change. Each
  transition record of that write SHALL store it, and a write without one stores none.
- Each transition record SHALL hold the operation, the lane address, the writer, the list
  version after the write and the ticket ids, with the relay server's time. The history SHALL
  be retained and read under the bounds of FR-066 (4,096 entries, seven days, bounded
  pages, exclusive cursor).
- Any caller MAY read the lists and the history without consuming mail and without a
  registered pane; the read SHALL NOT change either.
- A lane replaced, moved or renamed under
  [FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md) SHALL keep its list under
  the new address and record a transition naming both addresses.
- A list that cannot follow its lane, because the new address already holds a list or the store
  fails, SHALL stay where it is and SHALL NOT overwrite another list. The relay SHALL record a
  `move_failed` transition naming both addresses and the reason and a defect that `agent-msg
  doctor` reports until a write to the lane at the new address settles it; a defect never expires while the list is still under the old address.
- The assignment list SHALL NOT be read as proof that the lane works on the ticket or that
  the ticket remains open; the lane's focus and tracker state are separate facts.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-072-AC-1 | Given an orchestrator `set` of three tickets on a lane address, a read returns the three in order with text references, a version and a write time, and the history holds one transition. | Test |
| FR-072-AC-2 | Given `append`, `remove` and `reorder`, the list and version follow in order, and each write adds exactly one transition holding the operation and writer. | Test |
| FR-072-AC-3 | Given a write from a lane's own session and from a caller without the orchestrator role, the write is refused with a named reason and list and history are unchanged. | Test |
| FR-072-AC-4 | Given 51 tickets, an overlong id or reference, and a lane address not in the registry, each write is refused and the list is unchanged. | Test |
| FR-072-AC-5 | Given more than 4,096 writes and history older than seven days, the history keeps its bounds and reports truncation with a retained lower bound. | Test |
| FR-072-AC-6 | Given a read of one lane and of all lanes, repeated twice, the state and history are byte-identical before and after and no inbox is popped. | Test |
| FR-072-AC-7 | Given a lane moved or renamed (including a case-only rename) to a new address, the list is readable under the new address and the history records a transition naming both addresses; given a new address that already holds a list, a store failure, or two lanes that swap tab names, no list is overwritten, a `move_failed` transition and a doctor-reported defect are recorded. | Test |
| FR-072-AC-8 | Given a lane holding an assignment and no hook events, no series or read presents the assigned ticket as worked or focused. | Test |
| FR-072-AC-9 | Given a write naming a lane of a host whose registry lease has expired, the write is refused with `host_registry_stale`, the list is unchanged and the probe defect is raised; after the host refreshes its lease the same write succeeds. | Test |
| FR-072-AC-10 | Given writes with and without an `order_id`, and one with an overlong or control-character `order_id`, each transition record holds exactly the id its write carried, and the invalid write is refused with the list unchanged. | Test |
| FR-072-AC-11 | Given `agent-msg assign` `set`, `append`, `remove`, `reorder`, `show` and `history`, the writes need the orchestrator role of the configuration and the reads need none, and each prints the list or the transitions as JSON. | Test |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-066](./FR-066-bounded-work-history.md),
  [FR-071](./FR-071-order-ack-deadline-and-timeout.md),
  [FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md).
- Consumed by ix-board `fleet assign`, `fleet plan-coverage` and `fleet tick`, outside
  this repository.
