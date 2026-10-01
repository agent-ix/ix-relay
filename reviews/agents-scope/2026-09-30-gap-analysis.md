---
id: SR-056
title: "Agent and scope views implementation gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@42c4dec91fd38be598182e2269a9fe38076a962b; README.md, crates/relay-tui/src/monitor.rs, monitor_main.rs, monitor_tracker.rs, monitor_ui.rs, monitor_views.rs, spec/functional/FR-048-session-work-monitor.md; diff against f918c887011186b3380f85e6ea6eb6d33940f5f2"
review_set: subset
---
## Summary

Mapped the source diff to FR-048. AC-6 states that missing scope or tracker data cannot hide explicit agents and defines view selection and retained snapshot behavior. AC-7 states the scoped group count semantics, unknown prerequisite handling, stage detail, and overlapping group interpretation. Existing AC-1 and AC-4 continue to govern native session evidence and separation of agent status from ticket stage. No additional transport or automatic native subagent discovery is claimed.

## Verdict

**PASS** — no findings in the reviewed diff.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Evidence

The terminal tests measure agent identity, focus ticket identifiers, scrolling, details, stale source labeling and view reset. The progress test measures one completed scoped leaf, one ready leaf, one blocked leaf with an unknown prerequisite, and exact remaining-stage counts for both project and initiative. Existing scoped initiative tests exclude other projects and avoid counting parent tickets.

## Evidence limitations

No applicable AssuranceProfile was found. The installed quire has no matrix command; no strict computed matrix verdict is claimed. Full spec validation retains existing advisory EARS/module warnings. The live IR session currently has no reported scope or assignments, so populated views are verified with terminal fixtures rather than inferred from IR's open tickets. Remote watcher publication is outside this diff.
