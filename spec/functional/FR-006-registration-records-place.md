---
id: FR-006
title: "Genuine-session startup registration"
type: FR
---
# FR-006: Genuine-session startup registration

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- SessionStart shall register the genuine harness session only after live Herdr establishes its unique location; inherited pane claims and `AGENT_MSG_NAME` shall not choose identity.
- Cached hooks shall use only the opaque queue bound to their genuine payload session and shall not overwrite verified placement with an empty registration.
- Interactive Herdr Codex starts and resumes shall use `--no-daemon`, preserving existing environment and unrelated CLI commands.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-006-AC-1 | Genuine startup ignores stale pane variables and stale alias metadata. | Test |
| FR-006-AC-2 | Prompt, tool and stop hooks deliver through the genuine session queue; subagent hooks do not enroll or consume lead mail. | Test |
| FR-006-AC-3 | Launcher tests verify isolation for interactive starts/resumes and passthrough for CLI commands. | Test |
