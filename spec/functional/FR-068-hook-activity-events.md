---
id: FR-068
title: "Hooks report lane activity events to the bus"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-009"
    type: "implements"
---
# FR-068: Hooks report lane activity events to the bus

## Description

When a harness hook fires for a registered lead, the hook handler SHALL record one
compact activity event on the bus, so that lane activity is known from the lane's own
reports. The contract covers `agent-msg hook claude` and `agent-msg hook codex`; it does
not depend on how a harness plugin wires the handler to its events, except for the
registration requirement below. The recording is telemetry: it never delays or fails the
agent's turn, and it never changes a tool permission.

## Inputs

- A harness hook payload for one of the events: `PreToolUse`, `PostToolUse`,
  `UserPromptSubmit`, `Notification`, `Stop`, `SessionStart`, `SessionEnd`. A harness that
  does not emit an event (for example a Codex build without `Notification`) produces no
  event of that kind.
- The lead's identity and place as the hook already derives them.

## Outputs

- One event appended to the activity stream and the lead's last-activity record on the bus.
- Nothing on stdout for `PreToolUse`, `Notification` and `SessionEnd`.

## Behavior

### Event schema

- The hook handler SHALL write each event with these fields: `addr` (the lane address
  `server:workspace:tab`, the join key for every consumer), `session` (the harness session
  id), `ts` (Unix milliseconds on the writing host), `kind`, `cwd`, `branch` and `harness`
  (`claude` or `codex`). The relay SHALL also record the Redis server time with each write.
- `kind` SHALL be one of `pre_tool`, `post_tool`, `prompt`, `blocked`, `idle`, `stop`,
  `session_start`, `session_end`.
- For `pre_tool` and `post_tool` the event SHALL carry `tool`, the tool name only, never
  tool input, tool output or a command line.
- For `Notification`, the event SHALL be `blocked` with `reason` `permission` when the
  harness reports a permission prompt, `idle` when it reports an idle prompt, and no event
  for any other notification type.
- For `Stop`, the event SHALL carry `last_class`: `question` when the final assistant
  message, after trailing whitespace, closing quotes, closing brackets and emphasis marks
  are removed, ends with `?`, or its last paragraph contains one of the phrases `should I`,
  `shall I`, `want me to`, `would you like`, `do you want`, `please confirm`, `which
  option` (case-insensitive); `done` for any other available message; and `unknown` when the
  message is unavailable. The classification SHALL read the final message only, SHALL NOT
  store any of its text, and SHALL NOT use the network.
- For `session_start`, the event SHALL carry `role` and `skill_versions`: the role-spec
  and skill versions the session loaded, as `name=version` pairs, at most 16 pairs, each
  name at most 64 bytes and each version at most 32 bytes; an unknown version SHALL be
  recorded as `unknown`, not omitted.
- `branch` SHALL be the checked-out branch of `cwd` at event time, or empty when `cwd` is
  not a git work tree or the branch cannot be read within the time budget.
- Subagent hook calls (payloads carrying an agent id) SHALL NOT produce events, following
  [FR-019](./FR-019-subagents-never-use-the-bus.md).

### Hook output

- For `PreToolUse`, `Notification` and `SessionEnd`, the hook handler SHALL print nothing
  on stdout and SHALL NOT print a permission decision of any kind (approve, deny, ask or
  block), because stdout of `PreToolUse` is the tool permission channel.

### Bounds and retention

- Every string field SHALL be at most 256 bytes (`cwd` at most 1,024), truncated on a
  character boundary, and control characters SHALL be removed. An event SHALL be at most
  4 KiB encoded.
- The activity stream SHALL be capped at 20,000 events and SHALL drop events older than
  24 hours; the writer SHALL trim in the same request as the append.
- The last-activity record SHALL hold, per lead, the newest event of each kind and the
  newest event overall, with an expiry of 7 days, so last activity survives stream
  trimming.
- Event writes SHALL be one pipelined request that adds at most one round trip to the
  hook's existing request count.

### Failure behavior and backoff

- If the event cannot be written (Redis unreachable, a Redis error, or the time budget
  spent), then the hook handler SHALL drop the event, record one hook failure under
  [FR-030](./FR-030-hook-failures-are-spooled-and-reported.md) with stage `activity`, and
  continue with its other work and exit 0. The handler SHALL NOT spool dropped events for
  replay, because a replayed event would report stale activity as fresh.
- After a recorded emit failure, the hook handler SHALL skip event writes for a backoff
  period that starts at 5 seconds, doubles after each further failure and is capped at 5
  minutes, so that an outage does not add the write's time to every tool call. A
  successful write SHALL end the backoff. The backoff SHALL be kept per host in the hook's
  local state and SHALL NOT affect mail delivery.
