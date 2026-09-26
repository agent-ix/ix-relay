---
id: SR-022
title: "Gap analysis of ix-relay PR #23 (AGE-2033) against FR-041 and the FR-010/FR-032 criteria it amends"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@e1ff66f4feed6f56a9c0949911703328c2f4b169; spec/functional/FR-041-send-prints-the-recipients-state.md, spec/functional/FR-010-send-queues-one-copy-to-the-addressed-lead.md (AC-7), spec/functional/FR-032-sender-sees-undeliverable-recipients.md (AC-1, AC-3), spec/tests.md (FR-010-AC-7, FR-032-AC-1..3, FR-041 rows, TC-010-07, TC-032-01..03, TC-041-01..04), crates/agent-msg/src/send_state.rs, crates/agent-msg/src/main.rs, crates/relay-core/src/store/health.rs, crates/agent-msg/tests/send_state.rs, crates/agent-msg/tests/round_trips.rs, crates/agent-msg/tests/address.rs, crates/relay-core/tests/health.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-041"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-032"
    type: "reviews"
---
# SR-022: Gap analysis of ix-relay PR #23 (AGE-2033)

## Summary

Ticket: AGE-2033. PR: ix-relay#23, head e1ff66f. This is a planless gap analysis. It follows
each FR-041 criterion, and the FR-010-AC-7 and FR-032-AC-1/AC-3 amendments, from the Test
Matrix to the tagged tests and then to the code. Plan completion: not assessed.

Every FR-041 Behavior bullet has code behind it: `send_state::observe`/`describe`,
`Relay::recipient_probe`, and the `[one]` arm of `main.rs`'s send. The tests the matrix
names are real, not stubs, and each dies under the mutant that removes its own behaviour
(see SR-021, Verification). `make check-matrix` passes: 312 rows, and Integration 127.
FR-032-AC-1 and AC-3 are now delivered through FR-041's test, and the matrix says so.

Code with no owning requirement: none. `RecipientProbe` and `recipient_registration` both
serve FR-041.

## AC-by-AC

| AC | Test(s) | Result |
|---|---|---|
| FR-041-AC-1 | `tc_041_01` (binary + Redis: no heartbeat, live, herdr failing, no pane); unit `tc_041_01` (stuck, unreadable, no `DOWN`) | covered |
| FR-041-AC-2 | `tc_010_07` (counting proxy, 6 requests, no `HGETALL`) | covered, but untagged for TC-041-02 (FND-001); the sixth request's content is not pinned (SR-021 FND-005) |
| FR-041-AC-3 | `tc_041_03` (stall proxy) | covered as written; does not pin the Behavior's 250 ms (FND-003) |
| FR-041-AC-4 | `tc_041_04` (pop and render carry no state; broadcast with 1 recipient prints no state) | covered |
| FR-041 Behavior: machine name at most 32 chars, no control characters | untagged unit test `a_machine_name_is_bounded_and_carries_no_control_character` | no AC, no matrix row (FND-002) |
| FR-010-AC-7 (amended) | `tc_010_07` | covered |
| FR-032-AC-1 | `tc_041_01` (no-heartbeat line, exit 0, message queued) | covered |
| FR-032-AC-3 (amended) | `tc_041_01` (heartbeat line with last tick) | covered |

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | The matrix row TC-041-02 cites `tc_010_07_send_by_name_or_alias_reads_only_the_recipient`, but that test carries only `Tracing: TC-010-07` / `ACs: FR-010-AC-7`. A tag-driven trace recount finds no test for FR-041-AC-2. Add `TC-041-02` / `FR-041-AC-2` to its tags. | crates/agent-msg/tests/round_trips.rs:363-364, spec/tests.md:285 |
| FND-002 | low | The FR-041 Behavior bullet "every machine name in the state as one line free of control characters and at most 32 characters long" has no acceptance criterion and no matrix row. Its only test, `a_machine_name_is_bounded_and_carries_no_control_character`, is untagged. That test is the only one that kills the bound-removed and sanitiser-removed mutants, so the security-relevant clause has no traced evidence. Add an AC (for example FR-041-AC-5) with a row, and tag the test. | spec/functional/FR-041-send-prints-the-recipients-state.md:43-44, crates/agent-msg/src/send_state.rs:168-177 |
| FND-003 | medium | The Behavior "arm that request with a timeout of 250 ms" has no test that fails when the timeout is raised. Mutant `PROBE_TIMEOUT = 1000 ms` survives every test, because TC-041-03 bounds the whole stalled send at 2.5 s and the real cost is about 2 x the timeout (see SR-021 FND-001). Tighten TC-041-03's bound, together with the AC wording (SR-023 FND-002). | spec/functional/FR-041-send-prints-the-recipients-state.md:34, crates/agent-msg/tests/send_state.rs:269 |

## Verdict

**PASS WITH FINDINGS.** FR-041's criteria are implemented and backed by real tests, and
FR-032-AC-1 and AC-3 are closed honestly. The gaps are traceability (FND-001, FND-002) and
one Behavior clause that no test enforces (FND-003). None blocks merge. FND-003 should be
fixed with SR-021 FND-001 in the fix round.
