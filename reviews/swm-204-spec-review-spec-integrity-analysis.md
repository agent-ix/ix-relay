---
id: SR-1870
title: "Spec integrity — ix-relay SWM-204 FR-045 host series rows"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md"
review_set: subset
---

## Summary

Ticket: SWM-204. PR agent-ix/ix-relay#53 at `da01f4386ae17b64a6753a5c32cf690367a20b58` adds three rows to the FR-045 series table: `ix.fleet.host.memory_used_bytes`, `ix.fleet.host.memory_total_bytes` and `ix.fleet.host.temperature`. Each was checked against FR-069's host measurements (memory as used and total bytes, MemAvailable based; temperature as the highest hwmon Tctl or package sensor in degrees Celsius, only where the profile expects it), against FR-070's rule that a failed source's measured series are omitted, and against the code catalogue in `otlp/series.rs`.

## Verdict

**PASS.** The rows are consistent with FR-069 and FR-070 and with the exported catalogue. Their units (`By`, `Cel`) match OTLP/UCUM use elsewhere in the table, and the absence rules match FR-070. No findings.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |
