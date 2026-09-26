---
id: SR-028
title: "Spec review (integrity) of ix-relay PR #33, AGE-2061: spec 15 inventory, 15.3 external formats, fixtures MANIFEST, README"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay@5fe6eaa46c4b9dcd7f52f1c7506060266539c250; spec/spec.md (section 15 table rows, 15.1 gap list, new 15.3 table), spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md (Provenance), spec/tests.md (StR-001 trace, FR-046 rows, GAP-001, method table), README.md (Wire formats, crates table), CLAUDE.md (crate list), crates/claude-wire/fixtures/claude-2.1.283/MANIFEST; checked against: every serde_json::Value / .get( / pointer read in crates/relay-watcher, crates/agent-msg, crates/relay-ops, crates/herdr-cli, crates/relay-core, crates/relay-tui (non-test), relay-core socket_status.rs, agent-msg hook.rs, identity.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-046"
    type: "reviews"
---
# SR-028: Spec review (integrity) of ix-relay PR #33, AGE-2061

## Summary

Ticket: AGE-2061. This checks the new 15.3 "Every external format" table, the updated
section-15 rows and gap list, the README, CLAUDE.md, tests.md and the fixtures MANIFEST.
They are checked for consistency with each other and for truth against the code at 5fe6eaa.

What holds:

- The three Claude Code rows in 15.1 now point at `claude_wire` types, and the new
  transcript `origin.msg_id` row is added.
- Every test name cited in the Claude Code rows exists.
- The cited doctor codes exist: `herdr_unchecked`, `herdr_shape_mismatch`,
  `herdr_unsupported_version`, `watcher_otlp_failing`, `registration_undecodable`,
  `socket_fallback` and the two new codes.
- The `wire_golden` test file and the key grep guard exist.
- The tests.md recount matches `check_test_matrix.py` (341 rows), and GAP-001 is updated.
- README and CLAUDE.md list the crate. The prior PRs' reviewer artifacts remain in
  `spec/reviews`.

An untyped-read sweep found four `serde_json::Value` reads of data the relay does not own,
outside `claude-wire`:

- `agent-msg/src/identity.rs:608`: the transcript `customTitle` (recorded as a gap);
- `relay-watcher/src/otlp/export.rs:355`: `rejected_points`, the OTLP response body;
- `relay-watcher/src/otlp/relay.rs:120`: `hook_failure`, a hook-health stream record
  written by another process;
- `relay-ops/src/herdr_check.rs:389`: the herdr shape check. This one is untyped on
  purpose, then decoded with `from_value`, so it is not a gap.

The herdr write calls (`agent prompt`, `notification show`, `agent start`, `tab close`)
decode their result as `Value` and discard it, so they read nothing.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | 15.3 is titled "Every external format" but leaves some out. Missing: `config.toml` (strict `FileConfig`, in 15.1 only); `/proc/<pid>/stat` start time and the pid-namespace link (`start_time_from_stat`, `Procs::domain`), kernel formats with no drift signal; the emitted `--json` outputs read by the opencode plugin; and the rendered `[ix-relay] from` line agents parse. Two untyped external reads are not stated as gaps: the OTLP response's `partialSuccess.rejectedDataPoints` (a renamed field silently reads 0 rejected) and the hook-health record read by `hook_failure` in the OTLP relay. Add the rows, or scope the title to what the table covers. | spec/spec.md:433-461, crates/relay-watcher/src/otlp/export.rs:355, crates/relay-watcher/src/otlp/relay.rs:120, crates/relay-core/src/socket_status.rs:198 |
| FND-002 | low | The updated 15.1 gap line says the `customTitle` reader "is the one Claude Code format read outside `claude-wire`". That is false. The hook stdin payload (`agent_msg::hook::HookInput`: `session_id`, `hook_event_name`, `agent_id`, `source`...) is a Claude Code format read in `agent-msg`, and 15.3 lists it with drift signal "none". Name both, or move `HookInput` under `claude-wire` in a follow-up. | spec/spec.md:374-376, spec/spec.md:446 |
| FND-003 | low | The fixture provenance labels overstate what was captured. The MANIFEST uses `captured` for shapes whose values were reconstructed: the held receipt's ids and reason text, the session record's placeholders, the consumed transcript line's surrounding fields, and `transcript-not-yet.jsonl`, which is an arrangement. `envelope-line.json` and `wrapper.txt` are bytes our own code emits, so they are goldens rather than captures. `receipt-refused.json` is noted as "live bytes", but its `from` is `uds:/run/user/1000/cc-socks/1.sock` (pid 1) and its ids are 8-hex placeholders, so it was edited. README ("`captured` (seen live)") and the FR-046 Provenance paragraph repeat the claim. Recommend a fourth label, `captured-shape` (a live shape with reconstructed values), with `captured` kept for unedited live bytes, and correct the refused note. No real session ids, tokens, usernames or home paths are in the fixtures. | crates/claude-wire/fixtures/claude-2.1.283/MANIFEST:4-22, README.md:407-408, spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md:28-36 |

## Verdict

The documents agree with each other and with the tests they cite. The inventory is
useful, but not yet "every" format, and two statements overclaim (FND-001, FND-002). The
provenance labels should distinguish live bytes from live shapes (FND-003). All three are
low and do not block merge.
