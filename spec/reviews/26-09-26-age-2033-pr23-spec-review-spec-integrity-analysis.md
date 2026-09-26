---
id: SR-024
title: "Integrity of FR-041, FR-010, FR-032, spec.md and the test matrix at ix-relay#23 head e1ff66f"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay@e1ff66f4feed6f56a9c0949911703328c2f4b169; spec/functional/FR-041-send-prints-the-recipients-state.md, spec/functional/FR-010-send-queues-one-copy-to-the-addressed-lead.md, spec/functional/FR-032-sender-sees-undeliverable-recipients.md, spec/spec.md (gap table, interface table), spec/tests.md (FR-010-AC-7, FR-032, FR-041, TC-010-07, TC-032-01..03, TC-041-01..04 rows, GAP-001, GAP-002, Test Execution Summary), README.md, plugins/claude/skills/ix-relay/SKILL.md, plugins/codex/AGENTS.md, plugins/opencode/AGENTS.md"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-041"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-032"
    type: "reviews"
---
# SR-024: Integrity of the FR-041 change at ix-relay#23

## Summary

Ticket: AGE-2033. PR: ix-relay#23, head e1ff66f, based on main b2715ae.

Checked and clean:

- FR-041 was free on `main` at b2715ae, and is still free at the current `origin/main`
  584f8b7.
- The frontmatter `implements` US-006 resolves.
- The cross-links FR-010 to FR-041, FR-032 to FR-041 and FR-041 to FR-028/FR-032 resolve.
- The spec.md gap row and interface-table row agree with the code.
- `make check-matrix` passes: 312 rows, Integration 127.
- GAP-001 adds up: 228 written + 19 partial + 65 without a test = 312. That is 222 + 4 new
  FR-041 rows + 2 FR-032 rows moved to written, and 67 - 2 = 65.
- GAP-002 went from 17 to 15, and the Integration summary row is 93/127 = 73%.
- The README example matches the binary's output.
- The three agent-doc sentences are identical.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | The TC-032-03 description in the TC table still reads "Given a recipient with a live watcher and a pane hosting its session, `agent-msg send` prints no state line." FR-032-AC-3 was amended to "prints `watcher heartbeat from <machine>` with its last tick", and the same row's status cell cites that test. The row now contradicts its own AC. Update the description to the amended AC text. | spec/tests.md:558 |
| FND-002 | low | FR-041's last Dependencies bullet is garbled. "The sender may be an agent, its tool result shows the line" appears twice ("... needs the owner's sign-off. The sender may be an agent: its tool result shows the line, and the recipient's context does not.") on an unwrapped 200+ character line, unlike the rest of the file. Merge it into one sentence. | spec/functional/FR-041-send-prints-the-recipients-state.md:61-63 |
| FND-003 | low | Merge precondition: `git merge-tree` of this head against the current `origin/main` 584f8b7 (AGE-2030, #26, landed after b2715ae) reports a content conflict in `spec/tests.md`. #26 also changed the matrix, so GAP-001, GAP-002 and the Test Execution Summary counts here are stale once rebased. Rebase, resolve, and re-run `make check-matrix`, then recount. README.md, spec/spec.md and relay-core/src/lib.rs auto-merge. | spec/tests.md |

## Verdict

**PASS WITH FINDINGS.** The spec change is internally consistent and the counts add up at
this head. There is one stale TC description, one garbled sentence, and a rebase needed
before merge because of a `spec/tests.md` conflict with `main`.
