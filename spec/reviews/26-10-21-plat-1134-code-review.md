---
id: SR-070
title: "PLAT-1134 ix-cli-kit adoption code review"
type: SpecReview
analysis: code-review
scope: "ix-relay plat-1134-kit-adoption candidate versus origin/main@b9e0e797; Cargo.lock; crates/agent-msg/Cargo.toml; crates/agent-msg/src/{json.rs,main.rs,work.rs,doctor/report.rs}; crates/agent-msg/tests/{doctor.rs,inbox_json.rs,location_enrollment.rs,version.rs,work_json.rs}; crates/relay-watcher/{Cargo.toml,tests/startup.rs,tests/version.rs}; spec/functional/FR-049-shared-cli-json-and-version.md; .github/workflows (unchanged)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-049"
    type: "reviews"
---

## Summary

Applied the Rust-review and code-review checklists to the complete PLAT-1134 candidate and its fix round. The implementation routes five typed CLI JSON outputs through ix-cli-kit, adds version agreement tests for both compiled binaries, and leaves hook, spool, watcher runtime, domain exits, and CI recipes unchanged. The initial candidate's whole-line allocation and synthetic-only byte test were corrected in the fix round.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | `inbox --json --max N` accepts any nonzero `usize` and `Relay::pop_inbox` LPOPs the requested messages before rendering. The new `json::write_line` allocates a second complete JSON String where the old writer streamed. With a sufficiently large valid inbox, this extra allocation can exhaust memory after the only queued copies were removed and before any output is written. Preserve streaming encoding or impose a justified bounded pop before removal. | `crates/agent-msg/src/json.rs:10`; `crates/agent-msg/src/main.rs:501`; `crates/relay-core/src/store.rs:647` |
| FND-002 | medium | FR-049 requires exact existing JSON bytes, field order, escaping, and one newline for five commands, but the new AC-1 test serializes only a synthetic struct through the helper. Existing command tests mostly parse output into `serde_json::Value`, so a change in real CLI field order or newline count can pass. Assert the relevant actual-binary bytes, including a string needing escaping and a zero-row `find` result. | `crates/agent-msg/src/json.rs:20`; `spec/functional/FR-049-shared-cli-json-and-version.md:64` |

## Verdict

**PASS after fix round.** The implementation uses the authoritative kit streaming API without a whole-line allocation; the typed output schemas remain in their owning modules. Actual command tests now assert exact output bytes, including escaping, field order, zero-row `find`, and one newline. `doctor --json` retains its broken-pipe handling and a process test covers a closed pipe. Both version tests exercise compiled binaries and correctly keep `--help` out of Agreement because its Redis URL is misread as a version. The implementer owns the aggregate gate; this review did not rerun it. No CI workflow changed.

## Dispositions

| Finding | Outcome | Evidence |
| --- | --- | --- |
| FND-001 | fixed | `crates/agent-msg/src/json.rs` calls `ix_cli_kit::json::write_compact` from merged kit commit `7f3b58b6`, then writes the newline. The kit streams through `serde_json::to_writer`, preserving writer errors without allocating the full output. |
| FND-002 | fixed | Actual binary tests in `inbox_json.rs`, `location_enrollment.rs`, `doctor.rs`, and `work_json.rs` assert exact bytes and framing; inbox covers escaped newline and quotes, and find covers zero rows. |
