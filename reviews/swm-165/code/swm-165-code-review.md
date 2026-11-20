---
id: SR-087
title: "SWM-165 implementation code-review"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; SWM-165; relay-tui, Cargo manifests/lock, changed docs, FR-059 and FR-048; canonical dev-team FR-004 context"
review_set: subset
---

## Summary

Frozen head: 61427e2b5b4a91951c845b4743b2c14dee5f5d64. Base: a08e73f67022bcee4da7ef00b466175ad0aaafbd. Ticket: SWM-165; PR: ix-relay#37. Exact model identifier unavailable.

All 14 diff paths examined: Cargo.toml, Cargo.lock, README.md, crates/relay-tui/Cargo.toml, monitor.rs, monitor_department.rs, monitor_department_source.rs, monitor_main.rs, monitor_ops.rs, monitor_tracker.rs, monitor_ui.rs, docs/cli-reference.md, docs/department-monitor.md, docs/how-it-works.md. No spec/plan changes; no new spec review. No applicable AssuranceProfile exists in spec/. Canonical dev-team FR-004 and department-monitoring.md supply view semantics; ix-board owns formulas, assessments and comparisons.

## Verdict

**CONDITIONAL** — 2 findings require the original coder's fix round in the sole ticket worktree.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | The fixed two-line diagnostics paragraph clips combined source errors, hiding binding reasons and authentication recovery at 80x24. With the default no-retro-config guidance, MissingApiKey and an ambiguous requested lead, the actual renderer shows neither the binding reason nor ix-board auth login. Give each failure a visible compact summary and an accessible complete diagnostic. | crates/relay-tui/src/monitor_department.rs:568 |
| FND-002 | medium | Lead-only Status drops the bound workspace team fallback. prepare_app leaves app.team absent, and status_rows passes None to summarize_department, so a populated SWM snapshot resolves an empty scope and is displayed as structured 0/0 progress. Resolve the effective team consistently with the existing lead views, while preserving project-only scope semantics. | crates/relay-tui/src/monitor_department.rs:329 |

## Coverage

Rust lane included API, ownership, safety, typed source errors, worker cancellation, bounded channels, terminal restoration, dependency and persistence boundaries. The docs-style guide review found no additional accuracy defect; examples and saved-mode claims were checked against source and the built offline path. No formula/model vendoring or duplicate renderer engine identified. Existing session-bound refusal and replacement SID/PID guards remain intact. Unsupported input uses the shared strict decoder and preserves bytes by inspection and existing passing test.

## Examined scope

```json
[
  {
    "id": "FR-059-AC-1",
    "path": "spec/functional/FR-059-department-monitor-adapter.md",
    "role": "examined",
    "excerpt": "Injected startup fixtures for team-only, project-only and saved-report inputs reach a department view without a lead/session. Missing and ambiguous lead identity with explicit scope show unknown bound activity plus the binding reason; the same failures without explicit scope retain refusal. No nearby pane or session is selected."
  },
  {
    "id": "FR-059-AC-2",
    "path": "spec/functional/FR-059-department-monitor-adapter.md",
    "role": "examined",
    "excerpt": "A valid saved shared-model report opens with credential, network, Herdr and Redis adapters that fail if called; switching all three department views and quitting succeeds. Conflicting saved/live selectors and absent scope/report/lead fail before any adapter call. Malformed/unsupported input names the path and problem, exits nonzero and leaves input bytes unchanged."
  },
  {
    "id": "FR-059-AC-3",
    "path": "spec/functional/FR-059-department-monitor-adapter.md",
    "role": "examined",
    "excerpt": "One shared-model fixture and its serialized JSON show matching values, statuses, completeness, evidence references, routes and current/previous/baseline comparisons in all three views. `query_changed`, absent baseline, categorical evidence and missing routes retain their model meanings; a red/incomplete class stays red/incomplete. No live collection is needed to render the fixture."
  },
  {
    "id": "FR-059-AC-4",
    "path": "spec/functional/FR-059-department-monitor-adapter.md",
    "role": "examined",
    "excerpt": "TestBackend fixtures at 80x24 and 120x40 switch views with 1/2/3, retain scope, reset selection/scroll, reach the final row and clamp selection after shorter data. Details expose evidence and owner/action; long text and control characters cannot hide diagnostics or inject terminal controls. Existing compact Tickets/Agents/Scope fixtures and their keys retain their outputs."
  },
  {
    "id": "FR-059-AC-5",
    "path": "spec/functional/FR-059-department-monitor-adapter.md",
    "role": "examined",
    "excerpt": "A successful model followed by failed refresh keeps identical evidence/window timestamps, shows last-snapshot/error and remains navigable with the optional session-source status line hidden. Missing model, stale source, partial relation pages and unreachable host fixtures retain their distinct shared diagnostics; refreshing cannot make stale evidence fresh."
  },
  {
    "id": "FR-059-AC-6",
    "path": "spec/functional/FR-059-department-monitor-adapter.md",
    "role": "examined",
    "excerpt": "Injected saved-credential success with the environment override absent permits live scoped collection; injected missing-auth failure shows a credential-safe actionable error and normal quit returns nonzero. The same failure adapters are never called for an offline report. A blocked source read leaves quit responsive; normal quit and terminal-initialization/render error fixtures restore terminal state."
  },
  {
    "id": "FR-059-AC-7",
    "path": "spec/functional/FR-059-department-monitor-adapter.md",
    "role": "examined",
    "excerpt": "Bound-session replacement preserves the explicit restart requirement and prevents replacement activity/report attribution while department data remains navigable. Existing FR-048 reporting isolation and Tickets/Agents/Scope behavior remain unchanged; department reads produce no work-report, registration, tracker or retro-store write."
  },
  {
    "id": "FR-048-AC-9",
    "path": "spec/functional/FR-048-session-work-monitor.md",
    "role": "examined",
    "excerpt": "Scope defaults to the bound workspace team key, with an explicit --team override. Initiative, project and ticket selectors narrow that team boundary."
  },
  {
    "id": "FR-004-AC-1",
    "path": "/Users/peter/dev/worktrees/dev-team-swm-161/spec/functional/FR-004-terminal-views.md",
    "role": "examined",
    "excerpt": "Given the same versioned model, TUI and JSON show matching statuses, values, evidence and actions; the model can be rendered without terminal initialization for a later plugin/desktop consumer."
  }
]
```

## Validation evidence

cargo fmt --check: 0 (stable rustfmt reports existing nightly-option warnings). cargo clippy -p relay-tui --all-targets -- -D warnings: 0. cargo test -p relay-tui --bin team-leader-dashboard: 0, 36 passed. cargo test -p relay-tui --locked: 0, all crate tests passed. cargo deny check: 1, advisory database lock in read-only home denied; no escalation or alternate database attempted. Root owns the one final aggregate gate, currently blocked by unavailable Docker; no aggregate gate, Docker or Redis started by reviewer.

Source-free probe compiled with rustc against the exact source modules and retained target dependencies: 0; execution: 0. Actual 80x24 output in render-probe.log reports BINDING_VISIBLE=false AUTH_REMEDY_VISIBLE=false and IMPLICIT_LEAD_SCOPE=0 EXPLICIT_TEAM_SCOPE=1 for the same known SWM issue. No production source edits.

Built offline --report PTY, 80x24, session 41978: Status, Health, Retro, details, return to Status, q; exit 0 and identical stty before/after. Shell exited 0. Help exited 0. No real credential provider, Keychain, live collectors, native pane probe, registration or source mutation invoked. Existing historical SWM-91/136/156 symptoms remain reported, not reproduced. Every reviewer command completed; retained target cache untouched.


