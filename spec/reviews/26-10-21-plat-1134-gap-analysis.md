---
id: SR-071
title: "PLAT-1134 changed-scope gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "ix-relay plat-1134-kit-adoption candidate and fix round; FR-049-AC-1..4; crates/agent-msg/src/{json.rs,main.rs,work.rs,doctor/report.rs}; crates/agent-msg/tests/{doctor.rs,version.rs,inbox_json.rs,location_enrollment.rs,work_json.rs}; crates/relay-watcher/src/main.rs; crates/relay-watcher/tests/{startup.rs,version.rs}"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-049"
    type: "reviews"
---

## Summary

The initial matrix found binders for FR-049-AC-1 through AC-3 but none for AC-4. The initial AC-1 test covered only a synthetic helper, and the code-path review found the post-pop whole-line allocation described in SR-070. The fix round added direct binary byte assertions and watcher trace tags, and switched to the kit's streaming writer.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | The `inbox --json` path has no pre-pop output-size bound and the new whole-line allocation can fail after an at-most-once pop, leaving neither a queued message nor an emitted JSON report. This conflicts with the existing NFR-002 resource bound and FR-049-AC-2's preservation claim. | `crates/agent-msg/src/json.rs:10`; `crates/relay-core/src/store.rs:647`; `spec/non-functional/NFR-002-bounded-resources.md` |
| FND-002 | medium | FR-049-AC-1 has only the synthetic helper binder. Real `inbox`, `find`, `whoami`, `doctor`, and `work show` JSON bytes and their newline/field-order behavior lack changed-scope test evidence; parser-based assertions cannot verify the byte contract. | `crates/agent-msg/src/json.rs:20`; `spec/functional/FR-049-shared-cli-json-and-version.md:64` |
| FND-003 | medium | FR-049-AC-4 is untagged in the computed matrix. Existing watcher tests check version propagation with synthetic `9.9.9` inputs but do not bind the compiled binary's package version to heartbeat and OTLP metadata. | `crates/relay-watcher/src/main.rs:617`; `crates/relay-watcher/src/main.rs:626`; `spec/functional/FR-049-shared-cli-json-and-version.md:67` |

## Coverage

The recomputed matrix binds FR-049-AC-1 to eight tests, including actual command byte assertions; AC-2 to the doctor's closed-pipe test; AC-3 to both compiled-binary version tests; and AC-4 to the heartbeat and OTLP process tests. Semantic review: skipped (optional, not requested). Plan completion: not assessed.

## Verdict

**PASS for the changed scope after the fix round.** All four FR-049 criteria are now tagged, and the resource and actual-byte evidence gaps are fixed. This review does not claim repository-wide trace completeness.

## Dispositions

| Finding | Outcome | Evidence |
| --- | --- | --- |
| FND-001 | fixed | `crates/agent-msg/src/json.rs` now calls the kit's streaming `write_compact`; it does not allocate a complete JSON string after an inbox pop. |
| FND-002 | fixed | Exact actual-binary output assertions cover inbox, find, whoami, doctor, and work show, including escaping and zero-row find. |
| FND-003 | fixed | Both watcher process tests now carry `Trace: FR-049-AC-4`; the recomputed matrix binds AC-4 to both. They assert the heartbeat and OTLP service version against `env!("CARGO_PKG_VERSION")`. |
