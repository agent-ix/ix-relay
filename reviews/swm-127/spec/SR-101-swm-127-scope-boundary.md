---
id: SR-101
title: "Scope-boundary analysis - named focus contract and allocation notes (SWM-127)"
type: SpecReview
analysis: scope-boundary
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-062-session-active-focuses-and-adoption.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md; spec/functional/FR-064-named-focus-dashboard-reader.md; swm-127/spec-author/migration-and-allocation.md"
review_set: subset
---

## Summary

Ticket: SWM-127. PR ix-relay#32 at frozen 218a0ff3157d30022689b3c275fc72fd9e503961. Scope-boundary review against Peter's approved named-focus plan and the code lead's allocation comment on SWM-127. The contract itself stays on the approved scope: no nested focus, no automatic adoption or following, no worker budget or threshold in definition creation, no new daemon, no second Linear reader. Two findings: the author's allocation note, and unrequested surface.

## Verdict

**CONDITIONAL** - the specification's boundary is right; the migration note's ticket table is wrong and must be replaced before it is used to update tracker tickets.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | The author's allocation table repurposes SWM-132 and SWM-133 (historical metrics, to stay parked) and omits the approved SWM-129, 130, 131, 134 and 135 ownership, contradicting the code lead's allocation comment on SWM-127 | swm-127/spec-author/migration-and-allocation.md:37 |
| FND-002 | low | Tombstones (365 days, 256), ownership history (32 events), `prune` and its 90-day archive rule add storage and CLI surface the approved plan did not ask for, with no stated user need beyond the pin leak | spec/functional/FR-061-named-focus-definitions.md:187 |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID unavailable; run id unavailable. Linear marker id=SR-101 method=spec-review/scope-boundary lang=spec. No AssuranceProfile applies. The migration note lives outside the repo at /Users/peter/dev/.dev-dept-recovery/swm-127/spec-author/migration-and-allocation.md; the repo diff itself names no ticket number. Compared against Linear comment 32879c7a (code-lead decision) and 71df7bad (approved plan).

## Examined Scope

```yaml
scope:
  - {id: "migration-and-allocation", path: "swm-127/spec-author/migration-and-allocation.md", role: examined, excerpt: "| SWM-132 | FR-064 named reader states, async refresh, bounded cache, legacy fallback separation |\n| SWM-133 | FR-064 TUI presentation in ix-cli-kit dashboard (headings, `f` key, union line, marks, \"Now\") |\n| SWM-134 | cross-feature tests: two leads Completion/Hardening, concurrency, restart, outage; performance fixture before/after |\n| SWM-135 | documentation, dev-team agent instructions, acceptance demonstration |"}
  - {id: "SWM-127 comment 32879c7a", path: "linear:SWM-127", role: context_only, excerpt: "Author's guessed mapping that repurposes132/133/134/135 is NOT accepted: actual Linear132/133 already own history and drift metrics, which are outside this approved focus feature."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Each removal leaves a tombstone (focus ID, last name, removal time) kept for 365\n  days, at most 256 tombstones, so a reader reports \"removed\" rather than\n  unknown."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Out of scope: following another session's active set, planner publication,\n  worker budgets and goal thresholds (an optional monitor policy, never required\n  to create a definition), nested focuses, unarchive, and Linear writes."}
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "Out of scope: automatic adoption, following an owner's or another session's\n  active set, a mandatory worker or goal budget, and monitor thresholds."}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "Out of scope: department metrics and monitoring policy, a second tracker reader,\n  speeding the full-team snapshot, and nested focuses."}
```
