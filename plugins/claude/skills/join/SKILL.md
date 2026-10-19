---
name: join
description: Diagnose relay connectivity and genuine session placement. Enrollment is automatic and uses server:workspace:tab.
---

# Verify automatic enrollment

Run `agent-msg whoami --json` and `agent-msg doctor`. Report the verified
three-part address and connectivity separately. `AGENT_MSG_NAME` is preserved
but does not override the address. Never invent a name or session ID.

The relay enrolls session-verified live Herdr leads at startup, on send, and
in watcher reconciliation. Manual registration is not a repair for stale
caller context. If a session is unplaced, verify its genuine SessionStart and
matching Herdr agent session. For Codex, the Herdr launcher must add
`--no-daemon`; Herdr's session hook must precede the relay hook. Do not restart
active work without authorization.

If the machine has no relay configuration, use the setup skill to configure
machine connectivity. Preserve existing credentials and session environment.
A healthy Redis connection alone does not prove a verified session location.
