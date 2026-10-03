---
name: doctor
description: Diagnose ix-relay identity, startup registration, and message delivery. Repair an authorized Codex daemon-context failure using a temporary Herdr helper tab.
---

# Diagnose ix-relay

Run `agent-msg whoami --json` and `agent-msg doctor --json`. Report the verified
`server:workspace:tab` address, Redis connectivity, watcher health, and actual
delivery separately. Historical hook failures can keep doctor non-green after
a repair; preserve them. Never print credentials or rewrite `AGENT_MSG_NAME`.

If identity and delivery work, report that result and leave the session running.
Do not restart a healthy lead or duplicate a successfully queued message.

For a Codex lead with missing placement, stale inherited `HERDR_*`, or a live
process missing `--no-daemon`, follow
[Codex daemon recovery](references/codex-daemon-recovery.md). Include the case
where the user already restarted but an existing shell selected the underlying
NVM/npm executable instead of the installed launcher.

When the user requests this recovery, use a temporary Codex helper tab to resume
the original lead once it is idle, verify registration and receipt, then close
only the helper tab. A diagnostic-only invocation does not authorize a restart.
Never guess the target from focus, fabricate SessionStart, manually register an
inherited stale pane, restart the shared daemon, or interrupt other leads.

If the caller cannot prove its terminal, request the operator's confirmation of
the exact pane and session to resume before creating a restart handoff. Continue
read-only diagnosis while waiting. API/VS Code context alone is not proof.

For missing configuration, use the ix-relay setup guidance. For ambiguous tabs,
explain which genuine sessions share the tab; do not assign alias suffixes.
