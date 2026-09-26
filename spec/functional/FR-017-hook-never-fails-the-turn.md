---
id: FR-017
title: "The hook never fails the agent's turn"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-017: The hook never fails the agent's turn

## Description

When the harness runs `agent-msg hook claude` or `agent-msg hook codex`, the hook
handler shall exit with status 0 whatever fails, reporting any failure on stderr
only, and shall print on stdout either nothing or exactly one well-formed hook
output JSON object.

## Behavior

- If the command line cannot be parsed, then the hook handler shall report the
  error on stderr and exit 0.
- If the bus name given by `--as` or `AGENT_MSG_NAME` is not a valid bus name,
  then the hook handler shall report the error on stderr and exit 0.
- If the configuration file is malformed, then the hook handler shall report the
  file, line and column on stderr and exit 0.
- If stdin is not valid hook JSON, if Redis cannot be reached, if a Redis call
  fails or times out, if herdr fails, or if the time budget is exhausted, then the
  hook handler shall report the failure on stderr and exit 0.
- If the hook event is not one of `SessionStart`, `UserPromptSubmit`,
  `PostToolUse` or `Stop`, then the hook handler shall print nothing and exit 0.
- If no bus name is set and the process is not running in a herdr pane, then the
  hook handler shall print nothing, read no configuration file and exit 0.
- When the popped messages are empty, the hook handler shall print nothing, so a
  `Stop` hook never blocks the turn without a message to deliver.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-017-AC-1 | Every run of `agent-msg hook claude`, for any bytes on stdin and any hook event name, exits 0 with a stdout that is empty or one JSON object. | Test |
| FR-017-AC-2 | Given an unknown flag on the command line after `hook`, the process exits 0. | Test |
| FR-017-AC-3 | Given `AGENT_MSG_NAME=Bad Name`, a `Stop` hook exits 0 and prints nothing on stdout. | Test |
| FR-017-AC-4 | Given a config file containing `url = ` (a TOML syntax error), a hook in a herdr pane exits 0 and stderr names the file, line and column. | Test |
| FR-017-AC-5 | Given the Redis URL points at a closed port, each of the four hook events exits 0 with empty stdout. | Test |
| FR-017-AC-6 | Given no bus name and no herdr pane environment, a hook exits 0 without opening the configuration file, even when that file is unreadable. | Test |
| FR-017-AC-7 | Given a `Stop` event and an empty inbox, stdout is empty. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- **Downstream**: [NFR-001](../non-functional/NFR-001-hook-time-budget.md)
- Delivered by AGE-2010, AGE-2012 and AGE-2017.
