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



## Dispositions

Round1, frozen5b56910ba9f4300cadbde5ba9475c0d46cfa1637. Original findings and earlier verdict above are preserved verbatim.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | Each failing source receives its own visible summary line; e opens selectable full diagnostics and details scroll. Independent exact-source 80x24 probe covers all four simultaneous source failures in Status/Health/Retro and reads every complete diagnostic. The live tracker path supplies a compact typed authentication remedy. |
| FND-002 | fixed | Status and its scope header use the effective team. Verified bound workspace is the fallback when no explicit team/projects exist; project-only scope stays independent and an explicit team wins. Independent renderer probe shows 0/1 done for a known SWM issue instead of 0/0. |

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
      "artifact_id": "FR-059-AC-4",
      "related_ids": [
        "FR-059-AC-1",
        "FR-059-AC-6"
      ],
      "path": "crates/relay-tui/src/monitor_department.rs",
      "after_excerpt": "    let failures = diagnostic_rows(app);\n    let [header, tabs, source, body, diagnostics, controls] = Layout::vertical([\n        Constraint::Length(3),\n        Constraint::Length(1),\n        Constraint::Length(2),\n        Constraint::Min(3),\n        Constraint::Length(u16::try_from(failures.len()).unwrap_or(4)),\n        Constraint::Length(1),\n    ])\n    .areas(frame.area());",
      "reason": "Each failing source receives its own visible summary line; e opens selectable full diagnostics and details scroll. Independent exact-source 80x24 probe covers all four simultaneous source failures in Status/Health/Retro and reads every complete diagnostic. The live tracker path supplies a compact typed authentication remedy."
    },
    {
      "fnd": "FND-002",
      "outcome": "fixed",
      "artifact_id": "FR-048-AC-9",
      "related_ids": [
        "FR-059-AC-7"
      ],
      "path": "crates/relay-tui/src/monitor.rs",
      "after_excerpt": "    pub fn department_team(&self) -> Option<String> {\n        self.team.clone().or_else(|| {\n            if !self.projects.is_empty() {\n                return None;\n            }\n            self.binding\n                .as_ref()\n                .and_then(|binding| binding.address.parse::<relay_core::Address>().ok())\n                .map(|address| address.workspace().to_ascii_uppercase())\n        })\n    }",
      "reason": "Status and its scope header use the effective team. Verified bound workspace is the fallback when no explicit team/projects exist; project-only scope stays independent and an explicit team wins. Independent renderer probe shows 0/1 done for a known SWM issue instead of 0/0."
    }
  ]
}
```

## Disposition round1 conclusion

PASS for the reviewed implementation fixes: both original findings fixed; no new findings. Merge remains gated on root's final aggregate check.

