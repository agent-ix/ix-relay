---
id: SR-1865
title: "Integrity review of SWM-201 W1 lane ground truth spec PR #52"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay; PR #52; spec/functional/FR-017-hook-never-fails-the-turn.md, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md, spec/functional/FR-067-persistent-delivery-receipts.md, spec/functional/FR-068-hook-activity-events.md, spec/functional/FR-069-host-collector.md, spec/functional/FR-070-collector-source-health.md, spec/functional/FR-071-order-ack-deadline-and-timeout.md, spec/non-functional/NFR-001-hook-time-budget.md, spec/spec.md, spec/usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md"
review_set: subset
---
# SR-1865: Integrity review of SWM-201 W1 lane ground truth spec PR #52

## Summary

Ticket: SWM-201. Checked identifier uniqueness, cross-FR consistency of names, attributes and states, coverage of the SWM-202..207 acceptance criteria, and testability of every new AC. `quire validate` passes on all ten files (EARS warnings only, see the EARS review). `quire matrix` was run by this review and exposes a duplicate AC id that binds existing tests to an unbuilt criterion. Traceability US-009 -> FR-068..071 and the FR-067/FR-045 extensions is complete; every new AC carries a verification method.

## Scope examined

- `FR-067-AC-8` (examined, both rows), `FR-067-AC-9`, `FR-067-AC-10`: Acted and TimedOut receipts.
- `FR-068-AC-1`..`FR-068-AC-10` (examined): hook activity events.
- `FR-069-AC-1`..`FR-069-AC-10` (examined): host collector.
- `FR-070-AC-1`..`FR-070-AC-6` (examined): collector source health.
- `FR-071-AC-1`..`FR-071-AC-8` (examined): order ack deadline.
- `FR-045-AC-29`, `FR-045-AC-30` and the nine new catalogue rows (examined).
- `FR-017` new statement and `FR-017-AC-5` (examined); `NFR-001` new paragraph and benchmark row (examined).
- `US-009` (examined); SWM-202..207 ticket acceptance criteria (context_only).

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | FR-067 now has two rows with id FR-067-AC-8 (new Acted AC inserted before the existing regression AC). quire matrix binds the 11 existing receipt tests tagged FR-067-AC-8 to the unbuilt Acted criterion, reporting it tagged: false coverage. Renumber the new rows AC-9..AC-11 (or the old one) and update FR-071/US-009 references. | spec/functional/FR-067-persistent-delivery-receipts.md:123,126 |
| FND-002 | medium | ix.relay.orders.unacted has attribute lead in the FR-045 catalogue but is broken down by recipient in FR-071; one name, one attribute set. | FR-045 catalogue, FR-071 Behavior |
| FND-003 | medium | Class vocabularies do not map: FR-068 blocked reason permission, FR-069 pane classes queued_question and wait_loop have no stated mapping into ix.fleet.lane.blocked reasons (dialog, login, usage_limit, question) or lane.state. wait_loop (an agent sleeping in a poll loop, an overnight failure class) is detected and then exported nowhere. | FR-045 catalogue, FR-068, FR-069 pane-tail scan |
| FND-004 | medium | FR-069 says a host with no temperature sensor reports the source unavailable, never zero; FR-070 has only 1/0 and sets temperature without a sensor to 0. Contradiction; with FR-070 as written ProbeFailing fires permanently on draco (macOS) and terra (WSL has no hwmon). Needs a third state (e.g. unsupported excluded or separate label) so unsupported is distinct from failed. | FR-069-AC-4, FR-070 Behavior, FR-070-AC-2 |
| FND-005 | medium | FR-071 says an order is a message sent with a deadline and ordinary messages are unchanged, but also gives every send a default 5-minute deadline (AC-1: no option means 5 minutes). Either every message is an order (every reply/FYI can TimedOut and escalate) or none; the spec must pick one. | FR-071 Description, FR-071-AC-1 |
| FND-006 | medium | SWM-202 AC1 (plugins/claude/hooks/hooks.json and plugins/codex/hooks.json extended) is explicitly left out (plugin lane owns manifests) and no requirement or AC states that both plugins register PreToolUse, Notification and SessionEnd. Without it FR-068 can be fully green while no event is ever emitted in production. | FR-068 Dependencies, SWM-202 |
| FND-007 | medium | SWM-203 (runs natively on all hosts, terra inside WSL) and SWM-205 (series visible in Prometheus on all hosts) are covered only by fixtures (FR-069-AC-3). No AC demonstrates the collector and series on each real host, which is the exact gap of the overnight incident. Add a Demonstration AC per host (or move it to SWM-203/205 explicitly). | FR-069-AC-3, FR-045-AC-29 |
| FND-008 | medium | The new FR-017 statement (PreToolUse, Notification and SessionEnd print nothing) has no AC, and FR-017-AC-5 still says each of the four hook events. PreToolUse stdout is a tool permission decision channel, so empty stdout on those events needs its own assertion. | FR-017, FR-017-AC-5 |
| FND-009 | medium | FR-068 last_class question is defined as ending in a question addressed to the user or asking for a decision; the second half has no rule and AC-4 tests only a trailing question. Two implementers will classify differently; give the deterministic rule (patterns) and test each. | FR-068 Stop, FR-068-AC-4 |
| FND-010 | low | Editorial: FR-067 Dependencies gains an unheaded Extended-by bullet above Upstream with a stray blank line; NFR-001 paragraph repeats SessionEnd and Notification 900 ms in a parenthetical. | FR-067 Dependencies, NFR-001 |