- The hook handler SHALL count each skipped and each dropped event in that local state,
  and `relay-watcher` SHALL export the counts as `ix.relay.hook.activity_skipped`
  ([FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md)), so an outage is a visible
  defect.
- The hook handler SHALL attempt the event write after mail delivery work, within the
  event's budget in [NFR-001](../non-functional/NFR-001-hook-time-budget.md), so that
  telemetry never consumes the window mail needs.
- The hook handler SHALL NOT write events for a process that is not a registered or
  derivable lead (no bus name and no herdr pane).

### Plugin registration

- The Claude and Codex plugins SHALL register the hook handler for every event in the
  input list that the harness supports, with a harness timeout of at least the event's
  budget plus 1 second. This is implemented by the plugin lane, coordinated with
  draco:ix-relay:dashboard, and this requirement states the contract, not the manifest
  layout.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-068-AC-1 | Given a `PostToolUse` payload for a registered lead, the stream gains exactly one event with `addr`, `session`, `ts`, `kind=post_tool`, `cwd`, `branch`, `harness` and `tool` equal to the tool name, and no tool input or output text anywhere in the event. | Test |
| FR-068-AC-2 | Given the same payloads for `claude` and `codex`, both harnesses produce events of the same schema, differing only in `harness`, and every event's `addr` is the lane address. | Test |
| FR-068-AC-3 | Given `Notification` payloads of type permission prompt, idle prompt and another type, the events are `blocked/permission`, `idle`, and none. | Test |
| FR-068-AC-4 | Given final messages ending in `?`, ending in `?"` or `?**`, ending in a statement whose last paragraph contains `want me to`, ending in a statement with none of the phrases, and unavailable, `last_class` is `question`, `question`, `question`, `done` and `unknown`; no event contains any message text, seeded with a secret marker. | Test |
| FR-068-AC-5 | Given a `SessionStart` with 20 skill versions and an overlong name, the event holds at most 16 pairs within the length bounds; a skill without a version is recorded `unknown`. | Test |
| FR-068-AC-6 | Given a field of 10 KiB, a control character and a multi-byte character at the cut, the stored event is at most 4 KiB, holds no control character and is valid UTF-8. | Test |
| FR-068-AC-7 | Given 20,001 appended events, and separately events aged 25 hours, the stream holds at most 20,000 and none older than 24 hours, while the lead's last-activity record still shows its newest event of each kind. | Test |
| FR-068-AC-8 | With Redis accepting connections and never answering, every event kind exits 0 within its [NFR-001](../non-functional/NFR-001-hook-time-budget.md) budget, records one `activity` hook failure when Redis later answers, and replays no event. | Test |
| FR-068-AC-9 | A subagent payload, and a process with no bus name outside a herdr pane, write no event. | Test |
| FR-068-AC-10 | A `cwd` outside a git work tree gives an empty `branch`, and a `cwd` whose branch read hangs gives an empty `branch` within budget. | Test |
| FR-068-AC-11 | With Redis blackholed, after one failed write the next 5 seconds of tool-call hooks add no write time and are counted as skipped; the backoff doubles to the 5-minute cap on repeated failures; a successful write ends it; `ix.relay.hook.activity_skipped` reports the skips and drops. | Test |
| FR-068-AC-12 | A `PreToolUse`, `Notification` and `SessionEnd` call writes its event and prints nothing on stdout, including for a tool that the harness would otherwise ask about; no output contains an approve, deny, ask or block decision. | Test |
| FR-068-AC-13 | The Claude and Codex plugin registrations are parsed by a test that fails unless every supported event of the input list maps to a registered hook with a timeout of at least its budget plus 1 second. | Test |

## Dependencies

- **Upstream**: [US-009](../usecase/US-009-know-what-each-lane-is-doing-from-its-own-reports.md)
- **Related**: [FR-017](./FR-017-hook-never-fails-the-turn.md),
  [FR-030](./FR-030-hook-failures-are-spooled-and-reported.md),
  [NFR-001](../non-functional/NFR-001-hook-time-budget.md),
  [NFR-002](../non-functional/NFR-002-bounded-resources.md).
- Consumed by [FR-069](./FR-069-host-collector.md) (cwd to lane map) and
  [FR-067](./FR-067-persistent-delivery-receipts.md) (`Acted`).
- Implementation dependency: coordinate with the plugin lane (draco:ix-relay:dashboard),
  which owns the plugin manifests.
- The matching `_lead-bin` shell activity probes are replaced by this contract and
  [FR-069](./FR-069-host-collector.md); no Python and no logic-bearing shell.
