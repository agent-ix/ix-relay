---
id: SR-049
title: "Scoped progress and cooldown code and Rust review"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@9b991e9fd2150c4839fb669fcfaa91bf0e3ddd7f; Cargo.toml, Cargo.lock, crates/relay-tui/src/monitor_ops.rs, crates/relay-tui/src/monitor_tracker.rs, crates/relay-tui/src/monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 558c186"
review_set: subset
---
## Summary

Reviewed the full diff using repository conventions and the Rust checklist. No CI gates changed. ix-board is pinned to its merged authoritative revision, with no copied source. Cooldowns are selected by the typed library error. The wait state coalesces manual refreshes, retains the force flag, admits stop/disconnection, uses Instant for elapsed time and checks expiry independently of queued refreshes. Channels remain bounded. Initiative aggregation groups by identity and intersects its member projects with the existing scoped leaf set, avoiding out-of-scope tasks and repeated project membership. Public local view fields have docs; display text is sanitized and bounded.

## Verdict

**PASS** — no findings in this diff. Thirteen scoped dashboard tests and scoped Clippy pass. Rendering checks project progress, leaf counts, initiative aggregation, assignments and short-pane controls. Wait-state checks exercise repeated refreshes, timeout, stop and disconnection; existing worker tests cover manual refresh and nonblocking stop. Full gate results are recorded in the PR. One initial full run failed in the existing agent-msg stalled-Redis timing test; its targeted retry passed unchanged.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Evidence limitations

The installed quire CLI lacks matrix. Requirement validation emits the existing FR-048 description EARS warnings. Live Linear quota recovery and lead-generated reports are external state, not claimed by local tests.
