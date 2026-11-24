---
id: SR-6107
title: "Gap analysis — ix-relay SWM-205 fleet lane state from hook events"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; FR-045-AC-28..33, FR-068-AC-11, FR-069-AC-5/6/8/13, FR-070-AC-9; crates/relay-watcher/src/fleet/**, crates/relay-watcher/src/main.rs, crates/relay-watcher/src/otlp/**"
review_set: subset
---

## Summary

Ticket: SWM-205 (plus the rest of SWM-203). PR agent-ix/ix-relay#55 at `6b34f2e94c99ddc9ceb514186aaff60bb6e3baf4`. This is a planless repository audit of the criteria the diff touches. It used the computed matrix from `quire matrix --scope . --format json`, a reverse-gap scan of the diff, and a stub check.

## Verdict

**CONDITIONAL.** The computed matrix shows every criterion in scope as `tagged`. No stubs or `todo!`/`unimplemented!` were found in the diff. The reverse gaps are limited to config validation bounds (1 s to 7 d). That is a guard on documented keys, not behaviour without a spec. The `unregistered` reason now has an FR-045 row.

The evidence is hollow in two places. The lane-state tests never put a pane class beside the hook events, which is the production condition (FND-001). No test covers a lane with no event and herdr working (FND-002). One trace tag binds to the wrong AC (FND-003).

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | TC-045-30 and TC-069-08 call `fill()` without a pane scan, so no fresh pane class exists. In production `sample()` scans panes before every push, and that path (pane `none` newer than the hook event) is untested. The tagged tests pass while FR-045-AC-30 fails in production (code-review SR-6106 FND-001). | crates/relay-watcher/src/fleet/feed_tests.rs:79-151,153-170 |
| FND-002 | medium | No test fixes the state of a lane with no hook event and herdr `working`. FR-045-AC-30's "no event → idle" case uses a herdr-idle fixture, so the `working` read of SR-6106 FND-002 is untested. | crates/relay-watcher/src/fleet/feed_tests.rs:84,113 |
| FND-003 | low | The unit test of the ticket parser carries the trace tag FR-045-AC-28 (health snapshot read-failure counters), which it does not test. Its subject is the focus_info ticket of FR-045 / AC-29. | crates/relay-watcher/src/fleet/feed.rs:366-374 |

## Coverage

- Criteria in scope: FR-045-AC-28, AC-29, AC-30, AC-31, AC-32 and AC-33; FR-068-AC-11; FR-069-AC-5, AC-6, AC-8 and AC-13; FR-070-AC-9. All 12 are `tagged` in the computed matrix, and none is untagged or tagged only by ignored tests.
- Run evidence: `cargo test -p relay-core -p agent-msg` passes. The relay-watcher feed, fleet, otlp and main-Redis tests this diff adds pass on macOS against a throwaway redis:7. The coder reports terra green.
- Semantic review: skipped. This is a reviewer subagent run with no user to opt in. The code-review lane (SR-6106) did the intent-to-code check for the touched rules.
- Plan completion: not assessed
