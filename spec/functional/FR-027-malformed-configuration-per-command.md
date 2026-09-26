---
id: FR-027
title: "Malformed configuration: reported everywhere, fatal except in hooks"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-005"
    type: "implements"
---
# FR-027: Malformed configuration: reported everywhere, fatal except in hooks

## Description

If the configuration file is unreadable or malformed, then the configuration
loader shall report on stderr the file's path, and for a syntax error its line
and column, for every ix-relay command.

## Behavior

- If the configuration file is unreadable or malformed, then the hook
  handler shall report it and exit 0.
- If the configuration file is unreadable or malformed, then each other command
  (`agent-msg` subcommands and `relay-watcher`) shall exit 1 without contacting
  Redis.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-027-AC-1 | Given a file containing `url = ` on line 3, `agent-msg who` exits 1 and stderr names the file, line 3 and a column. | Test |
| FR-027-AC-2 | Given the same file, `agent-msg hook claude` for a lead in a herdr pane exits 0 with empty stdout and stderr naming the file. | Test |
| FR-027-AC-3 | Given the same file, `relay-watcher --once` exits 1 and stderr names the file. | Test |
| FR-027-AC-4 | Given the same file, no command opens a connection to the Redis URL. | Test |
| FR-027-AC-5 | Given no file at the configured path, every command runs with the defaults and reports no configuration error. | Test |

## Dependencies

- **Upstream**: [US-005](../usecase/US-005-configure-a-machine-once.md),
  [FR-026](./FR-026-configuration-precedence.md)
- Delivered by AGE-2017.
