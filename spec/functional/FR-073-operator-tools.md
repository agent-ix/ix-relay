---
id: FR-073
title: "Operator tools: matrix recount, namespace archive and Codex session-start capture"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-006"
    type: "depends_on"
---
# FR-073: Operator tools: matrix recount, namespace archive and Codex session-start capture

## Description

Three binaries of the `relay-tools` crate serve the maintainers and operators of a relay and are
not part of the bus: `check-test-matrix` keeps the test matrix honest in the CI gate,
`archive-relay-namespace` takes the backlog of a prior key namespace off Redis before a
namespace change, and `capture-codex-session-start` records Codex `SessionStart` metadata for
diagnosing a stale terminal context. The launcher that keeps interactive Herdr Codex sessions in
their own terminal context is owned by [FR-006](./FR-006-registration-records-place.md).

## Behavior

- `check-test-matrix` SHALL recount `spec/tests.md`: the Test Case Summary rows by type against the Test Execution Summary totals, the row count GAP-001 states, duplicate test ids, and any Test Case row inside a traceability table.
- `check-test-matrix` SHALL print every problem, sorted, and exit 1 when there is one.
- `check-test-matrix` SHALL print the recount and exit 0 when there is none.
- `check-test-matrix` SHALL count a Test Case row after any Unicode line break as a row,.
- `check-test-matrix` SHALL report a total that is not an ASCII number as a problem.
- `archive-relay-namespace` SHALL write the keys of a named prior namespace (never the `ixr:v3:` namespace, never an empty prefix) to a new file with their binary-safe dumps and remaining TTLs,.
- `archive-relay-namespace` SHALL NOT write to, delete from or modify the source.
- `archive-relay-namespace` SHALL create the output exclusively with mode 0600 so an existing archive is never overwritten.
- `archive-relay-namespace` SHALL fail within 10 seconds of an unresponsive server.
- `archive-relay-namespace` SHALL resolve the Redis URL as `agent-msg` does (`IX_RELAY_URL`, else `url` of the file named by `IX_RELAY_CONFIG` or `~/.ix/relay/config.toml`, else the default).
- `archive-relay-namespace` SHALL validate the config file.
- `archive-relay-namespace` SHALL work with `HOME` unset when the URL comes from the environment.
- `archive-relay-namespace` SHALL refuse a `rediss://` URL with a message that names why: no relay build carries a TLS library, and the relay is reached over WireGuard.
- `capture-codex-session-start` SHALL, for a top-level `SessionStart` payload only, append one JSON line holding `hook_event_name`, `session_id`, `transcript_path` and the allowlisted Herdr and Codex thread environment, to `$CODEX_HOME/ix-relay/session-start.jsonl`.
- `capture-codex-session-start` SHALL write that file with mode 0600.
- `capture-codex-session-start` SHALL NOT follow a symlink or write to a file another user owns.
- `capture-codex-session-start` SHALL record nothing else from the payload.
- `capture-codex-session-start` SHALL skip subagent events.
- `capture-codex-session-start` SHALL NOT fail or delay the hooks that run beside it.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-073-AC-1 | Given a test matrix with a duplicate id, a wrong GAP-001 count, a category total that differs from its rows, a Test Case row in a traceability table (after any Unicode line break), a non-ASCII or oversized total or a missing type cell, `check-test-matrix` prints each problem and exits 1; given a consistent matrix it prints the recount and exits 0. | Test |
| FR-073-AC-2 | Given keys with binary names and values and a TTL under a prior prefix, `archive-relay-namespace` archives them with their TTL, leaves the source unchanged, creates the file with mode 0600, refuses to overwrite it, refuses the new namespace and a `rediss://` URL, and fails within the timeout against a server that never replies. | Test |
| FR-073-AC-3 | Given `IX_RELAY_CONFIG` naming a file with a `url`, and `IX_RELAY_URL` set or unset and `HOME` unset, `archive-relay-namespace` uses `IX_RELAY_URL` first, else the file's `url`, and refuses a malformed file before connecting. | Test |
| FR-073-AC-4 | Given a `SessionStart` payload, `capture-codex-session-start` appends one 0600 line holding only the allowlisted fields; given a subagent event, another event, a malformed payload or a symlinked capture file, it records nothing and does not fail. | Test |

## Dependencies

- **Related**: [FR-006](./FR-006-registration-records-place.md) (the launcher beside the capture hook),
  [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md) (no TLS library in the tree).
