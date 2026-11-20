---
id: SR-096
title: "IDs, direct contracts and adverse-case coverage for SWM-127 named focuses"
type: SpecReview
analysis: base
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-062-session-active-focuses-and-adoption.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md; spec/functional/FR-064-named-focus-dashboard-reader.md; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-060-focused-scope-presentation.md"
review_set: subset
---

## Summary

Ticket: SWM-127. PR ix-relay#32 at frozen 218a0ff3157d30022689b3c275fc72fd9e503961. Base checklist: `quire validate` on the six changed files exits 0 and `quire matrix` mints 51 criteria (FR-061 14, FR-062 12, FR-063 13, FR-064 12), all `untagged`, which is expected before implementation and is not a finding. No hash, digest or pin antipattern. Three coverage and criterion-quality gaps.

## Verdict

**CONDITIONAL** - IDs are well formed and unique; FR-049 is untouched and FR-061..064 do not collide with it. Three medium/low findings on criterion quality and an uncovered refusal.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-064-AC-10 mixes a Test and a Demonstration in one criterion and includes "recorded in the pull request", which no test can fail | spec/functional/FR-064-named-focus-dashboard-reader.md:166 |
| FND-002 | low | FR-061-AC-10 bundles about eleven independent boundary assertions in one criterion, so one failing limit hides the rest | spec/functional/FR-061-named-focus-definitions.md:254 |
| FND-003 | low | FR-062 behavior of `already_active`, the same-revision no-op and name-based `deactivate` has no criterion | spec/functional/FR-062-session-active-focuses-and-adoption.md:82 |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID unavailable; run id unavailable. Linear marker id=SR-096 method=spec-review/base lang=spec. No AssuranceProfile applies. quire 0.52.0 (engine 0.62.2). Evidence: validate-six.stdout/stderr and matrix.tsv in the evidence directory. Absence of tests, trace tags and spec/tests.md rows for FR-061..064 is not raised: the PR is spec-only.

## Examined Scope

```yaml
scope:
  - {id: "FR-064-AC-10", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "before and after results for the same fixture are recorded in the pull request and the cache stays bounded. | Test and Demonstration |"}
  - {id: "FR-061-AC-10", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Given boundary-sized valid input (128 selectors, 128-byte IDs and name, 512-byte description, 32 KiB revision) it is accepted; one extra selector or byte, the 129th live definition, the 257th definition, and the 33rd adopting session each fail with `limit` and no change."}
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "`activate` of a focus already active at the same revision is a no-op for that\n  focus. Activation of one already active at another revision is refused\n  (`already_active`); `adopt` is the operation that changes the revision."}
  - {id: "FR-061-AC-1", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "`focuses create` stores revision 1 with canonical IDs only, a `foc-` ID, and the caller as owner at epoch 1."}
  - {id: "FR-062-AC-3", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "`adopt F@3` then makes it current; `adopt F@2` is refused; adopt on a focus not active is refused."}
  - {id: "FR-063-AC-6", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "Given an empty inclusion list, the result is an empty focus with no rows and zero leaves, not the whole team; with zero active focuses, the combined result is \"no active focus\"."}
  - {id: "FR-064-AC-1", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "Given a session with no named record, the Scope view output is identical to the FR-060 fixtures, including whole-team fallback and ticket-only labels"}
  - {id: "FR-049", path: "spec/functional/FR-049-shared-cli-json-and-version.md", role: context_only, excerpt: "id: FR-049 title: \"Shared CLI JSON encoding and version agreement\" (untouched; no id collision with FR-061..064)"}
```
