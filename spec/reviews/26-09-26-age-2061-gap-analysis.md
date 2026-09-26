---
id: SR-026
title: "Gap analysis of ix-relay PR #33, AGE-2061: FR-046 acceptance criteria against tests and code"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@5fe6eaa46c4b9dcd7f52f1c7506060266539c250; spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md (AC-1..AC-11), spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (amended bullet), spec/tests.md (FR-046 rows, TC-046-01..11, GAP-001, method counts), crates/claude-wire/src/*.rs, crates/claude-wire/tests/contract.rs, crates/claude-wire/fixtures/claude-2.1.283/*, crates/relay-watcher/src/socket/deliverer.rs, crates/relay-watcher/src/socket/transcript.rs, crates/relay-watcher/src/socket/session.rs, crates/relay-watcher/src/socket/tests.rs (tc_046_*), crates/relay-core/src/socket_status.rs, crates/agent-msg/src/doctor/checks.rs (tc_046_10, tc_046_11), crates/agent-msg/src/doctor/report.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-046"
    type: "reviews"
---
# SR-026: Gap analysis of ix-relay PR #33, AGE-2061

## Summary

Ticket: AGE-2061. This is a planless gap analysis ("Plan completion: not assessed"). It
checks FR-046's 11 acceptance criteria against the tagged tests, the Test Matrix and the
code at 5fe6eaa. Every TC-046 row names tests that exist, carry `Tracing: TC-046-NN` /
`ACs: FR-046-AC-N` tags, and pass. The tests ran on a scratch copy: claude-wire had 26 unit
tests, 5 contract tests and the proptests, and relay-watcher had 98 socket tests. The
`tc_046` tests ran 20 times with no failure. `check_test_matrix.py` recounts 341 rows.
The GAP-001 totals and the per-method table are consistent with the 11 new rows.

## Verdict

Coverage of FR-046 as written is complete, with low-severity strength gaps. The larger gap
is that no AC or test covers what happens to a `ShapeUnrecognised` batch after its lease.
That is the delivery-path consequence recorded in SR-025 FND-001 and SR-029 FND-001. The
rows here are low, and the merge decision rests on SR-025.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | No AC or test covers a `ShapeUnrecognised` batch after the ack wait and lease: redelivery through the socket, the `MAX_REDELIVERIES` bound, the dead letter and a notification per attempt. `tc_046_05_*` stops at "the next batch is still posted to the socket". This is the behaviour behind SR-025 FND-001, and whatever FR-046 finally specifies there needs an integration test of the whole cycle. | crates/relay-watcher/src/socket/tests.rs:4084-4101, spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:67-71 |
| FND-002 | low | FR-046-AC-3 says "a receipt for another message [is] not counted", and the test comment claims it, but `tc_046_03_unrecognised_and_non_json_reply_lines_are_counted_and_change_nothing` only exercises a liveness connection. No receipt for another msg_id is sent. | crates/relay-watcher/src/socket/tests.rs:4037-4042 |
| FND-003 | low | Two test-strength gaps. For AC-8, the contract test checks only that `drift-*` files are `synthetic`. Relabelling `receipt-delivered.json` from `derived-from-program-text` to `captured` still passes (checked on a scratch copy), so a provenance label is not pinned per file. For AC-1, `the_library_does_no_io` is a literal-substring scan for `std::fs` and the like, so `use std::{fs, io}` passes. The `cargo tree` dependency check is what actually holds. | crates/claude-wire/tests/contract.rs:158-188, crates/claude-wire/tests/contract.rs:308-326 |
| FND-004 | low | The new partial-line drop in `Watch::records` (a tail window starting inside a line) has no test, and it has the exact-boundary defect in SR-025 FND-002: an origin line starting exactly at the window start is lost. A property test that splits a transcript at every byte offset around the origin line would catch it (reproduced on a scratch copy at offset 0). | crates/relay-watcher/src/socket/transcript.rs:112-138 |

## AC-by-AC

| AC | Evidence | Assessment |
|----|----------|------------|
| FR-046-AC-1 | `the_crate_depends_on_serde_and_serde_json_only`, `the_library_does_no_io`; `cargo tree` confirms | met (FND-003 on strength) |
| FR-046-AC-2 | six proptests plus `every_verdict_is_reached_by_its_exact_strings`, `each_line_class_is_exact`; reviewer fuzz 25k+ per entry point, deep nesting | met |
| FR-046-AC-3 | `tc_046_03_*` (2) | partial: the other-message receipt is not exercised (FND-002) |
| FR-046-AC-4 | `tc_046_04_an_unknown_receipt_status_is_ambiguous_and_counted` | met |
| FR-046-AC-5 | `tc_046_05_*` (2); mutation-checked | met as written; the post-lease consequence is unspecified (FND-001) |
| FR-046-AC-6 | `tc_046_06_*`, four `transcript::tests` | met |
| FR-046-AC-7 | `the_verdict_is_typed_for_each_way_to_be_unsupported`, `a_record_decodes_camel_case...`, `versions_parse_and_compare`, fixtures | met |
| FR-046-AC-8 | `every_fixture_parses_to_its_exact_typed_result`, `the_manifest_the_directory_and_the_expectations_agree`; delete or extra fixture fails | met (FND-003 on labels) |
| FR-046-AC-9 | seven envelope tests; dropping the `<` escape fails four | met |
| FR-046-AC-10 | `tc_046_10_doctor_shows_the_captured_version_and_flags_an_unverified_one` | met |
| FR-046-AC-11 | `tc_046_11_...`, codes pinned in `tc_029_06` | met |

## Code with no owning requirement

- The tail-window partial-line drop in `socket/transcript.rs` is owned implicitly by
  FR-044-AC-4 and FR-046-AC-6, but no AC names it (FND-004).
- `RecordVerdict::Malformed` maps to `Unresolved::NoSocket`, and the
  `messaging_socket_path.ok_or_else` after it is now unreachable. That is harmless and
  owned by FR-044-AC-2.

## Stub and inflation check

No stubs, and no tautological asserts in the new tests. Each counter assertion compares an
exact count read from the status file on disk. The TC-046 rows claim mutation checks. Five
of them were re-run independently on a scratch copy and all hold: `ShapeUnrecognised` read
as absent, the counters dropped, `settle` acking the shape, a substring status match, and
the version or protocol gate dropped (SR-025 Summary).
