---
id: SR-124
title: "EARS conformance - FR-065 copy clarification delta (SWM-179, disposition pass 1)"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay; spec/functional/FR-065-dashboard-view-selector-and-help.md (the edited statements only)"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-065", type: references }
---

## Summary

Ticket: SWM-179, PR ix-relay#44, fix 7584112 (base c2f3dff). A bounded EARS-conformance read of the statements the developer-directed copy clarification added or changed in FR-065. It is not a new spec wave: the six full spec methods of PR #43 are not re-run, no AC id changed, and the future FR-064/named-focus/metrics scope is not used.

Edited statements examined: Permitted differences item 4 (warning palette); the Help bullet (one row per entry, views only for partial entries, no requirement identifiers or owners in operator Help, Owner column spec-only); the Fields absent state; the no-failure-styling bullet for Not configured, Waiting and Fields absent; and the extended text of AC-3, AC-9, AC-10 and AC-12. Acceptance criteria rows are test descriptions, not EARS statements, so they were read for consistency only. The ownership ids stay in the spec table and out of operator text, which the spec says in one place. 1 finding: low.

The base checklist method was not run: the delta changes no id, adds no option or constraint, and none of it mentions a hash, digest, pin or version, which I read for. No other method was run.

## Verdict

**CONDITIONAL** - no high or medium finding; 1 low finding, to be fixed in this PR.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | low | The Help bullet joins four obligations (list one row per entry, state views for partial entries, show no requirement identifiers or owners, mark no key where it has no effect) and one non-requirement sentence in a single statement, so a test or reviewer cannot cite one | spec/functional/FR-065-dashboard-view-selector-and-help.md:124 |

## Finding details

### FND-001 (low) The Help bullet is a compound requirement

Check: compound. Lines 124-128 are one bullet with three `SHALL` clauses and a `SHALL NOT` joined by "and", followed by a fourth `SHALL NOT mark` sentence and the statement "The Owner column is traceability for this specification only", which carries no obligation. The earlier list bullets state one obligation each, for example "While help is open, F1, `?` and Escape SHALL close it without quitting and without changing any view state" and "The dropdown SHALL clamp the highlight at the first and last entry without wrap". AC-3 repeats the clauses in one sentence ("shows no requirement identifiers or owner text, lists every table key as one row, and states views only for entries not applicable everywhere"), so one failing clause cannot be named by its own criterion. Risk: low; the meaning is unambiguous and the test asserts each part. Fix: split the bullet into one bullet per obligation (one row per entry; views stated only for partial entries; no requirement identifiers, owners or requirement text in operator Help; no applicable mark where a key has no visible effect) and move the Owner column sentence into the lead-in of the table as a note.

## Examined statements without a finding

- Permitted item 4: one statement, testable through the AC-12 palette test; "existing warning palette" has a referent in the colour constants.
- Fields absent: three outcomes ("The message says...", "It is not Not configured, not a failed read and not healthy", "It performs no read, write or calculation") are stated in the indicative like the neighbouring Not configured, Waiting, Failed and Stale entries, so the list is consistent. I did not file this as a finding because the whole list shares the style and the governing `SHALL` is the lead-in "Health and Retro SHALL each choose one state".
- The no-failure-styling bullet: one `SHALL NOT` clause list that names exactly the three states; consistent with AC-9.
- AC-3, AC-9, AC-10, AC-12 extensions: consistent with the Behavior text; all 13 AC ids unchanged.

## Review Context

Reviewer model claude-sonnet-5-5; native run id unavailable. Marker id=SR-124 method=spec-review/ears-conformance lang=spec. I read quoin 0.29.4 spec-review and spec-ears-analysis, and FR-065 lines 40-305 at the current head. I ran no tool and no gate; this is a reading of text.

## Verdict after disposition pass 1

Not run. If the finding is fixed the same session disposes it.
