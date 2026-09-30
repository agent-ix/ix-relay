---
id: SR-050
title: "Scoped progress and cooldown acceptance gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@9b991e9fd2150c4839fb669fcfaa91bf0e3ddd7f; Cargo.toml, Cargo.lock, crates/relay-tui/src/monitor_ops.rs, crates/relay-tui/src/monitor_tracker.rs, crates/relay-tui/src/monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 558c186"
review_set: subset
---
## Summary

FR-048-AC-3/4: the compact rendering regression now measures project progress in the scope line. The initiative regression measures one selected project's leaves, then expands the scope to a second project and checks the shared initiative once with the new denominator. Initiative totals are explicitly scoped; no workspace-wide denominator is inferred. AC-5: typed Linear cooldowns protect scheduling from forced refresh, while existing independent source handling retains previous successful snapshots. The wait-state test exercises three coalesced refreshes followed by timeout, as well as normal timeout, stop and disconnection. Focus remains an explicit report; this diff does not fabricate assignments.

## Verdict

**PASS** — no implementation gaps in the scoped diff. The consumer uses the merged ix-board transport fix. The current lead's unreported focus/agents remain unknown. Live API recovery remains external to acceptance testing.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |
