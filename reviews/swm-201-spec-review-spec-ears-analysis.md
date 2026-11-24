---
id: SR-1867
title: "EARS review of SWM-201 W1 lane ground truth spec PR #52"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay; PR #52; spec/functional/FR-017-hook-never-fails-the-turn.md, spec/functional/FR-067-persistent-delivery-receipts.md, spec/functional/FR-068-hook-activity-events.md, spec/functional/FR-069-host-collector.md, spec/functional/FR-070-collector-source-health.md, spec/functional/FR-071-order-ack-deadline-and-timeout.md, spec/non-functional/NFR-001-hook-time-budget.md"
review_set: subset
---
# SR-1867: EARS review of SWM-201 W1 lane ground truth spec PR #52

## Summary

Ticket: SWM-201. New and edited requirement statements checked for EARS shape with `quire validate` grammar diagnostics plus manual reading. Most statements are well-formed event, unwanted-behaviour or ubiquitous statements with a named subject (hook handler, collector, watcher, relay). Three new statements are agentless or compound. The NFR-001 line 27 warning is pre-existing and out of scope.

## Scope examined

- `FR-067` new Acted and TimedOut statements (examined).
- `FR-068` all statements (examined).
- `FR-069` all statements (examined).
- `FR-070` all statements (examined).
- `FR-071` all statements (examined).
- `FR-017` new statement (examined); `NFR-001` new paragraph (examined).

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | Agentless passive: the order-level TimedOut receipt SHALL be recorded names no component (FR-071 says relay-watcher); quire flags ears:unclassifiable and missing-subject. | spec/functional/FR-067-persistent-delivery-receipts.md:69 |
| FND-002 | low | Agentless passive: A series SHALL be exported for every source names no component; quire flags quality:agentless-passive. | spec/functional/FR-070-collector-source-health.md:23 |
| FND-003 | low | The new NFR-001 paragraph is one sentence with three obligations (budget, ordering after mail work, drop on spent budget) and a redundant parenthetical; split into three statements so each maps to its benchmark row. | spec/non-functional/NFR-001-hook-time-budget.md:66 |

## Verdict

PASS with low findings: wording fixes only, none blocks merge.

## New findings (disposition pass 2)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-004 | low | The new FR-072 stale-host statement carries three SHALLs (refuse with host_registry_stale, raise the probe defect, never accept silently); quire flags ears:non-singular. Split it. | spec/functional/FR-072-ordered-lane-assignment-list.md:46 |

## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | The relay-watcher SHALL record the order-level `AckExpired` receipt; quire no longer flags the line. |
| FND-002 | fixed | The watcher SHALL export `ix_fleet_probe_ok` for every expected source on every push; the agentless passive is gone. |
| FND-003 | fixed | NFR-001 activity paragraph split into four EARS statements, each with the hook handler as subject. |
| FND-004 | fixed | FR-072 stale-host rule split into three single-SHALL statements; quire no longer flags ears:non-singular. |
