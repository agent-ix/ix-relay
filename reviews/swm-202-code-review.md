---
id: SR-1871
title: "Code review — hook activity event emission (PR 54)"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay PR54 head b558490; crates/agent-msg/src/activity.rs, crates/agent-msg/src/activity_state.rs, crates/agent-msg/src/hook.rs, crates/agent-msg/src/hook_run.rs, crates/agent-msg/src/hook_run/tests.rs, crates/agent-msg/src/main.rs, crates/agent-msg/src/spool.rs, crates/agent-msg/tests/round_trips.rs, crates/relay-core/src/activity.rs, crates/relay-core/src/store/activity.rs, crates/relay-core/src/store/keys.rs, crates/relay-core/src/store/lua/activity.lua, crates/relay-core/tests/activity.rs, spec/spec.md"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-068", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-017", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-030", type: references }
  - { target: "ix://agent-ix/ix-relay/NFR-001", type: references }
---

# SR-1871: Code review — hook activity event emission (PR 54)

## Summary

Ticket: SWM-202. This review covers PR 54 at head b558490 against FR-068, FR-017 and NFR-001 on origin/main (ded300c). It runs the code-review method with the rust-review lane folded in. The diff is Rust plus one Lua script and one spec table row.

Evidence:

- `cargo test -p agent-msg -p relay-core --no-fail-fast` passes against a throwaway redis:7, and the Redis-backed `relay-core/tests/activity.rs` ran rather than skipped. `cargo clippy -p agent-msg -p relay-core --all-targets -D warnings` and `cargo fmt --check` are clean.
- The debug binary was timed, from spawn to exit, against a refused port and against a paused (accepting, never answering) Redis, reusing one config directory per scenario:
  - Refused: 6 `PreToolUse` calls each took 13–21 ms. Each one left a `connect` spool record, and no `spool/activity.json` was ever created.
  - Hung: 3 `PreToolUse` calls each took 321–325 ms. 3 `SessionEnd` and 3 `PostToolUse` calls each took 915–941 ms. Every call left a `derive`/`timeout` spool record, and still no `activity.json` existed.

## Verdict

**FAIL**, because of one high finding.

The local backoff never engages in a real Redis outage. During an outage every tool call keeps paying the network timeout and is never counted as skipped or dropped.

What is sound:

- `PreToolUse`, `Notification` and `SessionEnd` never pop mail and print nothing, so no permission decision is possible (`respond` returns `None`, and `deliver_mail` is not called).
- The event holds only tool names, a `last_class` and bounded strings. Tool input and output are never decoded, the Stop message is classified and dropped, and failure records carry no text.
- The bounds hold: control characters are stripped, cuts fall on character boundaries, there are at most 16 skill pairs, and events fit within 4 KiB including the relay's additions.
- The Lua script appends with `MAXLEN =` and trims with `MINID` using Redis server time, in the same `EVAL`. It renews the 7-day `EXPIRE` on every write. The Redis tests cover all three.
- The `.git/HEAD` read is bounded by a stranded thread with a 100 ms (or remaining-budget) `recv_timeout`. A FIFO HEAD is tested.
- `activity.json` writes go to a per-pid temporary file and are then renamed, so no torn file is possible, and a corrupt file reads as defaults without breaking later hooks.
- The existing four events keep their mail order. The activity write comes after the acknowledgement, and the existing round-trip pins rise by exactly the one `EVAL`.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | The backoff never engages in a Redis outage. `emit_activity` is the only place that checks `skip()` and calls `failed()`, and it runs only after connect, the session probe, derivation and mail have all succeeded. A refused or hung Redis fails earlier, at `connect` or `derive`, and returns. So `activity.json` is never written, nothing is counted as skipped or dropped, and the next call pays the full timeout again. Measured: `PreToolUse` takes about 320 ms and `SessionEnd`/`Notification` about 920 ms on every call against a hung Redis, each leaving a `derive` spool record. The new activity-only events (`PreToolUse` on every tool call) now open a connection where they previously did nothing. FR-068 says the backoff exists "so that an outage does not add the write's time to every tool call" (AC-11). The fix is to check the backoff before connecting for activity-only events, and to call `failed()` on any connect, derive or budget failure of a call that had a pending event. | crates/agent-msg/src/hook_run.rs:713-727, crates/agent-msg/src/hook_run.rs:936-975 |
| FND-002 | medium | A registered lead whose registration yields no lane address writes nothing and is counted nowhere. `record_activity` returns `Ok(false)`, and `emit_activity` treats every `Ok(_)` as success: it calls `succeeded()` (which also ends a running backoff) and records nothing. The lead's intent is that every registered lane is visible, so this silence should at least be counted, for example as a dropped event or its own count in `activity.json`, and exported with the other counts. | crates/agent-msg/src/hook_run.rs:967-972, crates/relay-core/src/store/lua/activity.lua:6-12 |
| FND-003 | medium | The Lua script builds `addr` from any stored `place`. It never checks that the place is verified. The Rust definition of a lead's address (`Registration::full_address` → `verified_place`) accepts a place only when `herdr_ids.session_id` and `herdr_ids.pane` match the registration's own session and pane. Otherwise the placement is `Unverifiable` and has no address. So activity can be attributed to a stale or unverified lane, which is exactly the place the address book refuses to print. The script also allows 256-byte addresses, while `AddressBook` rejects anything longer than `MAX_LABEL_BYTES` (96). The consumer join key and the printed addresses can therefore disagree. | crates/relay-core/src/store/lua/activity.lua:8-12, crates/relay-core/src/message.rs:161-191 |
| FND-004 | medium | The `session_start` role and skill versions are not the versions the session loaded. They come from `AGENT_MSG_ROLE` and `AGENT_MSG_SKILLS`, which no launcher or plugin sets, so every event reads `role=unknown`. Because of `skip_serializing_if = "Vec::is_empty"`, `skill_versions` is omitted entirely, although FR-068 says `session_start` SHALL carry it. The lead's decision: the skill version is the installed dev-team plugin version, read from the Claude plugin install manifest (`~/.claude/plugins/installed_plugins.json`, `plugins["dev-team@agent-ix"][].version`) or the Codex equivalent. The role comes from `AGENT_MSG_ROLE` set by the launcher, and `unknown` is allowed only when it is unset. | crates/agent-msg/src/activity.rs:21-24, crates/agent-msg/src/activity.rs:82-92, crates/relay-core/src/activity.rs:131 |
| FND-005 | medium | A failed mail call can now leave two failure records. When the pop or the write fails late (for example a pop that times out), `emit_activity` still runs with no budget left and records `ActivityBudget`, and then `mail?` records the mail stage. FR-030-AC-8 requires exactly one record per failing call. The PR changes the test helpers rather than the behaviour: `all_records` and the FR-030 proptest now filter out `stage: activity`, so the one-record checks can no longer see the second record. Either the spec has to say that the activity record is extra, or the event should be dropped silently (counted in `activity.json`) when the mail path has already failed. | crates/agent-msg/src/hook_run.rs:794-811, crates/agent-msg/src/hook_run/tests.rs:497-501, crates/agent-msg/src/hook_run/tests.rs:2176-2183 |
| FND-006 | low | `activity.json` is updated by an unlocked read-modify-write. Concurrent hooks on one host (four or more lanes) each read the counts, add one and rename their own copy, so increments are lost and `skipped`/`dropped` under-report. These counts are the outage signal FR-068 asks for. The rename makes the file safe from tearing, but the counts are not exact. A non-blocking `try_lock` on a sidecar file, or per-pid count files summed by the reader, would keep the counts exact without ever making a hook wait. | crates/agent-msg/src/activity_state.rs:71-122 |
| FND-007 | low | The activity write now runs before `flush_health`. On an `EVAL` that stalls, `set_timeout(left)` gives the write the whole remaining budget, so that call ships no spooled failure records. Telemetry thus takes the window that FR-030 shipping had before. Capping the write's timeout, or flushing first on calls with a non-empty spool, would keep health shipping ahead of activity telemetry. | crates/agent-msg/src/hook_run.rs:800-812, crates/agent-msg/src/hook_run.rs:962-966 |
| FND-008 | low | `_cx` keeps its unused-binding underscore although `try_run` now reads `_cx.harness` and `_cx.activity`. Rust style reserves the leading underscore for bindings that are intentionally unused. | crates/agent-msg/src/hook_run.rs:713, crates/agent-msg/src/hook_run.rs:805 |