## Verdict

CHANGES REQUESTED. FND-001 must be fixed before merge (it corrupts the matrix today). The rest are consistency and coverage gaps that are cheap to close in this PR. Testability is otherwise good: every new AC names fixtures, injected clocks or seeded secret markers and can fail. On the matrix question: running `quire matrix` is required for this spec PR, not to close coverage (every new AC is expected untagged until SWM-202..207 land, and spec.md lists them as not built) but because it is the check that catches id collisions like FND-001; SWM-201 AC3 (test-matrix gaps closed) cannot be met by a spec-only PR and should be read as satisfied by SWM-202..207.

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-011 | medium | FR-072 refuses a write naming a lane address not in the registry, but the registry is defined in FR-069 as each host's collector state. The orchestrator writes from one host for lanes on all hosts, so the spec must say where a fleet-wide registry is stored (e.g. each watcher publishes its registry to Redis with an expiry) and what happens when a host's watcher is down (refuse, or accept and flag). | FR-072 Behavior, FR-072-AC-4, FR-069 Lane registry |
| FND-012 | low | FR-071 states an obligation on ix-board (`fleet assign` SHALL treat an order as accepted only on a focus match) and FR-071-AC-11 verifies it by Demonstration, but this repository cannot build or test that consumer. Keep the relay side here and move the acceptance rule and its AC to the ix-board fleet spec. | FR-071 Order acceptance, FR-071-AC-11 |
| FND-013 | low | FR-068-AC-13 describes a test that parses the plugin registrations and fails, but its verification method is Inspection; make it Test. | FR-068-AC-13 |

## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | New rows renumbered FR-067-AC-9..AC-12; quire matrix shows one FR-067-AC-8 (the original regression AC) and no duplicate AC ids. |
| FND-002 | fixed | FR-045 catalogue and FR-071 both use `recipient` (the lane address). |
| FND-003 | fixed | FR-045 lists seven blocked reasons; FR-069 maps every pane class and hook cause; FR-069-AC-12 enumerates them. |
| FND-004 | fixed | Host profile with `probe_unavailable`; FR-069-AC-4 and FR-070-AC-7 separate unavailable from failed. |
| FND-005 | fixed | Orders are explicit (`send --order` or `fleet assign`); FR-071-AC-2 asserts that plain messages never expire. |
| FND-006 | fixed | FR-068 Plugin registration requirement and FR-068-AC-13. |
| FND-007 | fixed | FR-069-AC-15 Demonstration on draco, titan, luna and terra (WSL), queried in Prometheus. |
| FND-008 | fixed | FR-017-AC-5 lists all seven events; FR-017-AC-8 and FR-068-AC-12 assert empty stdout with no permission decision. |
| FND-009 | fixed | Deterministic rule (trailing ? after stripping, fixed phrase list in last paragraph); FR-068-AC-4 tests each. |
| FND-010 | fixed | FR-067 Dependencies has a headed Extended-by bullet; NFR-001 paragraph split into four statements. |
| FND-011 | fixed | FR-069 adds a fleet registry: each watcher publishes its lane list to Redis under a 90 s lease; expired hosts' lanes stay listed as stale; FR-072 refuses writes to stale lanes with host_registry_stale (FR-069-AC-16, FR-072-AC-9). |
| FND-012 | fixed | FR-071-AC-11 removed; Dependencies names ix-board#86 FR-035 as owner of acceptance by focus match. The FR-071 Behavior text still restates the consumer rule as a SHALL, now without an AC here. |
| FND-013 | fixed | FR-068-AC-13 verification is Test. |
