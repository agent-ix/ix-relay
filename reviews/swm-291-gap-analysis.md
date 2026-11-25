---
id: SR-6124
title: "Gap analysis — ix-relay SWM-291 relay-tools port"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; crates/relay-tools/**, spec/functional/FR-006-registration-records-place.md, spec/spec.md, spec/tests.md (context)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-006"
    type: "references"
---

# SR-6124: Gap analysis — ix-relay SWM-291 relay-tools port

## Summary

Ticket: SWM-291. PR agent-ix/ix-relay#65 at `2d773e9d8d9737f7c4ec475308b183674d09a6df`. This planless audit covers the criteria that own the ported tools, the computed Test Matrix (`quire matrix --scope . --format json`, quire 0.52.1), the new tests and the new source. The PR says "No FR covers any of these scripts", but FR-006-AC-3 owns the Codex launcher. Its named evidence was the deleted Python test, and the Rust replacement tests carry no trace tag.

## Verdict

**FAIL**: FR-006-AC-3 is an untagged criterion whose evidence this diff replaced. The skill's rule fails any untagged in-scope criterion. The fix is one `#[trace("FR-006-AC-3")]` per launcher test plus the test name in `spec/tests.md`.

FR-006-AC-3 was also untagged on origin/main: the Python test had no `Trace:` line, and 431 criteria repo-wide are untagged. Only criteria the diff touches are judged here. The source has no stubs: every module holds real logic, and every test asserts behaviour that a source change would break. Nothing inflates coverage.

## Findings

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-001 | medium | FR-006-AC-3 ("Launcher tests verify isolation for interactive starts/resumes and passthrough for CLI commands") is owned by the ported `codex-herdr-launcher`, so the PR's statement that no FR covers these scripts is wrong. `spec/tests.md` TC-006-03 still names the deleted Python test `test_interactive_context_and_existing_environment` as its evidence. The Rust replacements are `launcher::tests::interactive_starts_resumes_and_forks_gain_no_daemon`, `noninteractive_and_outside_herdr_pass_through` and `tests/launcher.rs`, and none carries `#[trace("FR-006-AC-3")]`. `quire matrix` reports the criterion `untagged`. | spec/functional/FR-006-registration-records-place.md:22; spec/tests.md:48, 786; crates/relay-tools/src/launcher.rs:100, 116; crates/relay-tools/tests/launcher.rs:31 |
| FND-002 | low | Reverse gap: `archive-relay-namespace`, `check-test-matrix` and `capture-codex-session-start` have no owning FR or AC. The archive rests only on the scope sentence in spec.md §2 ("Old namespaces are archived, never imported"). The gate behaviour of check-test-matrix and the capture hook's privacy contract (allowlist, 0600, no symlink) are specified nowhere. This was already true of the Python, and the port leaves it so. | spec/spec.md:46; crates/relay-tools/src/archive.rs; crates/relay-tools/src/matrix.rs; crates/relay-tools/src/session_start.rs |

## Coverage

- Plan completion: not assessed
- Criteria in scope: FR-006-AC-3, untagged (the computed matrix lists no binders). FR-006-AC-1 and FR-006-AC-2 were read for context only; this diff does not touch them.
- Reverse gap: four new binaries. One (the launcher) has an owning criterion; three have none (FND-002).
- Stubs and coverage inflation: none found in `crates/relay-tools`.
- Semantic review (intent↔test↔code): skipped, because it was not requested in the reviewer brief. The fidelity of each port to the deleted Python was checked by differential runs and is recorded in SR-6123.
