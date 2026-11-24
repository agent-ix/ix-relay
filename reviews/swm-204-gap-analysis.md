---
id: SR-1869
title: "Gap analysis — ix-relay SWM-204 host collector against FR-069, FR-070, FR-045"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-069-host-collector.md, spec/functional/FR-070-collector-source-health.md, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md, crates/relay-watcher/src/fleet/**, crates/relay-watcher/src/otlp/**, crates/relay-core/src/fleet.rs, crates/relay-core/src/store/fleet.rs"
review_set: subset
---

## Summary

Ticket: SWM-204 (plus the host half of SWM-203). PR agent-ix/ix-relay#53 at `da01f4386ae17b64a6753a5c32cf690367a20b58`. Planless gap audit: Plan completion was not assessed.

Matrix: `quoin matrix --json` (quoin 0.36.10) reports every FR-069 and FR-070 criterion as `untagged`. It reports 498 untagged and 69 tagged repo-wide, so it does not read this repository's `/// Tracing:` and `/// ACs:` doc-comment tags. This is a pre-existing tooling limit and not a finding of this PR. The AC-to-test map below was built by grepping the tags in the diff and reading each tagged test.

Tagged in this PR: FR-069-AC-1, 2, 3, 4, 7, 8, 9, 11, 12, 14 and 16; FR-070-AC-1 through 8; FR-045-AC-11, 29 and 31.
Untagged, and deferred by the PR's own ticket comment as seams: FR-069-AC-5, 6 and 13 (compile attribution) and FR-070-AC-9 (hook_events and no_events).
Inspection or demonstration: FR-069-AC-10 holds by inspection. The crate has no `.py` or `.sh` file and no shell-interpreter spawn, and the retired probe scripts are named only in the spec. FR-069-AC-15 (a live run on four hosts) is not demonstrated in this PR.

Reverse gaps: none. Every new production module maps to FR-069, FR-070 or FR-045.
Stubs: the deliberate `#[allow(dead_code)]` seams (`hook_feed_attached`, `set_hook_signal`, `compilers`, `failed_sources`) are documented as the hook feed's attach points. They are not presented as finished behaviour.

## Verdict

**GAPS.** One high: FR-070-AC-6 is tagged but not met. Two medium: partial ACs are tagged as covered, and the deferred ACs have no follow-up ticket ids.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | FR-070-AC-6 ("with the watcher running and Redis down, redis is 0 and the other sources still report") is tagged on TC-070-06, which only exercises the collector in isolation; the watcher pushes nothing when Redis is down (see SR-1868 FND-001). | crates/relay-watcher/src/fleet/mod.rs:735-746 |
| FND-002 | medium | FR-070-AC-5 is tagged on TC-070-05, which covers only the log line; the heartbeat and doctor halves (naming the failed source, `collector_source_failed`) are not implemented. FR-045-AC-31's doctor `fleet_lanes_dropped` half is likewise absent. Both read as covered in a tag scan. | crates/relay-watcher/src/fleet/health.rs:238-260; crates/relay-watcher/src/fleet/mod.rs:917-937 |
| FND-003 | medium | The deferred criteria (FR-069-AC-5, AC-6, AC-13, FR-070-AC-9, the heartbeat/doctor halves of FR-070-AC-5 and FR-045-AC-31, and the NOT_YET_EXPORTED series) are listed as "seams" in the ticket comment with no follow-up ticket ids, so nothing tracks them to closure. | crates/relay-watcher/src/fleet/mod.rs:10-13; crates/relay-watcher/src/otlp/mod.rs:482-492 |
