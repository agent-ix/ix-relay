---
id: SR-032
title: "Spec review (EARS) of ix-relay PR #35, AGE-2064: FR-045 lookup bullet and AC-24"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay@d2c1dc842ed1f01a4c7bfc773df3050e64256a5a; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md (new Export bullet, FR-045-AC-24); checked against crates/relay-watcher/src/otlp/http.rs and crates/relay-watcher/src/otlp/export.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-045"
    type: "reviews"
---
# SR-032: Spec review (EARS) of ix-relay PR #35, AGE-2064

## Summary

Ticket: AGE-2064. This checks the one Export bullet FR-045 adds, and FR-045-AC-24,
against the EARS patterns. It also checks that each is true of the code at d2c1dc8.
`quire validate` on FR-045 reports no errors, only module-registry warnings that are
unrelated to this PR.

Both are testable, and AC-24 names its oracle: an injected resolver and clock. The bullet
is not single-clause. It holds three separate obligations, a rationale clause and two
constraints, which is the same compound style as the neighbouring FR-045 bullets. Its
lead clause also overstates what it guarantees.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | The bullet is compound. It holds five statements: reuse for 5 minutes, do not cache failures, drop on all-fail, keep the per-address timeout, and do the lookup on the exporter thread only. It also has a "so that" rationale. AC-24 bundles four clauses. Both are testable, but a single failing clause fails the whole AC. Split them into a ubiquitous "shall reuse the resolved addresses (at most 4) for 5 minutes" and an unwanted-behaviour "If a lookup fails or returns no address, the exporter shall not cache it", plus "If every cached address fails to connect, the exporter shall drop the cache". The rationale can go to a note. This matches the file's existing style, so it is not a blocker. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:135-140, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:245 |
| FND-002 | low | "Shall resolve the collector's host name at most once per 5 minutes" is not true as stated. The bullet itself makes exceptions: failed lookups are repeated on every push, and an all-fail drop looks up again at once. Reword it as "shall reuse resolved addresses for up to 5 minutes", which is the ceiling the code actually implements. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:135-136 |
| FND-003 | low | The bullet says nothing about the request deadline, so "when every cached address fails to connect the exporter shall drop the cache" is not quite true of the code. With 4 blackholed addresses the 5 s budget ends before the 4th is tried, and the cache is kept until the TTL (SR-030 FND-002). A lookup longer than 5 s also fails the push that pays for it (SR-030 FND-001). State the deadline case explicitly, whichever way the code settles. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:138-139, crates/relay-watcher/src/otlp/http.rs:204-218 |

## Verdict

Approve with low findings only. The statements are testable and backed by tests. The
precision issues (FND-002 and FND-003) are worth a one-line edit alongside any SR-030 fix.
