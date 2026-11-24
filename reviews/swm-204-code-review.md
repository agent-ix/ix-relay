---
id: SR-1868
title: "Code review — ix-relay SWM-204 host collector and probe self-health"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-watcher/src/fleet/**, crates/relay-watcher/src/main.rs, crates/relay-watcher/src/otlp/{mod,encode,registry,series}.rs, crates/relay-core/src/{fleet,config,lib,store}.rs, crates/relay-core/src/store/{fleet,keys}.rs, crates/relay-core/tests/fleet.rs"
review_set: subset
---

## Summary

Ticket: SWM-204 (plus the host half of SWM-203). PR agent-ix/ix-relay#53 at `da01f4386ae17b64a6753a5c32cf690367a20b58`, reviewed as `git diff origin/main...HEAD` against FR-069, FR-070 and FR-045 on main. The rust-review lane is folded into this file.

Evidence gathered:

- macOS (aarch64-apple-darwin): `cargo test -p relay-watcher -p relay-core` passes (relay-watcher 371 + 14 + 1). `cargo clippy -p relay-watcher --all-targets -- -D warnings` FAILS: `ticks_from_macos` is never used (host.rs:58). `cargo fmt --check` clean. `scripts/check_unsafe_comments.sh` clean.
- Linux: `cargo clippy --target x86_64-unknown-linux-gnu` cannot run on this Mac (no `x86_64-linux-gnu-gcc` for the `cc` build scripts). Ran on terra inside WSL (Ubuntu, toolchain 1.98.1) from a fresh clone at da01f43 with `nice -n 10`, `-j 4`; clone removed afterwards. `cargo test -p relay-watcher` passes (368 + 14 + 1). `cargo clippy -p relay-watcher --all-targets -- -D warnings` FAILS: `ErrorClass::Unsupported` is never constructed (health.rs:76).

## Verdict

**CHANGES REQUESTED.** Two high findings: the OTLP push still depends on a live Redis connection, so a full Redis outage exports nothing (FR-070-AC-6), and `make lint` is red on both Linux and macOS. Pane-tail health has two medium defects.

Checked and clean:

- Thread lifetime (point 3). `Slot` refuses a second read while one is running (`busy` swap), so a source that hangs on every tick holds at most one thread. There are 8 slots at most (7 profile sources with a read plus `native_windows`), so leaked threads are bounded at 8. The release guard frees the slot before the result is sent. A panicking job reads as `Unreadable`. herdr and tasklist reads go through `RealProcessRunner`, which kills the process group on timeout.
- Secret hygiene (point 4). `pane::classify` returns only a `PaneClass`. The tail `String` lives only inside the scan closure. Log lines are built only from `Source::name` and `ErrorClass::name`. herdr's stderr is never read into a message. TC-069-08 seeds a marker and checks both the encoded body and the log lines. Compiler `cwd`s are kept in memory and not exported.
- relay-core additions (point 6). These are a new `fleet` module, a new `fleet-lanes` hash key (prefixed, and covered by the key tests), two new `Relay` methods and a `[fleet]` raw table. None of them changes an existing key, type or wire format. agent-msg does not call them. `cargo check --workspace --all-targets` builds. One deploy-order caveat is filed as FND-008.
- The FR-045 series catalogue, the `as_double` encoding, the finite-only `Double` gauge and the lane cap (TC-045-31).

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | Full Redis outage exports no metrics at all: the push runs only at the end of a tick that already got a Redis connection and passed every store read, so probe_ok{redis}=0 is never pushed when it matters (FR-070-AC-6). | crates/relay-watcher/src/main.rs:797-799,896-919,1104-1115 |
| FND-002 | high | `cargo clippy -p relay-watcher --all-targets -D warnings` is red on Linux (ErrorClass::Unsupported never constructed) and on macOS (ticks_from_macos never used); `make lint` fails on every host. | crates/relay-watcher/src/fleet/health.rs:76; crates/relay-watcher/src/fleet/host.rs:58 |
| FND-003 | medium | Stale pane classes keep exporting ix.fleet.lane.blocked while pane_tail reads 0: the Timeout and busy-slot path leaves `classes` untouched, and `fill` never checks pane_tail health or the class age. | crates/relay-watcher/src/fleet/mod.rs:373-374,401-406,456-477 |
| FND-004 | medium | The pane scan can exceed its own source budget: a read may start at 399 ms with a 300 ms runner timeout, against a 500 ms slot budget. When herdr is slow, the whole scan times out, its completed reads are discarded and scan_from does not advance. | crates/relay-watcher/src/fleet/mod.rs:52,99,354-376 |
| FND-005 | medium | TC-069-02 (FR-069-AC-2) tests `ticks_from_macos`, which production does not call; `mac::cpu_ticks` computes busy/idle inline, so the macOS path under test is not the one that runs. | crates/relay-watcher/src/fleet/host.rs:58,263-275; crates/relay-watcher/src/fleet/mac.rs:33-38 |
| FND-006 | medium | TC-069-08 asserts state `working` when an `idle` hook event newer than a `dialog` tail wins; FR-069-AC-8 says the lane is reported with the hook event's state (idle). | crates/relay-watcher/src/fleet/lanes.rs:127-138,216-222 |
| FND-007 | medium | tc_045_11 now tags FR-045-AC-29 while 4 fleet series and the order series of AC-29 sit in NOT_YET_EXPORTED; AC-29 and AC-11 ("exactly the series table") read as covered but are not met. | crates/relay-watcher/src/otlp/mod.rs:433,482-512 |
| FND-008 | low | `[fleet]` sits under `deny_unknown_fields` on FileConfig. An agent-msg or hook binary built before this change refuses the whole config.toml once a host adds `[fleet]`, so the watcher and agent-msg must be upgraded before the config is edited. | crates/relay-core/src/config.rs:131-148 |
| FND-009 | low | The inserted herdr-health block splits an existing comment mid-sentence ("one" / "line per episode"). | crates/relay-watcher/src/main.rs:446-454 |

### Detail

**FND-001.** In `RedisBus`, `connect()` must succeed before `poll()` and `tick()` run. Inside `tick()`, `relay.agents()?`, `apply_settlements(..)?`, `refresh_places(..)?` and the reap and inbox reads all return on a connection error before `publish_if_due` at line 1107. The comment on `publish_if_due` ("a metrics read can never skip the rest of the tick") covers only the opposite direction. TC-070-06 calls `set_push_redis(false)` on a collector in isolation and never drives the watcher with Redis down. The OTLP exporter posts to the collector, not to Redis. FR-070-AC-6 requires that "with the watcher running and Redis down, `redis` is 0 and the other sources still report". This is the failure class of the 2026-11-24 titan Redis starvation. Fix: run the collector sample and push on their own timer, outside the Redis-gated tick (also while connect is failing), with `set_push_redis(false)` and the relay series left out. Add a watcher-level test with an unreachable Redis URL that asserts a push carrying probe_ok{redis}=0 and probe_ok{cpu}=1.

**FND-002.** The `cfg_attr(allow(dead_code))` covers `Parse` off Linux but not `Unsupported` on Linux or macOS. `ticks_from_macos` is `cfg(any(macos, test))` and unused by the macOS bin. The coder's macOS evidence evidently did not include `--all-targets -D warnings` on the bin.

**FND-003.** FR-070 says "omit a source's measured series while that source reads 0, so that an old value is never exported as current". Lane blocked reasons are pane_tail's measured series. Two cases also hold on a healthy source: (a) with round-robin over more lanes than fit in 400 ms, a lane keeps whatever class it got on its last read, with no age bound. (b) A `dialog` read once keeps the lane `blocked` until that lane is read again.

**FND-004.** `PANE_SCAN_BUDGET` (400 ms) is checked only before each read starts. Each read has a 300 ms runner timeout, and `SOURCE_BUDGET` is 500 ms. At about 250 ms or more per read, a scan of two or more lanes passes 500 ms. `Slot::run` then returns Timeout, every finished classification is dropped, and the next scan starts from the same `start`. Fix: make the scan budget plus the per-read timeout fit inside `SOURCE_BUDGET`, and/or pass results back incrementally.

**FND-006.** The hook seam is not wired yet, so no shipped behaviour changes. The tagged test still fixes the wrong expectation for SWM-202/205 to inherit. FR-045's state rules never list "herdr working" as a reason for `working`.

**FND-007.** Point 5. The allowlist is self-expiring and checks in both directions: each entry must be in the spec and must not be exported, and the row count is pinned. It therefore does not silently hide a new unexported series, which is acceptable as a known-gap mechanism. It does mean the AC-11 test passes while AC-11 is false, and the new `FR-045-AC-29` tag claims coverage of series that do not exist yet. Remove the AC-29 tag, or narrow it to a test that only asserts the exported fleet subset, and give each NOT_YET_EXPORTED entry the follow-up ticket that removes it.
