---
id: SR-067
title: "Gap analysis — ix-relay FR-029, FR-030, FR-046, FR-053, FR-054, FR-055 (fix/doctor-signal-stranded-mail)"
type: SpecReview
analysis: gap-analysis
scope: "spec/functional/FR-029, FR-030, FR-046, FR-053, FR-054, FR-055; their tagged tests and implementing code in crates/ at 02691bd (diff 9875a0f..02691bd)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/spec"
    type: "references"
---

## Summary

Planless gap analysis of six requirements against their tests and code at 02691bd.
`quire matrix --strict` (quire 0.36.2) reports every criterion in the repository as
`untagged`, including criteria unrelated to this branch. The cause is that this repo binds
tests with `/// ACs: FR-xxx-AC-n`, and quire's declared tag forms do not recognise that line.
The repo already worked this way before the branch, so this review does not count it as a
gap the branch introduced. A grep index of `ACs:`/`Trace:` lines finds a tagged test for
all 68 criteria (FR-029: 16, FR-030: 12, FR-046: 19, FR-053: 13, FR-054: 4, FR-055: 4).
Four gaps remain, all in FR-053, plus one FR-055 timing claim the code does not meet.

## Verdict

**CONDITIONAL**: no `high` findings. FR-053's reporting bullet and its "replaced only"
forwarding text disagree with the code, two criteria are only partly met by design, and
the FR-055 "no delay" claim does not hold on macOS. "Every criterion tagged" here rests on
the grep fallback, not on quire's computed matrix.

## Findings

| ID      | Severity | Summary | Refs | Escape Cause |
| ------- | -------- | ------- | ---- | ------------ |
| FND-001 | medium | FR-053's Reporting bullet ("`agent-msg who` and `agent-msg doctor` shall stop reporting a registration as undeliverable or `registration_pane_gone` for mail this requirement moved") has no code and no test. A forwarded-from registration stays registered by design. `who` keeps printing `UNDELIVERABLE` from the replaced evidence regardless of its inbox. Doctor's `registration_pane_gone` keeps saying "mail queued for it waits until the lead registers again" until the ordinary prune. Fix: make both conditions depend on a non-empty inbox (or reword them), and add a test. | spec/functional/FR-053-stranded-mail-is-forwarded-or-dead-lettered.md (Reporting); crates/agent-msg/src/main.rs:561; crates/agent-msg/src/doctor/checks.rs:1650 | correct-requirement-no-evidence |
| FND-002 | medium | FR-053 says a session whose address is held by one verified registration is forwarded. The code decides this from an in-memory address map, so after a watcher restart the address is unknown and the mail is dead-lettered at 24 h instead. No test covers a pass with an unknown address and a live holder. See SR-066 FND-003. | crates/relay-watcher/src/stranded.rs:156-163 | implementation-bug-despite-evidence |
| FND-003 | low | FR-053's "A new session holds the address" bullet says "When a session is replaced". The code also forwards when the old session is gone and exactly one verified holder has the address (`Left::Gone` → `Holder::Verified`). The Description ("no longer hosts that session") supports the code. Fix: amend the bullet to "replaced or gone" and add an AC/test for the gone-with-holder case (none exists). | crates/relay-watcher/src/stranded.rs:157-158 | wrong-requirement |
| FND-004 | low | FR-053-AC-10 and AC-13 require dead letters "with reason `recipient-pruned`". The dead-letter list has no reason field yet (FR-031's reason field is not built), so the reason exists only in the watcher's log line, and TC-053-10/13 assert the log. Both criteria are partly met. Fix: open a tracking ticket that links AC-10/13 to the FR-031 reason work. In socket mode, a `DeadLetterNote` could be recorded through the existing status-file reason channel in the meantime. | crates/relay-core/src/store/stranded.rs:120-125; crates/relay-watcher/src/stranded.rs:225-245 | correct-requirement-no-evidence |
| FND-005 | low | FR-055 says entries are dropped "without delaying ... any delivery". On macOS, each tick's liveness check forks `ps` per resolved lead before delivery, under the deliverer lock. TC-055-0x use a fake `Procs`, so nothing measures this. See SR-066 FND-002. | crates/relay-watcher/src/socket/deliverer.rs:784-798 | implementation-bug-despite-evidence |

## Coverage

- Plan completion: not assessed
- Matrix: `quire matrix --strict` exits 1. All 68 in-scope criteria (and every other
  criterion in the repo) show `untagged` because quire does not parse the `/// ACs:` form
  this repo uses. Grep fallback: 68/68 criteria have at least one tagged test.
  `make check-matrix` (the repo's own spec/tests.md recount) passes.
- Underspecified code: no new behavior without an owning requirement. The new public APIs
  (`HookHealth::distinct`, `Relay::forward_stranded`, `Relay::dead_letter_stranded`,
  `DriftBook`, `claude_wire::scan_detail`, `claude_wire::line_shape`,
  `relay_core::tries_socket`) each trace to FR-030/FR-053/FR-046/FR-054 and have callers.
- Stubs / inflation: no `todo!`/`unimplemented!` added. Test-strength problems are in
  SR-066 FND-004 (TC-053-04/05 can pass vacuously). TC-053-02 asserts the notice through
  `AddressBook::render`, the watcher prompt (a whole tick) and the hook
  (tests/stranded_notice.rs). The socket batch and the inbox listing reuse the same
  renderer (`PoppedBatch::rendered`, `inbox_report` via `AddressBook::render`).
- Semantic review (intent↔test↔code): not run as a separate opt-in pass, because the
  reviewer could not ask the owner. The code review (SR-066) covered intent↔code for the
  scoped requirements.
