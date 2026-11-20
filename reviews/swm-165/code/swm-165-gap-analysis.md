---
id: SR-088
title: "SWM-165 implementation gap-analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; SWM-165; relay-tui, Cargo manifests/lock, changed docs, FR-059 and FR-048; canonical dev-team FR-004 context"
review_set: subset
---

## Summary

Frozen head: 61427e2b5b4a91951c845b4743b2c14dee5f5d64. Base: a08e73f67022bcee4da7ef00b466175ad0aaafbd. Ticket: SWM-165; PR: ix-relay#37. Exact model identifier unavailable.

All 14 diff paths examined: Cargo.toml, Cargo.lock, README.md, crates/relay-tui/Cargo.toml, monitor.rs, monitor_department.rs, monitor_department_source.rs, monitor_main.rs, monitor_ops.rs, monitor_tracker.rs, monitor_ui.rs, docs/cli-reference.md, docs/department-monitor.md, docs/how-it-works.md. No spec/plan changes; no new spec review. No applicable AssuranceProfile exists in spec/. Canonical dev-team FR-004 and department-monitoring.md supply view semantics; ix-board owns formulas, assessments and comparisons.

## Verdict

**FAIL** — 3 findings require the original coder's fix round in the sole ticket worktree.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | All seven new FR-059 criteria remain untagged in the declared traceability engine. New tests use doc-comment Trace lines, while the active profile admits rust-trace-attribute only; comments bind nothing. Supply real supported trace attributes and their compilable macro dependency or an explicit repository trace declaration that binds the authored form. No trace macro dependency is currently present. Do not repair unrelated legacy criteria. | crates/relay-tui/src/monitor_main.rs:404 |
| FND-002 | medium | The saved-credential success fixture bypasses credential resolution entirely: its injected factory constructs a credential-free FixtureSource, does not control the environment override, and never reaches scoped startup or department collection. It would still pass if production required only LINEAR_API_KEY. Exercise the existing approved credential-provider path with a fake saved provider and no override through the real startup/source seam, without accessing real credentials. | crates/relay-tui/src/monitor_ops.rs:254 |
| FND-003 | medium | The rendered parity oracle compares the same renderer with a model and its serde round trip, so identical wrong or absent painted values pass. Subsequent assertions inspect row helpers, not the displayed cells/details. Assert source-model-derived values, statuses, evidence, routes and comparisons in actual TestBackend output, including nonempty class metrics and loop evidence. | crates/relay-tui/src/monitor_department.rs:751 |

## Coverage

Plan completion: not assessed

Reconciliation: quoin matrix --repo /Users/peter/dev/worktrees/ix-relay-swm-165 --json (quoin 0.36.7, quire 0.52.0 engine 0.62.2), exit 0. Full-repository computed results: 60 requirements, 386 untagged criteria, 3 method-without-symbol; no run evidence. FR-059-AC-1..7 each untagged with zero binders. quire coverage --scope /Users/peter/dev/worktrees/ix-relay-swm-165 --json exited 0: Rust census 1222 candidate symbols, 705 containing tags, zero bound symbols. The active spec-artifacts-process v0.29.2 profile declares rust-trace-attribute only. New FR-059 comment annotations produce 18 test-marker diagnostics plus two module-comment diagnostics. Legacy 379 untagged criteria and other existing extraction/method diagnostics remain disclosed, not assigned as broad repair work for this ticket. Raw matrix and coverage JSON are retained next to this artifact.

Reverse inventory examined all workspace crate source surfaces by CLI/module/spec ownership and searched 232 crate files for stubs, then read all changed production code and its callers/tests. Existing commands map to FR-001..058; the seven new startup flags (lead/team/project/report/retro-config/session/help), three department views, read-only decoder/collector worker, navigation, binding fence, diagnostics and exit/terminal lifecycle map to FR-059 and FR-048. No new unowned public behavior or hollow production stub found. No copied formula/model/validator; shared Record/SavedReport/Delta, collect_snapshot, assessment and comparison APIs are consumed directly. Two new test-oracle gaps are recorded below. No coverage-percentage claim. Optional extra semantic fan-out not run; behavioral test intent was examined as required by the code-review and stub procedures.

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


