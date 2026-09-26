---
id: SR-031
title: "Gap analysis of ix-relay PR #35, AGE-2064: FR-045-AC-24 against tests and code"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@d2c1dc842ed1f01a4c7bfc773df3050e64256a5a; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md (Export bullet on lookups, AC-24), spec/tests.md (FR-045-AC-24 row, TC-045-24, GAP-001, method counts), crates/relay-watcher/src/otlp/http.rs (addrs, connect, post, tc_045_24_*), crates/relay-watcher/src/otlp/export.rs (exporter thread)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-045"
    type: "reviews"
---
# SR-031: Gap analysis of ix-relay PR #35, AGE-2064

## Summary

Ticket: AGE-2064. Planless run. Plan completion: not assessed. This analysis checks the
new FR-045 Export bullet and FR-045-AC-24 against the four `tc_045_24_*` tests and the
code in `otlp/http.rs`. It also looks for code that no requirement owns.

## Verdict

Each clause of AC-24 is backed by a real test, and a mutation of each clause is caught.
The test-matrix row is backed by real tracking tags. There are no stubs and no coverage
inflation. One behaviour has no owning requirement and no test: what happens to the cache
when the request deadline runs out before or partway through the addresses (FND-001).
Low.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | `connect` has a deadline early return that leaves the cache in place. It fires when the budget is spent before an address is tried, and also partway through: with 4 blackholed addresses the 4th is never tried (SR-030 FND-002). No FR-045 bullet or AC states this behaviour, and no test exercises it. A mutation that drops the cache on that path survives every test. Either state the rule and add a TC-045-24 clause, or make the path drop the cache like the loop exhaustion does. | crates/relay-watcher/src/otlp/http.rs:207-210, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:135-140 |
| FND-002 | low | Two statements in the bullet have no AC-24 clause: "Each address keeps the 2 second connect timeout" and "only the exporter thread ever does the lookup". Both are true of the code. The first is already covered by the older TC-045-10 connect-timeout tests. The second follows from the existing exporter-thread ACs, because `post` runs only on `otlp-exporter`. No new test is needed, but the AC or the trace should say which existing TC covers them. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:135-140, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:245 |

## AC-by-AC

| AC-24 clause | Test | Mutation killed |
| --- | --- | --- |
| Two pushes within 5 min make one lookup | `tc_045_24_addresses_are_resolved_once_per_lifetime` (two posts, then `addrs()` at 299 s) | never-expire does not apply; the count is asserted |
| A push at 5 min makes another | `tc_045_24_an_expired_cache_is_resolved_again` | never-expire; `<` to `<=` |
| After every address failed, the next push looks up again with no time passed | `tc_045_24_when_every_cached_address_fails_the_next_push_resolves_again` | no drop after all-fail |
| A failed or empty lookup is not cached | `tc_045_24_a_failed_or_empty_resolve_is_not_cached` | caching a failed lookup. Caching an empty one is an equivalent mutant: the all-fail drop clears it. |

All four tests pass with `IX_RELAY_TEST_URL` unset, so they are Unit tests with no Redis,
and they carry the tags `Tracing: TC-045-24` and `ACs: FR-045-AC-24`.

## Code with no owning requirement

- The deadline early return in `connect` that keeps the cache (FND-001).
- `Resolve` and `with_resolver` are test seams. They need no requirement.

## Stub and inflation check

There are no stubs. The fake resolver counts real calls. The fake collector serves real
HTTP over loopback, and the tests assert the collector's statuses and the lookup counts.
Nothing is asserted against a constant. The tests.md FR row counts one row, and the Unit
totals 136 and 102 match `make check-matrix`.
