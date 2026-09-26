---
id: SR-008
title: "Spec review (Test Matrix) of ix-relay PR #29, AGE-2056"
type: SpecReview
analysis: base
scope: "agent-ix/ix-relay@b10190314cfd711e6f7f470eb7251cc265439f91; spec/tests.md (TC-022-03 trace row, line 169, and matrix row, line 469)"
review_set: subset
---
# SR-008: Spec review (Test Matrix) of ix-relay PR #29

## Summary

Ticket: AGE-2056. The only spec artifact the PR changed is `spec/tests.md`,
in one trace row: TC-022-03 now names
`tc_022_03_a_candidate_ending_in_a_number_still_gets_the_dash_two_variant`.
That test exists at the reviewed sha, carries the tag
`Tracing: TC-022-03 / ACs: FR-022-AC-3`, and `make check-matrix` passes
(290 rows). `fuzz/README.md` also changed, but it is not a spec artifact and
is covered by SR-006. The row is accurate, apart from one wording issue and a
test-level label that predates this PR.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | The edited row now reads "`tc_022_01_every_collision_…` and `tc_022_03_a_candidate_ending_…`, exhaustive over 456 combinations against the real registry". The "exhaustive over 456 combinations" qualifier reads as applying to both tests, but `tc_022_03` covers 4 cases (31 and 32 bytes, `-2` and `-5`). | spec/tests.md:169 |
| FND-002 | low | The matrix row for TC-022-03 classifies it as `Unit`. Its evidence, the old `tc_022_01_every_collision_…` and the new `tc_022_03_…`, is Redis-backed integration testing (it skips without `IX_RELAY_TEST_URL`). The label predates this PR, and the PR adds a second integration test under it. | spec/tests.md:469 |

## Verdict

**PASS**. The trace is correct and the matrix recount passes. Both findings
are wording and classification only, and neither blocks merge.

## Dispositions

Disposition pass 1, reviewed at `ea19b51a8f53d6ca692547089be97efd3a67b749`.

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed ea19b51 | The row now says `tc_022_03_...` covers "(4 cases, real registry)" and "the first is exhaustive over 456 combinations". |
| FND-002 | fixed ea19b51 | TC-022-03 is relabelled `Integration`; the summary counts move with it (Integration 114/78, Unit 112/78, Property 48/38 with TC-022-10) and GAP-001 reads 205 of 291; `make check-matrix` passes. |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-003 | low | The edited Behavior line "The name resolver shall check every widened or suffixed name for a collision before choosing it, and shall check a name that both a widened and a numbered suffix produce once." is now a compound requirement (two `shall` clauses), and `quire validate` reports a new warning on it, `[ears:non-canonical-trigger] non-EARS trigger 'before'` at FR-022 line 35, absent on `main`. "check ... once" also reads as "at least once"; "only once" or a separate statement ("The name resolver shall not check the same name twice.") is unambiguous and splits the clause. | spec/functional/FR-022-name-collisions-resolve-to-a-free-name.md:34-36 |
