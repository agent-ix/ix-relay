---
id: FR-059
title: Adapt the existing dashboard to shared department reports
type: FR
relationships:
  - target: ix://agent-ix/dev-team/US-001
    type: satisfies
  - target: ix://agent-ix/dev-team/FR-004
    type: depends_on
  - target: ix://agent-ix/ix-board/FR-029
    type: depends_on
  - target: ix://agent-ix/ix-relay/FR-048
    type: constrains
---
# FR-059: Adapt the existing dashboard to shared department reports

## Description

The existing `team-leader-dashboard` SHALL adapt the shared ix-board department
report to the terminal-view contract of
[dev-team FR-004](ix://agent-ix/dev-team/FR-004).
This requirement defines the relay CLI, source lifecycle and navigation boundary.
The canonical requirement owns the Status, Health and Retro contents;
[ix-board FR-029](ix://agent-ix/ix-board/FR-029) owns the versioned department
model, assessments and comparisons. Their metric definitions are not duplicated
in this component.

## Inputs

- Explicit team and/or project selection, independent of Herdr pane identity.
- A saved ix-board report, decoded using the shared model and supported version
  rules of ix-board FR-029; its scope and evidence timestamps remain authoritative.
- Optional explicit lead/session identity, subject to
  [FR-048](./FR-048-session-work-monitor.md).
- Live source results and classified collection errors through the approved
  ix-board library interface, including its existing saved-credential resolver
  and documented environment override.

## Behavior

### Startup and source selection

- The CLI SHALL accept `--team <team>`, repeatable `--project <project-id>` and
  `--report <path>` as explicit department startup inputs without requiring
  `--lead` or a native session. Project selectors identify source-native projects;
  ix-board resolves scope rather than the TUI inferring it from ticket text.
- When `--report` is supplied, the monitor SHALL read the saved report through
  the shared ix-board decoder without starting credential resolution, network
  collection, Herdr probes or Redis workers. The CLI help SHALL identify the
  shared serialized report format accepted by that decoder.
- If `--report` is combined with `--team`, `--project`, `--lead` or `--session`,
  then the CLI SHALL reject the conflicting inputs before source access.
  Saved scope is not silently replaced by live selectors.
- If neither explicit scope, a saved report nor a lead is supplied, then the
  CLI SHALL reject the invocation with usage guidance before source access.
- When explicit scope is supplied and a requested lead cannot be uniquely
  verified, the monitor SHALL retain the scoped department view with a visible
  binding diagnostic and unknown identity-dependent fields. Without explicit
  scope, the existing session-bound refusal remains in force.
- If a previously verified session is replaced, then the monitor SHALL retain
  the existing restart-to-rebind fence while continuing independent department
  reads. It SHALL NOT attach reports or activity to the replacement session.
- The department adapter SHALL remain read-only: it does not write work reports,
  register sessions, change tracker state or append/rewrite retro history.

### Presentation and navigation

- The adapter SHALL use the shared Rust model and comparison/assessment results
  for all department views without local metric calculation, classification,
  threshold assessment, readiness ordering or trend arithmetic.
- The keys `1`, `2` and `3` SHALL select Status, Health and Retro respectively.
  Explicit unbound scope and saved reports SHALL start in Status. Existing
  lead-only startup SHALL retain its Tickets default and `t`, `a`, `p` navigation.
- When a department view is selected, the monitor SHALL preserve scope and reset
  row selection and scroll, with selection clamped after a refresh shrinks rows.
  Existing scrolling, details, refresh and quit controls remain available.
- The monitor SHALL expose department source age and availability diagnostics
  in each department view even when the optional session-source status line is
  hidden. Source timestamps refer to model evidence rather than time of decode
  or successful UI refresh.
- If refresh fails, then the monitor SHALL retain the last successful model
  with its original evidence/window timestamps and a visible last-snapshot error.
  With no successful model, it SHALL show unavailable with the source reason.
- The monitor SHALL preserve shared missing, stale, partial, unknown, unassessed,
  unrouted, unavailable and `query_changed` results without replacing absent
  values, baselines or routes with successful zero/green/empty equivalents.
  Fields not supplied by a saved report remain explicitly unavailable.

### Collection outcome and terminal lifecycle

- The monitor SHALL keep live collection outcome separate from render/quit
  success. If missing authentication prevents requested live collection, then
  normal quit SHALL return nonzero and retain a safe actionable diagnostic;
  opening and quitting a valid offline report SHALL succeed without authentication.
- If saved input is unreadable, malformed or has an unsupported version, then
  the monitor SHALL return nonzero naming the path and decoding/version problem
  without modifying it. Diagnostic text SHALL exclude credential values.
- The monitor SHALL restore terminal state after normal quit and startup/render
  errors through the existing terminal lifecycle guard.

## Acceptance Criteria

| ID | Criteria | Verification |
|---|---|---|
| FR-059-AC-1 | Injected startup fixtures for team-only, project-only and saved-report inputs reach a department view without a lead/session. Missing and ambiguous lead identity with explicit scope show unknown bound activity plus the binding reason; the same failures without explicit scope retain refusal. No nearby pane or session is selected. | Test |
| FR-059-AC-2 | A valid saved shared-model report opens with credential, network, Herdr and Redis adapters that fail if called; switching all three department views and quitting succeeds. Conflicting saved/live selectors and absent scope/report/lead fail before any adapter call. Malformed/unsupported input names the path and problem, exits nonzero and leaves input bytes unchanged. | Test |
| FR-059-AC-3 | One shared-model fixture and its serialized JSON show matching values, statuses, completeness, evidence references, routes and current/previous/baseline comparisons in all three views. `query_changed`, absent baseline, categorical evidence and missing routes retain their model meanings; a red/incomplete class stays red/incomplete. No live collection is needed to render the fixture. | Test |
| FR-059-AC-4 | TestBackend fixtures at 80x24 and 120x40 switch views with 1/2/3, retain scope, reset selection/scroll, reach the final row and clamp selection after shorter data. Details expose evidence and owner/action; long text and control characters cannot hide diagnostics or inject terminal controls. Existing compact Tickets/Agents/Scope fixtures and their keys retain their outputs. | Test |
| FR-059-AC-5 | A successful model followed by failed refresh keeps identical evidence/window timestamps, shows last-snapshot/error and remains navigable with the optional session-source status line hidden. Missing model, stale source, partial relation pages and unreachable host fixtures retain their distinct shared diagnostics; refreshing cannot make stale evidence fresh. | Test |
| FR-059-AC-6 | Injected saved-credential success with the environment override absent permits live scoped collection; injected missing-auth failure shows a credential-safe actionable error and normal quit returns nonzero. The same failure adapters are never called for an offline report. A blocked source read leaves quit responsive; normal quit and terminal-initialization/render error fixtures restore terminal state. | Test |
| FR-059-AC-7 | Bound-session replacement preserves the explicit restart requirement and prevents replacement activity/report attribution while department data remains navigable. Existing FR-048 reporting isolation and Tickets/Agents/Scope behavior remain unchanged; department reads produce no work-report, registration, tracker or retro-store write. | Test |

## Dependencies

- [dev-team FR-004](ix://agent-ix/dev-team/FR-004), canonical view/startup
  behavior, and its linked evidence/assessment contracts. The accepted
  [counting design](https://github.com/agent-ix/dev-team/blob/174dc0c31537d03c548401cae069111340a6714b/docs/department-monitoring.md)
  owns metric definitions and department assessment semantics.
- [ix-board FR-029](ix://agent-ix/ix-board/FR-029), implemented by SWM-163 and
  extended by SWM-164. Integration waits for the shared model/reader release;
  this spec change does not claim it is available in relay's current dependency.
- [FR-048](./FR-048-session-work-monitor.md), existing session-bound views and
  reporting fences; [FR-047](./FR-047-herdr-placement-must-be-verified.md),
  verified placement; [FR-016](./FR-016-render-neutralises-control-characters.md),
  terminal text safety.
- SWM-133 owns existing evidence-backed TUI work; SWM-91, SWM-136 and SWM-156
  own reported credential/binding symptoms. Reuse those defect owners rather
  than infer missing saved-auth behavior or weaken identity verification here.
  These reported symptoms were not reproduced by this specification change.
