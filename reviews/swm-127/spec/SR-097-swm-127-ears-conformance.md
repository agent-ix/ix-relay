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

## Dispositions

Round 1 against source head e4931a965ef90d3ccbd3a37bee169ea8c0ebd611 (fix commits a39c0bd and e4931a9 on the reviewed frozen head 218a0ff3157d30022689b3c275fc72fd9e503961). Original findings above are unchanged. No new finding for this method.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed a39c0bd | Every criterion of FR-062, FR-063 and FR-064 at e4931a9 reads as a direct assertion with one Verification method. The quire warning count was not re-run by this reviewer; the root records that result separately. |
| FND-002 | still-open | The named-mode rules of FR-062 are still plain declaratives: lines 50-52 (named mode), 56 (16-focus limit), 59-67 (layout) and 71-75 (no time-to-live). Only the empty-set bullet (lines 53-55) and the snapshot bullet (lines 68-70) carry SHALL. The fix report says obligation verbs were added; the file at e4931a9 does not show it. Restate those bullets with SHALL. |
| FND-003 | fixed a39c0bd | The term `pin` is defined once in FR-061 "Adoption pins" (lines 231-257) and FR-061-AC-15; "live adoption" no longer appears in FR-061 or FR-062. |

### Round 2

Round 2 against source head 8b5a68f91373b76a061445cc066e57585e40bea2 (the diff from e4931a9 changes FR-061, FR-062 and FR-063 only). Only the finding whose latest outcome was still-open is rated.

| FND | outcome | reason |
|---|---|---|
| FND-002 | fixed 8b5a68f | Every bullet of FR-062 "Named mode and the active record" (lines 50-86) now states an obligation with an actor and SHALL or SHALL NOT: mode determination, record creation and reset, empty set, 16-focus limit, record key and fields, field and read bounds, snapshot copy, no time-to-live, removal only by reset, supersession or reclamation, the 64-record bound, and legacy fields unchanged. The meaning is unchanged: no time-to-live, empty never means whole team, reset keeps the record with mode legacy. |

Regression check at 8b5a68f: FND-001 (criteria as direct assertions) still holds, including the new FR-063-AC-17 and AC-18 and the extended FR-061-AC-8, AC-9 and AC-20; FND-003 (the term pin) is untouched. The quire warning count is recorded separately by the root.

## Verdict after disposition pass 1

**CONDITIONAL** for this method. FND-001 and FND-003 are fixed; FND-002 is still-open (low) and goes back to the author. No new finding.

## Verdict after disposition pass 2

**PASS** for this method. All three findings are fixed and nothing regressed; no new finding.
