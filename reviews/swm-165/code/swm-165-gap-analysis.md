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



## Dispositions

Round1, frozen5b56910ba9f4300cadbde5ba9475c0d46cfa1637. Original findings and earlier verdict above are preserved verbatim.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | Actual ix-trace-rs v0.1.1 git tag, package 0.1.0 at 2ce4ebf, compiles bare attributes imported from the crate. Computed FR-059 AC1..7 bindings are 5/4/4/4/1/7/1, all tagged. No trace profile was weakened. Legacy comments still produce diagnostics, and 379 unrelated criteria remain untagged; repository assurance is incomplete. |
| FND-002 | fixed | Accepted ix-board8381a601 delegates production from_env to the same provider seam. Tests use SecretStore<SavedBackend>, override None, real SDK HTTP client against authenticated loopback, LiveLinear::with_source with owned cache, parsed scoped startup, actual DepartmentWorker and shared collect_snapshot. Saved/denied/missing/offline tests pass; denied store never falls back or makes HTTP/cache calls; missing auth quits nonzero and offline mode resolves no provider and succeeds. No real credentials used. |
| FND-003 | fixed | Typed and decoded models are independently checked against actual painted cells and scrolled details at 80x24 and120x40. Assertions cover model value/unit, class severity/completeness, missing reasons, route owner/action, metric evidence, previous query_changed and unavailable baseline, and nonempty loop event IDs/timestamps/participants/dedup key. The same-renderer equality oracle was removed. |

## Disposition round1 evidence

Frozen disposition head: 5b56910ba9f4300cadbde5ba9475c0d46cfa1637, independently matching GitHub PR37. Sole worktree clean. Fix commits 3a53df3, 4cc9092, 14ac84f, 78cef2b and5b56910 inspected. Accepted board pin8381a601; kit remains7f3b58b. No spec changes, no new spec review.

Independent checks: cargo test -p relay-tui --all-targets --locked exit0 (45 inbox +41 dashboard); cargo clippy -p relay-tui --all-targets --locked -- -D warnings exit0; cargo fmt --check exit0 (existing stable rustfmt option warnings); cargo deny --offline check sources exit0. The latter verifies the new exact git allowlist entry, not advisory/license/full-gate status. Root owns one final aggregate make ci; none run by reviewer.

Independent Rust probe compiled against exact source modules/current retained dependency artifacts: rustc0, execution0. All four concurrent diagnostics are visibly summarized across1/2/3 at80x24 and their complete details accessible. Known bound SWM issue paints0/1; project-only remains unbound; explicit team wins. Logs in render-probe.log. Production sources untouched.

quoin matrix exit0 (0.36.7, quire0.52.0 engine0.62.2): 7 tagged FR059 criteria with5/4/4/4/1/7/1 binders; 379 legacy untagged and3 method-without-symbol. quire coverage exit0:18 real Rust symbols bound,1227 candidates. Existing comment-marker diagnostics remain, including17 redundant FR059 comment annotations despite real attributes; these comments do not contribute the successful bindings. No run evidence in the store. Repository-wide assurance remains INCOMPLETE; this pass fixes the new-ticket bindings rather than claiming full coverage. Plan completion: not assessed.

Saved/denied/missing-provider test bodies and real SDK constructors inspected; only fake providers and127.0.0.1 were exercised. No SecretStore::system, real Keychain/environment credential lookup, native-pane or external live collector invoked by reviewer. No global environment mutation, store/report/registration writes or service start. Historical91/136/156 remain reported, not reproduced. Diff whitespace check reports only trailing blank lines in preserved original reviewer artifacts; original finding bytes are intentionally unchanged.

Every own compiler/test/probe/matrix command completed with final status; BUILD QUIESCENT. Retained target cache untouched. No PTY repeat: the changed paths are exercised via actual TestBackend/source fixtures; the earlier guard/PTY evidence remains historical rather than relabeled as a new run. No new defects found in the fix diff.

## Disposition round1 scope and after excerpts

```json
{
  "scope": [
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
  ],
  "dispositions": [
    {
      "fnd": "FND-001",
      "outcome": "fixed",
      "artifact_id": "FR-059-AC-1",
      "related_ids": [
        "FR-059-AC-2",
        "FR-059-AC-3",
        "FR-059-AC-4",
        "FR-059-AC-5",
        "FR-059-AC-6",
        "FR-059-AC-7"
      ],
      "path": "crates/relay-tui/src/monitor_main.rs",
      "after_excerpt": "    #[trace(\"FR-059-AC-1\", \"FR-059-AC-2\")]\n    #[test]\n    fn explicit_scopes_and_saved_input_need_no_native_binding_or_live_factory() {",
      "reason": "Actual ix-trace-rs v0.1.1 git tag, package 0.1.0 at 2ce4ebf, compiles bare attributes imported from the crate. Computed FR-059 AC1..7 bindings are 5/4/4/4/1/7/1, all tagged. No trace profile was weakened. Legacy comments still produce diagnostics, and 379 unrelated criteria remain untagged; repository assurance is incomplete."
    },
    {
      "fnd": "FND-002",
      "outcome": "fixed",
      "artifact_id": "FR-059-AC-6",
      "related_ids": [
        "FR-004-AC-3"
      ],
      "path": "crates/relay-tui/src/monitor_department_source/auth_tests.rs",
      "after_excerpt": "    let source = LinearGraphQlSource::from_credential_store(\n        store,\n        None,\n        || panic!(\"saved credential or denied store must not select fallback\"),\n        endpoint,\n    )\n    .map_err(|error| crate::monitor_ops::source_diagnostic(&error))?;\n    let linear = retro::live::LiveLinear::with_source(source, cache)\n        .map_err(|error| crate::monitor_ops::source_diagnostic(&error))?;",
      "reason": "Accepted ix-board8381a601 delegates production from_env to the same provider seam. Tests use SecretStore<SavedBackend>, override None, real SDK HTTP client against authenticated loopback, LiveLinear::with_source with owned cache, parsed scoped startup, actual DepartmentWorker and shared collect_snapshot. Saved/denied/missing/offline tests pass; denied store never falls back or makes HTTP/cache calls; missing auth quits nonzero and offline mode resolves no provider and succeeds. No real credentials used."
    },
    {
      "fnd": "FND-003",
      "outcome": "fixed",
      "artifact_id": "FR-059-AC-3",
      "related_ids": [
        "FR-004-AC-1"
      ],
      "path": "crates/relay-tui/src/monitor_department.rs",
      "after_excerpt": "                let status = screen(&mut app, width, height)\n                    .split_whitespace()\n                    .collect::<Vec<_>>()\n                    .join(\" \");\n                assert!(\n                    status.contains(&format!(\"{} ok {number}\", metric.id)),\n                    \"{status}\"\n                );",
      "reason": "Typed and decoded models are independently checked against actual painted cells and scrolled details at 80x24 and120x40. Assertions cover model value/unit, class severity/completeness, missing reasons, route owner/action, metric evidence, previous query_changed and unavailable baseline, and nonempty loop event IDs/timestamps/participants/dedup key. The same-renderer equality oracle was removed."
    }
  ]
}
```

## Disposition round1 conclusion

All three original findings fixed; no new findings. Ticket-specific new traceability and test-oracle gaps are closed. Repository-wide assurance remains INCOMPLETE because379 pre-existing criteria remain untagged; no full-assurance PASS is claimed. Merge remains gated on root's final aggregate check.

