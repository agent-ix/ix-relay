---
id: SR-054
title: "Optional status and footer acceptance gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@208ca9b44b3377a4a6d57fe69eeea5e8a8634f6b; crates/relay-tui/README.md, crates/relay-tui/src/monitor.rs, crates/relay-tui/src/monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 9a5c16d1514359e3a430d33218367329a04a4ca0"
review_set: subset
---
## Summary

FR-048-AC-4: rendered cells prove only arrow/d/r/q/s hotkeys use primary blue, while words and separators are muted. FR-048-AC-4/5: the actual s-key handler exposes source timestamps only on demand; default and closed status omit them. A simulated tracker failure remains rendered while status is hidden. The existing countdown, source-retention and runtime freshness tests continue to exercise error presentation. README documents the new control and always-visible errors.

## Verdict

**PASS** — no gaps in the scoped change. Hiding the status line changes presentation, not source polling intervals or failure retention. The requested Terra target awaits an address reachable from the local machine; this PR does not guess one.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |
