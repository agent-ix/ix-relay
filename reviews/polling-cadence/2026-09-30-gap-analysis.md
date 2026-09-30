---
id: SR-052
title: "Dashboard polling cadence acceptance gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@2ba6b4678239b74a7c205f54cfc8dba32edbad84; crates/relay-tui/README.md, crates/relay-tui/src/monitor.rs, crates/relay-tui/src/monitor_ops.rs, crates/relay-tui/src/monitor_runtime.rs, crates/relay-tui/src/monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 0fcc276fbfcce3e4a50728691e89bef299f7b2b9"
review_set: subset
---
## Summary

FR-048-AC-5 now documents the requested intervals: 30-minute Linear reads and separate one-minute Herdr/Redis reads. The scheduling regression measures the effective rate-limited and generic retry delays, including an hour-long transport metadata value. The rendering regression measures countdown values at injected elapsed times, cached leaf progress retention and success clearing the rate-limit state. The observation freshness regression covers successful activity between polls and unknown activity at the two-minute boundary or on source error. The existing independent-reader test exercises reports arriving while the Herdr read is blocked. README text now describes the actual explicit-session monitor and source responsibilities.

## Verdict

**PASS** — no gaps in the reviewed diff. Startup reads remain immediate; periodic reads use the requested timers. UI rendering does not call the external sources. The fix reduces polling and corrects the false hour-long wait; it does not claim to remove limits imposed by other API consumers.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |
