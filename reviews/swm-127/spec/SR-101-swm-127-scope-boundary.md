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

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-003 | low | The ordering paragraph of the allocation note says SWM-128/129 can start after acceptance and that the category API blocks only completed-project hiding, but the ix-board owner's handoff says SWM-178 blocks SWM-128 and SWM-131 and the project category (SWM-177) is already done | swm-127/spec-author/migration-and-allocation.md:54-57 |

Found while rechecking the fixed allocation note against the owner's contract handoff. FND-003 has no outcome yet and goes back to the author. Its source text is in the machine record of the disposition comment.

## Dispositions

Round 1 against source head e4931a965ef90d3ccbd3a37bee169ea8c0ebd611 (fix commits a39c0bd and e4931a9 on the reviewed frozen head 218a0ff3157d30022689b3c275fc72fd9e503961). Original findings above are unchanged. FND-003 is new this round and has no outcome yet.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed a39c0bd | The allocation table (migration-and-allocation.md lines 38-52) now matches the accepted allocation: 128 definitions and CLI, 129 active records, 130 adoption and claim liveness, 131 SDK adapter and dashboard named view, 134 skills, 135 two-lead acceptance. SWM-132 and SWM-133 are not repurposed and not dependencies. The note is outside the repository, so the fix commit is the one that revised it. Its ordering paragraph is a separate new finding (FND-003). |
| FND-002 | fixed a39c0bd | Tombstones, ownership history and `prune` are removed and listed as out of scope (FR-061 lines 426-429). One mechanism goes beyond pin release: removal of archived definitions at the 256-definition limit, by `create` only, at most 8, archived at least 90 days and holding no pin (FR-061 lines 291-296, FR-061-AC-17). It is bounded, adds no command and is needed so archived definitions cannot fill the limit, so it is accepted. |

### Round 2

Round 2 against source head 8b5a68f91373b76a061445cc066e57585e40bea2 and the private notes `spec-author/migration-and-allocation.md` and `spec-author/upstream-api-needs.md` as they stand after fix round 2. Only the finding whose latest outcome was open is rated.

| FND | outcome | reason |
|---|---|---|
| FND-003 | fixed (private note; repository commit 8b5a68f) | The ordering in migration-and-allocation.md (lines 54-70) now reads: SWM-176 delivery and the accepted SWM-127 contract, then SWM-178 accepted, then SWM-128, 129, 130, 131, 134 and 135, and says SWM-178 blocks SWM-128 and SWM-131 and is blocked by SWM-127 and SWM-176. It states that the SWM-177 category is already done and blocks nothing, and that SWM-132 and SWM-133 stay parked, are not repurposed and are not prerequisites. `upstream-api-needs.md` (lines 33-34) gives the same sequence. This matches the ix-board owner's handoff. |

Regression check: the allocation table (lines 44-52) is unchanged and still matches the accepted allocation (FND-001). The diff from e4931a9 to 8b5a68f touches no tombstone, history or `prune` text, adds no worker budget, threshold, nested focus, automatic adoption, daemon or Linear writer, and its new criteria (FR-063-AC-17, AC-18) come from the owner's evidence on blockers and catalog classification, so FND-002 holds and no scope has widened.

## Verdict after disposition pass 1

**CONDITIONAL** for this method. FND-001 and FND-002 are fixed. One new low finding (FND-003) is open and goes back to the author.

## Verdict after disposition pass 2

**PASS** for this method. All three findings are fixed and nothing regressed; no new finding.
