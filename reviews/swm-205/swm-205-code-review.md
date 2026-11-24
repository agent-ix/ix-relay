---
id: SR-6106
title: "Code review — ix-relay SWM-205 fleet lane state from hook events"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-watcher/src/fleet/{feed,feed_tests,lanes,mod,pane}.rs, crates/relay-watcher/src/main.rs, crates/relay-watcher/src/otlp/{mod,relay,series}.rs, crates/relay-core/src/config.rs, docs/configuration.md, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md"
review_set: subset
---

## Summary

Ticket: SWM-205 (plus the rest of SWM-203). PR agent-ix/ix-relay#55 at `6b34f2e94c99ddc9ceb514186aaff60bb6e3baf4`, reviewed as `git diff origin/main...HEAD` against FR-045, FR-068, FR-069 and FR-070 on main. The rust-review lane is folded into this file.

Evidence: macOS (aarch64), `CARGO_BUILD_JOBS=4`, target dir inside the worktree, throwaway docker `redis:7`. `cargo clippy -p relay-watcher -p relay-core --all-targets -- -D warnings` is clean. Test results are recorded under Verdict. Linux was not re-run; the coder reports Linux clippy and tests green on terra.

## Verdict

**CHANGES REQUESTED.** There are two high findings. Both change which state the main series reports.

- In production, a pane class of `none` read after the lane's newest hook event throws the hook verdict away, and herdr's status decides instead (FND-001). This hides `stuck`, `ended`, recent `working`, `permission` and `question` on most lanes.
- A lane with no hook event still reads `working` from herdr's status, which FR-045 does not allow (FND-002).

Event ages also mix the watcher's clock with the relay's clock (FND-003), and focus_info ignores the 6-hour mapping expiry (FND-004).

These deviations and checks are clean:

- **activity_skipped reasons** (`backoff`/`failed`/`unregistered`). This matches the merged hook state in `crates/agent-msg/src/activity_state.rs`. That file keeps three counters: `skipped`, `dropped` and `unregistered`. A budget overrun calls `failed()`, so it counts as a drop. No `budget` counter exists, so the FR-045 row edit describes the real counters. The hook's `unregistered` count follows FR-068's "SHALL NOT write events for a process that is not a registered or derivable lead".
- **Reading only the `activity-last` hash.** FR-068 makes that record hold the newest event of each kind and the newest overall, with a 7-day expiry. Every lane-state, no_events and attribution rule can be decided from the newest event per kind, so the stream is not needed.
- **50-lane bound.** state, last_activity, blocked, no_events, focus_info and compile_seconds are all emitted inside `lanes.iter().take(MAX_LANES)`. `lane.dropped` counts the rest. `lanes_of` returns lanes in address order, so the exported 50 are stable from push to push. Compile time is still attributed to lanes past 50, but it is not exported.
- **6 h mapping TTL for compile attribution.** `live_cwd` uses the newest event's age against `MAPPING_TTL`, and TC-069-06 covers the 5 h and 7 h cases.
- **compile_seconds.** Each sample adds the time since the last sample, capped at two intervals. It adds nothing on the first sample or after a failed compiler read. The value is monotonic while the lane is registered.
- **focus_info cardinality.** There is one series per exported lane, at most 50. `ticket` has a bounded form (2 to 10 letters, then 1 to 6 digits). `branch` is capped at 256 bytes by FR-068. The ticket label is not unbounded per push. A new series appears only when a lane changes branch, as for any info metric.
- **ix.fleet.config.** The three windows default to 600, 1800 and 7200, are validated to 1 s to 7 d, and are used by the rules.
- **hook_events.** The rule matches FR-070 except for FND-005. A lane counts as unhealthy when it is non-idle and silent past `no_events_after`. The source reads 0 only when at least one lane is non-idle and every non-idle lane is silent, or when an activity failure is newer than every lane's newest event. A lane whose newest event is stop, idle or session_end is quiet. A herdr-working lane with no events is timed from first sight.

