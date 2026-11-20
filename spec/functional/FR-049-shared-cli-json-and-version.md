---
id: FR-049
title: "Shared CLI JSON encoding and version agreement"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
  - target: "ix://agent-ix/ix-cli-kit"
    type: "depends_on"
---
# FR-049: Shared CLI JSON encoding and version agreement

## Description

When `agent-msg` emits machine-readable results for `inbox --json`, `find
--json`, `whoami --json`, `doctor --json`, or `work show`, it shall encode the
existing typed result through ix-cli-kit's compact JSON encoder.

The CLI shall keep each command's existing JSON bytes and exit behavior.

The ix-relay test suite shall check the compiled `agent-msg` and
`relay-watcher` binaries with ix-cli-kit's version agreement utility against
their own package versions.

## Inputs

- The typed result each named command already produces.
- The package version of each binary and that built binary's `--version`
  output. The binary's `--help` result is checked separately.

## Outputs

- Each named command's existing compact JSON result on standard output.
- A test verdict for each compiled binary's version agreement.

## Behavior

- For `inbox --json`, `whoami --json`, `doctor --json`, and `work show`,
  `agent-msg` shall write one compact JSON value followed by exactly one
  newline. For `find --json`, it shall write one compact JSON object and one
  newline per matching row, and no JSON line when there are no rows.
- The named commands shall retain their current field names, field order,
  omitted or null fields, array order, string escaping, and empty-result
  shapes. They shall not sort object keys to create a canonical form.
- The named commands shall retain their current standard-error diagnostics,
  including rejected inbox entries and skipped `find` rows, and their
  existing command-specific exit statuses. A `doctor --json` output pipe that
  closes early shall retain the doctor's health verdict as its exit status.
- The version-agreement check shall compare each compiled binary's
  `--version` output with that binary's package version. The test suite shall
  check that `--help` succeeds without treating unrelated numbers in its
  usage text as a version. A command that cannot run or a reported version
  mismatch shall fail the check.
- `relay-watcher` shall keep its current package-version string in its
  heartbeat and OTLP service metadata.
- The CLI-kit adoption shall not change hook output encoding, hook failure
  spooling, stored or transport JSON, hook timing and zero-exit policy,
  configuration precedence, verified address identity, or message delivery.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-049-AC-1 | Given the same typed results, `inbox --json`, `whoami --json`, `doctor --json`, and `work show` emit exactly their existing compact JSON bytes followed by one newline; `find --json` emits exactly the existing bytes and one newline per row, or no JSON line for zero rows. Existing field order, null and omitted fields, array order, and escaping are unchanged. | Test |
| FR-049-AC-2 | Given a rejected inbox entry, a skipped `find` row, an unhealthy doctor report, and an output pipe closed during `doctor --json`, each command keeps its existing stderr and exit behavior; the hook still exits zero on failure and retains its output and spool behavior. | Test |
| FR-049-AC-3 | For each compiled `agent-msg` and `relay-watcher` binary, the ix-cli-kit agreement check compares the package manifest with `--version`; matching versions pass and a mismatch or command failure fails. A separate assertion checks that `--help` exits successfully without changing its output or interpreting its unrelated numbers as a version. | Test |
| FR-049-AC-4 | The watcher heartbeat and OTLP service metadata continue to report the binary package version, while the kit adoption changes no configuration precedence, verified address, queue, or delivery result. | Test |

## Dependencies

- [US-006](../usecase/US-006-see-when-the-relay-is-failing.md) owns the
  operator-facing visibility need.
- [FR-017](./FR-017-hook-never-fails-the-turn.md) and
  [NFR-001](../non-functional/NFR-001-hook-time-budget.md) govern hook exits
  and timing.
- [FR-026](./FR-026-configuration-precedence.md),
  [FR-029](./FR-029-doctor-reports-relay-health.md), and
  [FR-041](./FR-041-send-prints-the-recipients-state.md) retain their
  existing command and identity contracts.
