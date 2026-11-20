---
id: SR-086
title: "Code review of SWM-165 specification diff"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md"
review_set: subset
---

## Summary

The frozen diff changes two Markdown functional requirements only. FR-059 assigns relay-owned startup, source lifecycle, navigation and terminal/exit behavior to the existing dashboard while referencing dev-team FR-004 and ix-board FR-029 for view and shared-model semantics. FR-048 limits its mandatory binding to session-bound monitoring and explicitly preserves report authority fences.

## Verdict

PASS — no defects found in this bounded specification change. The eight-method independent review is complete; the frozen candidate is mergeable as a specification. Implementation delivery is not asserted.

## Tracker Record

Linear comment: 3e97d373-9f2c-4e89-8f23-f6c2eb46a116; method=code-review; run=D50359B5-550A-454D-87E1-92577C20D39C. Read-back passed with a real YAML parser, byte-for-byte body comparison, 39 matching scope units and clean/findings agreement with the SR placeholder. The issue already carries has-review. No finding disposition is needed.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | low | No findings (placeholder) | - |

## Ownership and Duplication

Compared the changed requirements against accepted dev-team FR-004, its department-monitoring design, ix-board FR-029, the source audit and author intent mapping already examined in this review. The producer owns the wire model, calculations, severity/completeness, readiness ordering, categorical evidence and comparisons. FR-059 requires consuming those existing results. It introduces no local equation, threshold, class policy, wire-schema copy, generated duplicate, source snapshot, repository-file tracking mechanism or extra gate.

The new obligations are the consumer boundary: explicit CLI inputs and conflicts, saved-input source isolation, keys 1/2/3, navigation selection, retention of original evidence timestamps, presentation diagnostics, collection-versus-quit outcome and terminal lifecycle. Existing local FR-048/047/016 remain the owners of binding/reporting and terminal text rules; links and AC-7 preserve those owners. No helper, executable, fixture, schema or production source is added.

## Integrity and Document Correctness

The two paths have unique local FR identities; all seven new ACs have direct observable outcomes and Test verification methods. Local links resolve to existing FR-048/047/016 paths. Cross-repository links explicitly identify agent-ix/dev-team and agent-ix/ix-board, avoiding collisions with local numbered FRs. The pinned design URL is an accepted source citation, not a mandated tool pin, hash ledger or file-tracking check.

The description qualification distinguishes optional unbound department startup from existing session-bound refusal. Saved input forbids source adapters and selector substitution; an explicit scoped view cannot authorize work reports or associate replacement session activity. Reported auth/binding failures remain reported and linked to existing owners. The approved saved-credential provider is reused rather than replaced with another auth implementation.

## Test Intent and Scope Limits

ACs specify meaningful future adapter assertions: adapters fail when offline mode touches them; projections match the supplied shared model; TestBackend fixtures exercise 80x24/120x40 and existing compact behavior; refresh failure preserves original timestamps; missing auth controls the live exit; replacement sessions and write adapters remain fenced. These contracts can fail for actual consumer regressions. This spec-only PR does not implement the behavior and does not claim existing tests bind its new ACs.

No changed production/test code exists for code-test alignment or language-specific execution. Rust/React/Python review and implementation gap-analysis are therefore not applicable to this bounded diff, as expressly confirmed by the corrected dispatch. No build, test suite, matrix, full-tree scan, UI, credential resolver, collector or pane probe was run in this supplemental method.

## Review Context

Ticket: SWM-165. PR: ix-relay#36. Frozen head: 172410960d9d0cdce765dc8ccfb5a76661474986. Base: 6a3d2409962a675f634fb808e44612cef0f09967.

This supplemental mandatory code-review method corrects the initial dispatch's exclusion of code-review. It supersedes only the code-review exclusion notes recorded in SR-079 through SR-085; their seven completed methods, findings and validations remain unchanged. This run does not repeat them. The final method set is code-review plus base, ears-conformance, integrity, dependency, evidence, failure-domain and scope-boundary.

No applicable AssuranceProfile was found by the prior repository inspection, and no new profile was added. Exact model ID is unavailable to this harness; model=unavailable is recorded rather than invented. Existing installed SpecReview contract is spec-artifacts-process@v0.29.2. Quoin 0.36.7 / quire 0.52.0 (engine 0.62.2) were established in the original pass. No frozen-branch edit, push or merge was performed.

