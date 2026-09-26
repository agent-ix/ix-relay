---
id: SR-016
title: "Spec review (EARS) of ix-relay PR #31, AGE-2058: FR-045 rewritten for OTLP push"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay@2fcdbd10a3c970968b674317ce17a543ee0b810a; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md, spec/functional/FR-026-configuration-precedence.md (Description), spec/functional/FR-029-doctor-reports-relay-health.md (check table), spec/non-functional/NFR-002-bounded-resources.md (OTLP bullet)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-045"
    type: "reviews"
---
# SR-016: Spec review (EARS) of ix-relay PR #31

## Summary

Ticket: AGE-2058. The review covers the requirement statements and ACs the PR rewrites.
FR-045's Description is a correct Optional-feature EARS statement: "Where an OTLP endpoint
is configured, `relay-watcher` shall push ...; Where no endpoint is configured, the watcher
shall send nothing". Most Behaviour bullets are event-driven or unwanted-behaviour
statements, each with a subject and "shall". The FR-026 and FR-029 edits are renames inside
statements that already conform. The NFR-002 bullet is a bound list, as the rest of that NFR
is. `quire validate` passes on every changed spec file.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | FR-045-AC-13 is heavily compound: about 16 independent obligations in one AC (backoff doubling, not-before skip, reset, Retry-After, 4xx refusal, 3xx, outage logging, partial success, stuck collector, queue bound, drop counting, a non-blocking submit, panic recovery, four real-listener cases, header secrecy). FR-045-AC-2 is similar, at about 12. Each traces to one TC row naming ten tests, so the matrix cannot show which obligation a failing or missing test leaves open. Split them into separately traceable ACs, for example backoff, refusal and redirect, outage logging, queue and isolation, and secrecy. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:182, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:193 |
| FND-002 | low | Several Behaviour sentences state facts, with no system subject and no "shall", so they are not EARS requirements: "`OTEL_RESOURCE_ATTRIBUTES` pairs that are not `key=value` are skipped", "A panic in the send path is a failed push", "None has a command-line flag, and a set-but-empty environment variable is unset", and "points past it are left out and counted". Rewrite each as "The configuration loader shall skip ..." or "The exporter shall treat a panic ... as a failed push". | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:67-68, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:80-82, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:112-113, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:116 |
| FND-003 | low | "The watcher shall push about every interval" is vague ("about"). The ACs never test the cadence. State the rule instead: at most one push per interval, the first on the first tick, measured at a tick boundary. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:99 |
| FND-004 | low | "The exporter thread shall be the only thing that touches the network" is false as written. The tick's per-push store read goes to Redis over the network (FR-045's own last bullet), so the intended meaning, "the only thing that talks to the collector", should be the statement. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:103, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:168-176 |

## Verdict

**PASS WITH FINDINGS.** The statements are mostly well formed and testable. Splitting
AC-13 and AC-2 (FND-001) is the one change that matters for traceability. The rest are
wording.

## Dispositions

Disposition pass 1, reviewed at agent-ix/ix-relay@3ff496ca5aa075ef5ad69787e0be38f0b119e0f8.
`quire validate` passes on every spec file.

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | 5552a29: AC-13 now holds only the backoff; its other obligations moved to AC-17 (Retry-After, refusal, redirect), AC-18 (outage lines, partial and total rejection), AC-19 (queue, isolation, panic), AC-20 (real-listener sequence, header secrecy) and AC-21 (counters, stuck); AC-2 split into AC-2 (precedence), AC-14 (relay refusals), AC-15 (agent-msg tolerance) and AC-16 (ignored generic settings); each has its own TC row naming its tests |
| FND-002 | still-open | Three of the four named sentences now have a subject and "shall" (the resource-attribute skip, the panic, the 4 MiB cut). The fourth is unchanged at FR-045:63-64: "None has a command-line flag, and a set-but-empty variable is unset." Still needed: for example "The configuration loader shall treat a set-but-empty variable as unset; no OTLP setting has a command-line flag." |
| FND-003 | fixed | 5552a29: FR-045 states the cadence rule (at most once per interval, first push on the first tick, each later one on the first tick at or after the previous push plus the interval) and AC-22 tests it |
| FND-004 | fixed | 5552a29: FR-045 now says "The exporter thread shall be the only thing that talks to the collector." |
