---
id: SR-023
title: "Spec review (EARS) of ix-relay PR #23, AGE-2033: FR-041 and the amended FR-010-AC-7, FR-032-AC-1/AC-3"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay@e1ff66f4feed6f56a9c0949911703328c2f4b169; spec/functional/FR-041-send-prints-the-recipients-state.md (Description, Behavior, AC-1..AC-4), spec/functional/FR-010-send-queues-one-copy-to-the-addressed-lead.md (AC-7), spec/functional/FR-032-sender-sees-undeliverable-recipients.md (AC-1, AC-3, Dependencies)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-041"
    type: "reviews"
---
# SR-023: Spec review (EARS) of ix-relay PR #23

## Summary

Ticket: AGE-2033. PR: ix-relay#23, head e1ff66f. This checks EARS conformance and whether
each criterion can fail, for the new FR-041 and the three criteria the PR amends.
`quire validate --scope . 'spec/**/*.md'` is clean for these files. Its only warnings are
the two FR-030 line-41 EARS warnings, which come from `main` and not this PR.

Clean:

- The FR-041 Description is a well-formed event-driven statement ("When `agent-msg send`
  queues a message for a single lead, `agent-msg send` shall print ...").
- The timeout, the failure fallback and the exit code are split into separate
  single-`shall` bullets, with proper unwanted-behaviour (`If ... then ... shall`) forms.
- The stdout-only, nothing-to-inbox, broadcast and machine-name bullets are atomic.
- AC-2 and AC-4 are concrete, and each can fail. The amended FR-010-AC-7 and
  FR-032-AC-1/AC-3 are concrete and consistent with FR-041.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | The first Behavior bullet is compound. It holds three `shall` clauses, two non-`shall` statements ("reads as `looks stuck`", "as `reports herdr failing its check`"), and one untestable clause, "The clause shall carry no advice". FR-041-AC-1's "No variant ... advises" has no oracle: the test checks only for the absence of `DOWN` and `waits for`. Split the bullet into one `shall` per variant, and replace "no advice" with a closed list: "the clause shall be exactly one of the following forms", which AC-1 then enumerates. | spec/functional/FR-041-send-prints-the-recipients-state.md:20-29, spec/functional/FR-041-send-prints-the-recipients-state.md:50 |
| FND-002 | low | FR-041-AC-3's bound "exits 0 in under 2.5 s" does not verify the Behavior it traces to ("a timeout of 250 ms"). The AC holds with a 1 s timeout (mutant measured at about 2.1 s). The real worst case at 250 ms is about 0.5 s, one timeout per pipelined reply (SR-021 FND-001). State the bound the design gives, for example "within 1 s", and word the Behavior as a per-reply timeout or a total deadline. | spec/functional/FR-041-send-prints-the-recipients-state.md:34, spec/functional/FR-041-send-prints-the-recipients-state.md:52 |
| FND-003 | low | Missing unwanted-behaviour clause: a heartbeat whose last tick is later than the sender's clock (skew between machines, or a bad writer). FR-041 defines only "older than two sweeps and ten seconds". The code then prints a negative age (`last tick -29s ago`) and treats any future tick as live (SR-021 FND-002). Add: "If the heartbeat's last tick is later than the sender's clock, then the sender shall print its age as 0 s" (or a named clock-ahead variant), with an AC. | spec/functional/FR-041-send-prints-the-recipients-state.md:20-29 |

## Verdict

**PASS WITH FINDINGS.** The requirement is well formed apart from one compound bullet with
an untestable "no advice" clause, an AC whose bound is too loose to verify the 250 ms
Behavior, and one missing clock-skew clause. All are low, and none blocks.
