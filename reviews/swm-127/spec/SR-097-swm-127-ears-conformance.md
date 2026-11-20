---
id: SR-097
title: "Requirement grammar and semantic triggers for SWM-127 named focuses"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-062-session-active-focuses-and-adoption.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md; spec/functional/FR-064-named-focus-dashboard-reader.md"
review_set: subset
---

## Summary

Ticket: SWM-127. PR ix-relay#32 at frozen 218a0ff3157d30022689b3c275fc72fd9e503961. EARS and criterion-grammar review of FR-061..064. Descriptions use "SHALL" with a clear subject. `quire validate` reports seven `ac:non-canonical-shape` warnings on the new files and none of the FR-048 `ears:` warnings come from the new text (the two FR-048 warnings at line 16 pre-date this PR).

## Verdict

**CONDITIONAL** - three low findings: given-when-then criteria flagged by quire, behavior bullets with no obligation verb, and an undefined trigger term.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | low | Seven criteria are given-when-then shaped and quire flags them as non-canonical (FR-062-AC-3, -5, -7; FR-063-AC-3, -13; FR-064-AC-4, -10) | spec/functional/FR-062-session-active-focuses-and-adoption.md:140 |
| FND-002 | low | Core FR-062 state rules are plain declaratives with no SHALL, so their obligation level is implicit | spec/functional/FR-062-session-active-focuses-and-adoption.md:43 |
| FND-003 | low | "live adoption" is the trigger for keeping revisions and definitions but is never defined, and FR-062 calls the same thing a pin | spec/functional/FR-061-named-focus-definitions.md:180 |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID unavailable; run id unavailable. Linear marker id=SR-097 method=spec-review/ears-conformance lang=spec. No AssuranceProfile applies. Evidence: validate-six.stderr lists each warning by line.

## Examined Scope

```yaml
scope:
  - {id: "FR-062-AC-3", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "Given F active at 2 and the owner publishes revision 3, the session reads update pending (adopted 2, latest 3) and its effective scope remains revision 2."}
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "- A session is in **named mode** once it has run any `work focuses` change and\n  until it runs `work focuses reset`. Before that it is in legacy mode."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Beyond the 8 newest revisions, a revision is deleted in the same atomic\n  operation that writes a newer one, unless a live adoption pins it."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "The relay SHALL store a **named focus definition**: a named, shareable\nselection of Linear work for one team."}
  - {id: "FR-063", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "The membership of a named focus, its exclusions, leaf deduplication, progress,\nreadiness and the failure state of each source SHALL be computed by the shared\nix-board library from the current workspace snapshot."}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "The existing `team-leader-dashboard` Scope view SHALL present a bound session's\nnamed active focuses"}
```
