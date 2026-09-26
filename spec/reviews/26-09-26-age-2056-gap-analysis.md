---
id: SR-007
title: "Gap analysis of ix-relay PR #29 (AGE-2056) against FR-022"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@b10190314cfd711e6f7f470eb7251cc265439f91; spec/functional/FR-022-name-collisions-resolve-to-a-free-name.md, spec/functional/FR-021-derived-names-are-valid.md, spec/tests.md, crates/relay-core/src/naming.rs, crates/relay-ops/src/naming.rs, crates/relay-core/tests/names.rs, crates/relay-ops/tests/collision.rs"
review_set: subset
---
# SR-007: Gap analysis of ix-relay PR #29 against FR-022

## Summary

Ticket: AGE-2056. This is a planless, diff-scoped audit of the change against
FR-022 (AC-1 to AC-9) and FR-021, the Test Matrix rows TC-022-01 to TC-022-06,
and the tagged tests. Plan completion was not assessed. The change makes the
Description rule ("a valid bus name that differs from the candidate") hold at
the `with_suffix` level for every input the reviewer swept. It also adds the
missing resolver-level evidence for FR-022-AC-3. That evidence fails on `main`
and passes on the branch against real Redis. The one gap is that the
behaviour this PR adds (changing the first letter) has no line in FR-022.

## Verdict

**PASS**. Every AC touched by the change (FR-022-AC-1 and FR-022-AC-3) has
tagged, non-tautological evidence, and mutation shows that evidence fails on
`main`. The matrix recount passes (`make check-matrix`, 290 rows). No stub
code was found and there is no coverage inflation. One low finding
(missing-requirement) is recorded below.

## Findings

| ID | Severity | Summary | Refs | Escape Cause |
|----|----------|---------|------|--------------|
| FND-001 | low | FR-022's Behavior says only that the resolver "shall shorten the candidate before adding a suffix so that the suffixed name is at most 32 characters and still ends with the suffix". The branch adds behaviour this does not cover: when the shortening rebuilds the candidate, it changes the candidate's first letter (to `a`, or to `b` when it was `a`). So a variant no longer necessarily keeps the candidate's prefix. The coder's fix table says "FR-022 line 28 stays true, no spec change". That is correct, since line 28 still holds, but the added rule is code with no owning requirement line. Suggested line: "If shortening and suffixing would give the candidate itself, the name resolver shall change the candidate's first letter, keeping the suffix whole." | spec/functional/FR-022-name-collisions-resolve-to-a-free-name.md:27-28, crates/relay-core/src/naming.rs:201-215 | missing-requirement |

## AC-by-AC (touched ACs)

- FR-022-AC-1 (every chosen name is valid, and is either the candidate or
  unregistered to another session). The evidence is the
  `tc_022_01_*` tests in `relay-core/tests/names.rs` (grid, numbered,
  31/32-byte regression, seed corpus) and the unit proptest in `naming.rs`.
  They are tagged `Tracing: TC-022-01 / ACs: FR-022-AC-1`. On main's
  `naming.rs` 4 tests fail, so the evidence is real. The fuzz lanes are traced
  too, but see SR-006 FND-001 on their oracle.
- FR-022-AC-3 (a 32-character candidate held live on the same machine gives a
  name ending in `-2`). The new evidence is
  `tc_022_03_a_candidate_ending_in_a_number_still_gets_the_dash_two_variant`,
  tagged `TC-022-03 / FR-022-AC-3`. It uses real Redis, candidates of 31 and
  32 bytes, and suffixes `-2` and `-5`. It asserts `ends_with("-2")`,
  `!= candidate`, `== with_suffix(candidate, "2")`, and that the registrations
  are unchanged. It fails on main (`...-2 -> ...-3`).
- FR-022-AC-2 and AC-4 to AC-9 are not touched by the diff. Their fixtures
  build variant sets through `with_suffix`, so they follow the new behaviour.
  `make fuzz` and the lead's `make ci` cover them.

## Code with no owning requirement

The head-letter branch in `with_suffix` (FND-001). Nothing else in the diff is
production code.

## Stub and inflation check

No new stubs. The new tests assert concrete values, not `is_ok()` alone. The
matrix row for TC-022-03 names the new test, and the test exists with a
matching tag.

## Dispositions

Disposition pass 1, reviewed at `ea19b51a8f53d6ca692547089be97efd3a67b749`.

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed ea19b51 | FR-022 Behavior gains the rebuild rule (replace the byte next to the dash, keep the suffix whole) and FR-022-AC-10, with TC-022-10 in the trace and Test Matrix; the code at `naming.rs:211-221` now has an owning requirement. `make check-matrix` passes (291 rows). |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs | Escape Cause |
|----|----------|---------|------|--------------|
| FND-002 | low | The new TC-022-10 row is not backed by any tracking tag. No test carries `Tracing: TC-022-10`: the `names.rs` tests the row names are tagged `Tracing: TC-022-01` / `ACs: FR-022-AC-1, FR-022-AC-10`, and `collision.rs` `tc_022_03_a_candidate_ending_in_a_number_still_gets_the_dash_two_variant`, also named in the row, is tagged `Tracing: TC-022-03` / `ACs: FR-022-AC-3` only. The repo already uses multi-TC tags (`Tracing: TC-022-01, TC-022-02, TC-022-03, TC-022-04`), so `Tracing: TC-022-01, TC-022-10` (and `TC-022-03, TC-022-10` with `FR-022-AC-10` on the resolver test) would close it. `make check-matrix` does not check tags, so it passes. | spec/tests.md:176, spec/tests.md:477, crates/relay-core/tests/names.rs:265-266, crates/relay-ops/tests/collision.rs:295-296 | trace |
