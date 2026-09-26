---
id: SR-017
title: "Spec review (integrity) of ix-relay PR #31, AGE-2058: FR-045 rename and cross-references"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay@2fcdbd10a3c970968b674317ce17a543ee0b810a; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md (renamed from FR-045-watcher-serves-pane-names-as-metrics.md), spec/functional/FR-026-configuration-precedence.md, spec/functional/FR-029-doctor-reports-relay-health.md, spec/functional/FR-038-watcher-wakes-when-a-message-is-sent.md, spec/non-functional/NFR-002-bounded-resources.md, spec/usecase/US-006-see-when-the-relay-is-failing.md, spec/spec.md (wire inventory), spec/tests.md, README.md"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-045"
    type: "reviews"
---
# SR-017: Spec review (integrity) of ix-relay PR #31

## Summary

Ticket: AGE-2058. The rename is clean. No file or link still names
`FR-045-watcher-serves-pane-names-as-metrics.md`. US-006, FR-026, FR-029 and NFR-002 link
the new file. The spec.md wire inventory drops the `GET /metrics` row and adds the OTLP
request row. Its heartbeat row now describes `OtlpStatus`. No reference to `metrics_port`,
`metrics_listen`, `watcher_metrics_*` or `/metrics` is left in the spec, README or
tests.md. The series table, the `series.rs` catalogue and the README table agree, and
a test pins them. The matrix recount (295 rows) is consistent. `quire validate` passes on
the changed files. Four problems remain, one of them medium.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | Two requirements conflict. FR-038-AC-5 says an idle tick "makes exactly two requests". FR-045 adds a store read on the tick once per push (FR-045 last bullet; NFR-002 lists it). A reviewer probe measured 16 requests on a push tick, 10 of them SCANs. Neither requirement scopes the other: FR-038-AC-5 does not say "with no OTLP endpoint" or "on a tick that does not push", and FR-045 does not state a request bound for the push tick. | spec/functional/FR-038-watcher-wakes-when-a-message-is-sent.md:70, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:168-176, spec/non-functional/NFR-002-bounded-resources.md:26-35 |
| FND-002 | low | The spec is incomplete in two places: bounds and credential tiers. FR-045 and NFR-002 give the push interval a floor (5 s) and no ceiling, while NFR-002 claims every setting is bounded, and the code panics on a huge value. FR-026 precedence is applied per key, so the endpoint and the headers (credentials) can come from different tiers. No requirement says headers belong to the endpoint's tier, so a header meant for one collector can go to another over plain http. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:59-68, spec/non-functional/NFR-002-bounded-resources.md:24-35, spec/functional/FR-026-configuration-precedence.md:13-18 |
| FND-003 | low | FR-045 contradicts itself on logging. "When none is [configured], it shall log nothing about OTLP" (and AC-1) conflicts with "the watcher shall log one line counting" the skipped `OTEL_RESOURCE_ATTRIBUTES` pairs, which the code prints with or without an endpoint. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:80-82, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:90-93 |
| FND-004 | low | Undecided and survey text sits in the normative FR. The "Open question for the owner" (TLS), the collector survey (checkouts under `~/dev`, `local/k8s/tempo.yaml`, the `fastapi-service` chart, "no `ix-local-observability` checkout exists") and "to be confirmed with the observability owner" are decision and research notes, not requirements. They date the spec and describe one workstation. Move them to the ticket or an ADR, and leave a one-line constraint (plain http only, TLS tracked in <ticket>). The README repeats the survey. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:40-56, README.md:320-330 |

## Verdict

**PASS WITH FINDINGS.** The rename and the cross-references are complete and consistent.
FND-001 needs FR-038-AC-5 and FR-045 reconciled; SR-014 FND-002 has the code side. The
rest are small completeness and hygiene edits.

## Dispositions

Disposition pass 1, reviewed at agent-ix/ix-relay@3ff496ca5aa075ef5ad69787e0be38f0b119e0f8.
`quire validate` passes on every spec file and `make check-matrix` passes (305 rows).

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | 5552a29: FR-038-AC-5 scopes the two-request tick to no OTLP endpoint and defers the push tick to FR-045-AC-22; FR-045 states the bound (at most two pipelined requests, no key scan, none inside the interval); NFR-002 agrees |
| FND-002 | fixed | 5552a29: FR-045 and NFR-002 give the interval a 3,600 s ceiling; FR-026 and FR-045 state that headers come from the endpoint's tier only, and that other tiers' headers are recorded as ignored |
| FND-003 | fixed | 5552a29: FR-045 now says the skipped-pairs line is logged only when a push is starting, and AC-1 says nothing about OTLP is logged without a setting, including that line |
| FND-004 | fixed | 5552a29: the TLS open question and the collector survey are out of the normative FR, which keeps a one-line constraint. The README still carries both (see FND-005) |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-005 | low | The README now contradicts itself and the code on the protocol variable. Line 344 still says "`OTEL_EXPORTER_OTLP_PROTOCOL` is not read", but line 328, FR-045 and `config/otlp.rs` read it: a value other than http/json makes the generic endpoint ignored (measured on the binary: `OTEL_EXPORTER_OTLP_PROTOCOL=grpc` gives "OTEL_EXPORTER_OTLP_ENDPOINT ignored: its protocol is not http/json"). The same paragraph still holds the `~/dev` collector survey ("Of the emitters found under `~/dev` ... confirm the collector with the observability owner") and the open TLS question, which describe one workstation. Fix: drop "and `OTEL_EXPORTER_OTLP_PROTOCOL` is not read", and move the survey to the ticket. | README.md:338-347, README.md:326-330 |
