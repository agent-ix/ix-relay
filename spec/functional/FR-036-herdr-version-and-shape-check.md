---
id: FR-036
title: "herdr's version and output shape are checked before use"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-036: herdr's version and output shape are checked before use

## Description

When `relay-watcher` starts, and again when a herdr call fails to decode, the
watcher shall check that herdr's reported version is at least the minimum version
ix-relay declares and that herdr's JSON responses decode into the shapes ix-relay
expects.

## Behavior

- The check shall verify that the version is at least the minimum and that `agent
  list` and `agent get` decode into the members the watcher cannot type without:
  the pane, the workspace and tab, the agent's status and its change counter.
- The check shall not require a member herdr's own schema makes optional.
  `agent_session` is one: a pane reports it only once the harness integration has
  told herdr its session, so a healthy pane that has not reported one lacks it.
- The check shall order a pre-release before its release, so `0.9.1-rc1` is below
  `0.9.1`, and shall accept a leading `v` and a `+build` suffix.
- If a member the watcher cannot type without fails to decode, then the watcher shall type into no pane.
- If that member fails to decode, then the watcher shall notify the operator once per failing episode, naming the field.
- While that member fails to decode, the watcher shall re-check every place-refresh period.
- The watcher shall keep typing guarded per lead by the session comparison, which
  refuses a lead whose pane shows no session id.
- A pane without a session shall cost that one lead a delivery, and no other lead
  on the machine.
- The watcher shall run the check on its own thread with its own time budget, so
  a hung herdr does not delay a tick.
- If herdr is older than the minimum and its output has a good shape, then the
  watcher shall notify the operator once per failing episode naming the found and
  the required version, and shall keep delivering: that is advisory.
- If herdr cannot be run, then the watcher shall notify once per failing episode
  and keep delivering.
- If the check could not run (a timeout, a herdr server not up yet), then the
  watcher shall announce nothing, treat it as unknown everywhere (the heartbeat,
  the notification, `doctor` locally and remotely) and re-check it.
- The watcher shall show the check's result in its heartbeat
  ([FR-028](./FR-028-watcher-heartbeat.md)), and shall log one line per failing
  episode, not one per re-check.
- While the check does not pass, the watcher shall re-check it every place-refresh
  period; a decode failure shall also trigger a check, at most once in five
  seconds.
- When the check passes again, the watcher shall end the episode and resume
  delivery without a restart, so a later failure notifies again.
- `agent-msg doctor` shall run the same check and report its result
  ([FR-029](./FR-029-doctor-reports-relay-health.md)).
- The ix-relay source shall declare the minimum supported herdr version in one
  place.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-036-AC-1 | Given a herdr stub reporting a version below the declared minimum and a good shape, the check names both versions and does not block delivery. | Test |
| FR-036-AC-2 | Given a herdr stub whose `agent get` response lacks `state_change_seq`, the check reports a shape mismatch that names the missing field. | Test |
| FR-036-AC-3 | Given the stub is replaced by a supported herdr while the watcher runs, the check passes again within one place-refresh period and a later failure notifies again. | Test |
| FR-036-AC-4 | Given a supported herdr, the check raises no notification. | Test |
| FR-036-AC-5 | Across five polls of a failing herdr the watcher raises exactly one notification, and a check that could not run raises none. | Test |
| FR-036-AC-6 | Given a herdr whose `agent list` has a claude pane with no `agent_session` beside one that has it, the check passes and does not block delivery. | Test |
| FR-036-AC-7 | Given that herdr, a watcher tick types the healthy lead's mail into its pane and keeps the other lead's queued; given a member the watcher cannot type without failing to decode, the same tick types into no pane; given only an advisory finding, it delivers. | Test |
| FR-036-AC-8 | Given a check that hangs, the watcher's loop is not delayed and one check is outstanding at a time. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Delivered by AGE-2033. The minimum version is `MIN_HERDR_VERSION` in
  `relay-ops`'s `herdr_check` module, the one place it is declared. Only a
  member typing cannot work without failing to decode fails closed. An optional
  member (`agent_session`) never does: the per-lead guard already refuses a lead
  whose pane shows no id, and failing closed on it stopped delivery to every lead
  on a healthy machine whenever one claude pane had not reported a session. A
  missing binary, an old version with a good shape and a timeout are advisory.
