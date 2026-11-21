---
id: SR-095
title: "Code review (spec) - named focus contract FR-061 to FR-064 (SWM-127)"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-062-session-active-focuses-and-adoption.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md; spec/functional/FR-064-named-focus-dashboard-reader.md; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-060-focused-scope-presentation.md"
review_set: subset
---

## Summary

Ticket: SWM-127. PR ix-relay#32 at frozen 218a0ff3157d30022689b3c275fc72fd9e503961 (base 8dd31a8). Spec-only code review (lang=spec; no Rust, Cargo or test change in the diff, so rust-review has nothing to run). The contract keeps membership in ix-board, adds no relay calculator, leaves the accepted FR-049 untouched and sets no mandatory worker budget. It restates the upstream ix-board interface in the relay spec, and its two new CLI names differ by one letter.

## Verdict

**CONDITIONAL** - two medium findings, no high. Duplication check: I searched spec/ for a second statement of the ix-board selection types and of the bound values. The bounds (128 agents, 128-byte selectors, 512-byte focus, 64 KiB field) match crates/relay-core/src/store/work.rs and src/work.rs, so they are cited correctly, not copied.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-063 restates ix-board's existing types and the six-item interface ix-board must build, a second statement of an upstream contract inside the relay spec | spec/functional/FR-063-shared-focus-membership-and-summaries.md:110 |
| FND-002 | medium | `agent-msg work focus <summary>` and the new `agent-msg work focuses <verb>` differ by one letter, so a typo silently stores free text instead of activating a focus | spec/functional/FR-061-named-focus-definitions.md:207 |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID unavailable; run id unavailable. Linear marker id=SR-095 method=code-review lang=spec. No AssuranceProfile applies. gap-analysis not run: the diff has no production code. Read-only apart from evidence artifacts and Linear comments. This reviewer is a fresh session, distinct from spec author session 76e51a64.

## Examined Scope

```yaml
scope:
  - {id: "FR-063", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "The pinned ix-board revision `8381a601` provides `FocusSelection` and\n`ScopeSelection` (inclusion lists only), `FocusRecord` and `FocusMember` (four\nmember kinds, a schema version, no team, no exclusions), and `ScopeSummary` (leaf\nIDs, done and remaining for one selection)."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "`agent-msg focuses` is a new command family. The existing `agent-msg focus\n[text] [--clear]` (FR-057) and `agent-msg work focus <summary>` keep their meanings,\nso no new subcommand is placed under either name."}
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "| `work focuses activate <id>@<rev>...` | add focuses to the active set at the stated revisions |"}
  - {id: "FR-063-AC-12", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "Inspection of the relay confirms it contains no membership, exclusion, deduplication, progress or readiness calculation for named focuses and no Linear reader beyond the shared library's read service."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Out of scope: following another session's active set, planner publication,\n  worker budgets and goal thresholds (an optional monitor policy, never required\n  to create a definition), nested focuses, unarchive, and Linear writes."}
  - {id: "FR-048", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: "For a session\n  in named mode only, FR-064 replaces the absent-scope whole-team default of AC-3\n  and AC-9 with \"no active focus\"."}
  - {id: "FR-060", path: "spec/functional/FR-060-focused-scope-presentation.md", role: examined, excerpt: "Named focuses: for a session in named mode only, the Scope view follows\n  [FR-064](./FR-064-named-focus-dashboard-reader.md), which states exactly which\n  statements of this requirement it takes precedence over."}
  - {id: "FR-049", path: "spec/functional/FR-049-shared-cli-json-and-version.md", role: context_only, excerpt: "The CLI shall keep each command's existing JSON bytes and exit behavior."}
```

## Dispositions

Round 1 against source head e4931a965ef90d3ccbd3a37bee169ea8c0ebd611 (fix commits a39c0bd and e4931a9 on the reviewed frozen head 218a0ff3157d30022689b3c275fc72fd9e503961). Original findings above are unchanged. No new finding for this method.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed a39c0bd | FR-063 no longer lists the six-item interface or type descriptions. The relay needs are conditions (FR-063 lines 133-155), the owning ix-board contracts are referenced (FR-010, FR-012, FR-016), and FR-063-AC-16 inspects that the relay restates no selection types. |
| FND-002 | fixed a39c0bd | The session group is `work named` (FR-062 lines 27-29) and the library group is `focuses` (FR-061 lines 325-327); neither is a prefix of, or one letter from, `work focus`. FR-061-AC-22 and FR-062-AC-20 assert that `work focus` text is still stored as free text. |

## Verdict after disposition pass 1

**PASS** for this method. Both findings are fixed; no new finding. Spec-only: no implementation, test or runtime claim.