Tests: `cargo test -p relay-watcher -p relay-core -p agent-msg` against `IX_RELAY_TEST_URL=redis://127.0.0.1:<port>` (throwaway redis:7) runs with 0 failures, including the Redis-lane `tc_070_09_the_layout_refresh_feeds_the_collector_from_the_activity_records`.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | A pane class `none` read after the newest hook event overrides the hook verdict, and herdr's status decides. Every sample stamps each scanned pane with `now`, so in production most lanes lose `stuck`, `ended`, `working`, `permission` and `question`. | crates/relay-watcher/src/fleet/lanes.rs:139-148; crates/relay-watcher/src/fleet/mod.rs:598,619-622 |
| FND-002 | high | A lane with no hook event reads `working` when herdr says working. FR-045 `lane.state` says working comes only from hook events or a compiler, and a lane with no event is `idle`. | crates/relay-watcher/src/fleet/lanes.rs:133-138,147,163-166 |
| FND-003 | medium | Event ages compare the relay's `server_ms` with the watcher's wall clock, so host skew moves working_window, stuck, no_events and the 6 h TTL. `failure_newer` also compares a hook-host `at_ms` with relay-clock event times. Relay time is available in the same refresh, from `publish_lane_list().refreshed_ms`. | crates/relay-watcher/src/main.rs:384,400-420; crates/relay-watcher/src/fleet/feed.rs:152-153; crates/relay-watcher/src/otlp/mod.rs:379-387 |
| FND-004 | medium | `focus_info` exports the newest event's branch and ticket with no 6 h mapping expiry. A lane silent for days still reports the branch from up to 7 days ago (the `activity-last` TTL). FR-069 says the cwd and branch mapping expires after 6 h without an event. | crates/relay-watcher/src/fleet/mod.rs:708-716; crates/relay-watcher/src/fleet/feed.rs:219-229 |
| FND-005 | low | `last_state_non_idle` treats a lane whose newest event is `blocked` as quiet. FR-070 lists only stop, idle and session_end as quiet, and a permission block after a prompt with no later stop is non-idle by its own definition. The deviation is not recorded in the spec. | crates/relay-watcher/src/fleet/feed.rs:232-247 |
| FND-006 | low | The activity read is one blocking HGETALL per registry lane, for every lane (herdr view up to 2,000), sequential and unpipelined, inside the tick and outside any source budget. Over a tunnelled relay this grows with the lane count, against NFR-003's 500 ms per source. | crates/relay-watcher/src/main.rs:399-419 |
| FND-007 | low | The ticket parser takes the first `letters-digits` pair in the branch or in any cwd component, so `release-2026`, `fix-12-x` and `/x/utf-8/y` become tickets `RELEASE-2026`, `FIX-12` and `UTF-8` on focus_info. | crates/relay-watcher/src/fleet/feed.rs:264-285 |
| FND-008 | low | An unreadable or unparsable `spool/activity.json` silently drops `activity_skipped` with no log line or health signal. A missing file exports zeros, although FR-070 says the collector SHALL NOT treat a missing file as zero. | crates/relay-watcher/src/fleet/mod.rs:541-546,733-745; crates/relay-watcher/src/fleet/feed.rs:307-313 |

### Detail

**FND-001.** `scan_panes` inserts `(class, now)` for every pane it reads, and `signals()` keeps a class younger than one interval. `resolve` sets `pane_newer = p > h`, where `h = now - age` of the newest hook event. Whenever the newest event is older than the last pane read, which is the normal case, the class decides alone. A class with no reason (`none`) falls through to `busy_state` (compiler or herdr). Examples:

- A `pre_tool` open for 3 h with herdr working reads `working`, not `stuck`.
- `session_end` with herdr idle reads `idle`, not `ended`.
- A prompt 2 min ago with herdr idle reads `idle`.
- A question Stop reads `idle`, and its `blocked{question}` disappears.
- A permission `blocked` event reads blocked only if the pane catalogue also matches a dialog, and then with reason `dialog`.

FR-069 gives a newer pane class precedence only to map it to its blocked reason. FR-045's rules give `none` no weight over hook evidence. TC-045-30 and TC-069-08 never call `sample()` before `fill()`, so no pane class is present and the production path is untested. Fix: a newer pane class overrides the hook only when it maps to a reason. Otherwise the hook verdict stands. Add a test that runs a pane scan of `plain output` before `fill` for each FR-045-AC-30 lane.

**FND-002.** Both the `None` branch and the pane-newer branch return `busy_state`, which includes `herdr_status == "working"`. FR-045: "`working` when the newest prompt or post_tool is within the working window, or … pre_tool …, or a compiler process is attributed to the lane; … otherwise `idle`, including a lane that has no event." The SWM-204 review (SR-1868 FND-006) already noted that herdr working is not a reason for `working`, and the program rule says weak signals alone do not make a state. Expected behaviour: such a lane reads `idle`. FR-070 still counts it as non-idle for health, so after `no_events_after` it carries `ix.fleet.lane.no_events`, which is the weak-evidence marker the spec provides. `last_state_non_idle` already handles this side correctly. Only the state needs to change.

**FND-003.** The PR body names the skew as a known deviation. FR-068 has the relay record Redis server time on every write precisely so that consumers measure against one clock, as FR-067 and FR-071 do. `publish_lane_list` already queries Redis `TIME` in the same refresh, and `main.rs` discards the result (`.is_ok()`). Pass `refreshed_ms` as `now_ms` to `set_activity`. For the failure comparison, convert both sides to relay time, or compare the failure with the read's relay time.

**FND-004.** `live_cwd` applies `MAPPING_TTL`, but `branch()` and `cwd()` do not. Apply the same expiry to focus_info. Once the mapping has expired, export an empty ticket and branch.
