---
id: SR-001
title: "Base review of the ix-relay specification"
type: SpecReview
analysis: base
scope: "spec/spec.md, spec/stakeholder, spec/usecase, spec/functional, spec/non-functional"
review_set: subset
---
# SR-001: Base review of the ix-relay specification

## Summary

Checklist review of the ix-relay specification, re-measured against `main` at
de799f9 (after AGE-2026): 3 StR, 7 US, 37 FR and 4 NFR — 206 obligations: 184 FR
acceptance criteria, 2 NFR acceptance criteria and 20 NFR measurement rows. Identifiers are well formed,
sequential and unique; every FR links a US and every US a StR; `quire validate` is
clean. The open items are the absence of test cases (expected: evidence is the
next stage, AGE-2029), the known gaps against `main` that the spec names
deliberately, and a few acceptance criteria that are weaker than they look.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | No acceptance criterion has a test case yet; the Test Matrix rows are all pending. Expected at this stage — evidence is written next, keyed to these ids. | spec/tests.md |
| FND-002 | medium | 52 obligations belong to requirements `main` (de799f9) does not yet meet (spec.md section 12): FR-018-AC-4 and the NFR-002 watcher-prompt row (AGE-2031), FR-020-AC-1/3 (AGE-2032), FR-022-AC-6/7 and the NFR-001 stdin row (no ticket yet), NFR-003 rows 2 and 3 and FR-028 to FR-037 (AGE-2033). Evidence written first will be red, which is the point. | FR-018, FR-020, FR-022, FR-028..FR-037, NFR-001, NFR-002, NFR-003 |
| FND-003 | low | The name-before-pane-id-before-address priority in FR-002 is not observable between names and the other two forms: bus names cannot contain `:`, while herdr pane ids and addresses always do, so they never compete. An acceptance criterion claiming to test that precedence would be vacuous; FR-002-AC-2 was narrowed to the name path during review. The precedence that is observable is pane id before address (FR-002-AC-5). | FR-002-AC-2, FR-002-AC-5 |
| FND-004 | low | FR-010-AC-6 and FR-012-AC-4 both state at-most-once delivery under concurrent pops; one suite can discharge both, but they should not be counted as independent evidence. | FR-010-AC-6, FR-012-AC-4 |
| FND-005 | low | User stories carry no Priority and Risk section; priority is tracked on the Linear tickets instead. | US-001, US-002, US-003, US-004, US-005 |
| FND-006 | low | Resolved on re-measurement: FR-016 now names the substitute (U+FFFD), matching `relay_core::text::sanitise`. | FR-016 |
| FND-007 | medium | Re-measurement after AGE-2026 changed three statements to match the code, which is right: a pane id typed to `send` now resolves on the sender's machine only (FR-002, FR-004-AC-2, FR-009); the unsafe-character rule is wider than first written (FR-015 adds U+2028/9, bidi controls and invisible format characters); escalation ids are `^e[0-9]{1,20}$`, so `e0` is a valid id that is simply never issued (FR-023). | FR-002, FR-004, FR-009, FR-015, FR-023 |
| FND-008 | medium | Re-measurement found a real defect: `agent-msg`'s `soften_collision` keeps the plain candidate when `resolve_collision` fails with `NoFreeName` or a herdr error, so a lead can register over a live holder. Specified as FR-022-AC-6/7 (red); needs a ticket. | FR-022-AC-6, FR-022-AC-7 |
| FND-009 | low | FR-035 (no grace window for an old address) rests on the operator's decision of 2026-09-25 as relayed by the AGE-2029 coordinator; it deliberately leaves bus-name aliases (FR-002) untouched. Confirm that aliases after `/rename` should keep working while addresses do not. | FR-035, FR-002 |
