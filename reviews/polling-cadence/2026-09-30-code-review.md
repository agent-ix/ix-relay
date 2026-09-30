---
id: SR-051
title: "Dashboard polling cadence code and Rust review"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@2ba6b4678239b74a7c205f54cfc8dba32edbad84; crates/relay-tui/README.md, crates/relay-tui/src/monitor.rs, crates/relay-tui/src/monitor_ops.rs, crates/relay-tui/src/monitor_runtime.rs, crates/relay-tui/src/monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 0fcc276fbfcce3e4a50728691e89bef299f7b2b9"
review_set: subset
---
## Summary

Reviewed the entire PR diff using repository conventions and the Rust checklist. No CI files changed. Scheduling now uses one 30-minute Linear constant and one one-minute runtime constant. The rate-limit variant selects the fixed polling schedule rather than treating a rolling quota reset as a mandatory hour-long pause. Manual refreshes remain coalesced during rate limiting. Herdr freshness allows two minutes, so activity does not oscillate to unknown between successful one-minute reads. Countdown state uses Instant and checked addition; fixed-time rendering tests advance the supplied Instant without sleeping. Snapshot staleness uses the configured cadence and a pure timestamp comparison. Existing source retention, bounded channels and stop behavior remain intact. No compatibility layer or copied dependency implementation was introduced.

## Verdict

**PASS** — no findings. Sixteen scoped dashboard tests and scoped Clippy pass. Tests exercise a 3600-second transport error against a 1800-second read schedule, coalesced refresh behavior, advancing countdown, retained counts, error recovery and observation freshness between minute polls. Full gate results are recorded in the PR.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Evidence limitations

The installed quire CLI lacks matrix. Scoped requirement validation emits the existing FR-048 description EARS warnings. No assertion depends on live Linear quota recovery; other consumers share that quota.
