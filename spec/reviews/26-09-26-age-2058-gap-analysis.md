---
id: SR-015
title: "Gap analysis of ix-relay PR #31 (AGE-2058) against FR-045 and the requirements it touches"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@2fcdbd10a3c970968b674317ce17a543ee0b810a; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md (AC-1..AC-13), spec/functional/FR-026-configuration-precedence.md, spec/functional/FR-029-doctor-reports-relay-health.md, spec/functional/FR-038-watcher-wakes-when-a-message-is-sent.md (AC-5), spec/non-functional/NFR-002-bounded-resources.md, spec/tests.md (FR-045 and TC-045 rows), crates/relay-watcher/src/otlp/*, crates/relay-watcher/src/main.rs, crates/relay-core/src/config/otlp.rs, crates/relay-core/src/health.rs, crates/agent-msg/src/doctor/*"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-045"
    type: "reviews"
---
# SR-015: Gap analysis of ix-relay PR #31 (AGE-2058)

## Summary

Ticket: AGE-2058. PR: ix-relay#31, head 2fcdbd1. This is a planless gap analysis:
requirement to Test Matrix to tagged test to code, for FR-045 and the requirements the PR
edits or affects. Plan completion: not assessed.

Every FR-045 AC (1 to 13) has a matrix row, a TC row and tagged tests (`Tracing: TC-045-NN`
/ `ACs: FR-045-AC-N`) that exist and pass (Redis lane included). `make check-matrix` passes
(295 rows). The coder's own deviations are disclosed in the spec and the README: the
`sent`/`denied`/latency/escalation series are not built; `hook.failures` is a gauge over the
capped stream; `delivered` counts only watcher deliveries. They match the code and the
series catalogue test. No stub or inflated row was found. Five gaps remain, listed below.

## Verdict

**GAPS FOUND (one medium).** FR-038-AC-5 still holds as written only with OTLP disabled.
The other gaps are low. Each is an obligation the spec does not state or a test does not
reach, and the code-review findings in SR-014 cover the same defects.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | FR-038-AC-5 ("a tick makes exactly two requests") is verified only with OTLP disabled (`WatcherState::default()`). With an endpoint set, a push tick makes 16 requests in a reviewer probe on the same fixture, 10 of them SCANs. No AC or test pins or bounds the push tick, and FR-038 was not amended. So the obligation is silently scoped, not met, under the new feature. | spec/functional/FR-038-watcher-wakes-when-a-message-is-sent.md:70, crates/relay-watcher/src/main.rs:3469-3506, crates/relay-watcher/src/otlp/relay.rs:136-219 |
| FND-002 | low | FR-045 bounds the push interval below (at least 5) but not above, and no AC covers an extreme value. The code accepts any u64 and panics on the first push past about 2^63 seconds (SR-014 FND-001). | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:59-61, crates/relay-watcher/src/otlp/mod.rs:252 |
| FND-003 | low | FR-045-AC-1 ("logs nothing about OTLP" without an endpoint) is tested by a substring filter on `metrics`/`otlp`. The `OTEL_RESOURCE_ATTRIBUTES` skipped-pairs line gets past that filter, and it is printed with no endpoint and with `--once` (measured). | crates/relay-watcher/tests/startup.rs:318-330, crates/relay-watcher/src/main.rs:557-563 |
| FND-004 | low | The code has no owning observable requirement: `export::Stats` (`sent`, `failed`, `dropped_snapshots`, `rejected_points`) is written on every send and read only by tests. FR-045 says rejected points and snapshots dropped while backing off are "counted", but no requirement says where that count can be seen. So AC-13's "a partial success counts the rejected points" passes against a count nobody can read. | crates/relay-watcher/src/otlp/export.rs:130-142, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:193 |
| FND-005 | low | `ix.relay.hook.failures.recent` is specified as "failures ... still in the capped `health:hooks` stream". The read takes the oldest 1,000 entries (`XRANGE - +`), but the stream holds up to about 1,099 (`MAXLEN ~`), so the newest can be missed. No test has more than 1,000 entries. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:148, crates/relay-watcher/src/otlp/relay.rs:195-217 |

## AC-by-AC

| AC | Tests (exist and pass at head) | Result |
|----|------|--------|
| FR-045-AC-1 | `an_unset_endpoint_binds_nothing_and_logs_nothing_about_otlp`, `tc_045_01_no_endpoint_starts_no_exporter`, `once_sends_nothing`, `a_configured_endpoint_is_announced_by_target_only_and_binds_nothing` | covered; filter gap FND-003 |
| FR-045-AC-2 | `tc_045_02_*` (relay-core config and otlp), `an_https_endpoint_is_refused_at_startup_naming_its_source`, `a_bad_otlp_value_in_the_file_is_refused_with_its_line`, `watcher_only_config` tests | covered; no upper interval bound (FND-002) |
| FR-045-AC-3 | `tc_045_03_*` (byte-exact golden, hostile labels, budget, resource tier) | covered; golden independently confirmed by the official OTLP proto parser (SR-014) |
| FR-045-AC-4 | `tc_045_04_*`, `tc_045_03_a_pane_herdr_lists_twice_is_one_series`, `tc_045_03_the_pane_set_is_bounded` | covered |
| FR-045-AC-5 | `tc_045_05_*` including the Redis-lane tick test | covered |
| FR-045-AC-6 | `a_running_watcher_pushes_to_the_collector_and_reports_sending` | covered (the built binary, real Redis, a real listener) |
| FR-045-AC-7 | the three startup tests, reading `/proc/<pid>/net/tcp*` joined to the fd table | covered; independently re-measured |
| FR-045-AC-8 | `tc_045_08_*`, the heartbeat tolerance test, the wire_golden goldens | covered |
| FR-045-AC-9 | `tc_045_09_*` | covered; the point-cap mutant is killed |
| FR-045-AC-10 | `tc_045_10_*`, `a_response_over_the_cap_is_read_only_that_far` | covered; the deadline mutant is killed by a hang |
| FR-045-AC-11 | `tc_045_11_the_exported_series_are_the_catalogue_and_the_catalogue_is_the_spec` | covered |
| FR-045-AC-12 | `tc_045_12_*`, `the_stats_fill_every_outcome_even_at_zero_and_the_tick_histogram` | covered |
| FR-045-AC-13 | `tc_045_13_*` (10 tests) | covered; the Retry-After-bound mutant is killed; counts unobservable (FND-004) |

## Code with no owning requirement

`export::Stats` (FND-004). Everything else under `otlp/` traces to an FR-045 behaviour
bullet or to the series table.

## Stub and inflation check

No stub was found. All 13 FR-045 rows are "written" and name real tests, checked by name
against the source. The recount in the tests.md summary (295 rows; Unit 116 and
Integration 115) matches `make check-matrix`.

## Dispositions

Disposition pass 1, reviewed at agent-ix/ix-relay@3ff496ca5aa075ef5ad69787e0be38f0b119e0f8. The
matrix recount passes (305 rows) and every FR-045 AC (1 to 23) traces to tests that exist
and pass with the Redis lane on (none skipped).

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | 5552a29: FR-038-AC-5 now scopes the two-request idle tick to no endpoint and cites FR-045-AC-22 for the push tick; `tc_045_22_a_push_tick_adds_two_pipelined_requests_and_the_next_tick_none` runs with OTLP on and pins 2+2 requests and no SCAN; MONITOR with 0/5/50 leads agrees |
| FND-002 | fixed | 5552a29: FR-045 and NFR-002 bound the interval at 5 to 3,600 s; FR-045-AC-14 names 4, 3,601, 9223372036854775807 and 18446744073709551615 from env and file; `an_out_of_range_interval_is_refused_at_startup_and_never_panics` covers them on the built binary |
| FND-003 | fixed | 5552a29: the AC-1 test runs with a malformed `OTEL_RESOURCE_ATTRIBUTES` and filters on metrics/otlp/otel/malformed/pair, so the line would fail it; `skipped_resource_attribute_pairs_are_counted_when_a_push_starts` covers the other side |
| FND-004 | fixed | 5552a29: FR-045-AC-21 makes the counters observable (heartbeat and doctor); three `tc_045_21_*` tests cover round-trip, live counters and doctor lines |
| FND-005 | fixed | 5552a29: FR-045-AC-23 and `tc_045_23_the_hook_gauge_counts_the_newest_failures_past_a_thousand_entries` (1,100 entries, precondition asserted; killed by reverting to `hook_health`) |
