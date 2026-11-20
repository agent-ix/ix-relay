---
id: SR-099
title: "Cross-repository prerequisites and cycle review for SWM-127 named focuses"
type: SpecReview
analysis: dependency
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-062-session-active-focuses-and-adoption.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md; spec/functional/FR-064-named-focus-dashboard-reader.md"
review_set: subset
---

## Summary

Ticket: SWM-127. PR ix-relay#32 at frozen 218a0ff3157d30022689b3c275fc72fd9e503961. Dependency review of the `relationships:` edges and prose dependencies of FR-061..064. The declared edges form a chain FR-061 <- FR-062 <- FR-063 <- FR-064 with no declared cycle, but FR-061's own criteria need FR-062's pin mechanism, and the ix-board prerequisite is not an edge.

## Verdict

**CONDITIONAL** - two medium findings, one low. The shared-interface prerequisite and "one dependency update after acceptance" are stated in prose and match the approved plan. The ix-board owns the interface; no local shadow of the calculation is specified.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-061-AC-9 to AC-12 cannot pass without the FR-062 pin and adopter mechanism, yet FR-062 depends on FR-061, a semantic cycle that makes the definition and active-record deliveries order-dependent | spec/functional/FR-061-named-focus-definitions.md:255 |
| FND-002 | medium | The hard prerequisite on ix-board's unbuilt interface (and the project status category) is only in prose; FR-061, FR-063 and FR-064 declare no relationship edge to it | spec/functional/FR-063-shared-focus-membership-and-summaries.md:1 |
| FND-003 | low | Prose dependencies on FR-057, FR-063, FR-064, NFR-002 and NFR-004 (FR-061), FR-059 (FR-064) and FR-061 (FR-064 owner state) have no relationship edge | spec/functional/FR-061-named-focus-definitions.md:5 |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID unavailable; run id unavailable. Linear marker id=SR-099 method=spec-review/dependency lang=spec. No AssuranceProfile applies. No Jev client or criterion-strength tool was used. I could not run `git ls-remote` (denied), so the author's note that no remote branch uses FR-050..052 or FR-061+ is not independently verified; main's spec/functional has no FR-050..052 and no FR-061+ before this PR.

## Examined Scope

```yaml
scope:
  - {id: "FR-061-AC-11", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Given 20 updates with one revision pinned by a live adoption, the pinned revision and the 8 newest remain and no other revision does."}
  - {id: "FR-061-AC-10", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "the 129th live definition, the 257th definition, and the 33rd adopting session each fail with `limit` and no change."}
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "relationships:\n  - target: \"ix://agent-ix/ix-relay/FR-048\"\n    type: \"constrains\"\n  - target: \"ix://agent-ix/ix-relay/FR-061\"\n    type: \"depends_on\""}
  - {id: "FR-063", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "ix-board owns the interface in \"Required shared interface\". The relay waits for\n  that accepted contract and, once it exists, one dependency update; it does not\n  pin intermediate revisions."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "relationships:\n  - target: \"ix://agent-ix/ix-relay/FR-048\"\n    type: \"constrains\"\n  - target: \"ix://agent-ix/ix-relay/FR-049\"\n    type: \"depends_on\"\n  - target: \"ix://agent-ix/ix-relay/FR-047\"\n    type: \"depends_on\""}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "relationships:\n  - target: \"ix://agent-ix/ix-relay/FR-048\"\n    type: \"constrains\"\n  - target: \"ix://agent-ix/ix-relay/FR-060\"\n    type: \"constrains\""}
  - {id: "FR-049", path: "spec/functional/FR-049-shared-cli-json-and-version.md", role: context_only, excerpt: "  - target: \"ix://agent-ix/ix-cli-kit\"\n    type: \"depends_on\""}
```