## Checks

Re-read the exact origin/main...HEAD diff and verified the unchanged frozen head and clean worktree. SR-086 is validated independently at its explicit persistent path. Prior changed-FR validation and diff-check passed; prior full-tree structural failures are unchanged baseline context, not new defects or a claim that the full tree passes.

## Examined Scope

```yaml
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
      "id": "FR-048",
      "path": "spec/functional/FR-048-session-work-monitor.md",
      "role": "examined",
      "excerpt": "When session-bound monitoring is requested, the monitor SHALL bind an explicit\nHerdr address to one measured native harness session and process incarnation."
    },
    {
      "id": "FR-048",
      "path": "spec/functional/FR-048-session-work-monitor.md",
      "role": "examined",
      "excerpt": "Explicit department scope and saved-report startup are governed by\n[FR-059](./FR-059-department-monitor-adapter.md). An unbound department view does\nnot establish a session binding or authorize a work report."
    },
    {
      "id": "FR-048-AC-1",
      "path": "spec/functional/FR-048-session-work-monitor.md",
      "role": "context_only",
      "excerpt": "Resolve exactly one live agent by address, verify its native session and process, refuse missing/conflicting identity, and require a restart to bind a replacement. Reports are isolated by machine, native session, and process incarnation."
    },
    {
      "id": "FR-048-AC-9",
      "path": "spec/functional/FR-048-session-work-monitor.md",
      "role": "context_only",
      "excerpt": "Scope defaults to the bound workspace team key, with an explicit --team override. Initiative, project and ticket selectors narrow that team boundary. Foreign-team tasks never inflate counts; unprojected tasks belong to whole-team scope. Project focus does not display associated initiatives as initiative focus. Explicit --all-team clears narrower selectors without removing agent reports; the command refuses conflicting or absent selections."
    },
    {
      "id": "FR-048-AC-10",
      "path": "spec/functional/FR-048-session-work-monitor.md",
      "role": "context_only",
      "excerpt": "Binding accepts omitted or null argv as specified by Herdr protocol 22 while retaining verified native session and nonzero PID requirements. Omitted foreground-process lists provide no binding evidence; malformed argument types are refused."
    },
    {
      "id": "FR-004",
      "path": "/Users/peter/dev/worktrees/dev-team-swm-161/spec/functional/FR-004-terminal-views.md",
      "role": "context_only",
      "excerpt": "The monitoring TUI SHALL render status, health and retro from one versioned\nreport model independent of terminal widget code."
    },
    {
      "id": "FR-029",
      "path": "/Users/peter/dev/worktrees/ix-board-swm-163/spec/functional/FR-029-department-retro-contract.md",
      "role": "context_only",
      "excerpt": "Later collectors and TUI consumers SHALL use\nthis model and assessment engine rather than compute parallel metrics."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "The existing `team-leader-dashboard` SHALL adapt the shared ix-board department\nreport to the terminal-view contract of\n[dev-team FR-004](ix://agent-ix/dev-team/FR-004).\nThis requirement defines the relay CLI, source lifecycle and navigation boundary.\nThe canonical requirement owns the Status, Health and Retro contents;\n[ix-board FR-029](ix://agent-ix/ix-board/FR-029) owns the versioned department\nmodel, assessments and comparisons. Their metric definitions are not duplicated\nin this component."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- Explicit team and/or project selection, independent of Herdr pane identity."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- A saved ix-board report, decoded using the shared model and supported version\n  rules of ix-board FR-029; its scope and evidence timestamps remain authoritative."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- Optional explicit lead/session identity, subject to\n  [FR-048](./FR-048-session-work-monitor.md)."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- Live source results and classified collection errors through the approved\n  ix-board library interface, including its existing saved-credential resolver\n  and documented environment override."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The CLI SHALL accept `--team <team>`, repeatable `--project <project-id>` and\n  `--report <path>` as explicit department startup inputs without requiring\n  `--lead` or a native session. Project selectors identify source-native projects;\n  ix-board resolves scope rather than the TUI inferring it from ticket text."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- When `--report` is supplied, the monitor SHALL read the saved report through\n  the shared ix-board decoder without starting credential resolution, network\n  collection, Herdr probes or Redis workers. The CLI help SHALL identify the\n  shared serialized report format accepted by that decoder."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- If `--report` is combined with `--team`, `--project`, `--lead` or `--session`,\n  then the CLI SHALL reject the conflicting inputs before source access.\n  Saved scope is not silently replaced by live selectors."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- If neither explicit scope, a saved report nor a lead is supplied, then the\n  CLI SHALL reject the invocation with usage guidance before source access."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- When explicit scope is supplied and a requested lead cannot be uniquely\n  verified, the monitor SHALL retain the scoped department view with a visible\n  binding diagnostic and unknown identity-dependent fields. Without explicit\n  scope, the existing session-bound refusal remains in force."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- If a previously verified session is replaced, then the monitor SHALL retain\n  the existing restart-to-rebind fence while continuing independent department\n  reads. It SHALL NOT attach reports or activity to the replacement session."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The department adapter SHALL remain read-only: it does not write work reports,\n  register sessions, change tracker state or append/rewrite retro history."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The adapter SHALL use the shared Rust model and comparison/assessment results\n  for all department views without local metric calculation, classification,\n  threshold assessment, readiness ordering or trend arithmetic."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The keys `1`, `2` and `3` SHALL select Status, Health and Retro respectively.\n  Explicit unbound scope and saved reports SHALL start in Status. Existing\n  lead-only startup SHALL retain its Tickets default and `t`, `a`, `p` navigation."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- When a department view is selected, the monitor SHALL preserve scope and reset\n  row selection and scroll, with selection clamped after a refresh shrinks rows.\n  Existing scrolling, details, refresh and quit controls remain available."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The monitor SHALL expose department source age and availability diagnostics\n  in each department view even when the optional session-source status line is\n  hidden. Source timestamps refer to model evidence rather than time of decode\n  or successful UI refresh."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- If refresh fails, then the monitor SHALL retain the last successful model\n  with its original evidence/window timestamps and a visible last-snapshot error.\n  With no successful model, it SHALL show unavailable with the source reason."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The monitor SHALL preserve shared missing, stale, partial, unknown, unassessed,\n  unrouted, unavailable and `query_changed` results without replacing absent\n  values, baselines or routes with successful zero/green/empty equivalents.\n  Fields not supplied by a saved report remain explicitly unavailable."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The monitor SHALL keep live collection outcome separate from render/quit\n  success. If missing authentication prevents requested live collection, then\n  normal quit SHALL return nonzero and retain a safe actionable diagnostic;\n  opening and quitting a valid offline report SHALL succeed without authentication."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- If saved input is unreadable, malformed or has an unsupported version, then\n  the monitor SHALL return nonzero naming the path and decoding/version problem\n  without modifying it. Diagnostic text SHALL exclude credential values."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- The monitor SHALL restore terminal state after normal quit and startup/render\n  errors through the existing terminal lifecycle guard."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- [dev-team FR-004](ix://agent-ix/dev-team/FR-004), canonical view/startup\n  behavior, and its linked evidence/assessment contracts. The accepted\n  [counting design](https://github.com/agent-ix/dev-team/blob/174dc0c31537d03c548401cae069111340a6714b/docs/department-monitoring.md)\n  owns metric definitions and department assessment semantics."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- [ix-board FR-029](ix://agent-ix/ix-board/FR-029), implemented by SWM-163 and\n  extended by SWM-164. Integration waits for the shared model/reader release;\n  this spec change does not claim it is available in relay's current dependency."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- [FR-048](./FR-048-session-work-monitor.md), existing session-bound views and\n  reporting fences; [FR-047](./FR-047-herdr-placement-must-be-verified.md),\n  verified placement; [FR-016](./FR-016-render-neutralises-control-characters.md),\n  terminal text safety."
    },
    {
      "id": "FR-059",
      "path": "spec/functional/FR-059-department-monitor-adapter.md",
      "role": "examined",
      "excerpt": "- SWM-133 owns existing evidence-backed TUI work; SWM-91, SWM-136 and SWM-156\n  own reported credential/binding symptoms. Reuse those defect owners rather\n  than infer missing saved-auth behavior or weaken identity verification here.\n  These reported symptoms were not reproduced by this specification change."
    }
  ]
}
```

